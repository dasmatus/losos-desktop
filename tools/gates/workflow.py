#!/usr/bin/env python3
"""Check that the release workflow's artifact names agree across jobs.

The bug this exists for shipped silently and would only have surfaced at a
tagged release. `build` named its artifact after a channel it derived from the
event; `verify` and `ota` hardcoded `losos-nightly-<arch>`. On a tag the build
uploaded `losos-stable-<arch>`, the download asked for a name that did not
exist, download-artifact failed, and `publish` -- which needs both -- never ran.
Nightly was green throughout, so nothing pointed at it until someone tagged a
release and found the stable path had never worked.

Three properties, all cheap to check and none visible by reading one job:

  * every artifact a job downloads is one some job uploads, for EVERY value the
    channel can take;
  * the channel is derived in exactly one place. Two derivations that agree
    today are two that can disagree after one edit;
  * so is the pm commit, in every workflow that checks pm out, for the same
    reason and one worse consequence. It is what .github/actions/pm keys pm's
    build cache on, so a copy someone bumps while another stays put is not a
    disagreement about a version -- it is a cached binary, built from a tree
    nobody reviewed, handed to every job below without a word. The `master`
    that this pin replaced is the same failure arriving the slow way.

It also checks that write permission is not granted workflow-wide, and that one
applies to EVERY workflow here rather than to this one. Both of them check out
a ref and execute the tree's own code from it -- images.yml on `pull_request`,
update-sources.yml on a `workflow_dispatch` against any ref -- and a job running
tree-supplied code has no business holding a token that can write to the
repository. The job that genuinely publishes asks for the permission itself.

Every job also runs in an Arch userspace. The hosted runner labels still name
Ubuntu because GitHub provides the VM, not an Arch runner; the job container
is what decides which distribution executes the steps.

pm's Wasmtime dependency also sets a Rust floor. The container and the CI
setup must move together when pm moves, or one builds while the other cannot.
"""

import copy
import re
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = REPO / ".github" / "workflows"
# The release workflow, which is the only one with artifacts to agree about.
WORKFLOW = WORKFLOWS / "images.yml"

# The channel values the workflow can produce. Written here rather than parsed
# out of the shell, because this is the list the check is asserting against.
CHANNELS = ("nightly", "stable")
CHANNEL_REF = re.compile(r"\$\{\{\s*needs\.channel\.outputs\.name\s*\}\}")

# The pm pin: the one expression allowed to stand for it, and the shape the
# value itself has to have. A branch name resolves to whatever it points at
# today, which is exactly what a cache key must not do.
PM_REPO = "dichhead/pm"
PM_ACTION = ".github/actions/pm"
PM_REF_USE = re.compile(r"\$\{\{\s*env\.PM_REF\s*\}\}")
COMMIT = re.compile(r"\A[0-9a-f]{40}\Z")

ARCH_IMAGES = {
    "ubuntu-24.04": "docker.io/library/archlinux:base",
    "ubuntu-24.04-arm": "docker.io/menci/archlinuxarm:base",
}


def check_package_handoff(doc):
    """Reject `needs` references GitHub would treat as an invalid workflow."""
    jobs = doc.get("jobs") or {}
    names = set(jobs)
    failures = []
    for job, spec in jobs.items():
        needs = (spec or {}).get("needs") or []
        if isinstance(needs, str):
            needs = [needs]
        for need in needs:
            if need not in names:
                failures.append(
                    f"images.yml: {job} needs {need!r}, which is not a job.\n"
                    "    GitHub rejects the whole workflow before any job starts."
                )
    return failures


def check_arch_jobs(path, doc):
    """Keep runner architecture and job userspace paired, including matrices."""
    failures = []
    for job, spec in (doc.get("jobs") or {}).items():
        where = f"{path.name}: {job}"
        container = spec.get("container") or {}
        image = container.get("image") if isinstance(container, dict) else container
        matrix = (spec.get("strategy") or {}).get("matrix") or {}
        legs = matrix.get("include") or [{}]
        for leg in legs:
            runner = spec.get("runs-on")
            resolved = image
            if runner == "${{ matrix.runner }}":
                runner = leg.get("runner")
            if image == "${{ matrix.image }}":
                resolved = leg.get("image")
            if runner not in ARCH_IMAGES or resolved != ARCH_IMAGES[runner]:
                failures.append(
                    f"{where} pairs runner {runner!r} with container {resolved!r}; "
                    "use Arch Linux on x86_64 and Arch Linux ARM on aarch64."
                )

        shell = ((spec.get("defaults") or {}).get("run") or {}).get("shell")
        if shell is None:
            shell = ((doc.get("defaults") or {}).get("run") or {}).get("shell")
        if shell != "bash":
            failures.append(
                f"{where} must default to bash; container jobs otherwise use sh, "
                "which cannot preserve the build pipeline's PIPESTATUS."
            )
        for step in spec.get("steps") or []:
            commands = "\n".join(
                line for line in str(step.get("run", "")).splitlines()
                if not line.lstrip().startswith("#")
            )
            if re.search(r"\bapt-get\b|apparmor_restrict_unprivileged_userns", commands):
                failures.append(
                    f"{where} still configures the Ubuntu host from an Arch job."
                )
    return failures


