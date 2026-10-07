# Binary cache

`proxy/` is a Vercel edge function whose decisions are a WebAssembly module
compiled from Rust (`proxy/src/lib.rs`); the JavaScript around it only
fetches. It serves these from this project's GHCR namespace:

- **A Nix binary cache.** `tools/nix-cache-push` pushes each store path as
  one OCI artifact, `nix-cache:<store hash>`, holding its signed narinfo and
  its NAR. The proxy answers `/<hash>.narinfo` (with its `URL:` pointed back
  at itself) and redirects `/nar/<hash>/<file>` to GHCR's blob storage, so a
  NAR never passes through it. A real `nix copy` pulled and verified a signed
  path through it against a fake registry; the tests are in `proxy/test/`.
- **Build caches.** `/build-cache/<name>/<arch>/<part>` redirects to one
  part of a compiler cache CI keeps in GHCR: `losos` for the overlay's
  patched GTK and Qt (`losos-ccache:<arch>`). It is a zstd tarball split
  into 4 GB parts, `ccache.tar.zst.part-00` on, and
  `tools/build-cache-fetch` fetches and unpacks one, through the proxy
  first and from GHCR with oras when the proxy has nothing.
- **Updates.** `/updates/<channel>/<arch>/<file>` serves a file of
  `images:<channel>-<arch>`, which CI's publish job moves to each release
  that passed verification. CI sets `losos.update.baseUrl` to
  `<proxy>/updates/<channel>/<arch>/`, so sysupdate fetches manifests and
  images through the proxy. The GitHub nightly release is announcement-only:
  GitHub rejects release assets of 2 GiB or larger.
- **The web flasher's files.** `/flasher/<channel>/<arch>/<file>` serves the
  same release's `_gsi-` files and `SHA256SUMS`, streamed one byte range at a
  time rather than redirected, because a page on another origin cannot read
  GHCR's storage ([Web flasher](web-flasher.md)). Every read the proxy answers
  says `Access-Control-Allow-Origin: *`.

To use it for a build:

```
substituters = https://cache.nixos.org/ https://<proxy>/
trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY= <the public half of NIX_CACHE_SIGNING_KEY>
```

Keep cache.nixos.org in the substituter list when adding the project cache.
Without the optional project cache, Nix's default cache.nixos.org remains
enabled.

What has to be set up once, outside the repository:

1. **Vercel**: a project with root directory `proxy/` and environment
   variable `GHCR_REPOSITORY=dasmatus/losos-desktop`. `vercel.json` has the
   rest. Its build installs a pinned Rust with rustup when absent and adds
   the WebAssembly target when Rust is already installed.
2. **GHCR**: the `losos-desktop/nix-cache`, `losos-desktop/images` and
   `losos-desktop/losos-ccache` packages public, once CI has created them;
   or a read-only token in Vercel as `GHCR_TOKEN`, with its owner's GitHub
   login as `GHCR_USERNAME`.
3. **A signing key**: `nix key generate-secret --key-name losos-desktop-1`,
   stored as the secret `NIX_CACHE_SIGNING_KEY`; its public half, from `nix
   key convert-secret-to-public`, as the variable `NIX_CACHE_PUBLIC_KEY`.
4. **`LOSOS_PROXY_URL`**, the deployment's URL without a trailing slash, as a
   repository variable. CI requires it and uses it both for Nix cache
   substitutions and as the base of each image's update URL. Flake evaluation
   reads this environment variable, so CI uses `--impure` when building,
   checking and collecting the release. CI uploads its signed cache paths to
   GHCR whenever `NIX_CACHE_SIGNING_KEY` is configured, even if this URL is
   unset for a non-CI build; the proxy can serve them once configured.
5. **The update signing key**, which every image trusts and every release
   is signed with. Make it on a machine you trust, with no passphrase,
   because CI signs unattended:

   ```
   export GNUPGHOME=$(mktemp -d)
   mkdir -p nixos/keys
   gpg --batch --pinentry-mode loopback --passphrase '' --quick-gen-key 'LosOS Desktop updates' ed25519 sign never
   gpg --armor --export > nixos/keys/update-signing.asc
   gpg --armor --export-secret-keys   # paste into the secret UPDATE_SIGNING_KEY
   rm -rf "$GNUPGHOME"                # once the secret and an offline copy are saved
   ```

   Commit `nixos/keys/update-signing.asc`; `losos.update.pubring` picks it
   up and turns sysupdate's verification on. Put the key's fingerprint
   (`gpg --show-keys --with-colons nixos/keys/update-signing.asc`) in
   `UPDATE_SIGNING_FPR` in `ci.yml`. The `push` job fails without the secret,
   or with a secret for a different key, rather than publish a release the
   images would refuse. Keep an offline copy of the secret half: an image
   only ever trusts the key it shipped with, so a lost key means every
   installed machine stops taking updates until it is reinstalled.

Before setting `LOSOS_PROXY_URL`, check the public deployment without a
Vercel login: `/nix-cache-info` must return the cache metadata, an absent
store hash must return 404 rather than `502 token: 403`, and a published
release's `/updates/<channel>/<arch>/SHA256SUMS` must be readable. A ready
deployment alone does not prove GHCR access works. The variable also changes
the pm image's update source, so leave it unset while the registry is
inaccessible. Vercel deployment protection must allow anonymous clients.
