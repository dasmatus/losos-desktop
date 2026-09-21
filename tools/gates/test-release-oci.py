#!/usr/bin/env python3
"""Offline OCI regressions for release-directory transport."""

import contextlib
import hashlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import shutil
import subprocess
import tarfile
import unittest
from unittest import mock
import uuid

REPO = Path(__file__).resolve().parents[2]
SKOPEO = shutil.which("skopeo")
RUN = subprocess.run
loader = importlib.machinery.SourceFileLoader("release_oci", str(REPO / "tools/release-oci"))
spec = importlib.util.spec_from_loader(loader.name, loader)
if spec is None or spec.loader is None:
    raise ImportError(f"cannot load OCI helper from {REPO / 'tools/release-oci'}")
oci = importlib.util.module_from_spec(spec)
loader.exec_module(oci)


class ReleaseOCITransportTests(unittest.TestCase):
    def setUp(self):
        self.root = REPO / "out" / (".test-release-oci-" + uuid.uuid4().hex)
        self.source = self.root / "source"
        self.output = self.root / "output"
        self.source.mkdir(parents=True)
        self.output.mkdir()
        self.addCleanup(shutil.rmtree, self.root)
        self.files = {
            "SHA256SUMS": b"manifest\n",
            "losos-desktop_1.0_x86_64.efi": b"uki",
            "losos-desktop_1.0_root-x86-64.raw.xz": b"root",
            "losos-desktop_1.0_x86_64.iso": b"iso",
            "losos-desktop_1.0_x86_64.qcow2": b"qcow2",
        }
        for name, payload in self.files.items():
            (self.source / name).write_bytes(payload)
        self.registry = {}
        self.calls = []
        patcher = mock.patch.object(oci.subprocess, "run", side_effect=self.copy)
        patcher.start()
        self.addCleanup(patcher.stop)

    def copy(self, command, **kwargs):
        self.calls.append(command)
        self.assertEqual(command[:2], ["skopeo", "--override-arch"])
        self.assertIn(command[2], ("amd64", "arm64"))
        self.assertEqual(command[3:5], ["copy", "--preserve-digests"])
        self.assertTrue(kwargs["check"])
        self.assertNotIn("shell", kwargs)
        self.assertNotIn("env", kwargs)
        self.assertFalse(any("tls-verify" in part for part in command))
        source, target = command[-2:]
        if source.startswith("oci:"):
            layout = Path(source[4:].rsplit(":", 1)[0])
            digest = oci.read_json(layout / "index.json")["manifests"][0]["digest"]
            ref = target.removeprefix("docker://").rsplit(":", 1)[0] + "@" + digest
            stored = self.root / ("registry-" + uuid.uuid4().hex)
            shutil.copytree(layout, stored)
            self.registry[ref] = stored
            Path(command[command.index("--digestfile") + 1]).write_text(digest)
        else:
            layout = Path(target[4:].rsplit(":", 1)[0])
            shutil.copytree(self.registry[source.removeprefix("docker://")], layout)
        return subprocess.CompletedProcess(command, 0)

    def cli(self, *arguments, success=True):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = oci.main(list(map(str, arguments)))
        self.assertEqual(status, 0 if success else 1, stderr.getvalue())
        self.assertFalse(list(self.source.glob(".release-oci-*")))
        self.assertFalse(list(self.output.glob(".release-oci-*")))
        return stdout.getvalue().strip(), stderr.getvalue()

    def publish(self, arch="x86_64"):
        return self.cli("publish", "--repository", "ghcr.io/owner/images",
                        "--tag", f"nightly-1.0-{arch}", "--arch", arch,
                        "--source", self.source)[0]

    def pull(self, ref, arch="x86_64", success=True):
        return self.cli("pull", ref, "--arch", arch, "--output", self.output,
                        success=success)

    def image(self, ref):
        layout = self.registry[ref]
        index = oci.read_json(layout / "index.json")
        desc = index["manifests"][0]
        manifest = oci.read_json(layout / "blobs/sha256" / desc["digest"][7:])
        return layout, index, manifest

    def mutate(self, ref, config_change=None, manifest_change=None, layer_bytes=None):
        layout, index, manifest = self.image(ref)
        config = oci.read_json(layout / "blobs/sha256" / manifest["config"]["digest"][7:])
        if layer_bytes is not None:
            digest = "sha256:" + hashlib.sha256(layer_bytes).hexdigest()
            (layout / "blobs/sha256" / digest[7:]).write_bytes(layer_bytes)
            manifest["layers"] = [oci.descriptor(oci.LAYER, digest, len(layer_bytes))]
            config["rootfs"]["diff_ids"] = [digest]
        if config_change:
            config_change(config)
        manifest["config"] = oci.put_json(layout, oci.CONFIG, config)
        if manifest_change:
            manifest_change(manifest)
        desc = oci.put_json(layout, oci.MANIFEST, manifest)
        index["manifests"] = [desc]
        (layout / "index.json").write_bytes(oci.json_bytes(index))
        new_ref = ref.split("@")[0] + "@" + desc["digest"]
        self.registry[new_ref] = layout
        return new_ref

    def test_roundtrip_both_architectures_and_image_shape(self):
        for arch in oci.ARCHES:
            with self.subTest(arch=arch):
                ref = self.publish(arch)
                self.assertRegex(ref, r"^ghcr.io/owner/images@sha256:[0-9a-f]{64}$")
                layout, _, manifest = self.image(ref)
                config = oci.read_json(layout / "blobs/sha256" / manifest["config"]["digest"][7:])
                self.assertEqual(config["architecture"], oci.ARCHES[arch])
                self.assertEqual(config["os"], "linux")
                self.assertEqual(set(config["config"]), {"Labels"})
                listed = {
                    entry["name"]: self.files[entry["name"]]
                    for entry in self.assert_manifest(config["config"]["Labels"])
                }
                self.assertEqual(listed, self.files)
                layer = layout / "blobs/sha256" / manifest["layers"][0]["digest"][7:]
                with tarfile.open(layer, "r:") as archive:
                    members = archive.getmembers()
                    self.assertEqual([member.name for member in members],
                                     [f"release/{name}" for name in sorted(self.files)])
                self.pull(ref, arch)
                for name, payload in self.files.items():
                    self.assertEqual((self.output / name).read_bytes(), payload)
                    (self.output / name).unlink()

    def assert_manifest(self, labels):
        manifest = labels[oci.LABEL + "manifest"]
        entries = oci.json.loads(manifest)
        self.assertEqual([entry["name"] for entry in entries], sorted(self.files))
        for entry in entries:
            self.assertEqual(entry["sha256"], hashlib.sha256(self.files[entry["name"]]).hexdigest())
            self.assertEqual(entry["size"], len(self.files[entry["name"]]))
        return entries

    @unittest.skipUnless(SKOPEO, "skopeo is unavailable; mocked transport tests still run")
    def test_real_skopeo_local_roundtrip(self):
        if SKOPEO is None:
            self.skipTest("skopeo is unavailable; mocked transport tests still run")
        for arch, oci_arch in oci.ARCHES.items():
            with self.subTest(arch=arch), oci.workspace(self.root) as work:
                layout, copied = work / "source", work / "copied"
                url = "https://github.com/owner/losos-desktop"
                digest = oci.make_layout(self.source, layout, arch, url)
                digest_file = work / "digest"
                result = RUN([SKOPEO, "--override-arch", oci_arch, "copy",
                              "--preserve-digests", "--digestfile", str(digest_file),
                              f"oci:{layout}:release", f"oci:{copied}:release"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                self.assertEqual(digest_file.read_text().strip(), digest)
                output = work / "output"
                output.mkdir()
                oci.restore(copied, digest, arch, work, output)
                for name, payload in self.files.items():
                    self.assertEqual((output / name).read_bytes(), payload)
                manifest = oci.read_json(copied / "blobs/sha256" / digest[7:])
                config = oci.read_json(
                    copied / "blobs/sha256" / manifest["config"]["digest"][7:])
                self.assertEqual(config["config"]["Labels"][oci.SOURCE_LABEL], url)
        self.assertFalse(list(self.root.glob(".release-oci-*")))

    def test_optional_source_url_label_roundtrip(self):
        source = "https://github.com/owner/losos-desktop"
        ref = self.cli("publish", "--repository", "ghcr.io/owner/images",
                       "--tag", "nightly-1.0-x86_64", "--arch", "x86_64",
                       "--source", self.source, "--source-url", source)[0]
        layout, _, manifest = self.image(ref)
        config = oci.read_json(layout / "blobs/sha256" / manifest["config"]["digest"][7:])
        self.assertEqual(config["config"]["Labels"][oci.SOURCE_LABEL], source)
        self.pull(ref)
        for name, payload in self.files.items():
            self.assertEqual((self.output / name).read_bytes(), payload)

    def test_invalid_source_url_rejected(self):
        for source in ("not-a-url", "file:///etc/passwd", "******github.com/repo",
                       "https://github.com/repo?token=secret", "https://github.com/repo#fragment",
                       "https://github.com/repo\n"):
            self.cli("publish", "--repository", "ghcr.io/owner/images",
                     "--tag", "valid", "--arch", "x86_64", "--source", self.source,
                     "--source-url", source, success=False)
        self.assertFalse(self.calls)
        ref = self.mutate(self.publish(), config_change=lambda c: c["config"]["Labels"].update(
            {oci.SOURCE_LABEL: "not-a-url"}))
        self.pull(ref, success=False)

    def test_unsafe_references_and_tags(self):
        for ref in ("ghcr.io/owner/image:latest", "docker://ghcr.io/owner/image",
                    "ghcr.io/owner/../image@" + "sha256:" + "a" * 64,
                    "ghcr.io/owner/image@sha512:" + "a" * 64,
                    "ghcr.io/owner/image@sha256:" + "A" * 64,
                    "ghcr.io/owner/image:tag@sha256:" + "a" * 64,
                    "--help", "ghcr.io/owner/image@sha256:bad"):
            with self.subTest(ref=ref):
                if ref.startswith("--"):
                    with self.assertRaises(oci.OCIError):
                        oci.reference(ref)
                else:
                    self.pull(ref, success=False)
        for tag in ("bad/tag", ""):
            with self.subTest(tag=tag):
                self.cli("publish", "--repository", "ghcr.io/owner/repo",
                         "--tag", tag, "--arch", "x86_64", "--source", self.source,
                         success=False)

    def test_rejects_symlinks_and_directories(self):
        (self.source / "subdir").mkdir()
        self.cli("publish", "--repository", "ghcr.io/owner/images",
                 "--tag", "valid", "--arch", "x86_64", "--source", self.source,
                 success=False)
        shutil.rmtree(self.source / "subdir")
        (self.source / "pointer").symlink_to("SHA256SUMS")
        self.cli("publish", "--repository", "ghcr.io/owner/images",
                 "--tag", "valid", "--arch", "x86_64", "--source", self.source,
                 success=False)

    def test_refuses_existing_targets(self):
        ref = self.publish()
        (self.output / "SHA256SUMS").write_text("existing")
        self.pull(ref, success=False)

    def test_restore_cleans_partial_candidates_on_failure(self):
        ref = self.publish()
        layout, _, manifest = self.image(ref)
        layer_path = layout / "blobs/sha256" / manifest["layers"][0]["digest"][7:]
        broken = self.mutate(ref, layer_bytes=layer_path.read_bytes()[:-600])
        work = self.root / "manual-work"
        work.mkdir()
        with self.assertRaises(oci.OCIError):
            oci.restore(layout, broken.split("@", 1)[1], "x86_64", work, self.output)
        self.assertFalse(list(work.iterdir()))
        self.assertFalse(list(self.output.iterdir()))


if __name__ == "__main__":
    unittest.main()
