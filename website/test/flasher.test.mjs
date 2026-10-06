// The flasher's byte handling, run by node: the pieces a sparse image is cut
// into must write exactly what the image does, and the streaming SHA-256
// must agree with node's own. The USB side is exercised against a simulated
// bootloader at the end.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { splitSparse, RAW, FILL, DONT_CARE, CRC32 } from "../static/flasher/sparse.js";
import { Sha256 } from "../static/flasher/sha256.js";
import { Fastboot } from "../static/flasher/fastboot.js";

const BLOCK = 4096;

// A sparse image from a list of [type, blocks, data] chunks, as img2simg
// would write one.
function sparse(chunks, totalBlocks) {
  const parts = [];
  const header = Buffer.alloc(28);
  header.writeUInt32LE(0xed26ff3a, 0);
  header.writeUInt16LE(1, 4);
  header.writeUInt16LE(28, 8);
  header.writeUInt16LE(12, 10);
  header.writeUInt32LE(BLOCK, 12);
  header.writeUInt32LE(totalBlocks, 16);
  header.writeUInt32LE(chunks.length, 20);
  parts.push(header);
  for (const [type, blocks, data] of chunks) {
    const chunk = Buffer.alloc(12);
    chunk.writeUInt16LE(type, 0);
    chunk.writeUInt32LE(blocks, 4);
    chunk.writeUInt32LE(12 + (data ? data.length : 0), 8);
    parts.push(chunk);
    if (data) parts.push(data);
  }
  return Buffer.concat(parts);
}

// What a bootloader does with one sparse image: write its chunks in order
// over `disk`, leaving skipped blocks as they were.
function apply(disk, image) {
  const view = Buffer.from(image);
  assert.equal(view.readUInt32LE(0), 0xed26ff3a);
  const total = view.readUInt32LE(16);
  const count = view.readUInt32LE(20);
  let o = 28;
  let at = 0;
  for (let c = 0; c < count; c++) {
    const type = view.readUInt16LE(o);
    const blocks = view.readUInt32LE(o + 4);
    const bytes = view.readUInt32LE(o + 8);
    const data = view.subarray(o + 12, o + bytes);
    if (type === RAW) data.copy(disk, at * BLOCK);
    if (type === FILL) for (let b = 0; b < blocks * BLOCK; b += 4) data.copy(disk, at * BLOCK + b, 0, 4);
    at += blocks;
    o += bytes;
  }
  assert.equal(at, total, "a piece covers the whole partition");
  assert.equal(o, view.length);
}

function streamOf(buffer, size = 1000) {
  let at = 0;
  return new ReadableStream({
    pull(controller) {
      if (at >= buffer.length) return controller.close();
      controller.enqueue(new Uint8Array(buffer.subarray(at, at + size)));
      at += size;
    },
  });
}

async function pieces(image, limit) {
  const out = [];
  for await (const piece of splitSparse(streamOf(image), limit)) out.push(piece);
  return out;
}

const fill = Buffer.from([0xde, 0xad, 0xbe, 0xef]);
const raw1 = randomBytes(BLOCK * 37);
const raw2 = randomBytes(BLOCK * 5);
const CHUNKS = [
  [RAW, 37, raw1],
  [DONT_CARE, 100, null],
  [FILL, 12, fill],
  [CRC32, 0, Buffer.alloc(4)],
  [RAW, 5, raw2],
  [DONT_CARE, 46, null],
];
const TOTAL = 200;

function expected() {
  const disk = Buffer.alloc(TOTAL * BLOCK, 0x55);
  apply(disk, sparse(CHUNKS.filter(([type]) => type !== CRC32), TOTAL));
  return disk;
}

test("pieces of any size write what the whole image writes", async () => {
  const image = sparse(CHUNKS, TOTAL);
  for (const limit of [28 + 36 + BLOCK, 10 * BLOCK, 64 * 1024, 1 << 20]) {
    const disk = Buffer.alloc(TOTAL * BLOCK, 0x55);
    const out = await pieces(image, limit);
    for (const piece of out) {
      assert.ok(piece.length <= limit, `piece of ${piece.length} under ${limit}`);
      apply(disk, piece);
    }
    assert.ok(disk.equals(expected()), `limit ${limit}`);
  }
});

test("an image that fits is one piece", async () => {
  const out = await pieces(sparse(CHUNKS, TOTAL), 1 << 20);
  assert.equal(out.length, 1);
});

test("a truncated image is an error, not a short write", async () => {
  const image = sparse(CHUNKS, TOTAL);
  await assert.rejects(pieces(image.subarray(0, image.length - 100), 1 << 20), /ends in the middle/);
});

