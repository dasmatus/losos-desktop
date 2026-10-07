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
// GHCR_USERNAME. For /ping, a Redis REST endpoint and its token, under the
// names Vercel's Upstash integration sets (KV_* for stores made as Vercel
// KV); without one, pings are answered but not counted. CHOICE_SCREENS
// (on, off or auto), CHOICE_SCREENS_AT (monthly active EU users) and
// CHOICE_SCREENS_DAILY_AT (daily EU pings, by distinct user) are read by
// src/lib.rs's `policy`.
export interface Env {
  GHCR_REPOSITORY?: string;
  GHCR_TOKEN?: string;
  GHCR_USERNAME?: string;
  KV_REST_API_URL?: string;
  KV_REST_API_TOKEN?: string;
  UPSTASH_REDIS_REST_URL?: string;
  UPSTASH_REDIS_REST_TOKEN?: string;
  CHOICE_SCREENS?: string;
  CHOICE_SCREENS_AT?: string;
  CHOICE_SCREENS_DAILY_AT?: string;
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
  ping(ptr: number, length: number): bigint;
  policy(ptr: number, length: number): bigint;
}

export interface Core {
  plan(path: string): string;
  pick(input: string): string;
  narinfo(input: string): string;
  ping(input: string): string;
  policy(input: string): string;
}

type Call = "plan" | "pick" | "narinfo" | "ping" | "policy";

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;
export type Handler = (request: Request) => Promise<Response>;

// Wrap an instantiated losos_proxy.wasm in the three calls it exports.
export function bind(instance: WebAssembly.Instance): Core {
  const wasm = instance.exports as unknown as Exports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const call = (name: Call, input: string): string => {
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
    ping: (input: string) => call("ping", input),
    policy: (input: string) => call("policy", input),
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

// A ping body is two short lines; anything much longer is not one.
const PING_MAX = 512;

// One /ping: count the id, then answer with the policy. Counting is
// best-effort: a store that is down or missing still gets the client its
// policy, from whatever count there is. No address, header or timestamp of
// the request is stored; Redis holds only the month's HyperLogLog, from
// which no id can be read back.
async function ping(core: Core, env: Env, fetchImpl: Fetch, request: Request): Promise<Response> {
  const json = (status: number, body: string) =>
    new Response(body, {
      status,
      headers: { "content-type": "application/json", "cache-control": "no-store" },
    });
  const body = await request.text();
  if (body.length > PING_MAX) return json(413, '{"error":"too long"}');
  const date = new Date().toISOString().slice(0, 10);
  // Vercel's edge network names the request's country; only the code goes
  // on, to decide whether the EU's rules apply. The address does not.
  const country = request.headers.get("x-vercel-ip-country") || "";
  const verdict = core.ping(`${date}\n${country}\n${body}`);
  if (!verdict.startsWith("count ")) {
    return json(400, JSON.stringify({ error: verdict.slice("bad ".length) }));
  }
  const [, id, region, adds, reads] = verdict.split(" ");

  const url = env.KV_REST_API_URL || env.UPSTASH_REDIS_REST_URL;
  const token = env.KV_REST_API_TOKEN || env.UPSTASH_REDIS_REST_TOKEN;
  let [monthly, daily] = ["", ""];
  if (url && token) {
    const add = adds.split(",").map((pair) => pair.split("="));
    try {
      const response = await fetchImpl(`${url.replace(/\/$/, "")}/pipeline`, {
        method: "POST",
        headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
        body: JSON.stringify([
          ...add.map(([key]) => ["PFADD", key, id]),
          ...add.map(([key, keep]) => ["EXPIRE", key, keep]),
          ...reads.split(",").map((key) => ["PFCOUNT", key]),
        ]),
      });
      if (response.ok) {
        const results = (await response.json()) as { result?: number }[];
        const [month, lastMonth, today, yesterday] = results.slice(-4).map((r) => Number(r?.result) || 0);
        // The month and the day that just started have barely been
        // counted, so each size is the larger of it and the one before.
        monthly = String(Math.max(month, lastMonth));
        daily = String(Math.max(today, yesterday));
      }
    } catch {
      // Counted next time; the policy below still goes out.
    }
  }
  const settings = [env.CHOICE_SCREENS || "auto", region, env.CHOICE_SCREENS_AT || "", env.CHOICE_SCREENS_DAILY_AT || ""];
  return json(200, core.policy([...settings, monthly, daily].join("\n")));
}

// Everything the proxy serves to a GET is public, so any page may read it,
// the web flasher above all, which lives on another origin. No cookie or
// credential is involved, which is what makes `*` safe; /ping, the one
// thing that is not a read, does not say this.
const CORS: Record<string, string> = {
  "access-control-allow-origin": "*",
  "access-control-expose-headers": "content-length, content-range, accept-ranges",
};

function withCors(response: Response): Response {
  const headers = new Headers(response.headers);
  for (const [name, value] of Object.entries(CORS)) headers.set(name, value);
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

// Build the request handler. `env` is read as described on Env.
export function handler(core: Core, env: Env, fetchImpl: Fetch = fetch): Handler {
  const handle = reads(core, env, fetchImpl);
  return async function cors(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const path = url.searchParams.has("path") ? `/${url.searchParams.get("path")}` : url.pathname;
    if (path === "/ping") return handle(request);
    // A browser asks first before a request with a Range it does not
    // consider simple.
    if (request.method === "OPTIONS") {
      return new Response(null, {
        status: 204,
        headers: {
          ...CORS,
          "access-control-allow-methods": "GET, HEAD",
          "access-control-allow-headers": "range",
          "access-control-max-age": "86400",
        },
      });
    }
    return withCors(await handle(request));
  };
}

function reads(core: Core, env: Env, fetchImpl: Fetch): Handler {
  const namespace = (env.GHCR_REPOSITORY || "").toLowerCase();

  return async function handle(request: Request): Promise<Response> {
    const url = new URL(request.url);
    // vercel.json rewrites every path to this function and passes the
    // original one as ?path=, without its leading slash.
    const path = url.searchParams.has("path") ? `/${url.searchParams.get("path")}` : url.pathname;
    if (path === "/ping") {
      if (request.method !== "POST") return text(405, "POST\n", "no-store");
      return ping(core, env, fetchImpl, request);
    }
    if (request.method !== "GET" && request.method !== "HEAD") {
      return text(405, "GET or HEAD\n", "no-store");
    }
    if (!/^[a-z0-9._-]+\/[a-z0-9._-]+$/.test(namespace)) {
      return text(500, "GHCR_REPOSITORY is not set to owner/name\n", "no-store");
    }

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

    if (kind === "stream") {
      // The range the flasher asked for, fetched through GHCR's redirect to
      // its storage (fetch drops the bearer token when it leaves ghcr.io)
      // and passed on as it arrives. A file name carries its version, so a
      // range of it never changes.
      const range = request.headers.get("range");
      const blob = await fetchImpl(blobUrl, { headers: range ? { ...auth, range } : auth });
      if (!blob.ok) {
        return text(blob.status === 416 ? 416 : 502, body(`blob: ${blob.status}\n`), "no-store");
      }
      const headers: Record<string, string> = {
        "content-type": "application/octet-stream",
        "accept-ranges": "bytes",
        "cache-control": "public, max-age=31536000, immutable",
      };
      for (const name of ["content-length", "content-range"]) {
        const value = blob.headers.get(name);
        if (value) headers[name] = value;
      }
      return new Response(body(blob.body), { status: blob.status, headers });
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
