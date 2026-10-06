// The web flasher: puts the GSI (nixos/halium/) on a phone from a browser,
// over WebUSB, as GrapheneOS's web installer does for Pixels.
//
// It writes three partitions and touches nothing else:
//
//   vbmeta     verification off, so the bootloader takes the next one
//   init_boot  the generic ramdisk: NixOS's systemd initrd
//   userdata   the OS's root filesystem, as sparse pieces
//
// A phone without init_boot, which launched before Android 13, gets its own
// boot image back in place of init_boot, with the same ramdisk added after
// its own (bootimg.js). Either way the phone keeps its own kernel and vendor
// partitions, which is why one image fits every Treble phone with a kernel
// systemd runs on (docs/web-flasher.md). The files come from the newest
// release through the proxy's /flasher/ path, checked against that
// release's SHA256SUMS, or from files the person already has.

import { Fastboot, FastbootError } from "./fastboot.js";
import { splitSparse } from "./sparse.js";
import { Sha256 } from "./sha256.js";
import { parseBootImage, withRamdisk, kernelVersion, stackRamdisks } from "./bootimg.js";

const ARCH = "aarch64";
// What each partition is written from, by the end of its file name in a
// release (nixos/halium/boot.nix, gsiImages).
const FILES = {
  vbmeta: "_gsi-vbmeta.img",
  init_boot: "_gsi-init_boot.img",
  userdata: "_gsi-userdata.simg.gz",
};
// One request per range keeps each well inside the proxy's time limit.
const RANGE = 32 << 20;
// The largest piece sent at once, whatever the bootloader would take: a
// piece is held in memory while it goes over USB.
const PIECE = 256 << 20;

// systemd's hard floor (its README, "REQUIREMENTS"): below it, the initrd
// does not start.
const KERNEL = [5, 10];

const $ = (id) => document.getElementById(id);
// stockBoot is the phone's own boot image, parsed, on a phone without
// init_boot; flashed says the install finished.
const state = { fastboot: null, device: null, files: null, version: null, stockBoot: null, flashed: false };

function log(message) {
  const line = document.createElement("div");
  line.textContent = message;
  $("log").append(line);
  line.scrollIntoView({ block: "nearest" });
}

function progress(id, done, total, label) {
  const bar = $(id);
  bar.hidden = false;
  bar.max = total;
  bar.value = done;
  if (label) bar.title = label;
}

function step(name, status, detail = "") {
  const section = document.querySelector(`[data-step="${name}"]`);
  section.dataset.status = status;
  section.querySelector(".detail").textContent = detail;
}

function enable(...ids) {
  for (const button of document.querySelectorAll("button[data-needs]")) {
    button.disabled = !ids.includes(button.id);
  }
}

// The buttons that make sense now, from what is known about the phone and
// the files.
function refresh() {
  const { device, files } = state;
  const ids = ["connect"];
  if (device && device.unlocked !== "yes") ids.push("unlock");
  if (device && device.unlocked === "yes") {
    ids.push("download", "pick");
    if (device.mode === "boot") ids.push("read-boot", "pick-boot");
    if (files && (device.mode === "init_boot" || state.stockBoot)) ids.push("flash");
  }
  if (state.flashed) ids.push("reboot");
  enable(...ids);
}

function megabytes(bytes) {
  return bytes < 1e6 ? `${Math.ceil(bytes / 1e3)} KB` : `${(bytes / 1e6).toFixed(0)} MB`;
}

async function guard(name, action) {
  try {
    await action();
  } catch (error) {
    step(name, "failed", error.message);
    log(`${name}: ${error.message}`);
    if (!(error instanceof FastbootError)) console.error(error);
    // Whatever failed, what can be tried again stays clickable.
    refresh();
  }
}

async function config() {
  try {
    const response = await fetch("config.json", { cache: "no-store" });
    if (response.ok) return await response.json();
  } catch {
    // Served without one, as from `npm start` in website/.
  }
  return {};
}

// --- The phone -------------------------------------------------------------

