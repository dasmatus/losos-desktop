#!/usr/bin/env python3
"""Offline regressions for fragmented releases and signed manifests.

Splitting and merging are two programs in two languages that must agree on a
file format, and signing is a step whose failure is silent: a release nobody's
machine accepts looks exactly like a release. So this cuts real files, joins
them with the shipped shell script, damages them in each way a download can
be damaged, and signs with a throwaway key.
"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
SPLIT = REPO / "tools" / "split-release"
MERGE = REPO / "tools" / "merge-release.sh"
SIGN = REPO / "tools" / "sign-release"
PUBRING = REPO / "overlay" / "usr" / "lib" / "systemd" / "import-pubring.pgp"

ISO = "losos-desktop_1_x86_64.iso"
UKI = "losos-desktop_1_x86_64.efi"


def run(*argv, **kwargs):
    return subprocess.run([str(a) for a in argv], capture_output=True,
                          text=True, **kwargs)


class FragmentTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="test-split-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.release = self.tmp / "release"
        self.release.mkdir()
        # Not a multiple of the fragment size, so the last one is short.
        self.payload = os.urandom(2500)
        (self.release / ISO).write_bytes(self.payload)
        (self.release / UKI).write_bytes(b"small")
        import hashlib
        sums = "".join(
            f"{hashlib.sha256((self.release / n).read_bytes()).hexdigest()}  {n}\n"
            for n in (ISO, UKI))
        (self.release / "SHA256SUMS").write_text(sums)
        (self.release / "SHA256SUMS.gpg").write_bytes(b"stale")

    def split(self, size="1000"):
        result = run(SPLIT, self.release, "--size", size)
        self.assertEqual(result.returncode, 0, result.stderr)

    def merge(self, *extra):
        return run("sh", MERGE, "-d", self.release, *extra, ISO)

    def test_only_large_files_are_cut(self):
        self.split()
        names = sorted(p.name for p in self.release.iterdir())
        self.assertIn(UKI, names)
        self.assertNotIn(ISO, names)
        self.assertEqual([n for n in names if ".part-" in n],
                         [f"{ISO}.part-00{i}" for i in range(3)])
        for part in (self.release / n for n in names if ".part-" in n):
            self.assertLessEqual(part.stat().st_size, 1000)

    def test_manifest_follows_the_split(self):
        self.split()
        listed = [line.split()[1] for line in
                  (self.release / "SHA256SUMS").read_text().splitlines()]
        self.assertNotIn(ISO, listed)
        self.assertIn(UKI, listed)
        self.assertIn(f"{ISO}.parts", listed)
        self.assertIn("merge-release.sh", listed)
        self.assertEqual(len([n for n in listed if ".part-" in n]), 3)
        self.assertFalse((self.release / "SHA256SUMS.gpg").exists(),
                         "a signature over the old manifest must not survive")
        checked = run("sha256sum", "-c", "SHA256SUMS", cwd=self.release)
        self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)

    def test_round_trip(self):
        self.split()
        result = self.merge()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.release / ISO).read_bytes(), self.payload)
        self.assertEqual(list(self.release.glob("*.part-*")), [])

    def test_keep_leaves_the_fragments(self):
        self.split()
        self.assertEqual(self.merge("-k").returncode, 0)
        self.assertEqual(len(list(self.release.glob("*.part-*"))), 3)

    def test_exact_multiple_has_no_empty_fragment(self):
        (self.release / ISO).write_bytes(self.payload[:2000])
        self.split()
        self.assertEqual(len(list(self.release.glob("*.part-*"))), 2)
        self.assertEqual(self.merge().returncode, 0)

    def test_damaged_fragment_is_refused(self):
        self.split()
        part = self.release / f"{ISO}.part-001"
        data = bytearray(part.read_bytes())
        data[0] ^= 1
        part.write_bytes(data)
        result = self.merge()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("damaged", result.stderr)
        self.assertFalse((self.release / ISO).exists())

    def test_missing_fragment_is_refused(self):
        self.split()
        (self.release / f"{ISO}.part-002").unlink()
        result = self.merge()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing fragment", result.stderr)
        self.assertFalse((self.release / ISO).exists())

    def test_wrong_whole_file_digest_is_refused(self):
        self.split()
        parts = self.release / f"{ISO}.parts"
        lines = parts.read_text().splitlines()
        lines[-1] = "0" * 64 + f"  {ISO}"
        parts.write_text("\n".join(lines) + "\n")
        result = self.merge()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.release / ISO).exists())
        self.assertFalse((self.release / f"{ISO}.merging").exists())

    def test_resplitting_is_refused(self):
        self.split()
        (self.release / ISO).write_bytes(self.payload)
        self.assertNotEqual(run(SPLIT, self.release, "--size", "1000").returncode, 0)


@unittest.skipUnless(shutil.which("gpg") and shutil.which("gpgv"),
                     "gpg and gpgv are needed to sign")
class SigningTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="test-sign-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        home = self.tmp / "gnupg"
        home.mkdir(mode=0o700)
        gpg = ["gpg", "--homedir", home, "--batch", "--no-tty",
               "--pinentry-mode", "loopback", "--passphrase", ""]
        made = run(*gpg, "--quick-generate-key", "test", "ed25519", "sign", "never")
        self.assertEqual(made.returncode, 0, made.stderr)
        self.secret = run(*gpg, "--armor", "--export-secret-keys").stdout
        self.pubring = self.tmp / "pub.pgp"
        self.pubring.write_bytes(subprocess.run(
            [str(a) for a in (*gpg, "--export")], capture_output=True).stdout)
        self.release = self.tmp / "release"
        self.release.mkdir()
        (self.release / "SHA256SUMS").write_text("0" * 64 + "  a\n")

    def sign(self, key=None):
        env = {**os.environ, "UPDATE_SIGNING_KEY": self.secret if key is None else key}
        return run(SIGN, self.release, "--pubring", self.pubring, env=env)

    def test_signature_verifies_and_detects_tampering(self):
        self.assertEqual(self.sign().returncode, 0)
        check = ["--check", "--pubring", self.pubring]
        self.assertEqual(run(SIGN, self.release, *check).returncode, 0)
        with (self.release / "SHA256SUMS").open("a") as manifest:
            manifest.write("1" * 64 + "  b\n")
        self.assertNotEqual(run(SIGN, self.release, *check).returncode, 0)

    def test_signature_is_binary_not_armored(self):
        self.sign()
        self.assertNotIn(b"BEGIN PGP", (self.release / "SHA256SUMS.gpg").read_bytes())

    def test_empty_key_is_an_error(self):
        self.assertNotEqual(self.sign(key="").returncode, 0)

    def test_a_key_the_image_does_not_trust_is_refused(self):
        # The committed keyring is not this throwaway key's: signing succeeds
        # and verification against what images carry must fail loudly.
        env = {**os.environ, "UPDATE_SIGNING_KEY": self.secret}
        result = run(SIGN, self.release, env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(PUBRING.name, result.stderr)

    def test_merge_checks_the_signature_when_given_a_keyring(self):
        release = self.release
        (release / "big").write_bytes(os.urandom(300))
        (release / "SHA256SUMS").write_text("0" * 64 + "  big\n")
        self.assertEqual(run(SPLIT, release, "--size", "100").returncode, 0)
        self.assertEqual(self.sign().returncode, 0)
        merge = ["sh", MERGE, "-d", release, "-g", self.pubring, "big"]
        self.assertEqual(run(*merge).returncode, 0)

    def test_merge_refuses_a_swapped_parts_file(self):
        release = self.release
        (release / "big").write_bytes(os.urandom(300))
        (release / "SHA256SUMS").write_text("0" * 64 + "  big\n")
        self.assertEqual(run(SPLIT, release, "--size", "100").returncode, 0)
        self.assertEqual(self.sign().returncode, 0)
        parts = release / "big.parts"
        parts.write_text(parts.read_text() + "\n")
        result = run("sh", MERGE, "-d", release, "-g", self.pubring, "big")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("signed SHA256SUMS", result.stderr)


class CommittedKeyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("gpg"), "gpg is needed to read the keyring")
    def test_the_committed_keyring_is_one_signing_key(self):
        listing = run("gpg", "--show-keys", "--with-colons", PUBRING)
        self.assertEqual(listing.returncode, 0, listing.stderr)
        primaries = [l for l in listing.stdout.splitlines() if l.startswith("pub:")]
        self.assertEqual(len(primaries), 1)
        # A secret key by mistake would be a published private key.
        self.assertFalse([l for l in listing.stdout.splitlines()
                          if l.startswith("sec:")])


if __name__ == "__main__":
    unittest.main()