def check_pm_pin(path, doc):
    """One pm commit per workflow, a commit, and named only as PM_REF."""
    steps = [
        (job, step)
        for job, spec in (doc.get("jobs") or {}).items()
        for step in (spec or {}).get("steps") or []
    ]
    wants_pm = [
        (job, step)
        for job, step in steps
        if PM_REPO in str((step.get("with") or {}).get("repository", ""))
        or PM_ACTION in str(step.get("uses", ""))
    ]
    if not wants_pm:
        return []

    failures = []
    pinned = str((doc.get("env") or {}).get("PM_REF") or "").strip()
    where = path.name

    if not pinned:
        failures.append(
            f"{where} checks pm out but declares no env.PM_REF. The commit "
            "belongs at the top of the workflow,\n    because it is also "
            "what pm's build cache is keyed on."
        )
    elif not COMMIT.match(pinned):
        failures.append(
            f"{where} pins pm to {pinned!r}, which is not a 40-character "
            "commit.\n    A branch makes the cache key a moving target: "
            "the same key, a different binary."
        )

    for job, step in wants_pm:
        ref = str((step.get("with") or {}).get("ref", ""))
        if not PM_REF_USE.fullmatch(ref.strip()):
            failures.append(
                f"{where}: {job} takes pm at ref {ref!r} rather than "
                "${{ env.PM_REF }}.\n    That is a second copy of the pin."
            )

    # A job or a step may declare its own `env:`, and one naming PM_REF shadows
    # the workflow's for everything under it. Every `${{ env.PM_REF }}` above
    # would still read as one pin while resolving to two, which is the failure
    # this check exists for wearing the shape that passes it.
    for job, spec in (doc.get("jobs") or {}).items():
        if "PM_REF" in ((spec or {}).get("env") or {}):
            failures.append(
                f"{where}: {job} declares its own env.PM_REF, which shadows "
                "the workflow's\n    for every step in it."
            )
        for step in (spec or {}).get("steps") or []:
            if "PM_REF" in (step.get("env") or {}):
                name = step.get("name") or step.get("uses") or "a step"
                failures.append(
                    f"{where}: {job} has a step ({name}) with its own "
                    "env.PM_REF.\n    It shadows the workflow's."
                )

    return failures


def check_pm_hosts(container, setup, workflows):
    """Keep the pm revision and its host toolchain consistent across CI."""
    failures = []
    pins = {
        str((doc.get("env") or {}).get("PM_REF"))
        for doc in workflows
        if (doc.get("env") or {}).get("PM_REF")
    }
    if len(pins) > 1:
        failures.append("workflows disagree on PM_REF; update all pm pins together.")

    rust = re.search(r"^ARG RUST_VERSION=(\d+\.\d+\.\d+)$", container, re.MULTILINE)
    installs = [
        step for step in (setup.get("runs") or {}).get("steps") or []
        if "rustup toolchain install" in str(step.get("run", ""))
    ]
    if not rust or not installs:
        failures.append("cannot find the container and CI Rust toolchain pins.")
    else:
        for step in installs:
            version = str((step.get("env") or {}).get("RUST_VERSION", ""))
            if version != rust.group(1):
                failures.append(
                    f"arch-setup Rust {version!r} differs from Containerfile "
                    f"Rust {rust.group(1)!r}; pm needs the same toolchain in both."
                )
    return failures


