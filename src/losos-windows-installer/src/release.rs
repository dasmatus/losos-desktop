//! Which release to install, and getting its files.
//!
//! A release is what CI publishes to the channel and what systemd-sysupdate
//! reads (docs/releases.md): a `SHA256SUMS` signed by `SHA256SUMS.gpg`, and
//! the files it lists. Three of them make an install: the UKI and the two
//! halves of a `/usr` slot, whose names carry the partition UUIDs the UKI
//! looks for (`<id>_<version>_<type>_<uuid>.raw.xz`). The names are matched
//! here the way update.nix's `MatchPattern=` matches them, so this program
//! installs exactly what an update would.
//!
//! The signature is checked against the release key the OS itself updates
//! with, compiled in, before any file is trusted; each file is then checked
//! against its line. Downloads go through Windows' own `curl.exe`, which uses
//! the system's certificate store and resumes a broken download.
use std::cmp::Ordering;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use miette::{IntoDiagnostic, Result, WrapErr, bail, miette};
use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use sha2::{Digest, Sha256};

use crate::guid::Guid;

/// The per-architecture names update.nix matches, from options.nix's table.
#[derive(Debug, Clone, Copy)]
pub struct Arch {
    pub name: &'static str,
    pub usr: &'static str,
    pub usr_type: &'static str,
    pub verity: &'static str,
    pub verity_type: &'static str,
    /// Root, which first boot makes: only `--uninstall` needs to know it.
    pub root_type: &'static str,
    /// The EFI machine type GRUB's file on the ESP is named after.
    pub efi: &'static str,
}

pub const ARCHES: [Arch; 2] = [
    Arch {
        name: "x86_64",
        usr: "usr-x86-64",
        usr_type: "8484680c-9521-48c6-9c11-b0720656f69e",
        verity: "usr-x86-64-verity",
        verity_type: "77ff5f63-e7b6-4633-acf4-1565b864c0e6",
        root_type: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709",
        efi: "x64",
    },
    Arch {
        name: "aarch64",
        usr: "usr-arm64",
        usr_type: "b0e01050-ee5f-4390-949a-9101b17104e9",
        verity: "usr-arm64-verity",
        verity_type: "6e11a4e7-fbca-4ded-b9e9-e1a512bb664e",
        root_type: "b921b045-1df0-41c3-af44-4c6f280d3fae",
        efi: "aa64",
    },
];

impl Arch {
    pub fn named(name: &str) -> Option<Arch> {
        ARCHES.into_iter().find(|a| a.name == name)
    }

    pub fn usr_type(&self) -> Guid {
        Guid::parse(self.usr_type).expect("a GUID literal")
    }

    pub fn verity_type(&self) -> Guid {
        Guid::parse(self.verity_type).expect("a GUID literal")
    }

    pub fn root_type(&self) -> Guid {
        Guid::parse(self.root_type).expect("a GUID literal")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub file: File,
    /// The partition's UUID, which the UKI finds `/usr` by.
    pub uuid: Guid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub uki: File,
    pub usr: Slot,
    pub verity: Slot,
}

impl Release {
    /// The partition label sysupdate reads a slot's version from, as
    /// image.nix writes it.
    pub fn label(&self, id: &str) -> String {
        format!("{id}_{}", self.version)
    }

    /// The UKI's name in `/EFI/Linux`: the image's own, uncounted form
    /// (boot.nix says why the first kernel carries no boot counter).
    pub fn uki_name(&self, id: &str) -> String {
        format!("{id}_{}.efi", self.version)
    }

    pub fn files(&self) -> [&File; 3] {
        [&self.uki, &self.verity.file, &self.usr.file]
    }
}

/// The (hash, file name) pairs of a SHA256SUMS file, in either of
/// sha256sum's text (`hash  name`) or binary (`hash *name`) forms.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(' ')?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            (hash.len() == 64 && !name.is_empty()).then(|| (hash.to_owned(), name.to_owned()))
        })
        .collect()
}

/// The newest version in `sums` for which all three files are there.
pub fn newest(sums: &str, id: &str, arch: &Arch) -> Result<Release> {
    let lines = parse_sums(sums);
    let prefix = format!("{id}_");
    let file = |name: &str, hash: &str| File {
        name: name.to_owned(),
        sha256: hash.to_owned(),
    };
    let mut releases: Vec<Release> = Vec::new();
    for (hash, name) in &lines {
        let Some(version) = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(&format!("_{}.efi", arch.name)))
        else {
            continue;
        };
        if version.is_empty() || version.contains('_') {
            continue;
        }
        let slot = |kind: &str| {
            let head = format!("{id}_{version}_{kind}_");
            lines.iter().find_map(|(hash, name)| {
                let uuid = name.strip_prefix(&head)?.strip_suffix(".raw.xz")?;
                Some(Slot {
                    file: file(name, hash),
                    uuid: Guid::parse(uuid)?,
                })
            })
        };
        if let (Some(usr), Some(verity)) = (slot(arch.usr), slot(arch.verity)) {
            releases.push(Release {
                version: version.to_owned(),
                uki: file(name, hash),
                usr,
                verity,
            });
        }
    }
    releases
        .into_iter()
        .max_by(|a, b| version_cmp(&a.version, &b.version))
        .ok_or_else(|| {
            miette!(
                "SHA256SUMS lists no complete {} release of {id} (a UKI, {} and {})",
                arch.name,
                arch.usr,
                arch.verity
            )
        })
}

