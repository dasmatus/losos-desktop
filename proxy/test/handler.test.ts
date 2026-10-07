// The proxy against a fake GHCR: the real module, the real handler, and a
// fetch that answers the way the registry does.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { bind, handler, type Fetch, type Handler } from "../lib/handler.ts";

const HASH = "0c0x1c0lyb5dh3dkw4mv1bmp8vd6gn8f";
const NAR = "1w1fff338fvdw53sqgamddn1b2xgds473pv6y13gizdbqjv4i5p3.nar.xz";
const NARINFO = `StorePath: /nix/store/${HASH}-hello\nURL: nar/${NAR}\nNarHash: sha256:x\nSig: k:s\n`;
const D1 = `sha256:${"1".repeat(64)}`;
const D2 = `sha256:${"2".repeat(64)}`;
const D3 = `sha256:${"3".repeat(64)}`;
const D4 = `sha256:${"4".repeat(64)}`;
const D9 = `sha256:${"9".repeat(64)}`;
const GSI = "0123456789abcdef";
// Not valid UTF-8 anywhere: the bytes a binary OpenPGP signature starts with.
const SIGNATURE = new Uint8Array([0x89, 0x02, 0x33, 0x04, 0x00, 0x01, 0x08, 0xff, 0xfe, 0x80]);
const layer = (digest: string, title: string) => ({
  digest,
  annotations: { "org.opencontainers.image.title": title },
});

const ARTIFACTS: Record<string, ReturnType<typeof layer>[]> = {
  [`dasmatus/losos-desktop/nix-cache:${HASH}`]: [layer(D1, `${HASH}.narinfo`), layer(D2, NAR)],
  "dasmatus/losos-desktop/images:nightly-x86_64": [
    layer(D3, "SHA256SUMS"),
    layer(D4, "SHA256SUMS.gpg"),
    layer(D2, "losos-desktop_1_x86_64.efi"),
  ],
  "dasmatus/losos-desktop/images:nightly-aarch64": [
    layer(D3, "SHA256SUMS"),
    layer(D2, "losos-desktop_1_aarch64.efi"),
    layer(D9, "losos-desktop_1_gsi-userdata.simg.gz"),
  ],
  "dasmatus/losos-desktop/losos-ccache:aarch64": [layer(D2, "ccache.tar.zst.part-00")],
};
const BLOBS: Record<string, string | Uint8Array> = {
  [D1]: NARINFO,
  [D3]: "abc  losos-desktop_1_x86_64.efi\n",
  [D4]: SIGNATURE,
  [D9]: GSI,
};

interface Logged {
  url: string;
  init: RequestInit;
}

function registry(log: Logged[] = []): Fetch {
  return async (url, init = {}) => {
    log.push({ url, init });
    const u = new URL(url);
    if (u.pathname === "/token") return Response.json({ token: "t" });
    const [, repo, what, ref] = u.pathname.match(/^\/v2\/(.+)\/(manifests|blobs)\/(.+)$/)!;
    assert.equal((init.headers as Record<string, string>).authorization, "Bearer t");
    if (what === "manifests") {
      const layers = ARTIFACTS[`${repo}:${ref}`];
      return layers ? Response.json({ layers }) : new Response("", { status: 404 });
    }
    if (init.redirect === "manual") {
      return new Response(null, { status: 307, headers: { location: `https://blob.example/${ref}` } });
    }
    // Storage answers a range as a 206, which is what the flasher reads.
    const range = (init.headers as Record<string, string>).range?.match(/^bytes=(\d+)-(\d+)$/);
    const whole = BLOBS[ref];
    if (range && typeof whole === "string") {
      const [start, end] = [Number(range[1]), Number(range[2])];
      return new Response(whole.slice(start, end + 1), {
        status: 206,
        headers: { "content-range": `bytes ${start}-${end}/${whole.length}`, "content-length": String(end - start + 1) },
      });
    }
    return new Response(whole as BodyInit);
  };
}

const core = bind(
  (await WebAssembly.instantiate(await readFile(new URL("../api/losos_proxy.wasm", import.meta.url))))
    .instance,
);
const env = { GHCR_REPOSITORY: "dasMatus/losos-desktop" };
const get = (h: Handler, path: string, method = "GET", headers: Record<string, string> = {}) =>
  h(new Request(`https://cache.example/api/proxy?path=${encodeURIComponent(path)}`, { method, headers }));

test("nix-cache-info needs no registry", async () => {
  const log: Logged[] = [];
  const response = await get(handler(core, env, registry(log)), "nix-cache-info");
  assert.equal(response.status, 200);
  assert.match(await response.text(), /^StoreDir: \/nix\/store\n/);
  assert.equal(log.length, 0);
});

test("a narinfo comes back with its URL pointing at the proxy", async () => {
  const response = await get(handler(core, env, registry()), `${HASH}.narinfo`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "text/x-nix-narinfo");
  assert.equal(await response.text(), NARINFO.replace("URL: nar/", `URL: nar/${HASH}/`));
});

