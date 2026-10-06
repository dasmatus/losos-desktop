// The boot image repacking a phone without init_boot gets (bootimg.js). The
// images here are laid out as AOSP's mkbootimg lays them out; the repacked
// ones were also compared byte for byte with mkbootimg's own output for
// header versions 0 to 4 when this was written.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { gzipSync, gunzipSync } from "node:zlib";
import { parseBootImage, withRamdisk, kernelVersion, unlz4Legacy, stackRamdisks } from "../static/flasher/bootimg.js";

const pad = (n, page) => Math.ceil(n / page) * page;

// A boot image with header `version`; parts holds kernel, ramdisk and, for
// v0 to v2, second, recovery_dtbo and dtb.
function bootImage(version, parts) {
  const page = version >= 3 ? 4096 : 2048;
  const header = Buffer.alloc(page);
  header.write("ANDROID!", 0);
  header.writeUInt32LE(parts.kernel.length, 8);
  header.writeUInt32LE(version, 40);
  let order;
  if (version >= 3) {
    header.writeUInt32LE(parts.ramdisk.length, 12);
    header.writeUInt32LE(version === 4 ? 1580 + 4 : 1580, 20);
    order = ["kernel", "ramdisk"];
  } else {
    header.writeUInt32LE(parts.ramdisk.length, 16);
    header.writeUInt32LE(parts.second.length, 24);
    header.writeUInt32LE(page, 36);
    order = ["kernel", "ramdisk", "second"];
    if (version >= 1) {
      order.push("recovery_dtbo");
      header.writeUInt32LE(parts.recovery_dtbo.length, 1632);
      const offset = page + order.slice(0, 3).reduce((n, name) => n + pad(parts[name].length, page), 0);
      header.writeBigUInt64LE(BigInt(offset), 1636);
    }
    if (version >= 2) {
      order.push("dtb");
      header.writeUInt32LE(parts.dtb.length, 1648);
    }
    header.set(sha1(order.map((name) => parts[name])), 576);
  }
  const body = order.map((name) => {
    const out = Buffer.alloc(pad(parts[name].length, page));
    parts[name].copy(out);
    return out;
  });
  return new Uint8Array(Buffer.concat([header, ...body]));
}

// mkbootimg's id: each section, then its length as 32 bits.
function sha1(sections) {
  const hash = createHash("sha1");
  for (const section of sections) {
    const length = Buffer.alloc(4);
    length.writeUInt32LE(section.length);
    hash.update(section).update(length);
  }
  return hash.digest();
}

const kernel = Buffer.concat([randomBytes(9000), Buffer.from("Linux version 5.15.104-android13-8 (build@host)\n")]);
const parts = {
  kernel,
  ramdisk: randomBytes(5001),
  second: randomBytes(300),
  recovery_dtbo: randomBytes(2100),
  dtb: randomBytes(777),
};

for (const version of [0, 1, 2, 3, 4]) {
  test(`header v${version}: another ramdisk, and everything else as it was`, async () => {
    const ramdisk = randomBytes(12345);
    const repacked = await withRamdisk(parseBootImage(bootImage(version, parts)), new Uint8Array(ramdisk));
    assert.ok(Buffer.from(repacked).equals(Buffer.from(bootImage(version, { ...parts, ramdisk }))));
    const again = parseBootImage(repacked);
    assert.ok(Buffer.from(again.parts.ramdisk).equals(ramdisk));
    assert.ok(Buffer.from(again.parts.kernel).equals(kernel));
  });
}

test("the kernel's version, raw and gzipped with device trees after it", async () => {
  assert.deepEqual(await kernelVersion(new Uint8Array(kernel)), [5, 15, 104]);
  const dtb = Buffer.concat([Buffer.from([0xd0, 0x0d, 0xfe, 0xed]), randomBytes(500)]);
  assert.deepEqual(await kernelVersion(new Uint8Array(Buffer.concat([gzipSync(kernel), dtb]))), [5, 15, 104]);
  assert.equal(await kernelVersion(new Uint8Array(randomBytes(1000))), null);
});

// `yes 'Linux version 6.1.75-android14-11 compressible text ' | head -c
// 20000 | lz4 -l -9`: the legacy frame, with long matches.
const LZ4 = Buffer.from(
  "AiFMGI4AAAD/JkxpbnV4IHZlcnNpb24gNi4xLjc1LWFuZHJvaWQxNC0xMSBjb21wcmVzc2libGUgdGV4dCAKNQD/////////////////////" +
    "//////////////////////////////////////////////////////////////////////////////////8hUDYuMS43",
  "base64",
);
const TEXT = "Linux version 6.1.75-android14-11 compressible text \n".repeat(400).slice(0, 20000);

test("legacy LZ4 decompresses as lz4 -d does", async () => {
  assert.equal(Buffer.from(unlz4Legacy(new Uint8Array(LZ4))).toString(), TEXT);
  assert.deepEqual(await kernelVersion(new Uint8Array(LZ4)), [6, 1, 75]);
});

test("the GSI's ramdisk follows the phone's at a 4-byte boundary, in gzip after gzip", async () => {
  const stock = gzipSync(randomBytes(1001));
  const stacked = Buffer.from(await stackRamdisks(new Uint8Array(stock), new Uint8Array(LZ4)));
  const at = pad(stock.length, 4);
  assert.ok(stacked.subarray(0, stock.length).equals(stock));
  assert.equal(gunzipSync(stacked.subarray(at)).toString(), TEXT);
  // After an LZ4 ramdisk the GSI's stays as it was built.
  const lz4Stock = Buffer.concat([LZ4, Buffer.from([1, 2])]);
  const kept = Buffer.from(await stackRamdisks(new Uint8Array(lz4Stock), new Uint8Array(LZ4)));
  assert.ok(kept.subarray(pad(lz4Stock.length, 4)).equals(LZ4));
});
