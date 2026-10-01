// The proxy's I/O: everything src/lib.rs decides, carried out against GHCR.
//
// Kept apart from api/proxy.js so the same code runs under Vercel's edge
// runtime and under `node --test` with a fake registry (test/handler.test.js).
// Nothing here chooses what a path means; it asks the module.

const REGISTRY = "https://ghcr.io";
const MANIFEST = "application/vnd.oci.image.manifest.v1+json";
const TITLE = "org.opencontainers.image.title";

// Wrap an instantiated losos_proxy.wasm in the three calls it exports.
export function bind(instance) {
  const wasm = instance.exports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const call = (name, input) => {
    const bytes = encoder.encode(input);
    const ptr = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    const packed = wasm[name](ptr, bytes.length);
    const outPtr = Number(packed >> 32n);
    const outLen = Number(packed & 0xffffffffn);
    const out = decoder.decode(new Uint8Array(wasm.memory.buffer, outPtr, outLen));
    wasm.free(outPtr, outLen);
    return out;
  };
  return {
    plan: (path) => call("plan", path),
    pick: (input) => call("pick", input),
    narinfo: (input) => call("narinfo", input),
  };
}

function text(status, body, cacheControl) {
  return new Response(body, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8", "cache-control": cacheControl },
  });
}

// A pull token for one repository. Anonymous for a public package; with
// GHCR_TOKEN (a token with read:packages) for a private one. GHCR pairs a
// personal access token with its owner's login, so GHCR_USERNAME names it.
// `token` serves for a GitHub Actions token, and is the default.
async function token(fetchImpl, repository, env) {
  const url = `${REGISTRY}/token?service=ghcr.io&scope=repository:${repository}:pull`;
  const headers = {};
  if (env.GHCR_TOKEN) {
    const user = env.GHCR_USERNAME || "token";
    headers.authorization = `Basic ${btoa(`${user}:${env.GHCR_TOKEN}`)}`;
  }
  const response = await fetchImpl(url, { headers });
  if (!response.ok) throw new Error(`token: ${response.status}`);
  return (await response.json()).token;
}

// Build the request handler. `env` carries GHCR_REPOSITORY (owner/name, the
// namespace every artifact lives under) and optionally GHCR_TOKEN and
// GHCR_USERNAME.
export function handler(core, env, fetchImpl = fetch) {
  const namespace = (env.GHCR_REPOSITORY || "").toLowerCase();

  return async function handle(request) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return text(405, "GET or HEAD\n", "no-store");
    }
    if (!/^[a-z0-9._-]+\/[a-z0-9._-]+$/.test(namespace)) {
      return text(500, "GHCR_REPOSITORY is not set to owner/name\n", "no-store");
    }

    const url = new URL(request.url);
    // vercel.json rewrites every path to this function and passes the
    // original one as ?path=, without its leading slash.
    const path = url.searchParams.has("path") ? `/${url.searchParams.get("path")}` : url.pathname;
    const plan = core.plan(path);
    const head = request.method === "HEAD";
    const body = (b) => (head ? null : b);

    if (plan.startsWith("static ")) {
      const newline = plan.indexOf("\n");
      return new Response(body(plan.slice(newline + 1)), {
        headers: {
          "content-type": plan.slice("static ".length, newline),
          "cache-control": "public, max-age=86400",
        },
      });
    }
    if (!plan.startsWith("layer ")) {
      return text(404, body(`${plan.slice("missing ".length)}\n`), "public, max-age=3600");
    }

    const [, suffix, tag, title, kind] = plan.split(" ");
    const repository = `${namespace}/${suffix}`;
    let bearer;
    try {
      bearer = await token(fetchImpl, repository, env);
    } catch (error) {
      return text(502, body(`${error.message}\n`), "no-store");
    }
    const auth = { authorization: `Bearer ${bearer}` };

    const manifest = await fetchImpl(`${REGISTRY}/v2/${repository}/manifests/${tag}`, {
      headers: { ...auth, accept: MANIFEST },
    });
    if (manifest.status === 404 || manifest.status === 401 || manifest.status === 403) {
      // A narinfo that is not there yet may be pushed by the next CI run, so
      // a miss is cached briefly; nix itself caches it for an hour anyway.
      return text(404, body("not in the cache\n"), "public, max-age=60");
    }
    if (!manifest.ok) {
      return text(502, body(`manifest: ${manifest.status}\n`), "no-store");
    }
    const layers = ((await manifest.json()).layers || [])
      .map((l) => `${l.digest}\t${(l.annotations || {})[TITLE] || ""}`)
      .join("\n");
    const digest = core.pick(`${title}\n${layers}`);
    if (!digest) {
      return text(404, body(`${tag} has no ${title}\n`), "public, max-age=60");
    }

    const blobUrl = `${REGISTRY}/v2/${repository}/blobs/${digest}`;
    if (kind === "redirect") {
      // GHCR answers a blob with a redirect to short-lived signed storage.
      // Handing that on means the bytes never pass through this function.
      const blob = await fetchImpl(blobUrl, { headers: auth, redirect: "manual" });
      const location = blob.headers.get("location");
      if (blob.status >= 300 && blob.status < 400 && location) {
        return new Response(null, {
          status: 302,
          headers: { location, "cache-control": "no-store" },
        });
      }
      if (blob.ok) {
        return new Response(body(blob.body), {
          headers: { "content-type": "application/octet-stream", "cache-control": "no-store" },
        });
      }
      return text(502, body(`blob: ${blob.status}\n`), "no-store");
    }

    const blob = await fetchImpl(blobUrl, { headers: auth });
    if (!blob.ok) {
      return text(502, body(`blob: ${blob.status}\n`), "no-store");
    }
    if (kind === "inline") {
      // Bytes, never text: SHA256SUMS.gpg is a binary OpenPGP signature, and
      // decoding it as UTF-8 would replace what is not valid UTF-8 and break
      // it. SHA256SUMS and its signature move with every release.
      const signature = title.endsWith(".gpg");
      return new Response(body(await blob.arrayBuffer()), {
        headers: {
          "content-type": signature ? "application/pgp-signature" : "text/plain; charset=utf-8",
          "cache-control": "public, max-age=60",
        },
      });
    }
    const content = await blob.text();
    if (kind === "narinfo") {
      const rewritten = core.narinfo(`${tag}\n${content}`);
      if (!rewritten) {
        return text(502, body("the stored narinfo is not one this cache wrote\n"), "no-store");
      }
      // Immutable: a store hash names exactly one narinfo.
      return new Response(body(rewritten), {
        headers: { "content-type": "text/x-nix-narinfo", "cache-control": "public, max-age=86400" },
      });
    }
    return text(502, body(`unknown kind ${kind}\n`), "no-store");
  };
}
