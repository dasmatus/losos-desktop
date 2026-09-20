#!/usr/bin/env python3
"""Offline OCI regressions, including real local transport when skopeo exists."""

import contextlib
import hashlib
import importlib.machinery
import importlib.util
import io
import os
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
loader = importlib.machinery.SourceFileLoader("pm_oci", str(REPO / "tools/pm-oci"))
spec = importlib.util.spec_from_loader(loader.name, loader)
if spec is None or spec.loader is None:
    raise ImportError(f"cannot load OCI helper from {REPO / 'tools/pm-oci'}")
oci = importlib.util.module_from_spec(spec)
loader.exec_module(oci)


class OCITransportTests(unittest.TestCase):
    def setUp(self):
        self.root = REPO / "out" / (".test-oci-" + uuid.uuid4().hex)
        self.source = self.root / "source"
        self.output = self.root / "output"
        self.source.mkdir(parents=True)
        self.addCleanup(shutil.rmtree, self.root)
        self.archive = self.source / "losos-base-1.0.cpkg"
        # Deliberately not an archive: transport must never unpack the package.
        self.payload = b"opaque pm bytes\0\xff" * 100
        self.archive.write_bytes(self.payload)
        self.signature = self.archive.with_name(self.archive.name + ".sig")
        self.signature_payload = b"opaque detached pm signature"
        self.signature.write_bytes(self.signature_payload)
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
        self.assertFalse(list(self.source.glob(".pm-oci-*")))
        self.assertFalse(list(self.output.glob(".pm-oci-*")))
        return stdout.getvalue().strip(), stderr.getvalue()

    def publish(self, arch="x86_64"):
        return self.cli("publish", "--repository", "ghcr.io/owner/packages",
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
        """Re-pin mutations so pull must validate more than the outer digest."""
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

    def tar(self, names, kind=tarfile.REGTYPE):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w", format=tarfile.PAX_FORMAT) as archive:
            for name in names:
                member = tarfile.TarInfo(name)
                member.type = kind
                if kind == tarfile.REGTYPE:
                    member.size = len(self.payload)
                else:
                    member.linkname = "../../outside"
                archive.addfile(member, io.BytesIO(self.payload))
        return stream.getvalue()

    def test_roundtrip_both_architectures_and_image_shape(self):
        for arch in oci.ARCHES:
            with self.subTest(arch=arch):
                ref = self.publish(arch)
                self.assertRegex(ref, r"^ghcr.io/owner/packages/losos-base-1.0@sha256:[0-9a-f]{64}$")
                layout, _, manifest = self.image(ref)
                config = oci.read_json(layout / "blobs/sha256" / manifest["config"]["digest"][7:])
                self.assertEqual(config["architecture"], oci.ARCHES[arch])
                self.assertEqual(config["os"], "linux")
                self.assertEqual(set(config["config"]), {"Labels"})
                layer = layout / "blobs/sha256" / manifest["layers"][0]["digest"][7:]
                with tarfile.open(layer, "r:") as archive:
                    members = archive.getmembers()
                    self.assertEqual(len(members), 2)
                    self.assertEqual(members[0].name, "packages/" + self.archive.name)
                    self.assertTrue(members[0].isreg())
                    first = archive.extractfile(members[0])
                    self.assertIsNotNone(first)
                    self.assertEqual(first.read(), self.payload)
                    self.assertEqual(members[1].name, "packages/" + self.signature.name)
                    self.assertTrue(members[1].isreg())
                    second = archive.extractfile(members[1])
                    self.assertIsNotNone(second)
                    self.assertEqual(second.read(), self.signature_payload)
                self.pull(ref, arch)
                self.assertEqual((self.output / self.archive.name).read_bytes(), self.payload)
                self.assertEqual((self.output / self.signature.name).read_bytes(), self.signature_payload)
                (self.output / self.archive.name).unlink()
                (self.output / self.signature.name).unlink()

    @unittest.skipUnless(SKOPEO, "skopeo is unavailable; mocked transport tests still run")
    def test_real_skopeo_local_roundtrip(self):
        assert SKOPEO is not None
        for arch, oci_arch in oci.ARCHES.items():
            with self.subTest(arch=arch), oci.workspace(self.root) as work:
                layout, copied = work / "source", work / "copied"
                url = "https://github.com/owner/losos-desktop"
                digest = oci.make_layout(self.archive, layout, arch, url)
                digest_file = work / "digest"
                # Keep real subprocess.run before setUp's registry mock. This
                # copy stays entirely local: no registry, auth, or network.
                result = RUN([SKOPEO, "--override-arch", oci_arch, "copy",
                              "--preserve-digests", "--digestfile", str(digest_file),
                              f"oci:{layout}:package", f"oci:{copied}:package"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                self.assertEqual(digest_file.read_text().strip(), digest)
                output = work / "output"
                output.mkdir()
                restored = oci.restore(copied, digest, arch, work, output)
                self.assertEqual(restored.read_bytes(), self.payload)
                self.assertEqual((output / self.signature.name).read_bytes(), self.signature_payload)
                manifest = oci.read_json(copied / "blobs/sha256" / digest[7:])
                config = oci.read_json(
                    copied / "blobs/sha256" / manifest["config"]["digest"][7:])
                self.assertEqual(config["config"]["Labels"][oci.SOURCE_LABEL], url)
        self.assertFalse(list(self.root.glob(".pm-oci-*")))

    def test_multiple_archives_have_separate_images(self):
        (self.source / "losos-other-2.cpkg").write_bytes(b"other")
        (self.source / "losos-other-2.cpkg.sig").write_bytes(b"other signature")
        refs = self.publish().splitlines()
        self.assertEqual(len(refs), 2)
        self.assertEqual(len(self.registry), 2)
        for ref in refs:
            self.pull(ref)
        self.assertEqual((self.output / "losos-other-2.cpkg").read_bytes(), b"other")

    def test_optional_source_url_label_roundtrip(self):
        source = "https://github.com/owner/losos-desktop"
        ref = self.cli("publish", "--repository", "ghcr.io/owner/packages",
                       "--tag", "nightly-1.0-x86_64", "--arch", "x86_64",
                       "--source", self.source, "--source-url", source)[0]
        layout, _, manifest = self.image(ref)
        config = oci.read_json(layout / "blobs/sha256" / manifest["config"]["digest"][7:])
        self.assertEqual(config["config"]["Labels"][oci.SOURCE_LABEL], source)
        self.pull(ref)
        self.assertEqual((self.output / self.archive.name).read_bytes(), self.payload)
        self.assertEqual((self.output / self.signature.name).read_bytes(), self.signature_payload)

    def test_invalid_source_url_rejected(self):
        for source in ("not-a-url", "file:///etc/passwd", "******github.com/repo",
                       "https://github.com/repo?token=secret", "https://github.com/repo#fragment",
                       "https://github.com/repo\n"):
            self.cli("publish", "--repository", "ghcr.io/owner/packages",
                     "--tag", "valid", "--arch", "x86_64", "--source", self.source,
                     "--source-url", source, success=False)
        self.assertFalse(self.calls)
        ref = self.mutate(self.publish(), config_change=lambda c: c["config"]["Labels"].update(
            {oci.SOURCE_LABEL: "not-a-url"}))
        self.pull(ref, success=False)

    def test_long_filename_pax_roundtrip(self):
        self.archive.rename(self.source / ("a" * 180 + ".cpkg"))
        self.signature.rename(self.source / ("a" * 180 + ".cpkg.sig"))
        ref = self.publish()
        self.pull(ref)
        self.assertEqual((self.output / ("a" * 180 + ".cpkg")).read_bytes(), self.payload)

    def test_large_package_header_uses_pax_size(self):
        header = oci.package_header("large.cpkg", 10 * 1024**3)
        self.assertIn(b"size=10737418240\n", header)

    def test_streaming_and_empty_packages(self):
        for payload in (b"x" * (oci.CHUNK * 2 + 41), b""):
            self.archive.write_bytes(payload)
            self.pull(self.publish())
            target = self.output / self.archive.name
            self.assertEqual(target.read_bytes(), payload)
            target.unlink()
            (self.output / self.signature.name).unlink()

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
        self.assertFalse(self.calls)
        with self.assertRaises(oci.OCIError):
            oci.repository("ghcr.io:99999/owner/image")

    def test_bad_publish_inputs(self):
        for repo, tag in (("https://ghcr.io/owner", "valid"),
                          ("ghcr.io/Owner/repo", "valid"),
                          ("ghcr.io/owner/repo", "bad/tag"),
                          ("ghcr.io/owner/repo", "")):
            self.cli("publish", "--repository", repo, "--tag", tag,
                     "--arch", "x86_64", "--source", self.source, success=False)
        for name in ("Upper.cpkg", ".hidden.cpkg", "bad name.cpkg", "bad;name.cpkg"):
            bad = self.source / name
            bad.write_bytes(b"bad")
            self.cli("publish", "--repository", "ghcr.io/owner/repo", "--tag", "valid",
                     "--arch", "x86_64", "--source", self.source, success=False)
            bad.unlink()
        self.assertFalse(self.calls)

    def test_symlink_archive_and_empty_source(self):
        self.archive.unlink()
        self.archive.symlink_to(self.root / "missing")
        self.cli("publish", "--repository", "ghcr.io/owner/repo", "--tag", "valid",
                 "--arch", "x86_64", "--source", self.source, success=False)
        self.archive.unlink()
        self.cli("publish", "--repository", "ghcr.io/owner/repo", "--tag", "valid",
                 "--arch", "x86_64", "--source", self.source, success=False)
        self.assertFalse(self.calls)

    def test_missing_or_nonregular_signature(self):
        self.signature.unlink()
        for kind in ("missing", "symlink", "directory"):
            with self.subTest(kind=kind):
                if kind == "symlink":
                    self.signature.symlink_to(self.archive)
                elif kind == "directory":
                    self.signature.mkdir()
                self.cli("publish", "--repository", "ghcr.io/owner/repo", "--tag", "valid",
                         "--arch", "x86_64", "--source", self.source, success=False)
                if kind == "symlink":
                    self.signature.unlink()
        self.assertFalse(self.calls)

    def test_manifest_config_and_layer_corruption(self):
        for blob in ("manifest", "config", "layer"):
            with self.subTest(blob=blob):
                ref = self.publish()
                layout, index, manifest = self.image(ref)
                desc = (index["manifests"][0] if blob == "manifest" else
                        manifest["config"] if blob == "config" else manifest["layers"][0])
                path = layout / "blobs/sha256" / desc["digest"][7:]
                raw = bytearray(path.read_bytes())
                raw[len(raw) // 2] ^= 1
                path.write_bytes(raw)
                self.pull(ref, success=False)
                self.assertFalse((self.output / self.archive.name).exists())

    def test_descriptor_sizes_and_media_types(self):
        for field in ("config", "layers"):
            for key, value in (("size", 0), ("mediaType", "application/octet-stream"),
                               ("digest", "sha256:../../escape"), ("urls", ["https://invalid/"])):
                with self.subTest(field=field, key=key):
                    def change(manifest):
                        desc = manifest[field] if field == "config" else manifest[field][0]
                        desc[key] = value
                    self.pull(self.mutate(self.publish(), manifest_change=change), success=False)

    def test_wrong_manifest_pin_and_wrong_architecture(self):
        ref = self.publish()
        wrong = ref.split("@")[0] + "@sha256:" + "0" * 64
        self.registry[wrong] = self.registry[ref]
        self.pull(wrong, success=False)
        self.pull(ref, "aarch64", success=False)

    def test_config_and_package_labels(self):
        changes = [
            lambda c: c.update(os="windows"),
            lambda c: c["config"].update(Entrypoint=["sh"]),
            lambda c: c["rootfs"].update(diff_ids=["sha256:" + "0" * 64]),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "format": "unknown"}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "filename": "../escape.cpkg"}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "sha256": "0" * 64}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "size": "-1"}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "size": "999"}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "signature-sha256": "0" * 64}),
            lambda c: c["config"]["Labels"].update({oci.LABEL + "signature-size": "999"}),
            lambda c: c["config"]["Labels"].pop(oci.LABEL + "signature-sha256"),
        ]
        for change in changes:
            self.pull(self.mutate(self.publish(), config_change=change), success=False)
            self.assertFalse((self.output / self.archive.name).exists())

    def test_extra_layers_rejected(self):
        ref = self.mutate(self.publish(), manifest_change=lambda m: m["layers"].append(m["layers"][0]))
        self.pull(ref, success=False)

    def test_compressed_layers_rejected(self):
        def compressed(manifest):
            manifest["layers"][0]["mediaType"] += "+gzip"
        self.pull(self.mutate(self.publish(), manifest_change=compressed), success=False)

    def test_blob_symlink_rejected(self):
        ref = self.publish()
        layout, _, manifest = self.image(ref)
        path = layout / "blobs/sha256" / manifest["layers"][0]["digest"][7:]
        external = self.root / "external-layer"
        path.rename(external)
        path.symlink_to(external)
        original_copy = self.copy

        def preserve_links(command, **kwargs):
            if command[-2].startswith("docker://"):
                destination = Path(command[-1][4:].rsplit(":", 1)[0])
                shutil.copytree(layout, destination, symlinks=True)
                return subprocess.CompletedProcess(command, 0)
            return original_copy(command, **kwargs)

        with mock.patch.object(oci.subprocess, "run", side_effect=preserve_links):
            self.pull(ref, success=False)

    def test_unsafe_tar_members_and_formats(self):
        name = "packages/" + self.archive.name
        cases = [
            self.tar(["../escape.cpkg"]),
            self.tar(["/absolute.cpkg"]),
            self.tar([name, name]),
            self.tar([name], tarfile.SYMTYPE),
            self.tar([name], tarfile.LNKTYPE),
            self.tar([name], tarfile.DIRTYPE),
            self.tar([name], tarfile.FIFOTYPE),
        ]
        # Equal-length mutations reach the header and trailer checks, rather
        # than being rejected just by the descriptor's overall layer length.
        header = oci.package_header(self.archive.name, len(self.payload))
        padding = (b"\0" * (-len(self.payload) % 512)
                   + oci.package_header(self.signature.name, len(self.signature_payload))
                   + self.signature_payload
                   + b"\0" * ((-len(self.signature_payload) % 512) + 1024))
        cases.extend([
            oci.package_header("x" * len(self.archive.stem) + ".cpkg", len(self.payload))
            + self.payload + padding,
            header + self.payload + padding[:-1] + b"x",
            b"not a tar" + b"\0" * (len(header) - 9) + self.payload + padding,
        ])
        for layer in cases:
            self.pull(self.mutate(self.publish(), layer_bytes=layer), success=False)
            self.assertFalse((self.output / self.archive.name).exists())
        self.assertFalse((self.root / "escape.cpkg").exists())

    def test_refuse_existing_file_and_symlink(self):
        ref = self.publish()
        self.output.mkdir()
        target = self.output / self.archive.name
        target.write_bytes(b"keep me")
        self.pull(ref, success=False)
        self.assertEqual(target.read_bytes(), b"keep me")
        target.unlink()
        target.symlink_to(self.root / "absent")
        self.pull(ref, success=False)
        self.assertTrue(target.is_symlink())
        self.assertFalse((self.root / "absent").exists())

    def test_atomic_no_replace_race(self):
        ref = self.publish()
        original_link = os.link

        def race(source, target, **kwargs):
            if str(target).endswith(".cpkg"):
                self.assertEqual((self.output / self.signature.name).read_bytes(),
                                 self.signature_payload)
                Path(target).write_bytes(b"concurrent writer")
            original_link(source, target, **kwargs)

        with mock.patch.object(oci.os, "link", side_effect=race):
            self.pull(ref, success=False)
        self.assertEqual((self.output / self.archive.name).read_bytes(), b"concurrent writer")
        self.assertFalse((self.output / self.signature.name).exists())

    def test_existing_signature_refused_before_package_publication(self):
        ref = self.publish()
        self.output.mkdir()
        signature = self.output / self.signature.name
        signature.write_bytes(b"existing signature")
        self.pull(ref, success=False)
        self.assertEqual(signature.read_bytes(), b"existing signature")
        self.assertFalse((self.output / self.archive.name).exists())
        signature.unlink()
        signature.symlink_to(self.root / "absent")
        self.pull(ref, success=False)
        self.assertTrue(signature.is_symlink())
        self.assertFalse((self.output / self.archive.name).exists())

    def test_signature_is_rolled_back_after_package_io_failure(self):
        ref = self.publish()
        original_link = os.link

        def failure(source, target, **kwargs):
            if str(target).endswith(".cpkg"):
                raise OSError("simulated I/O failure")
            original_link(source, target, **kwargs)

        with mock.patch.object(oci.os, "link", side_effect=failure):
            self.pull(ref, success=False)
        self.assertFalse((self.output / self.archive.name).exists())
        self.assertFalse((self.output / self.signature.name).exists())

    def test_signature_corruption_with_valid_layer_hash(self):
        ref = self.publish()
        layout, _, manifest = self.image(ref)
        path = layout / "blobs/sha256" / manifest["layers"][0]["digest"][7:]
        layer = path.read_bytes().replace(self.signature_payload, b"x" * len(self.signature_payload))
        self.pull(self.mutate(ref, layer_bytes=layer), success=False)
        self.assertFalse((self.output / self.archive.name).exists())
        self.assertFalse((self.output / self.signature.name).exists())

    def test_signature_replaced_concurrently_is_not_rolled_back(self):
        ref = self.publish()
        original_link = os.link

        def replace(source, target, **kwargs):
            if str(target).endswith(".cpkg"):
                signature = self.output / self.signature.name
                signature.unlink()
                signature.write_bytes(b"concurrent signature")
                raise OSError("simulated I/O failure")
            original_link(source, target, **kwargs)

        with mock.patch.object(oci.os, "link", side_effect=replace):
            self.pull(ref, success=False)
        self.assertEqual((self.output / self.signature.name).read_bytes(), b"concurrent signature")
        self.assertFalse((self.output / self.archive.name).exists())

    def test_transport_failure_and_missing_tool_cleanup(self):
        ref = self.publish()
        for error in (subprocess.CalledProcessError(9, ["skopeo"]), FileNotFoundError()):
            with mock.patch.object(oci.subprocess, "run", side_effect=error):
                self.cli("publish", "--repository", "ghcr.io/owner/repo", "--tag", "valid",
                         "--arch", "x86_64", "--source", self.source, success=False)
                self.pull(ref, success=False)

    def test_publish_requires_preserved_digest(self):
        original_copy = self.copy

        def wrong_digest(command, **kwargs):
            result = original_copy(command, **kwargs)
            Path(command[command.index("--digestfile") + 1]).write_text("sha256:" + "0" * 64)
            return result

        with mock.patch.object(oci.subprocess, "run", side_effect=wrong_digest):
            output, _ = self.cli("publish", "--repository", "ghcr.io/owner/repo",
                                 "--tag", "valid", "--arch", "x86_64",
                                 "--source", self.source, success=False)
        self.assertEqual(output, "")

    def test_duplicate_json_keys_rejected(self):
        ref = self.publish()
        layout = self.registry[ref]
        (layout / "index.json").write_text('{"schemaVersion":2,"schemaVersion":2}')
        self.pull(ref, success=False)

    def test_registry_auth_environment_is_not_replaced(self):
        with mock.patch.dict(os.environ, {"REGISTRY_AUTH_FILE": "host-auth-file"}):
            self.pull(self.publish())
            self.assertEqual(os.environ["REGISTRY_AUTH_FILE"], "host-auth-file")


if __name__ == "__main__":
    unittest.main()