/// Orders versions as sysupdate does, closely enough for the versions
/// options.nix allows: runs of digits compare as numbers, everything else
/// as text, so `20261007.102859` follows `20261006.235959`.
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    fn runs(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let bytes = s.as_bytes();
        for i in 1..=bytes.len() {
            if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[start].is_ascii_digit() {
                out.push((bytes[start].is_ascii_digit(), &s[start..i]));
                start = i;
            }
        }
        out
    }
    let (ra, rb) = (runs(a), runs(b));
    for ((da, sa), (db, sb)) in ra.iter().zip(&rb) {
        let order = if *da && *db {
            let (ta, tb) = (sa.trim_start_matches('0'), sb.trim_start_matches('0'));
            ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb))
        } else {
            sa.cmp(sb)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    ra.len().cmp(&rb.len())
}

/// Checks a detached signature over `sums` against a binary keyring holding
/// the release key, as systemd-pull does with gpg: any valid key or subkey
/// in the keyring is trusted, since being in it is what trust means here.
pub fn verify_signature(sums: &[u8], signature: &[u8], keyring: &[u8]) -> Result<()> {
    let signature = DetachedSignature::from_bytes(signature)
        .map_err(|e| miette!("SHA256SUMS.gpg is not an OpenPGP signature: {e}"))?;
    let keys = SignedPublicKey::from_bytes_many(keyring)
        .map_err(|e| miette!("the release keyring does not parse: {e}"))?;
    let mut any = false;
    for key in keys {
        let key = key.map_err(|e| miette!("the release keyring does not parse: {e}"))?;
        any = true;
        if key.verify_bindings().is_err() {
            continue;
        }
        if signature.verify(&key, sums).is_ok() {
            return Ok(());
        }
        for subkey in &key.public_subkeys {
            if subkey.verify_bindings(&key).is_ok() && signature.verify(subkey, sums).is_ok() {
                return Ok(());
            }
        }
    }
    if !any {
        bail!("the release keyring holds no key");
    }
    bail!("SHA256SUMS is not signed by the release key")
}

