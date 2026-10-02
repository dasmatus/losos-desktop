// The edge runtime's `?module` import yields a compiled WebAssembly.Module.
declare module "*.wasm?module" {
  const module: WebAssembly.Module;
  export default module;
}
