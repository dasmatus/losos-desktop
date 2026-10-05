// The proxy's I/O: everything src/lib.rs decides, carried out against GHCR.
//
// Kept apart from api/proxy.ts so the same code runs under Vercel's edge
// runtime and under `node --test` with a fake registry (test/handler.test.ts).
// Nothing here chooses what a path means; it asks the module.

const REGISTRY = "https://ghcr.io";
const MANIFEST = "application/vnd.oci.image.manifest.v1+json";
const TITLE = "org.opencontainers.image.title";

// The environment the handler reads: GHCR_REPOSITORY (owner/name, the
// namespace every artifact lives under) and optionally GHCR_TOKEN and
// GHCR_USERNAME.
export interface Env {
  GHCR_REPOSITORY?: string;
  GHCR_TOKEN?: string;
  GHCR_USERNAME?: string;
}

// What src/lib.rs exports. Each decision takes a string and returns one, both
// passed through linear memory; the result is a pointer and a length packed
// into one i64.
interface Exports {
  memory: WebAssembly.Memory;
  alloc(length: number): number;
  free(ptr: number, length: number): void;
  plan(ptr: number, length: number): bigint;
  pick(ptr: number, length: number): bigint;
  narinfo(ptr: number, length: number): bigint;
}

export interface Core {
  plan(path: string): string;
  pick(input: string): string;
  narinfo(input: string): string;
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;
export type Handler = (request: Request) => Promise<Response>;

// Wrap an instantiated losos_proxy.wasm in the three calls it exports.
export function bind(instance: WebAssembly.Instance): Core {
  const wasm = instance.exports as unknown as Exports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const call = (name: "plan" | "pick" | "narinfo", input: string): string => {
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
    plan: (path: string) => call("plan", path),
    pick: (input: string) => call("pick", input),
    narinfo: (input: string) => call("narinfo", input),
  };
}

// One entry of an OCI manifest's layer list.
interface Layer {
  digest: string;
  annotations?: Record<string, string>;
}

function text(status: number, body: BodyInit | null, cacheControl: string): Response {
  return new Response(body, {
    status,
    headers: { "content-type": "text/plain; charset=utf-8", "cache-control": cacheControl },
  });
}

// A pull token for one repository. Anonymous for a public package; with
// GHCR_TOKEN (a token with read:packages) for a private one. GHCR pairs a
// personal access token with its owner's login, so GHCR_USERNAME names it.
// `token` serves for a GitHub Actions token, and is the default.
async function token(fetchImpl: Fetch, repository: string, env: Env): Promise<string> {
  const url = `${REGISTRY}/token?service=ghcr.io&scope=repository:${repository}:pull`;
  const headers: Record<string, string> = {};
  if (env.GHCR_TOKEN) {
    const user = env.GHCR_USERNAME || "token";
    headers.authorization = `Basic ${btoa(`${user}:${env.GHCR_TOKEN}`)}`;
  }
  const response = await fetchImpl(url, { headers });
  if (!response.ok) throw new Error(`token: ${response.status}`);
  return ((await response.json()) as { token: string }).token;
}

// Build the request handler. `env` is read as described on Env.
export function handler(core: Core, env: Env, fetchImpl: Fetch = fetch): Handler {
  const namespace = (env.GHCR_REPOSITORY || "").toLowerCase();

  return async function handle(request: Request): Promise<Response> {
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
    const body = <T>(b: T): T | null => (head ? null : b);

    if (plan.startsWith("static ")) {
      const newline = plan.indexOf("\n");
      return new Response(body(plan.slice(newline + 1)), {
        headers: {
          "content-type": plan.slice("static ".length, newline),
          "cache-control": "public, max-age=86400",
        },
      });
    }
    if (plan.startsWith("flatpak-index ")) {
      return flatpakIndex(plan, url, fetchImpl, env, namespace, head);
    }
    if (plan.startsWith("registry ")) {
      return registryObject(plan, fetchImpl, env, namespace, head);
    }
    if (!plan.startsWith("layer ")) {
      return text(404, body(`${plan.slice("missing ".length)}\n`), "public, max-age=3600");
    }

    const [, suffix, tag, title, kind] = plan.split(" ");
    const repository = `${namespace}/${suffix}`;
    let bearer: string;
    try {
      bearer = await token(fetchImpl, repository, env);
    } catch (error) {
      return text(502, body(`${(error as Error).message}\n`), "no-store");
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
    const stored = (await manifest.json()) as { layers?: Layer[] };
    const layers = (stored.layers || [])
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

// The media types a Flatpak OCI image's manifest can come in: what
// `flatpak build-bundle --oci` writes, and what a copy to GHCR may turn it
// into.
const MANIFESTS = [MANIFEST, "application/vnd.docker.distribution.manifest.v2+json"].join(", ");

async function sha256(bytes: ArrayBuffer): Promise<string> {
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return `sha256:${Array.from(hash, (b) => b.toString(16).padStart(2, "0")).join("")}`;
}

// The index of a Flatpak OCI remote, which flatpak asks for as
// /flatpak/index/static?architecture=amd64&... and reads as a list of
// images, each with the labels `flatpak build-bundle --oci` put in its
// config (org.flatpak.ref, the metadata, and so on) and the digest to fetch
// it by from the registry the index names, which is this proxy again.
async function flatpakIndex(
  plan: string,
  url: URL,
  fetchImpl: Fetch,
  env: Env,
  namespace: string,
  head: boolean,
): Promise<Response> {
  const [, name, ...images] = plan.split(" ");
  const wanted = url.searchParams.get("architecture");
  const repository = `${namespace}/flatpak`;
  let auth: Record<string, string>;
  try {
    auth = { authorization: `Bearer ${await token(fetchImpl, repository, env)}` };
  } catch (error) {
    return text(502, head ? null : `${(error as Error).message}\n`, "no-store");
  }
  const found = [];
  for (const image of images) {
    const [tag, architecture] = image.split(":");
    if (wanted && wanted !== architecture) continue;
    const manifest = await fetchImpl(`${REGISTRY}/v2/${repository}/manifests/${tag}`, {
      headers: { ...auth, accept: MANIFESTS },
    });
    // An architecture CI has not pushed yet is simply not listed.
    if (manifest.status === 404) continue;
    if (!manifest.ok) return text(502, head ? null : `manifest: ${manifest.status}\n`, "no-store");
    // The digest of the bytes as served, which is what the registry
    // answers to, rather than any header's word for it.
    const bytes = await manifest.arrayBuffer();
    const digest = await sha256(bytes);
    const stored = JSON.parse(new TextDecoder().decode(bytes)) as {
      mediaType?: string;
      config?: { digest?: string };
    };
    const configDigest = stored.config?.digest || "";
    if (!/^sha256:[0-9a-f]{64}$/.test(configDigest)) {
      return text(502, head ? null : `${tag} has no config\n`, "no-store");
    }
    const config = await fetchImpl(`${REGISTRY}/v2/${repository}/blobs/${configDigest}`, {
      headers: auth,
    });
    if (!config.ok) return text(502, head ? null : `config: ${config.status}\n`, "no-store");
    const labels = ((await config.json()) as { config?: { Labels?: Record<string, string> } })
      .config?.Labels;
    if (!labels || !labels["org.flatpak.ref"]) continue;
    found.push({
      Tags: ["latest"],
      Digest: digest,
      MediaType: stored.mediaType || MANIFEST,
      OS: "linux",
      Architecture: architecture,
      Labels: labels,
    });
  }
  const index = { Registry: `${url.origin}/flatpak/`, Results: [{ Name: name, Images: found }] };
  return new Response(head ? null : JSON.stringify(index), {
    headers: { "content-type": "application/json", "cache-control": "public, max-age=60" },
  });
}

// A manifest or blob of one of the index's images, by digest: the
// registry half of the remote. A digest names its bytes, so either is
// cached for good; a blob is a redirect to GHCR's storage, as a NAR is.
async function registryObject(
  plan: string,
  fetchImpl: Fetch,
  env: Env,
  namespace: string,
  head: boolean,
): Promise<Response> {
  const [, suffix, what, digest] = plan.split(" ");
  const repository = `${namespace}/${suffix}`;
  let auth: Record<string, string>;
  try {
    auth = { authorization: `Bearer ${await token(fetchImpl, repository, env)}` };
  } catch (error) {
    return text(502, head ? null : `${(error as Error).message}\n`, "no-store");
  }
  const objectUrl = `${REGISTRY}/v2/${repository}/${what}/${digest}`;
  if (what === "manifests") {
    const manifest = await fetchImpl(objectUrl, { headers: { ...auth, accept: MANIFESTS } });
    if (!manifest.ok) {
      const status = manifest.status === 404 ? 404 : 502;
      return text(status, head ? null : `manifest: ${manifest.status}\n`, "no-store");
    }
    const bytes = await manifest.arrayBuffer();
    if ((await sha256(bytes)) !== digest) {
      return text(502, head ? null : "the manifest does not match its digest\n", "no-store");
    }
    return new Response(head ? null : bytes, {
      headers: {
        "content-type": manifest.headers.get("content-type") || MANIFEST,
        "docker-content-digest": digest,
        "cache-control": "public, max-age=31536000, immutable",
      },
    });
  }
  const blob = await fetchImpl(objectUrl, { headers: auth, redirect: "manual" });
  const location = blob.headers.get("location");
  if (blob.status >= 300 && blob.status < 400 && location) {
    return new Response(null, { status: 302, headers: { location, "cache-control": "no-store" } });
  }
  if (blob.ok) {
    return new Response(head ? null : blob.body, {
      headers: { "content-type": "application/octet-stream", "cache-control": "no-store" },
    });
  }
  return text(blob.status === 404 ? 404 : 502, head ? null : `blob: ${blob.status}\n`, "no-store");
}