pub fn sha256_file(path: &Path, mut progress: impl FnMut(u64)) -> Result<String> {
    let mut file = fs::File::open(path)
        .into_diagnostic()
        .wrap_err_with(|| path.display().to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut done = 0;
    loop {
        let n = file
            .read(&mut buffer)
            .into_diagnostic()
            .wrap_err_with(|| path.display().to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        done += n as u64;
        progress(done);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Where the release comes from: the channel, or a directory holding one,
/// as the ISO's installer takes a LOSOS-RELEASE disk.
#[derive(Debug, Clone)]
pub enum Source {
    Channel(String),
    Directory(PathBuf),
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Channel(url) => url.clone(),
            Source::Directory(dir) => dir.display().to_string(),
        }
    }

    /// `name` from the source, as a local file. A directory's files are used
    /// where they are; the channel's are downloaded into `cache`, resuming
    /// whatever an earlier attempt left there.
    pub fn fetch(&self, name: &str, cache: &Path) -> Result<PathBuf> {
        match self {
            Source::Directory(dir) => {
                let path = dir.join(name);
                if !path.is_file() {
                    bail!("{} is not in {}", name, dir.display());
                }
                Ok(path)
            }
            Source::Channel(url) => {
                fs::create_dir_all(cache)
                    .into_diagnostic()
                    .wrap_err_with(|| cache.display().to_string())?;
                let path = cache.join(name);
                let url = format!("{url}{name}");
                // -C - resumes; --retry-all-errors covers a connection the
                // proxy's redirect target drops halfway. curl's own progress
                // meter is the progress shown.
                let status = Command::new(curl())
                    .args(["-fL", "--retry", "5", "--retry-all-errors", "-C", "-", "-o"])
                    .arg(&path)
                    .arg(&url)
                    .status()
                    .into_diagnostic()
                    .wrap_err("could not run curl")?;
                if !status.success() {
                    // A finished file makes curl's resume fail with "range
                    // not satisfiable" (exit 33) rather than succeed.
                    if status.code() == Some(33) && path.is_file() {
                        return Ok(path);
                    }
                    bail!("downloading {url} failed ({status})");
                }
                Ok(path)
            }
        }
    }

    /// SHA256SUMS and its signature, always fetched afresh: an old manifest
    /// would name a release the channel may no longer serve.
    pub fn manifest(&self, cache: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
        let read = |path: PathBuf| {
            fs::read(&path)
                .into_diagnostic()
                .wrap_err_with(|| path.display().to_string())
        };
        if let Source::Channel(_) = self {
            for name in ["SHA256SUMS", "SHA256SUMS.gpg"] {
                let _ = fs::remove_file(cache.join(name));
            }
        }
        Ok((
            read(self.fetch("SHA256SUMS", cache)?)?,
            read(self.fetch("SHA256SUMS.gpg", cache)?)?,
        ))
    }
}

/// Windows 10 1803 and later carry curl; a path rather than a bare name, so
/// a curl.exe in the working directory is not the one run.
fn curl() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("curl.exe")
    } else {
        PathBuf::from("curl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &str = "\
308bb638e2b5f1fa27708af961a98dae6efcf046516142ea74ce558897700738  losos-desktop_20261007.102859_usr-x86-64-verity_80afbdea-1680-4d3e-322a-5f743f301a68.raw.xz
3e8ae9620f682110327a9770aa49a15e7d21fd521c8f1a2f380b9147d144b88d  losos-desktop_20261007.102859_usr-x86-64_a28067f9-25ea-dd94-670f-df04370bfc83.raw.xz
08863411434e65d44e67e2b9c885df6d27f6acaebb6a5f13ec31624fbb32ad36  losos-desktop_20261007.102859_x86_64-installer.iso
0293e6dae5c95a29b48b8b7f79d3ceadea07d9b4b4b92e4ab9b7c5f72cb3905c  losos-desktop_20261007.102859_x86_64.efi
1111111111111111111111111111111111111111111111111111111111111111  losos-desktop_20261006.090000_x86_64.efi
2222222222222222222222222222222222222222222222222222222222222222  losos-desktop_20261006.090000_usr-x86-64_a28067f9-25ea-dd94-670f-df04370bfc84.raw.xz
3333333333333333333333333333333333333333333333333333333333333333  losos-desktop_20261006.090000_usr-x86-64-verity_80afbdea-1680-4d3e-322a-5f743f301a69.raw.xz
4444444444444444444444444444444444444444444444444444444444444444  losos-desktop_20261008.000000_x86_64.efi
5555555555555555555555555555555555555555555555555555555555555555  losos-desktop_20261007.102859_aarch64.efi
";

    #[test]
    fn picks_the_newest_complete_release() {
        let arch = Arch::named("x86_64").unwrap();
        let release = newest(SUMS, "losos-desktop", &arch).unwrap();
        // 20261008 has a UKI and no /usr, so it is not a release.
        assert_eq!(release.version, "20261007.102859");
        assert_eq!(release.uki.name, "losos-desktop_20261007.102859_x86_64.efi");
        assert_eq!(
            release.usr.uuid.to_string(),
            "a28067f9-25ea-dd94-670f-df04370bfc83"
        );
        assert_eq!(
            release.verity.uuid.to_string(),
            "80afbdea-1680-4d3e-322a-5f743f301a68"
        );
        assert_eq!(
            release.label("losos-desktop"),
            "losos-desktop_20261007.102859"
        );
        assert_eq!(
            release.uki_name("losos-desktop"),
            "losos-desktop_20261007.102859.efi"
        );
    }

    #[test]
    fn refuses_a_manifest_without_this_architecture() {
        let arch = Arch::named("aarch64").unwrap();
        assert!(newest(SUMS, "losos-desktop", &arch).is_err());
    }

    #[test]
    fn orders_versions_like_sysupdate() {
        assert_eq!(
            version_cmp("20261007.102859", "20261006.235959"),
            Ordering::Greater
        );
        assert_eq!(version_cmp("9.1", "10.0"), Ordering::Less);
        assert_eq!(version_cmp("1.2", "1.2"), Ordering::Equal);
        assert_eq!(version_cmp("1.2.1", "1.2"), Ordering::Greater);
        assert_eq!(version_cmp("01", "1"), Ordering::Equal);
    }

    #[test]
    fn reads_both_sha256sum_forms() {
        let text = format!(
            "{}  a b\n{} *c\nnot a line\n",
            "0".repeat(64),
            "f".repeat(64)
        );
        assert_eq!(
            parse_sums(&text),
            vec![
                ("0".repeat(64), "a b".to_owned()),
                ("f".repeat(64), "c".to_owned())
            ]
        );
    }

    // nightly 20261007.102859's manifest and the signature CI made over it,
    // and nixos/keys/update-signing.asc dearmored, as options.nix does.
    const KEYRING: &[u8] = include_bytes!("../tests/data/import-pubring.gpg");
    const MANIFEST: &[u8] = include_bytes!("../tests/data/SHA256SUMS");
    const SIGNATURE: &[u8] = include_bytes!("../tests/data/SHA256SUMS.gpg");

    #[test]
    fn accepts_the_release_keys_signature() {
        verify_signature(MANIFEST, SIGNATURE, KEYRING).unwrap();
    }

    #[test]
    fn rejects_a_signature_over_other_bytes() {
        let mut tampered = MANIFEST.to_vec();
        tampered[0] ^= 1;
        assert!(verify_signature(&tampered, SIGNATURE, KEYRING).is_err());
    }
}
