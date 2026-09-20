#!/usr/bin/env python3
"""Prove image-only generation preserves the package closure and rebuild settings."""

import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent


def main():
    layers = yaml.safe_load((REPO / "manifest/layers.yaml").read_text())
    package, image = layers[-2]["name"], layers[-1]["name"]
    with tempfile.TemporaryDirectory(prefix="losos-prebuilt-") as temporary:
        work = Path(temporary)
        out = work / "out"
        archive = work / f"{package}-0.1.0.cpkg"
        archive.write_bytes(b"package closure fixture")
        command = [sys.executable, str(REPO / "tools/configure"), "--out", str(out)]

        def configure(*args, success=True):
            result = subprocess.run(command + list(args), capture_output=True, text=True)
            assert (result.returncode == 0) == success, result.stdout + result.stderr
            return result

        def recipe(name):
            return yaml.safe_load((out / "recipes" / name / "build.yaml").read_text())

        configure("--allow-unresolved")
        original = {layer["name"]: recipe(layer["name"]) for layer in layers}
        assert original[image]["dependencies"] == [f"../recipes/{package}/build.yaml"]
        configure("--allow-unresolved", "--arch=aarch64", "--channel=nightly",
                  "--version=20260920.1", "--prebuilt-packages", str(archive))
        assembled = recipe(image)
        assert assembled["dependencies"] == []
        assert assembled["steps"][0]["run"][:2] == [
            "mkdir -p /dest/deps",
            f"cp {out}/recipes/{image}/{archive.name} /dest/deps/{archive.name}",
        ]
        for layer in layers[:-1]:
            name = layer["name"]
            assert recipe(name)["dependencies"] == original[name]["dependencies"]
        staged = out / "recipes" / image / archive.name
        assert staged.read_bytes() == archive.read_bytes()
        remembered = (out / "configure.args").read_text().splitlines()
        assert f"--prebuilt-packages={archive}" in remembered
        assert "--arch=aarch64" in remembered
        assert "--version=20260920.1" in remembered
        configure(*remembered)
        assert recipe(image) == assembled
        assert staged.read_bytes() == archive.read_bytes()

        # An input in the disposable tree must fail BEFORE configure deletes it.
        configure("--allow-unresolved", "--prebuilt-packages", str(staged), success=False)
        assert staged.exists()
        configure("--allow-unresolved", "--prebuilt-packages",
                  str(work / "wrong.cpkg"), success=False)
        archive.unlink()
        configure(*remembered, success=False)
        archive.touch()
        configure(*remembered, success=False)
        configure("--allow-unresolved")
        assert recipe(image) == original[image]
        assert not staged.exists()
    print("prebuilt: image handoff, replay, invalid inputs and ordinary builds pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