async function connect() {
  if (state.fastboot) await state.fastboot.close();
  const fastboot = await Fastboot.request(log);
  const vars = {};
  for (const name of ["product", "unlocked", "current-slot", "is-userspace"]) {
    vars[name] = await fastboot.getvar(name);
  }
  // A/B devices name the slot; the partition is init_boot_a or _b then.
  const slot = vars["current-slot"] ? `_${vars["current-slot"].replace(/^_/, "")}` : "";
  const has = async (partition) => (await fastboot.getvar(`partition-size:${partition}${slot}`)) !== null;
  const initBoot = await has("init_boot");
  if (!initBoot && !(await has("boot"))) {
    step("connect", "failed", `${vars.product || "This phone"} reports neither init_boot nor boot.`);
    return;
  }
  state.fastboot = fastboot;
  state.device = {
    ...vars,
    slot,
    // Where the GSI's ramdisk goes: init_boot on its own, or boot with
    // the phone's kernel.
    mode: initBoot ? "init_boot" : "boot",
    vbmeta: await has("vbmeta"),
    maxDownload: await fastboot.maxDownload(),
  };
  state.flashed = false;
  log(`Connected to ${vars.product || "a device"}, slot ${slot || "none"}, unlocked: ${vars.unlocked}`);
  // The boot image read or chosen before belongs to that phone only.
  if (state.stockBoot && state.stockBoot.product !== vars.product) state.stockBoot = null;

  step("connect", "done", `${vars.product || "Phone"} in fastboot mode, slot ${slot.slice(1) || "-"}.`);
  $("boot-section").hidden = state.device.mode !== "boot";
  if (vars.unlocked === "yes") {
    step("unlock", "done", "The bootloader is unlocked.");
  } else {
    step("unlock", "ready", "");
  }
  refresh();
}

async function unlock() {
  step("unlock", "working", "Confirm on the phone with the volume and power keys.");
  // The phone asks on its own screen, then erases itself and may restart
  // into fastboot, which drops this connection.
  await state.fastboot.command("flashing unlock");
  step("unlock", "working", "Unlocked. If the phone restarted, connect it again.");
  enable("connect");
}

// --- The phone's own boot image, on a phone without init_boot -------------

// Keep `bytes` as the phone's boot image once its kernel is one the OS
// runs on.
async function useStockBoot(bytes, source) {
  const image = parseBootImage(bytes);
  const version = await kernelVersion(image.parts.kernel);
  if (version && (version[0] < KERNEL[0] || (version[0] === KERNEL[0] && version[1] < KERNEL[1]))) {
    throw new Error(
      `The phone's kernel is Linux ${version.join(".")}. LosOS needs ${KERNEL.join(".")} or newer, ` +
        "which phones that launched with Android 12 or later have.",
    );
  }
  state.stockBoot = { image, product: state.device.product };
  const kernel = version ? `Linux ${version.join(".")}` : "a kernel whose version could not be read";
  if (!version) log("The kernel's version could not be read; if it is older than 5.10, LosOS will not start.");
  step("boot", "done", `${source}: boot image v${image.version} with ${kernel}.`);
  refresh();
}

// fastbootd reads boot back; the bootloader does not, so from there the
// phone restarts into fastbootd first.
async function readBoot() {
  const { fastboot, device } = state;
  if (device["is-userspace"] !== "yes") {
    step("boot", "working", "Restarting into fastbootd, which can read boot. Connect the phone again when it shows fastbootd.");
    await fastboot.command("reboot-fastboot").catch(() => {});
    await fastboot.close();
    state.fastboot = null;
    enable("connect");
    return;
  }
  step("boot", "working", `Reading boot${device.slot} from the phone`);
  const bytes = await fastboot.fetch(`boot${device.slot}`, (done, total) => progress("boot-progress", done, total));
  await useStockBoot(bytes, `Read from boot${device.slot}`);
}

async function pickBoot(event) {
  const [file] = event.target.files;
  if (!file) return;
  await useStockBoot(new Uint8Array(await file.arrayBuffer()), file.name);
}

