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

It also checks that write permission is not granted workflow-wide, and that one
applies to EVERY workflow here rather than to this one. Both of them check out
a ref and execute the tree's own code from it -- images.yml on `pull_request`,
update-sources.yml on a `workflow_dispatch` against any ref -- and a job running
tree-supplied code has no business holding a token that can write to the
repository. The job that genuinely publishes asks for the permission itself.
"""

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

    # Least privilege, across every workflow in the tree rather than this one.
    # A rule that only ever looked at images.yml would say nothing about the
    # next workflow somebody adds, which is the one most likely to get it
    # wrong.
    for path in sorted(WORKFLOWS.glob("*.yml")):
        top = (yaml.safe_load(path.read_text()) or {}).get("permissions")
        if isinstance(top, dict) and top.get("contents") == "write":
            failures.append(
                f"{path.relative_to(REPO)} grants permissions.contents: write "
                "workflow-wide.\n    Every job in it then holds a token that can "
                "write to the repository, including\n    the ones that check out "
                "a ref and run the tree's own code. Grant it on the\n    job that "
                "needs it instead."
            )

    if failures:
        print("workflow: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(
        f"workflow: {len(downloads)} artifact download(s) resolve for "
        f"{len(CHANNELS)} channel(s); least privilege at the top of "
        f"{len(list(WORKFLOWS.glob('*.yml')))} workflow(s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
