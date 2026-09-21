#!/usr/bin/env python3
"""Compatibility wrapper for `tools/pm-toolchain report`."""

import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(REPO / "tools" / "lib"))

from pm_toolchain import cli_main


if __name__ == "__main__":
    sys.exit(cli_main(["report", *sys.argv[1:]]))