test("a limit smaller than one block is refused", async () => {
  await assert.rejects(pieces(sparse(CHUNKS, TOTAL), 1000), /cannot hold one block/);
});

test("the streaming SHA-256 matches node's, across odd chunk boundaries", () => {
  for (const size of [0, 1, 55, 56, 63, 64, 65, 1000, 100_003]) {
    const data = randomBytes(size);
    const hash = new Sha256();
    for (let at = 0; at < size; at += 7) hash.update(new Uint8Array(data.subarray(at, at + 7)));
    assert.equal(hash.hex(), createHash("sha256").update(data).digest("hex"), `${size} bytes`);
  }
});

// A bootloader on the other end of a WebUSB device: answers getvar, takes
// downloads and applies flashes to its partitions.
function bootloader(partitions) {
  const encoder = new TextEncoder();
  let answers = [];
  let expecting = 0;
  let buffer = [];
  const say = (text) => answers.push(encoder.encode(text));
  return {
    opened: true,
    configuration: {
      interfaces: [
        {
          interfaceNumber: 0,
          alternates: [
            {
              alternateSetting: 0,
              interfaceClass: 0xff,
              interfaceSubclass: 0x42,
              interfaceProtocol: 3,
              endpoints: [
                { type: "bulk", direction: "in", endpointNumber: 1 },
                { type: "bulk", direction: "out", endpointNumber: 1 },
              ],
            },
          ],
        },
      ],
    },
    async claimInterface() {},
    async close() {},
    async transferOut(_, data) {
      if (expecting > 0) {
        buffer.push(Buffer.from(data));
        expecting -= data.length;
        if (expecting === 0) say("OKAY");
        return { status: "ok" };
      }
      const command = new TextDecoder().decode(data);
      const [name, arg] = command.split(/:(.*)/s);
      if (name === "getvar" && arg === "max-download-size") say("OKAY0x00100000");
      else if (name === "getvar" && arg === "max-fetch-size") say("OKAY0x00010000");
      else if (name === "getvar" && arg.startsWith("partition-size:")) {
        const p = arg.slice("partition-size:".length);
        say(partitions[p] ? `OKAY0x${partitions[p].length.toString(16)}` : "FAILunknown partition");
      } else if (name === "getvar") say("FAILunknown variable");
      else if (name === "download") {
        expecting = parseInt(arg, 16);
        buffer = [];
        say(`DATA${arg}`);
      } else if (name === "fetch") {
        // fastbootd's upload: DATA with the size, the bytes, then OKAY.
        const [p, offset, size] = arg.split(":");
        const data = partitions[p].subarray(parseInt(offset, 16), parseInt(offset, 16) + parseInt(size, 16));
        say(`DATA${data.length.toString(16).padStart(8, "0")}`);
        for (let at = 0; at < data.length; at += 5000) answers.push(new Uint8Array(data.subarray(at, at + 5000)));
        say("OKAY");
      } else if (name === "flash") {
        const data = Buffer.concat(buffer);
        say("INFOwriting");
        if (data.readUInt32LE(0) === 0xed26ff3a) apply(partitions[arg], data);
        else data.copy(partitions[arg]);
        say("OKAY");
      } else say("FAILunknown command");
      return { status: "ok" };
    },
    async transferIn() {
      const next = answers.shift();
      return { data: new DataView(next.buffer, next.byteOffset, next.byteLength) };
    },
  };
}

test("a flash over the simulated bootloader writes every piece", async () => {
  const partitions = {
    userdata: Buffer.alloc(TOTAL * BLOCK, 0x55),
    init_boot_a: Buffer.alloc(64 * 1024),
  };
  const device = bootloader(partitions);
  const fastboot = new Fastboot(device);
  await fastboot.open();
  assert.equal(await fastboot.getvar("partition-size:init_boot_b"), null);
  assert.equal(await fastboot.maxDownload(), 1 << 20);
  const boot = randomBytes(5000);
  await fastboot.flash("init_boot_a", boot);
  assert.ok(partitions.init_boot_a.subarray(0, 5000).equals(boot));
  for await (const piece of splitSparse(streamOf(sparse(CHUNKS, TOTAL)), 64 * 1024)) {
    await fastboot.flash("userdata", piece);
  }
  assert.ok(partitions.userdata.equals(expected()));
});

test("fetch reads a partition back in max-fetch-size pieces", async () => {
  const boot = randomBytes(150_000);
  const fastboot = new Fastboot(bootloader({ boot_b: boot }));
  await fastboot.open();
  const read = await fastboot.fetch("boot_b");
  assert.ok(Buffer.from(read).equals(boot));
});
