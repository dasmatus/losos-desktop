// Android boot images: read one, and write it back with another ramdisk.
//
// A Treble phone that launched before Android 13 has no init_boot, and its
// boot partition holds the kernel and the ramdisk together. To boot the
// GSI there, the flasher keeps everything of the phone's own boot image
// (kernel, device tree, command line, load addresses) and puts the GSI's
// ramdisk after the phone's own, as the bootloader puts init_boot's after
// vendor_boot's on a newer phone: the kernel unpacks both, and this OS's
// /init wins. The layout is AOSP's mkbootimg.py, header versions 0 to 4.

const MAGIC = "ANDROID!";

const pad = (n, page) => Math.ceil(n / page) * page;

export function parseBootImage(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (new TextDecoder().decode(bytes.subarray(0, 8)) !== MAGIC) {
    throw new Error("not an Android boot image");
  }
  const version = view.getUint32(40, true);
  const sections = [];
  let page;
  const header = { version };
  if (version >= 3) {
    page = 4096;
    header.kernelSize = view.getUint32(8, true);
    header.ramdiskSize = view.getUint32(12, true);
    sections.push(["kernel", header.kernelSize], ["ramdisk", header.ramdiskSize]);
    if (version === 4) sections.push(["signature", view.getUint32(1580, true)]);
  } else {
    page = view.getUint32(36, true);
    header.kernelSize = view.getUint32(8, true);
    header.ramdiskSize = view.getUint32(16, true);
    sections.push(["kernel", header.kernelSize], ["ramdisk", header.ramdiskSize], ["second", view.getUint32(24, true)]);
    if (version >= 1) sections.push(["recovery_dtbo", view.getUint32(1632, true)]);
    if (version >= 2) sections.push(["dtb", view.getUint32(1648, true)]);
  }
  const parts = {};
  let at = page;
  for (const [name, size] of sections) {
    parts[name] = bytes.subarray(at, at + size);
    at += pad(size, page);
  }
  if (at > bytes.length + page) throw new Error("the boot image is shorter than its header says");
  return { header: bytes.subarray(0, page), version, page, parts };
}

function le32(view, offset, value) {
  view.setUint32(offset, value, true);
}

// The boot image again, with `ramdisk` in place of its own. Header v4's
// boot signature is dropped: it signs the old contents, and the vbmeta the
// flasher writes turns verification off.
export async function withRamdisk(image, ramdisk) {
  const { version, page, parts } = image;
  const header = new Uint8Array(image.header);
  const view = new DataView(header.buffer);
  const order =
    version >= 3
      ? ["kernel", "ramdisk"]
      : ["kernel", "ramdisk", "second", ...(version >= 1 ? ["recovery_dtbo"] : []), ...(version >= 2 ? ["dtb"] : [])];
  const contents = { ...parts, ramdisk };

  if (version >= 3) {
    le32(view, 12, ramdisk.length);
    if (version === 4) le32(view, 1580, 0);
  } else {
    le32(view, 16, ramdisk.length);
    if (version >= 1) {
      // recovery_dtbo_offset is absolute, so it moves with the ramdisk.
      const offset = page + order.slice(0, 3).reduce((n, name) => n + pad(contents[name].length, page), 0);
      view.setBigUint64(1636, BigInt(contents.recovery_dtbo.length ? offset : 0), true);
    }
    // The id is a SHA-1 of each section and its length, which some
    // bootloaders check.
    const hashed = [];
    for (const name of order) {
      const length = new Uint8Array(4);
      new DataView(length.buffer).setUint32(0, contents[name].length, true);
      hashed.push(contents[name], length);
    }
    const sha1 = new Uint8Array(await crypto.subtle.digest("SHA-1", await new Blob(hashed).arrayBuffer()));
    header.fill(0, 576, 608);
    header.set(sha1, 576);
  }

  const total = page + order.reduce((n, name) => n + pad(contents[name].length, page), 0);
  const out = new Uint8Array(total);
  out.set(header);
  let at = page;
  for (const name of order) {
    out.set(contents[name], at);
    at += pad(contents[name].length, page);
  }
  return out;
}

