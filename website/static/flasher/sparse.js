// Android sparse images, cut into pieces a bootloader can take one at a time.
//
// fastboot downloads into a buffer of the size the bootloader reports as
// max-download-size, and userdata's image is far larger. A sparse image is a
// header and a run of chunks (raw blocks, a fill pattern, or blocks to skip),
// and the bootloader writes each chunk at the block where the previous one
// ended. So a piece is a sparse image of the whole partition's size whose
// chunks before and after its own share are "skip": flashed in order, the
// pieces write the image. This is what fastboot's own resparsing does, done
// here as the image streams in, so the whole image is never in memory.

const MAGIC = 0xed26ff3a;
const FILE_HEADER = 28;
const CHUNK_HEADER = 12;
export const RAW = 0xcac1;
export const FILL = 0xcac2;
export const DONT_CARE = 0xcac3;
export const CRC32 = 0xcac4;

// Reads exact byte counts from a ReadableStream of Uint8Arrays.
class ByteReader {
  constructor(stream) {
    this.reader = stream.getReader();
    this.pending = new Uint8Array(0);
  }

  async read(n) {
    const out = new Uint8Array(n);
    let have = 0;
    while (have < n) {
      if (this.pending.length === 0) {
        const { value, done } = await this.reader.read();
        if (done) throw new Error("the sparse image ends in the middle of a chunk");
        this.pending = value;
        continue;
      }
      const take = Math.min(n - have, this.pending.length);
      out.set(this.pending.subarray(0, take), have);
      this.pending = this.pending.subarray(take);
      have += take;
    }
    return out;
  }
}

function chunkHeader(type, blocks, totalBytes) {
  const header = new Uint8Array(CHUNK_HEADER);
  const view = new DataView(header.buffer);
  view.setUint16(0, type, true);
  view.setUint32(4, blocks, true);
  view.setUint32(8, totalBytes, true);
  return header;
}

// Yields sparse images of at most `limit` bytes that, flashed in order,
// write what `stream` (one sparse image) describes. `onBlocks` is told how
// many of the image's blocks have been read, for a progress bar.
export async function* splitSparse(stream, limit, onBlocks = () => {}) {
  const input = new ByteReader(stream);
  const header = await input.read(FILE_HEADER);
  const view = new DataView(header.buffer);
  if (view.getUint32(0, true) !== MAGIC) throw new Error("not an Android sparse image");
  const fileHeaderSize = view.getUint16(8, true);
  const chunkHeaderSize = view.getUint16(10, true);
  const blockSize = view.getUint32(12, true);
  const totalBlocks = view.getUint32(16, true);
  const totalChunks = view.getUint32(20, true);
  if (fileHeaderSize > FILE_HEADER) await input.read(fileHeaderSize - FILE_HEADER);
  // Room each piece keeps for its header and the two skips around it.
  const overhead = FILE_HEADER + 2 * CHUNK_HEADER;
  if (limit < overhead + CHUNK_HEADER + blockSize) {
    throw new Error(`a download limit of ${limit} bytes cannot hold one block`);
  }

  let start = 0; // first block of the piece being built
  let at = 0; // block the next chunk writes
  let parts = [];
  let size = overhead;

  const piece = () => {
    const chunks = [];
    if (start > 0) chunks.push(chunkHeader(DONT_CARE, start, CHUNK_HEADER));
    chunks.push(...parts.map((p) => p.header));
    const count = chunks.length + (at < totalBlocks ? 1 : 0);
    const out = new Uint8Array(size - (start > 0 ? 0 : CHUNK_HEADER) - (at < totalBlocks ? 0 : CHUNK_HEADER));
    const head = new DataView(out.buffer);
    head.setUint32(0, MAGIC, true);
    head.setUint16(4, 1, true);
    head.setUint16(6, 0, true);
    head.setUint16(8, FILE_HEADER, true);
    head.setUint16(10, CHUNK_HEADER, true);
    head.setUint32(12, blockSize, true);
    head.setUint32(16, totalBlocks, true);
    head.setUint32(20, count, true);
    let o = FILE_HEADER;
    if (start > 0) {
      out.set(chunks[0], o);
      o += CHUNK_HEADER;
    }
    for (const p of parts) {
      out.set(p.header, o);
      o += CHUNK_HEADER;
      if (p.data) {
        out.set(p.data, o);
        o += p.data.length;
      }
    }
    if (at < totalBlocks) out.set(chunkHeader(DONT_CARE, totalBlocks - at, CHUNK_HEADER), o);
    return out;
  };
  const flush = function* () {
    if (parts.length > 0) yield piece();
    start = at;
    parts = [];
    size = overhead;
  };
  const add = function* (header, data, blocks) {
    const bytes = CHUNK_HEADER + (data ? data.length : 0);
    if (size + bytes > limit) yield* flush();
    parts.push({ header, data });
    size += bytes;
    at += blocks;
  };

  for (let c = 0; c < totalChunks; c++) {
    const raw = await input.read(chunkHeaderSize);
    const chunk = new DataView(raw.buffer);
    const type = chunk.getUint16(0, true);
    const blocks = chunk.getUint32(4, true);
    const bytes = chunk.getUint32(8, true) - chunkHeaderSize;
    if (type === RAW) {
      // Raw blocks are cut wherever a piece fills up.
      let left = blocks;
      while (left > 0) {
        let fit = Math.floor((limit - size - CHUNK_HEADER) / blockSize);
        if (fit < 1) {
          yield* flush();
          fit = Math.floor((limit - size - CHUNK_HEADER) / blockSize);
        }
        const take = Math.min(fit, left);
        const data = await input.read(take * blockSize);
        yield* add(chunkHeader(RAW, take, CHUNK_HEADER + data.length), data, take);
        left -= take;
        onBlocks(at, totalBlocks);
      }
    } else if (type === FILL) {
      const pattern = await input.read(4);
      yield* add(chunkHeader(FILL, blocks, CHUNK_HEADER + 4), pattern, blocks);
    } else if (type === DONT_CARE) {
      yield* add(chunkHeader(DONT_CARE, blocks, CHUNK_HEADER), null, blocks);
    } else if (type === CRC32) {
      // A checksum of what came before, which the pieces cannot carry over.
      await input.read(bytes);
    } else {
      throw new Error(`unknown sparse chunk type 0x${type.toString(16)}`);
    }
  }
  if (at !== totalBlocks) throw new Error(`the image's chunks cover ${at} of its ${totalBlocks} blocks`);
  yield* flush();
  onBlocks(totalBlocks, totalBlocks);
}