test("a NAR is a redirect to the registry's storage, never streamed", async () => {
  const response = await get(handler(core, env, registry()), `nar/${HASH}/${NAR}`);
  assert.equal(response.status, 302);
  assert.equal(response.headers.get("location"), `https://blob.example/${D2}`);
});

test("a store path not in the cache is a 404 nix can fall back on", async () => {
  const response = await get(handler(core, env, registry()), `${"a".repeat(32)}.narinfo`);
  assert.equal(response.status, 404);
});

test("sysupdate's SHA256SUMS is served inline, its images redirected", async () => {
  const h = handler(core, env, registry());
  const sums = await get(h, "updates/nightly/x86_64/SHA256SUMS");
  assert.equal(sums.status, 200);
  assert.equal(await sums.text(), BLOBS[D3]);
  const efi = await get(h, "updates/nightly/x86_64/losos-desktop_1_x86_64.efi");
  assert.equal(efi.status, 302);
});

test("SHA256SUMS.gpg comes back byte for byte", async () => {
  const response = await get(handler(core, env, registry()), "updates/nightly/x86_64/SHA256SUMS.gpg");
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("content-type"), "application/pgp-signature");
  assert.deepEqual(new Uint8Array(await response.arrayBuffer()), SIGNATURE);
});

test("the flasher gets a GSI file one range at a time, readable from any origin", async () => {
  const log: Logged[] = [];
  const h = handler(core, env, registry(log));
  const part = await get(h, "flasher/nightly/aarch64/losos-desktop_1_gsi-userdata.simg.gz", "GET", {
    range: "bytes=4-9",
  });
  assert.equal(part.status, 206);
  assert.equal(await part.text(), "456789");
  assert.equal(part.headers.get("content-range"), "bytes 4-9/16");
  assert.equal(part.headers.get("access-control-allow-origin"), "*");
  assert.match(part.headers.get("access-control-expose-headers") || "", /content-range/);
  // Streamed, so the proxy followed GHCR's redirect itself.
  assert.notEqual(log.at(-1)?.init.redirect, "manual");
  const sums = await get(h, "flasher/nightly/aarch64/SHA256SUMS");
  assert.equal(sums.headers.get("access-control-allow-origin"), "*");
  // The PC's files are sysupdate's, and stay under /updates/.
  assert.equal((await get(h, "flasher/nightly/aarch64/losos-desktop_1_aarch64.efi")).status, 404);
});

test("a browser's preflight is answered without the registry", async () => {
  const log: Logged[] = [];
  const response = await get(handler(core, env, registry(log)), "flasher/nightly/aarch64/SHA256SUMS", "OPTIONS");
  assert.equal(response.status, 204);
  assert.equal(response.headers.get("access-control-allow-headers"), "range");
  assert.equal(log.length, 0);
});

test("a ping does not invite other pages to read its answer", async () => {
  const response = await handler(core, env, registry())(
    new Request("https://cache.example/api/proxy?path=ping", { method: "POST", body: "x" }),
  );
  assert.equal(response.headers.get("access-control-allow-origin"), null);
});

test("a build cache part is redirected, and the part after the last is a 404", async () => {
  const h = handler(core, env, registry());
  const part = await get(h, "build-cache/losos/aarch64/ccache.tar.zst.part-00");
  assert.equal(part.status, 302);
  assert.equal(part.headers.get("location"), `https://blob.example/${D2}`);
  const after = await get(h, "build-cache/losos/aarch64/ccache.tar.zst.part-01");
  assert.equal(after.status, 404);
});

test("a crafted path never reaches the registry", async () => {
  const log: Logged[] = [];
  const h = handler(core, env, registry(log));
  for (const path of ["v2/_catalog", "../x.narinfo", `nar/${HASH}/../../x`, "updates/a/b/c"]) {
    assert.equal((await get(h, path)).status, 404, path);
  }
  assert.equal(log.length, 0);
});

test("HEAD answers without a body", async () => {
  const response = await get(handler(core, env, registry()), `${HASH}.narinfo`, "HEAD");
  assert.equal(response.status, 200);
  assert.equal(await response.text(), "");
});

test("GHCR_TOKEN is only ever sent to the token endpoint", async () => {
  const log: Logged[] = [];
  const h = handler(core, { ...env, GHCR_TOKEN: "secret" }, registry(log));
  await get(h, `${HASH}.narinfo`);
  const sent = log.filter((l) => JSON.stringify(l.init).includes(btoa("token:secret")));
  assert.deepEqual(
    sent.map((l) => new URL(l.url).pathname),
    ["/token"],
  );
});

test("GHCR_USERNAME pairs with GHCR_TOKEN, as a personal access token needs", async () => {
  const log: Logged[] = [];
  const h = handler(core, { ...env, GHCR_TOKEN: "secret", GHCR_USERNAME: "someone" }, registry(log));
  await get(h, `${HASH}.narinfo`);
  const sent = log.filter((l) => JSON.stringify(l.init).includes(btoa("someone:secret")));
  assert.deepEqual(
    sent.map((l) => new URL(l.url).pathname),
    ["/token"],
  );
});

test("an unset namespace is a configuration error, not a guess", async () => {
  const response = await get(handler(core, {}, registry()), "nix-cache-info");
  assert.equal(response.status, 500);
});

