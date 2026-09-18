#!/usr/bin/env python3
"""Check that the release workflow's artifact names agree across jobs.

The bug this exists for shipped silently and would only have surfaced at a
tagged release. `build` named its artifact after a channel it derived from the
event; `verify` and `ota` hardcoded `losos-nightly-<arch>`. On a tag the build
uploaded `losos-stable-<arch>`, the download asked for a name that did not
exist, download-artifact failed, and `publish` -- which needs both -- never ran.
Nightly was green throughout, so nothing pointed at it until someone tagged a
release and found the stable path had never worked.

Two properties, both cheap to check and neither visible by reading one job:

  * every artifact a job downloads is one some job uploads, for EVERY value the
    channel can take;
  * the channel is derived in exactly one place. Two derivations that agree
    today are two that can disagree after one edit.

It also checks that write permission is not granted workflow-wide, because this
workflow runs on `pull_request` -- it checks out and executes branch code, and a
token that can write to the repository has no business in that job.

And it holds every checkout of pm to one pinned commit. `ref: master` made this
repository's CI a function of another repository's tip: pm's plugin contract
changed upstream and every open pull request here went red within half an hour,
on trees nobody had touched.
"""

import re
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parent.parent.parent
WORKFLOW = REPO / ".github" / "workflows" / "images.yml"

# The channel values the workflow can produce. Written here rather than parsed
# out of the shell, because this is the list the check is asserting against.
CHANNELS = ("nightly", "stable")
CHANNEL_REF = re.compile(r"\$\{\{\s*needs\.channel\.outputs\.name\s*\}\}")

# The repository whose checkout has to be pinned, and what counts as a pin: a
# full commit id. A tag would do as well in principle and is deliberately not
# allowed, because a tag can be moved and this check would not notice.
PM_REPO = "dichhead/pm"
COMMIT = re.compile(r"^[0-9a-f]{40}$")


def main():
    if not WORKFLOW.exists():
        print(f"workflow: {WORKFLOW.relative_to(REPO)} absent; nothing to check")
        return 0

    text = WORKFLOW.read_text()
    doc = yaml.safe_load(text)
    jobs = doc.get("jobs") or {}
    failures = []

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

    top = doc.get("permissions")
    if isinstance(top, dict) and top.get("contents") == "write":
        failures.append(
            "permissions.contents: write is granted workflow-wide, and this "
            "workflow runs on pull_request.\n    Grant it on the publishing job "
            "instead."
        )

    # Every job that checks pm out must name one commit, and all of them the
    # same one. Two jobs on different pm commits means the gate approved a tree
    # against a pm the build never ran, and the comparison it made says nothing
    # about what shipped.
    pins = {}
    for job, spec in jobs.items():
        for step in (spec or {}).get("steps") or []:
            settings = step.get("with") or {}
            if str(settings.get("repository", "")) == PM_REPO:
                pins[job] = str(settings.get("ref", ""))

    floating = sorted(job for job, ref in pins.items() if not COMMIT.match(ref))
    if floating:
        failures.append(
            f"{PM_REPO} is not pinned to a commit in: {', '.join(floating)}.\n"
            f"    plugins/wit/plugin.wit is a copy of that checkout's contract, "
            f"so a push to pm goes red here on a tree nobody touched."
        )
    elif len(set(pins.values())) > 1:
        failures.append(
            "jobs check out different commits of "
            f"{PM_REPO}:\n    "
            + "\n    ".join(f"{job}: {ref}" for job, ref in sorted(pins.items()))
        )

    if failures:
        print("workflow: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(
        f"workflow: {len(downloads)} artifact download(s) resolve for "
        f"{len(CHANNELS)} channel(s); least privilege at the top; "
        f"{len(pins)} pm checkout(s) on one pin"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