// --- The files -------------------------------------------------------------

async function storage() {
  try {
    return await navigator.storage.getDirectory();
  } catch {
    return null;
  }
}

// Fetch `url` a range at a time into the browser's private file storage
// (or memory, where there is none), hashing as it goes.
async function fetchFile(url, name, onBytes) {
  const root = await storage();
  const handle = root && (await root.getFileHandle(name, { create: true }));
  const writable = handle && (await handle.createWritable());
  const parts = [];
  const hash = new Sha256();
  let total = Infinity;
  for (let at = 0; at < total; ) {
    const response = await fetch(url, { headers: { range: `bytes=${at}-${at + RANGE - 1}` } });
    if (!response.ok) throw new Error(`${name}: ${response.status} from the proxy`);
    const range = response.headers.get("content-range");
    total = range ? Number(range.split("/")[1]) : Number(response.headers.get("content-length"));
    const reader = response.body.getReader();
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      hash.update(value);
      if (writable) await writable.write(value);
      else parts.push(value);
      at += value.length;
      onBytes(at, total);
    }
    // A server that ignored the range sent the whole file at once.
    if (response.status === 200) break;
  }
  if (writable) await writable.close();
  return { blob: handle ? await handle.getFile() : new Blob(parts), sha256: hash.hex() };
}

async function hashBlob(blob, onBytes) {
  const hash = new Sha256();
  const reader = blob.stream().getReader();
  let at = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return hash.hex();
    hash.update(value);
    at += value.length;
    onBytes(at, blob.size);
  }
}

function parseSums(text) {
  const sums = {};
  for (const line of text.split("\n")) {
    const match = line.match(/^([0-9a-f]{64}) [ *](.+)$/);
    if (match) sums[match[2]] = match[1];
  }
  return sums;
}

async function download() {
  const { proxy, channel = "nightly" } = await config();
  if (!proxy) throw new Error("This copy of the flasher was built without the proxy's address; use files on this computer.");
  const base = `${proxy.replace(/\/$/, "")}/flasher/${channel}/${ARCH}`;
  step("download", "working", "Reading the newest release.");
  const response = await fetch(`${base}/SHA256SUMS`, { cache: "no-store" });
  if (!response.ok) throw new Error(`SHA256SUMS: ${response.status} from the proxy`);
  const sums = parseSums(await response.text());
  const names = {};
  for (const [partition, suffix] of Object.entries(FILES)) {
    names[partition] = Object.keys(sums).find((name) => name.endsWith(suffix));
    if (!names[partition]) throw new Error(`The ${channel} release has no ${suffix} yet.`);
  }
  state.version = names.userdata.split("_")[1];

  // Drop what an earlier visit downloaded and this release does not use.
  const root = await storage();
  if (root) {
    for await (const name of root.keys()) {
      if (!Object.values(names).includes(name)) await root.removeEntry(name);
    }
  }

  const files = {};
  for (const [partition, name] of Object.entries(names)) {
    step("download", "working", `Downloading ${name}`);
    const file = await fetchFile(`${base}/${name}`, name, (done, total) =>
      progress("download-progress", done, total, `${megabytes(done)} of ${megabytes(total)}`),
    );
    if (file.sha256 !== sums[name]) {
      throw new Error(`${name} does not match the release's SHA256SUMS; download again.`);
    }
    files[partition] = file.blob;
    log(`${name}: ${megabytes(file.blob.size)}, SHA-256 matches`);
  }
  state.files = files;
  step("download", "done", `LosOS ${state.version}, checked against the release's SHA256SUMS.`);
  refresh();
}

