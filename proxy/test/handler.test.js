// The proxy against a fake GHCR: the real module, the real handler, and a
// fetch that answers the way the registry does.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { bind, handler } from "../lib/handler.js";

const HASH = "0c0x1c0lyb5dh3dkw4mv1bmp8vd6gn8f";
const NAR = "1w1fff338fvdw53sqgamddn1b2xgds473pv6y13gizdbqjv4i5p3.nar.xz";
const NARINFO = `StorePath: /nix/store/${HASH}-hello\nURL: nar/${NAR}\nNarHash: sha256:x\nSig: k:s\n`;
const D1 = `sha256:${"1".repeat(64)}`;
const D2 = `sha256:${"2".repeat(64)}`;
const D3 = `sha256:${"3".repeat(64)}`;
const D4 = `sha256:${"4".repeat(64)}`;
// Not valid UTF-8 anywhere: the bytes a binary OpenPGP signature starts with.
const SIGNATURE = new Uint8Array([0x89, 0x02, 0x33, 0x04, 0x00, 0x01, 0x08, 0xff, 0xfe, 0x80]);
const layer = (digest, title) => ({
  digest,
  annotations: { "org.opencontainers.image.title": title },
});

const ARTIFACTS = {
  [`dasmatus/losos-desktop/nix-cache:${HASH}`]: [layer(D1, `${HASH}.narinfo`), layer(D2, NAR)],
  "dasmatus/losos-desktop/images:nightly-x86_64": [
    layer(D3, "SHA256SUMS"),
    layer(D4, "SHA256SUMS.gpg"),
    layer(D2, "losos-desktop_1_x86_64.efi"),
  ],
};
const BLOBS = { [D1]: NARINFO, [D3]: "abc  losos-desktop_1_x86_64.efi\n", [D4]: SIGNATURE };

function registry(log = []) {
  return async (url, init = {}) => {
    log.push({ url, init });
    const u = new URL(url);
    if (u.pathname === "/token") return Response.json({ token: "t" });
    const [, repo, what, ref] = u.pathname.match(/^\/v2\/(.+)\/(manifests|blobs)\/(.+)$/);
    assert.equal(init.headers.authorization, "Bearer t");
    if (what === "manifests") {
      const layers = ARTIFACTS[`${repo}:${ref}`];
      return layers ? Response.json({ layers }) : new Response("", { status: 404 });
    }
    if (init.redirect === "manual") {
      return new Response(null, { status: 307, headers: { location: `https://blob.example/${ref}` } });
    }
    return new Response(BLOBS[ref]);
  };
}

const core = bind(
  (await WebAssembly.instantiate(await readFile(new URL("../api/losos_proxy.wasm", import.meta.url))))
    .instance,
);
const env = { GHCR_REPOSITORY: "dasMatus/losos-desktop" };
const get = (h, path, method = "GET") =>
  h(new Request(`https://cache.example/api/proxy?path=${encodeURIComponent(path)}`, { method }));

test("nix-cache-info needs no registry", async () => {
  const log = [];
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

test("a crafted path never reaches the registry", async () => {
  const log = [];
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
  const log = [];
  const h = handler(core, { ...env, GHCR_TOKEN: "secret" }, registry(log));
  await get(h, `${HASH}.narinfo`);
  const sent = log.filter((l) => JSON.stringify(l.init).includes(btoa("token:secret")));
  assert.deepEqual(
    sent.map((l) => new URL(l.url).pathname),
    ["/token"],
  );
});

test("GHCR_USERNAME pairs with GHCR_TOKEN, as a personal access token needs", async () => {
  const log = [];
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
