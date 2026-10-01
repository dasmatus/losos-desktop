//! The decisions behind the GHCR proxy, compiled to WebAssembly.
//!
//! The proxy (`api/proxy.js`) serves two things from one host, both out of this
//! project's GitHub Container Registry namespace:
//!
//! * **A Nix binary cache.** `nix copy --to file://...` writes a narinfo and a
//!   NAR per store path; `tools/nix-cache-push` puts each pair in GHCR as one
//!   OCI artifact, `nix-cache:<store hash>`. Nix asks a cache for
//!   `/<hash>.narinfo` and then for whatever its `URL:` names, and this maps
//!   both onto that artifact. It replaces a hosted cache for builds, CI's and
//!   anyone's, and it is the only substituter the flake trusts.
//! * **systemd-sysupdate's update source.** CI pushes each release as an OCI
//!   artifact already (`images:<channel>-<version>-<arch>`), and also under
//!   the moving tag `<channel>-<arch>`. `/updates/<channel>/<arch>/<file>`
//!   serves a file of the newest one, `SHA256SUMS` included, which is all a
//!   `url-file` transfer asks of its source. That is the install side.
//!
//! What lives here is every decision the proxy makes: which request is which,
//! what is a valid name, which layer of a manifest answers it, and how a
//! narinfo is rewritten. The JavaScript around it only performs I/O, because
//! `fetch` is the one thing a module with no imports cannot do. So everything
//! that could let a crafted path reach something it should not is in code
//! that `cargo test` runs.
//!
//! # The interface
//!
//! Strings in, strings out, over linear memory: the host calls `alloc`,
//! writes UTF-8, calls a function, and gets back `(ptr << 32) | len` of a
//! string it must hand to `free` after copying. `alloc`'s buffer is consumed
//! by the call it is passed to. No JSON crosses: the host
//! parses the registry's JSON natively and passes plain lines.

use std::fmt::Write;

/// Nix's base-32 alphabet: the digits and lowercase letters without e, o, t, u.
const NIX32: &str = "0123456789abcdfghijklmnpqrsvwxyz";

/// The architectures a release is built for.
const ARCHES: &[&str] = &["x86_64", "aarch64"];

/// What a request maps to.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// A fixed answer: content type and body.
    Static(&'static str, &'static str),
    /// One layer of one OCI artifact in the namespace.
    Layer {
        /// Repository under the namespace, e.g. `nix-cache`.
        repository: &'static str,
        /// Tag of the artifact.
        tag: String,
        /// `org.opencontainers.image.title` of the layer that answers.
        title: String,
        /// How the layer's bytes reach the client.
        kind: Kind,
    },
    /// Nothing here; the reason goes in the 404's body.
    Missing(&'static str),
}

/// How a layer is served.
#[derive(Debug, PartialEq)]
pub enum Kind {
    /// Fetched and rewritten by [`narinfo`], because its `URL:` must point back
    /// here. A narinfo is a few hundred bytes.
    Narinfo,
    /// Redirected to GHCR's signed blob URL, so a NAR or a disk image is never
    /// streamed through the proxy.
    Redirect,
    /// Fetched and passed through byte for byte, for a small file whose client
    /// may not follow a redirect off the host it was pointed at. Bytes rather
    /// than text because `SHA256SUMS.gpg` is a binary OpenPGP signature, which
    /// a decode to UTF-8 and back would corrupt.
    Inline,
}

/// What `/nix-cache-info` says. Priority 30 puts it ahead of a default
/// cache.nixos.org (40) on a machine that has both configured.
const CACHE_INFO: &str = "StoreDir: /nix/store\nWantMassQuery: 1\nPriority: 30\n";

fn is_nix32(s: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&s.len()) && s.chars().all(|c| NIX32.contains(c))
}

/// A store path's hash part: exactly 32 characters of base 32.
fn is_store_hash(s: &str) -> bool {
    is_nix32(s, 32..=32)
}

