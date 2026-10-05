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
      KV_REST_API_URL: process.env.KV_REST_API_URL,
      KV_REST_API_TOKEN: process.env.KV_REST_API_TOKEN,
      UPSTASH_REDIS_REST_URL: process.env.UPSTASH_REDIS_REST_URL,
      UPSTASH_REDIS_REST_TOKEN: process.env.UPSTASH_REDIS_REST_TOKEN,
      CHOICE_SCREENS: process.env.CHOICE_SCREENS,
      CHOICE_SCREENS_AT: process.env.CHOICE_SCREENS_AT,
      CHOICE_SCREENS_DAILY_AT: process.env.CHOICE_SCREENS_DAILY_AT,
    });
  }
  return handle(request);
}
