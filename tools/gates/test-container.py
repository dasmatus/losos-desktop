#!/usr/bin/env python3
"""Check container reuse without a registry, daemon or real build."""

import argparse
import importlib.machinery
import importlib.util
import os
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
loader = importlib.machinery.SourceFileLoader("container", str(REPO / "tools/container"))
spec = importlib.util.spec_from_loader(loader.name, loader)
container = importlib.util.module_from_spec(spec)
loader.exec_module(container)


class ContainerTests(unittest.TestCase):
    def setUp(self):
        self.env = patch.dict(os.environ, {}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def test_default_image(self):
        self.assertEqual(container.image(), "ghcr.io/dasmatus/losos-desktop/build-host:latest")

    def test_image_override(self):
        for image in ("localhost/losos-build", "ghcr.io/example/host@sha256:" + "a" * 64):
            with self.subTest(image=image), patch.dict(os.environ, {"LOSOS_CONTAINER_IMAGE": image}):
                self.assertEqual(container.image(), image)

    def test_cached_image_never_pulls_or_builds(self):
        for engine in ("docker", "podman"):
            with self.subTest(engine=engine), patch.object(container.subprocess, "call", return_value=0) as call:
                self.assertEqual(container.ensure_image(engine), 0)
                call.assert_called_once_with(
                    [engine, "image", "inspect", container.IMAGE],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                )

    def test_missing_image_pulls(self):
        for engine in ("docker", "podman"):
            with self.subTest(engine=engine), patch.object(container.subprocess, "call", side_effect=[1, 0]) as call:
                self.assertEqual(container.ensure_image(engine), 0)
                self.assertEqual(call.call_count, 2)
                self.assertEqual(call.call_args.args[0], [engine, "pull", container.IMAGE])

    def test_pull_failure_propagates_without_build(self):
        with patch.object(container.subprocess, "call", side_effect=[1, 42]) as call:
            self.assertEqual(container.ensure_image("docker"), 42)
            self.assertEqual(call.call_count, 2)

    def test_explicit_pull_refreshes_selected_image(self):
        with patch.dict(os.environ, {"LOSOS_CONTAINER_IMAGE": "ghcr.io/example/host:fixed"}):
            with patch.object(container.subprocess, "call", return_value=0) as call:
                self.assertEqual(container.pull("podman", None), 0)
                call.assert_called_once_with(["podman", "pull", "ghcr.io/example/host:fixed"])

    def test_explicit_build_keeps_architecture_selection(self):
        for machine, base in container.BASE_IMAGES.items():
            with self.subTest(machine=machine), patch.object(container.platform, "machine", return_value=machine):
                with patch.dict(os.environ, {"LOSOS_CONTAINER_IMAGE": "localhost/test"}):
                    with patch.object(container.subprocess, "call", return_value=0) as call:
                        container.build("docker", argparse.Namespace(build_arg=[]))
                        call.assert_called_once_with([
                            "docker", "build", "-t", "localhost/test", "-f", str(REPO / "Containerfile"),
                            "--build-arg", f"BASE_IMAGE={base}", str(REPO),
                        ])

    def test_explicit_base_override(self):
        with patch.object(container, "base_image") as base:
            with patch.object(container.subprocess, "call", return_value=0) as call:
                container.build("podman", argparse.Namespace(build_arg=["BASE_IMAGE=custom:base"]))
                base.assert_not_called()
                self.assertIn("BASE_IMAGE=custom:base", call.call_args.args[0])

    def test_run_preserves_mounts_and_engine_options(self):
        for engine, option in (("docker", "systempaths=unconfined"), ("podman", "unmask=ALL")):
            with self.subTest(engine=engine), patch.object(container, "pm_root", return_value=REPO):
                with patch.object(container.sys.stdin, "isatty", return_value=False):
                    with patch.dict(os.environ, {
                        "PM": "/tmp/custom-pm/pm", "LOSOS_CONTAINER_IMAGE": "localhost/test",
                        "LOSOS_CONTAINER_HOST_NETWORK": "1",
                    }):
                        with patch.object(container.subprocess, "call", side_effect=[0, 0]) as call:
                            self.assertEqual(container.run(engine, argparse.Namespace(command=["--", "just", "check"])), 0)
                            command = call.call_args.args[0]
                            self.assertEqual(command[-3:], ["localhost/test", "just", "check"])
                            for argument in (option, f"{REPO}:{REPO}", "/tmp/custom-pm:/tmp/custom-pm",
                                             "PM=/tmp/custom-pm/pm", "--network", "host"):
                                self.assertIn(argument, command)

    def test_run_stops_on_failed_pull(self):
        with patch.object(container, "pm_root", return_value=REPO):
            with patch.object(container.subprocess, "call", side_effect=[1, 7]) as call:
                self.assertEqual(container.run("docker", argparse.Namespace(command=[])), 7)
                self.assertEqual(call.call_count, 2)

    def test_pull_subcommand(self):
        with patch.object(container.sys, "argv", ["container", "pull"]):
            with patch.object(container, "runtime", return_value="docker"):
                with patch.object(container.subprocess, "call", return_value=0) as call:
                    self.assertEqual(container.main(), 0)
                    call.assert_called_once_with(["docker", "pull", container.IMAGE])

    def test_fallback_does_not_build_host(self):
        import re
        justfile = (REPO / "Justfile").read_text()
        match = re.search(r"(?ms)^[ \t]*build_in_container\(\) \{\n(.*?)^[ \t]*\}[ \t]*$", justfile)
        self.assertIsNotNone(match, "build_in_container() not found in Justfile")
        fallback = match.group(1)
        self.assertNotRegex(fallback, r"\btools/container\"\s+build\b")
        self.assertRegex(fallback, r"\btools/container\"\s+run\b")


class PublicationTests(unittest.TestCase):
    def setUp(self):
        # BaseLoader preserves GitHub's `on` key rather than YAML 1.1's boolean.
        self.workflow = yaml.load(
            (REPO / ".github/workflows/container.yml").read_text(), Loader=yaml.BaseLoader,
        )

    def test_only_main_can_publish(self):
        self.assertEqual(set(self.workflow["on"]), {"push", "workflow_dispatch"})
        self.assertEqual(self.workflow["on"]["push"]["branches"], ["main"])
        for job in self.workflow["jobs"].values():
            self.assertEqual(job["if"], "github.ref == 'refs/heads/main'")
            self.assertEqual(job["permissions"]["packages"], "write")
        self.assertEqual(self.workflow["permissions"], {"contents": "read"})

    def test_manifest_waits_for_both_native_builds(self):
        jobs = self.workflow["jobs"]
        self.assertEqual(jobs["publish"]["needs"], "build")
        matrix = jobs["build"]["strategy"]["matrix"]["include"]
        self.assertEqual(
            {(leg["runner"], leg["arch"]) for leg in matrix},
            {("ubuntu-24.04", "amd64"), ("ubuntu-24.04-arm", "arm64")},
        )
        steps = jobs["publish"]["steps"]
        commands = "\n".join(step.get("run", "") for step in steps)
        for arch in ("amd64", "arm64"):
            self.assertIn(f"run-${{GITHUB_RUN_ID}}-{arch}", commands)
        self.assertIn('"docker://${image}:sha-${GITHUB_SHA}"', commands)
        self.assertIn('"docker://${image}:latest"', commands)
        # Failed-job retries keep successful legs from the original attempt.
        self.assertNotIn("GITHUB_RUN_ATTEMPT", commands)

    def test_build_is_tested_before_push(self):
        steps = self.workflow["jobs"]["build"]["steps"]
        names = [step.get("name") for step in steps]
        self.assertLess(names.index("Smoke test the build tools"), names.index("Push the native image"))
        for step in steps:
            if step.get("uses", "").startswith("actions/checkout@"):
                self.assertEqual(step["with"]["persist-credentials"], "false")
        self.assertEqual(self.workflow["concurrency"]["cancel-in-progress"], "false")


if __name__ == "__main__":
    unittest.main()
