// The Vercel edge function. build.sh compiles src/ to ./losos_proxy.wasm
// beside this file; `?module` is how the edge runtime imports one.
import module from "./losos_proxy.wasm?module";
import { bind, handler, type Handler } from "../lib/handler.js";

export const config = { runtime: "edge" };

let handle: Handler | undefined;

export default async function proxy(request: Request): Promise<Response> {
  if (!handle) {
    // No imports: the module can reach nothing but the strings it is handed.
    const instance = await WebAssembly.instantiate(module, {});
    handle = handler(bind(instance), {
      GHCR_REPOSITORY: process.env.GHCR_REPOSITORY,
      GHCR_TOKEN: process.env.GHCR_TOKEN,
      GHCR_USERNAME: process.env.GHCR_USERNAME,
    });
  }
  return handle(request);
}
