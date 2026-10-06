// The fastboot protocol over WebUSB: the few commands a flash needs, and
// nothing a bootloader would not also take from the fastboot program.
//
// A command is one ASCII packet out; the answer is packets of a four-letter
// status and a message: INFO and TEXT while the bootloader works, then OKAY,
// FAIL, or DATA with the byte count it will accept for a download.

// The interface every fastboot implementation presents: vendor class,
// subclass 0x42, protocol 3.
export const FILTER = { classCode: 0xff, subclassCode: 0x42, protocolCode: 0x03 };

// Bulk transfers of this size keep the USB stack busy without a large
// buffer per call.
const TRANSFER = 1 << 20;

export class FastbootError extends Error {}

export class Fastboot {
  constructor(device, log = () => {}) {
    this.device = device;
    this.log = log;
  }

  // Ask the person to pick a device in fastboot mode.
  static async request(log) {
    const device = await navigator.usb.requestDevice({ filters: [FILTER] });
    const fastboot = new Fastboot(device, log);
    await fastboot.open();
    return fastboot;
  }

  async open() {
    const device = this.device;
    if (!device.opened) await device.open();
    if (device.configuration === null) await device.selectConfiguration(1);
    for (const iface of device.configuration.interfaces) {
      for (const alt of iface.alternates) {
        if (
          alt.interfaceClass === FILTER.classCode &&
          alt.interfaceSubclass === FILTER.subclassCode &&
          alt.interfaceProtocol === FILTER.protocolCode
        ) {
          await device.claimInterface(iface.interfaceNumber);
          if (iface.alternates.length > 1) {
            await device.selectAlternateInterface(iface.interfaceNumber, alt.alternateSetting);
          }
          for (const endpoint of alt.endpoints) {
            if (endpoint.type !== "bulk") continue;
            if (endpoint.direction === "in") this.in = endpoint.endpointNumber;
            else this.out = endpoint.endpointNumber;
          }
          if (this.in !== undefined && this.out !== undefined) return;
        }
      }
    }
    throw new FastbootError("this USB device has no fastboot interface");
  }

  async close() {
    try {
      await this.device.close();
    } catch {
      // Already gone, as after a reboot.
    }
  }

  async response() {
    for (;;) {
      const result = await this.device.transferIn(this.in, 256);
      const text = new TextDecoder().decode(result.data);
      const status = text.slice(0, 4);
      const message = text.slice(4);
      if (status === "INFO" || status === "TEXT") {
        this.log(message);
      } else if (status === "OKAY") {
        return message;
      } else if (status === "FAIL") {
        throw new FastbootError(message || "the bootloader refused");
      } else if (status === "DATA") {
        return parseInt(message, 16);
      } else {
        throw new FastbootError(`unexpected answer from the bootloader: ${text}`);
      }
    }
  }

  async command(command) {
    if (command.length > 4096) throw new FastbootError("command too long");
    await this.device.transferOut(this.out, new TextEncoder().encode(command));
    return this.response();
  }

  // A variable, or null when the bootloader does not know it.
  async getvar(name) {
    try {
      return await this.command(`getvar:${name}`);
    } catch (error) {
      if (error instanceof FastbootError) return null;
      throw error;
    }
  }

  async maxDownload() {
    const value = await this.getvar("max-download-size");
    // Hexadecimal, usually with 0x, which parseInt takes either way.
    const size = value ? parseInt(value, 16) : NaN;
    // Some bootloaders say nothing; 64 MiB is the size every one takes.
    return Number.isFinite(size) && size > 0 ? size : 64 << 20;
  }

  // Send `bytes` into the bootloader's download buffer.
  async download(bytes, onBytes = () => {}) {
    const hex = bytes.length.toString(16).padStart(8, "0");
    const accepted = await this.command(`download:${hex}`);
    if (accepted !== bytes.length) {
      throw new FastbootError(`the bootloader offered ${accepted} bytes for a ${bytes.length}-byte download`);
    }
    for (let at = 0; at < bytes.length; at += TRANSFER) {
      await this.device.transferOut(this.out, bytes.subarray(at, Math.min(at + TRANSFER, bytes.length)));
      onBytes(Math.min(at + TRANSFER, bytes.length));
    }
    await this.response();
  }

  // Read a partition back, as `fastboot fetch` does. Only fastbootd, the
  // fastboot inside Android's recovery, takes this, only for boot images,
  // and only when unlocked; it sends at most max-fetch-size at a time.
  async fetch(partition, onBytes = () => {}) {
    const size = parseInt(await this.getvar(`partition-size:${partition}`), 16);
    if (!Number.isFinite(size)) throw new FastbootError(`the phone has no ${partition} partition`);
    const limit = parseInt(await this.getvar("max-fetch-size"), 16) || size;
    const out = new Uint8Array(size);
    for (let offset = 0; offset < size; ) {
      const length = Math.min(limit, size - offset);
      const hex = (n) => `0x${n.toString(16).padStart(8, "0")}`;
      const accepted = await this.command(`fetch:${partition}:${hex(offset)}:${hex(length)}`);
      if (accepted !== length) {
        throw new FastbootError(`the phone offered ${accepted} bytes of ${partition} where ${length} were asked for`);
      }
      for (let got = 0; got < length; ) {
        const result = await this.device.transferIn(this.in, Math.min(TRANSFER, length - got));
        const chunk = new Uint8Array(result.data.buffer, result.data.byteOffset, result.data.byteLength);
        out.set(chunk, offset + got);
        got += chunk.length;
        onBytes(offset + got, size);
      }
      await this.response();
      offset += length;
    }
    return out;
  }

  async flash(partition, bytes, onBytes) {
    await this.download(bytes, onBytes);
    await this.command(`flash:${partition}`);
  }

  async reboot() {
    try {
      await this.command("reboot");
    } catch {
      // The device may drop off the bus before it answers.
    }
    await this.close();
  }
}