// The kernel's own version, from the "Linux version x.y.z" string in its
// image, which may be raw, gzip or legacy LZ4 compressed. Null when it
// cannot be found.
export async function kernelVersion(kernel) {
  let image = kernel;
  if (kernel[0] === 0x1f && kernel[1] === 0x8b) {
    image = await gunzipPrefix(kernel);
  } else if (isLz4Legacy(kernel)) {
    image = unlz4Legacy(kernel);
  }
  const text = new TextDecoder("latin1").decode(image);
  const match = text.match(/Linux version (\d+)\.(\d+)\.(\d+)/);
  return match ? match.slice(1).map(Number) : null;
}

// A gzip kernel may be followed by its device trees (Image.gz-dtb), which a
// decompressor rejects as trailing garbage, dropping what it had not yet
// handed out. So on failure, try again ending where a device tree's magic
// starts.
async function gunzipPrefix(bytes) {
  try {
    return await gunzip(bytes);
  } catch {
    for (let at = bytes.indexOf(0xd0); at > 0; at = bytes.indexOf(0xd0, at + 1)) {
      if (bytes[at + 1] !== 0x0d || bytes[at + 2] !== 0xfe || bytes[at + 3] !== 0xed) continue;
      try {
        return await gunzip(bytes.subarray(0, at));
      } catch {
        // The magic's bytes inside the compressed stream; the next one.
      }
    }
    return new Uint8Array();
  }
}

async function gunzip(bytes) {
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

const LZ4_LEGACY = 0x184c2102;

export function isLz4Legacy(bytes) {
  return bytes.length >= 4 && new DataView(bytes.buffer, bytes.byteOffset).getUint32(0, true) === LZ4_LEGACY;
}

// LZ4's legacy frame, as the kernel and Android's ramdisks use it: the
// magic, then blocks of a 32-bit compressed size and an LZ4 block, each
// expanding to at most 8 MiB. Another magic starts the next frame.
export function unlz4Legacy(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const out = [];
  let at = 0;
  while (at + 4 <= bytes.length) {
    const size = view.getUint32(at, true);
    at += 4;
    if (size === LZ4_LEGACY) continue;
    if (size === 0 || at + size > bytes.length) break;
    out.push(lz4Block(bytes.subarray(at, at + size)));
    at += size;
  }
  const total = out.reduce((n, b) => n + b.length, 0);
  const result = new Uint8Array(total);
  let o = 0;
  for (const block of out) {
    result.set(block, o);
    o += block.length;
  }
  return result;
}

function lz4Block(src) {
  let out = new Uint8Array(Math.max(src.length * 4, 1 << 16));
  let o = 0;
  let i = 0;
  const grow = (need) => {
    if (o + need <= out.length) return;
    const bigger = new Uint8Array(Math.max(out.length * 2, o + need));
    bigger.set(out.subarray(0, o));
    out = bigger;
  };
  while (i < src.length) {
    const token = src[i++];
    let literals = token >> 4;
    if (literals === 15) {
      let b;
      do {
        b = src[i++];
        literals += b;
      } while (b === 255);
    }
    grow(literals);
    out.set(src.subarray(i, i + literals), o);
    o += literals;
    i += literals;
    if (i >= src.length) break;
    const offset = src[i] | (src[i + 1] << 8);
    i += 2;
    let length = token & 15;
    if (length === 15) {
      let b;
      do {
        b = src[i++];
        length += b;
      } while (b === 255);
    }
    length += 4;
    grow(length);
    for (let k = 0; k < length; k++, o++) out[o] = out[o - offset];
  }
  return out.subarray(0, o);
}

// The phone's ramdisk and the GSI's, one after the other. Each stays a
// whole compressed archive, which the kernel unpacks in turn; the GSI's
// is recompressed with gzip when the phone's is gzip, since that is the
// one compression every kernel that booted the phone is sure to have.
export async function stackRamdisks(stock, gsi) {
  let ours = gsi;
  if (stock[0] === 0x1f && stock[1] === 0x8b && isLz4Legacy(gsi)) {
    const cpio = unlz4Legacy(gsi);
    ours = new Uint8Array(
      await new Response(new Blob([cpio]).stream().pipeThrough(new CompressionStream("gzip"))).arrayBuffer(),
    );
  }
  // The kernel looks for the next archive at a 4-byte boundary.
  const first = pad(stock.length, 4);
  const out = new Uint8Array(first + ours.length);
  out.set(stock);
  out.set(ours, first);
  return out;
}