/// A NAR's file name as `nix copy` writes it: its file hash in base 32 and a
/// compression suffix.
fn is_nar_file(s: &str) -> bool {
    let Some((hash, suffix)) = s.split_once('.') else {
        return false;
    };
    is_nix32(hash, 1..=64) && ["nar", "nar.xz", "nar.zst", "nar.bz2"].contains(&suffix)
}

/// A release channel, such as `nightly` (`losos.channel`), and short
/// enough that `<channel>-<arch>` stays a valid OCI tag.
fn is_channel(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// A release file name. No slash, no leading dot, nothing a shell or a URL
/// would read twice.
fn is_release_file(s: &str) -> bool {
    (1..=128).contains(&s.len())
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

/// Map a request path to what answers it.
///
/// The path is taken as the client sent it, after the proxy has stripped any
/// query. Percent-escapes are not decoded: no valid name contains one, so a
/// path that has any is simply not found.
#[must_use]
pub fn route(path: &str) -> Route {
    let path = path.strip_prefix('/').unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["nix-cache-info"] => Route::Static("text/x-nix-cache-info", CACHE_INFO),
        [file] if file.ends_with(".narinfo") => {
            let hash = file.trim_end_matches(".narinfo");
            if !is_store_hash(hash) {
                return Route::Missing("not a store path hash");
            }
            Route::Layer {
                repository: "nix-cache",
                tag: hash.into(),
                title: (*file).into(),
                kind: Kind::Narinfo,
            }
        }
        ["nar", hash, file] => {
            if !is_store_hash(hash) || !is_nar_file(file) {
                return Route::Missing("not a NAR this cache serves");
            }
            Route::Layer {
                repository: "nix-cache",
                tag: (*hash).into(),
                title: (*file).into(),
                kind: Kind::Redirect,
            }
        }
        ["updates", channel, arch, file] => {
            if !is_channel(channel) || !ARCHES.contains(arch) || !is_release_file(file) {
                return Route::Missing("not a release file");
            }
            // sysupdate reads SHA256SUMS (and its signature) itself and follows
            // no redirect it did not expect; the images are what is large.
            let kind = if file.starts_with("SHA256SUMS") {
                Kind::Inline
            } else {
                Kind::Redirect
            };
            Route::Layer {
                repository: "images",
                tag: format!("{channel}-{arch}"),
                title: (*file).into(),
                kind,
            }
        }
        _ => Route::Missing("unknown path"),
    }
}

/// Render a [`Route`] as the one-line plan the host reads.
///
/// `static <content-type>\n<body>`, `layer <repository> <tag> <title> <kind>`
/// or `missing <reason>`. Every field of a layer plan was validated above and
/// contains no space.
#[must_use]
pub fn plan(path: &str) -> String {
    match route(path) {
        Route::Static(content_type, body) => format!("static {content_type}\n{body}"),
        Route::Layer {
            repository,
            tag,
            title,
            kind,
        } => {
            let kind = match kind {
                Kind::Narinfo => "narinfo",
                Kind::Redirect => "redirect",
                Kind::Inline => "inline",
            };
            format!("layer {repository} {tag} {title} {kind}")
        }
        Route::Missing(reason) => format!("missing {reason}"),
    }
}

/// Pick the layer titled `title` from a manifest's layers.
///
/// Input: the title on the first line, then one `<digest>\t<title>` per
/// layer. Output: the digest, or the empty string when no layer has that
/// title or its digest is not a sha256 one -- the only algorithm GHCR serves,
/// and a string that goes into a URL path next.
#[must_use]
pub fn pick(input: &str) -> String {
    let mut lines = input.lines();
    let Some(title) = lines.next() else {
        return String::new();
    };
    lines
        .filter_map(|line| line.split_once('\t'))
        .find(|(_, t)| *t == title)
        .map(|(digest, _)| digest)
        .filter(|digest| {
            digest
                .strip_prefix("sha256:")
                .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        })
        .map(String::from)
        .unwrap_or_default()
}

/// Point a narinfo's `URL:` back at this proxy.
///
/// Input: the store hash on the first line, then the narinfo as `nix copy`
/// wrote it. `URL: nar/<file>` becomes `URL: nar/<hash>/<file>`, which is the
/// one thing [`route`] needs to find the artifact again. Nothing else changes:
/// the signature covers the store path, the NAR's hash and size and the
/// references, not the URL, so a rewritten narinfo verifies exactly as the
/// original. Anything unexpected -- no URL, a URL elsewhere, two of them --
/// returns the empty string, and the proxy answers 502 rather than serve it.
#[must_use]
pub fn narinfo(input: &str) -> String {
    let Some((hash, text)) = input.split_once('\n') else {
        return String::new();
    };
    if !is_store_hash(hash) {
        return String::new();
    }
    let mut out = String::with_capacity(text.len() + 40);
    let mut urls = 0;
    for line in text.lines() {
        if let Some(url) = line.strip_prefix("URL: ") {
            urls += 1;
            match url.strip_prefix("nar/") {
                Some(file) if is_nar_file(file) => {
                    let _ = writeln!(out, "URL: nar/{hash}/{file}");
                }
                _ => return String::new(),
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if urls == 1 { out } else { String::new() }
}

// The linear-memory interface. Only compiled for the target the host loads,
// so `cargo test` exercises the functions above directly.

#[cfg(target_arch = "wasm32")]
mod abi {
    use std::ptr::slice_from_raw_parts_mut;

    /// Every buffer that crosses is a boxed slice, whose allocation is exactly
    /// its length -- so `(ptr, len)` is all either side needs to free it.
    fn leak(bytes: Box<[u8]>) -> (*mut u8, usize) {
        let len = bytes.len();
        (Box::into_raw(bytes).cast::<u8>(), len)
    }

    /// Reserve `len` bytes for the host to write a string into.
    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: usize) -> *mut u8 {
        leak(vec![0; len].into_boxed_slice()).0
    }

    /// Release a string a function below returned.
    ///
    /// # Safety
    ///
    /// `ptr` and `len` must be exactly what a function below returned, and
    /// must not be used afterwards.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
        drop(unsafe { Box::from_raw(slice_from_raw_parts_mut(ptr, len)) });
    }

    /// Take the host's string, run `f`, and hand back the answer.
    ///
    /// Invalid UTF-8 is answered with the empty string, which every caller
    /// already treats as "no".
    unsafe fn call(ptr: *mut u8, len: usize, f: fn(&str) -> String) -> u64 {
        let input = unsafe { Box::from_raw(slice_from_raw_parts_mut(ptr, len)) };
        let output = std::str::from_utf8(&input).map(f).unwrap_or_default();
        let (out_ptr, out_len) = leak(output.into_bytes().into_boxed_slice());
        ((out_ptr as u64) << 32) | out_len as u64
    }

    /// # Safety
    ///
    /// `ptr` and `len` must come from [`alloc`]; ownership passes here.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn plan(ptr: *mut u8, len: usize) -> u64 {
        unsafe { call(ptr, len, super::plan) }
    }

    /// # Safety
    ///
    /// As for [`plan`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn pick(ptr: *mut u8, len: usize) -> u64 {
        unsafe { call(ptr, len, super::pick) }
    }

    /// # Safety
    ///
    /// As for [`plan`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn narinfo(ptr: *mut u8, len: usize) -> u64 {
        unsafe { call(ptr, len, super::narinfo) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0c0x1c0lyb5dh3dkw4mv1bmp8vd6gn8f";
    const NAR: &str = "1w1fff338fvdw53sqgamddn1b2xgds473pv6y13gizdbqjv4i5p3.nar.xz";

    #[test]
    fn cache_info_is_static() {
        assert_eq!(
            plan("/nix-cache-info"),
            format!("static text/x-nix-cache-info\n{CACHE_INFO}")
        );
    }

    #[test]
    fn a_narinfo_and_its_nar_map_to_one_artifact() {
        assert_eq!(
            plan(&format!("/{HASH}.narinfo")),
            format!("layer nix-cache {HASH} {HASH}.narinfo narinfo")
        );
        assert_eq!(
            plan(&format!("/nar/{HASH}/{NAR}")),
            format!("layer nix-cache {HASH} {NAR} redirect")
        );
    }

    #[test]
    fn release_files_come_from_the_moving_tag() {
        assert_eq!(
            plan("/updates/nightly/x86_64/SHA256SUMS"),
            "layer images nightly-x86_64 SHA256SUMS inline"
        );
        assert_eq!(
            plan("/updates/stable/aarch64/losos-desktop_1.2_aarch64.efi"),
            "layer images stable-aarch64 losos-desktop_1.2_aarch64.efi redirect"
        );
    }

    #[test]
    fn crafted_paths_find_nothing() {
        for path in [
            "/",
            "/v2/_catalog",
            "/../etc/passwd.narinfo",
            "/0c0x1c0lyb5dh3dkw4mv1bmp8vd6gn8e.narinfo", // 'e' is not base 32
            "/0c0x1c0lyb5dh3dkw4mv1bmp8vd6gn8.narinfo",  // 31 characters
            &format!("/nar/{HASH}/../x.nar"),
            &format!("/nar/{HASH}/{NAR}/more"),
            &format!("/nar/{HASH}/abc.tar"),
            "/updates/nightly/riscv64/SHA256SUMS",
            "/updates/Nightly/x86_64/SHA256SUMS",
            "/updates/nightly/x86_64/.hidden",
            "/updates/nightly/x86_64/a%2Fb",
            "/updates/nightly%20x/x86_64/SHA256SUMS",
        ] {
            assert!(
                plan(path).starts_with("missing "),
                "{path} -> {}",
                plan(path)
            );
        }
    }

    #[test]
    fn pick_finds_the_titled_sha256_layer() {
        let a = format!("sha256:{}", "a".repeat(64));
        let b = format!("sha256:{}", "b".repeat(64));
        let layers = format!("{NAR}\n{a}\t{HASH}.narinfo\n{b}\t{NAR}\n");
        assert_eq!(pick(&layers), b);
        assert_eq!(pick(&format!("other\n{a}\tx\n")), "");
        assert_eq!(pick("x\nsha512:abc\tx\n"), "");
        assert_eq!(pick(&format!("x\nsha256:{}/../\tx\n", "a".repeat(60))), "");
    }

    #[test]
    fn narinfo_url_points_back_here_and_nothing_else_moves() {
        let original = format!(
            "StorePath: /nix/store/{HASH}-hello-2.12\nURL: nar/{NAR}\nCompression: xz\n\
             NarHash: sha256:x\nNarSize: 1\nReferences: \nSig: k:abc\n"
        );
        let rewritten = narinfo(&format!("{HASH}\n{original}"));
        assert_eq!(
            rewritten,
            original.replace("URL: nar/", &format!("URL: nar/{HASH}/"))
        );
    }

    #[test]
    fn a_narinfo_pointing_elsewhere_is_refused() {
        for url in ["https://evil.example/x.nar", "nar/../x.nar", "nar/a/b.nar"] {
            assert_eq!(narinfo(&format!("{HASH}\nStorePath: x\nURL: {url}\n")), "");
        }
        assert_eq!(narinfo(&format!("{HASH}\nStorePath: x\n")), "");
        assert_eq!(
            narinfo(&format!("{HASH}\nURL: nar/{NAR}\nURL: nar/{NAR}\n")),
            ""
        );
        assert_eq!(narinfo(&format!("bad\nURL: nar/{NAR}\n")), "");
    }
}