def self_test():
    """A matrix must prove both userspaces, not merely name an Arch image."""
    good = {
        "defaults": {"run": {"shell": "bash"}},
        "jobs": {
            "gates": {
                "runs-on": "ubuntu-24.04",
                "container": {"image": ARCH_IMAGES["ubuntu-24.04"]},
            },
            "packages": {
                "runs-on": "ubuntu-24.04",
                "container": {"image": ARCH_IMAGES["ubuntu-24.04"]},
            },
            "build": {
                "needs": ["packages"],
                "runs-on": "${{ matrix.runner }}",
                "container": {"image": "${{ matrix.image }}"},
                "strategy": {"matrix": {"include": [
                    {"runner": runner, "image": image}
                    for runner, image in ARCH_IMAGES.items()
                ]}},
            },
        },
    }
    assert not check_arch_jobs(WORKFLOW, good)
    assert not check_package_handoff(good)
    for mutation in ("no-container", "wrong-arch", "shell", "apt", "sysctl"):
        bad = copy.deepcopy(good)
        gate = bad["jobs"]["gates"]
        if mutation == "no-container":
            del gate["container"]
        elif mutation == "wrong-arch":
            for leg in bad["jobs"]["build"]["strategy"]["matrix"]["include"]:
                if leg.get("runner") == "ubuntu-24.04-arm":
                    leg["image"] = ARCH_IMAGES["ubuntu-24.04"]
                    break
            else:
                raise AssertionError("self-test fixture missing ubuntu-24.04-arm leg")
        elif mutation == "shell":
            del bad["defaults"]
        else:
            gate["steps"] = [{"run": (
                "apt-get update" if mutation == "apt" else
                "sysctl -w kernel.apparmor_restrict_unprivileged_userns=0"
            )}]
        assert check_arch_jobs(WORKFLOW, bad), mutation
    bad = copy.deepcopy(good)
    bad["jobs"]["build"]["needs"] = ["missing"]
    assert check_package_handoff(bad)
    container = "ARG RUST_VERSION=1.95.0\n"
    setup = {"runs": {"steps": [{
        "env": {"RUST_VERSION": "1.95.0"},
        "run": 'rustup toolchain install "$RUST_VERSION" --profile minimal',
    }]}}
    workflows = [{"env": {"PM_REF": "a" * 40}} for _ in range(2)]
    assert not check_pm_hosts(container, setup, workflows)
    for mutation in ("rust-mismatch", "rust-missing", "install-missing", "pm-mismatch"):
        bad_setup = copy.deepcopy(setup)
        bad_workflows = copy.deepcopy(workflows)
        bad_container = container
        if mutation == "rust-mismatch":
            bad_setup["runs"]["steps"][0]["env"]["RUST_VERSION"] = "1.94.0"
        elif mutation == "rust-missing":
            bad_container = ""
        elif mutation == "install-missing":
            bad_setup["runs"]["steps"] = []
        else:
            bad_workflows[1]["env"]["PM_REF"] = "b" * 40
        assert check_pm_hosts(bad_container, bad_setup, bad_workflows), mutation
    print("workflow: Arch job and pm host regression checks passed")
    return 0


def main():
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if not WORKFLOW.exists():
        print(f"workflow: {WORKFLOW.relative_to(REPO)} absent; nothing to check")
        return 0

    text = WORKFLOW.read_text()
    doc = yaml.safe_load(text)
    jobs = doc.get("jobs") or {}
    failures = check_package_handoff(doc)

    uploads, downloads = [], []
    for job, spec in jobs.items():
        for step in (spec or {}).get("steps") or []:
            uses = str(step.get("uses", ""))
            name = (step.get("with") or {}).get("name")
            if not name:
                continue
            if "upload-artifact" in uses:
                uploads.append((job, str(name)))
            elif "download-artifact" in uses:
                downloads.append((job, str(name)))

    for channel in CHANNELS:
        produced = {CHANNEL_REF.sub(channel, name) for _, name in uploads}
        for job, name in downloads:
            wanted = CHANNEL_REF.sub(channel, name)
            # Only the release artifacts are channel-scoped; console logs and
            # the like are uploaded by one job and downloaded by nobody.
            if not wanted.startswith("losos-"):
                continue
            if wanted not in produced:
                failures.append(
                    f"on a {channel} build, {job} downloads {wanted!r}, which no "
                    f"job uploads.\n    uploaded: {', '.join(sorted(produced))}"
                )

    # The channel must be decided once. Anything else reading the event to work
    # it out again is a second derivation.
    derivations = [
        line.strip()
        for line in text.splitlines()
        if "GITHUB_REF_TYPE" in line or "github.ref_type" in line
    ]
    if len(derivations) > 1:
        failures.append(
            "the channel is derived in more than one place:\n    "
            + "\n    ".join(derivations)
        )

    # Least privilege, across every workflow in the tree rather than this one.
    # A rule that only ever looked at images.yml would say nothing about the
    # next workflow somebody adds, which is the one most likely to get it
    # wrong.
    workflows = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        spec = yaml.safe_load(path.read_text()) or {}
        workflows.append(spec)
        failures.extend(check_pm_pin(path, spec))
        failures.extend(check_arch_jobs(path, spec))

        top = spec.get("permissions")
        if isinstance(top, dict) and top.get("contents") == "write":
            failures.append(
                f"{path.relative_to(REPO)} grants permissions.contents: write "
                "workflow-wide.\n    Every job in it then holds a token that can "
                "write to the repository, including\n    the ones that check out "
                "a ref and run the tree's own code. Grant it on the\n    job that "
                "needs it instead."
            )

    failures.extend(check_pm_hosts(
        (REPO / "Containerfile").read_text(),
        yaml.safe_load((REPO / ".github/actions/arch-setup/action.yml").read_text()),
        workflows,
    ))

    if failures:
        print("workflow: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(
        f"workflow: {len(downloads)} artifact download(s) resolve for "
        f"{len(CHANNELS)} channel(s); Arch jobs, matching pm/Rust pins and least privilege at the "
        f"top of {len(list(WORKFLOWS.glob('*.yml')))} workflow(s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