const ID = "0123456789abcdef".repeat(4);
const post = (h: Handler, body: string, country = "SK") =>
  h(
    new Request("https://cache.example/api/proxy?path=ping", {
      method: "POST",
      body,
      headers: country ? { "x-vercel-ip-country": country } : {},
    }),
  );

// A Redis REST endpoint that keeps each HyperLogLog as a plain set.
function redis(sets: Map<string, Set<string>>, log: Logged[] = []): Fetch {
  return async (url, init = {}) => {
    log.push({ url, init });
    assert.equal(url, "https://kv.example/pipeline");
    assert.equal((init.headers as Record<string, string>).authorization, "Bearer k");
    const commands = JSON.parse(init.body as string) as string[][];
    return Response.json(
      commands.map(([command, key, value]) => {
        const set = sets.get(key) ?? new Set();
        sets.set(key, set);
        if (command === "PFADD") return { result: set.has(value) ? 0 : (set.add(value), 1) };
        if (command === "PFCOUNT") return { result: set.size };
        return { result: 1 };
      }),
    );
  };
}
const kv = { ...env, KV_REST_API_URL: "https://kv.example/", KV_REST_API_TOKEN: "k" };
const ON = (region: string) => ({ region, choice_screens: { browser: true, search: true } });
const OFF = (region: string) => ({ region, choice_screens: { browser: false, search: false } });

test("a ping with no store is answered, not counted", async () => {
  const log: Logged[] = [];
  const response = await post(handler(core, env, registry(log)), `id=${ID}\narch=x86_64\n`);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), OFF("eu"));
  assert.equal(log.length, 0);
});

test("EU users are counted once each and turn the screens on in the EEA at the threshold", async () => {
  const sets = new Map<string, Set<string>>();
  const h = handler(core, { ...kv, CHOICE_SCREENS_AT: "2" }, redis(sets));
  const month = new Date().toISOString().slice(0, 7);
  assert.deepEqual(await (await post(h, `id=${ID}\narch=x86_64`, "DE")).json(), OFF("eu"));
  assert.deepEqual(await (await post(h, `id=${ID}\narch=x86_64`, "DE")).json(), OFF("eu"));
  // Outside the EU: counted in the total, not towards the threshold.
  assert.deepEqual(await (await post(h, `id=${"e".repeat(64)}\narch=x86_64`, "US")).json(), OFF("other"));
  assert.deepEqual(await (await post(h, `id=${"f".repeat(64)}\narch=aarch64`, "FR")).json(), ON("eu"));
  assert.deepEqual(await (await post(h, `id=${"d".repeat(64)}\narch=aarch64`, "NO")).json(), ON("eea"));
  assert.deepEqual(await (await post(h, `id=${"c".repeat(64)}\narch=aarch64`, "CH")).json(), OFF("other"));
  assert.equal(sets.get(`actives:${month}`)!.size, 5);
  assert.equal(sets.get(`actives:${month}:eu`)!.size, 2);
  assert.equal(sets.get(`actives:${month}:aarch64`)!.size, 3);
});

test("daily EU pings past their own threshold turn the screens on too", async () => {
  const sets = new Map<string, Set<string>>();
  const h = handler(core, { ...kv, CHOICE_SCREENS_DAILY_AT: "2" }, redis(sets));
  const day = new Date().toISOString().slice(0, 10);
  assert.deepEqual(await (await post(h, `id=${ID}\narch=x86_64`, "IT")).json(), OFF("eu"));
  assert.deepEqual(await (await post(h, `id=${ID}\narch=x86_64`, "IT")).json(), OFF("eu"));
  assert.deepEqual(await (await post(h, `id=${"f".repeat(64)}\narch=x86_64`, "PL")).json(), ON("eu"));
  assert.equal(sets.get(`actives:${day}:eu`)!.size, 2);
});

test("the screens can be forced either way", async () => {
  const forced = handler(core, { ...env, CHOICE_SCREENS: "on" }, registry());
  assert.deepEqual(await (await post(forced, `id=${ID}\narch=aarch64`, "")).json(), ON("other"));
  const sets = new Map<string, Set<string>>();
  const off = handler(core, { ...kv, CHOICE_SCREENS: "off", CHOICE_SCREENS_AT: "1" }, redis(sets));
  assert.deepEqual(await (await post(off, `id=${ID}\narch=aarch64`)).json(), OFF("eu"));
});

test("a malformed ping is refused before anything is stored", async () => {
  const log: Logged[] = [];
  const h = handler(core, kv, redis(new Map(), log));
  for (const body of [`id=${ID}`, `id=${ID.toUpperCase()}\narch=x86_64`, `id=x\narch=x86_64`, "x".repeat(600)]) {
    assert.ok([400, 413].includes((await post(h, body)).status), body);
  }
  assert.equal((await get(h, "ping")).status, 405);
  assert.equal(log.length, 0);
});

test("a store that is down still gets the client its policy", async () => {
  const down: Fetch = async () => {
    throw new Error("down");
  };
  const response = await post(handler(core, kv, down), `id=${ID}\narch=x86_64`);
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), OFF("eu"));
});