// Files the person built or downloaded themselves: matched by name, and
// checked against a SHA256SUMS when one is among them.
async function pick(event) {
  const chosen = Array.from(event.target.files);
  const files = {};
  for (const [partition, suffix] of Object.entries(FILES)) {
    files[partition] = chosen.find((file) => file.name.endsWith(suffix));
    if (!files[partition]) throw new Error(`Choose the ${suffix} file too.`);
  }
  const sumsFile = chosen.find((file) => file.name === "SHA256SUMS");
  if (sumsFile) {
    const sums = parseSums(await sumsFile.text());
    for (const file of Object.values(files)) {
      step("download", "working", `Checking ${file.name}`);
      const sha256 = await hashBlob(file, (done, total) => progress("download-progress", done, total));
      if (sha256 !== sums[file.name]) throw new Error(`${file.name} does not match SHA256SUMS.`);
    }
  }
  state.files = files;
  state.version = files.userdata.name.split("_")[1];
  step("download", "done", sumsFile ? "Checked against SHA256SUMS." : "Not checked: no SHA256SUMS was chosen.");
  refresh();
}

// --- Writing ---------------------------------------------------------------

async function flash() {
  const { fastboot, device, files } = state;
  enable();
  const writes = [];
  // A phone with no vbmeta partition verifies nothing to turn off.
  if (device.vbmeta) writes.push(["vbmeta", new Uint8Array(await files.vbmeta.arrayBuffer())]);
  const initBoot = new Uint8Array(await files.init_boot.arrayBuffer());
  if (device.mode === "init_boot") {
    writes.push(["init_boot", initBoot]);
  } else {
    const stock = state.stockBoot.image;
    const ramdisk = await stackRamdisks(stock.parts.ramdisk, parseBootImage(initBoot).parts.ramdisk);
    const boot = await withRamdisk(stock, ramdisk);
    const size = parseInt(await fastboot.getvar(`partition-size:boot${device.slot}`), 16);
    if (boot.length > size) {
      throw new Error(`The new boot image is ${megabytes(boot.length)}, larger than the phone's boot partition.`);
    }
    writes.push(["boot", boot]);
  }
  for (const [partition, bytes] of writes) {
    step("flash", "working", `Writing ${partition}`);
    await fastboot.flash(`${partition}${device.slot}`, bytes, (done) =>
      progress("flash-progress", done, bytes.length),
    );
    log(`${partition}${device.slot} written`);
  }

  // userdata has no slots. Each piece is a sparse image of the whole
  // partition, so the bootloader writes them one after another.
  const limit = Math.min(device.maxDownload, PIECE);
  const image = files.userdata.stream().pipeThrough(new DecompressionStream("gzip"));
  let n = 0;
  for await (const piece of splitSparse(image, limit, (done, total) => progress("flash-progress", done, total))) {
    n += 1;
    step("flash", "working", `Writing userdata, piece ${n} (${megabytes(piece.length)})`);
    await fastboot.flash("userdata", piece);
  }
  log(`userdata written in ${n} pieces`);
  step("flash", "done", `LosOS ${state.version} is on the phone.`);
  state.flashed = true;
  enable("reboot");
}

async function reboot() {
  await state.fastboot.reboot();
  state.fastboot = null;
  state.flashed = false;
  step("reboot", "done", "The phone is starting LosOS. The first boot grows the system to fill userdata.");
  enable("connect");
}

// --- Page ------------------------------------------------------------------

function init() {
  if (!("usb" in navigator)) {
    $("unsupported").hidden = false;
    enable();
    return;
  }
  $("connect").onclick = () => guard("connect", connect);
  $("unlock").onclick = () => guard("unlock", unlock);
  $("download").onclick = () => guard("download", download);
  $("pick").onclick = () => $("pick-files").click();
  $("pick-files").onchange = (event) => guard("download", () => pick(event));
  $("read-boot").onclick = () => guard("boot", readBoot);
  $("pick-boot").onclick = () => $("pick-boot-file").click();
  $("pick-boot-file").onchange = (event) => guard("boot", () => pickBoot(event));
  $("flash").onclick = () => guard("flash", flash);
  $("reboot").onclick = () => guard("reboot", reboot);
  // A phone that restarts into fastboot after unlocking comes back here.
  navigator.usb.addEventListener("disconnect", (event) => {
    if (state.fastboot && event.device === state.fastboot.device) {
      state.fastboot = null;
      log("The phone disconnected.");
      enable("connect");
    }
  });
  enable("connect");
}

init();
