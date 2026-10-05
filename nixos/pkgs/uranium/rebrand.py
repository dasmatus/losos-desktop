"""Rename Chromium to Uranium everywhere its UI says the product's name.

Run from the root of a Chromium source tree, after its patches. The name is
in about 700 strings, not in one place a patch could change, and a patch
over all of them would stop applying at every Chromium release, so this
rewrites them instead.

Rewriting the English text alone would lose every translation of those
strings: GRIT finds a translation by an id hashed from the English text,
and a changed text has a new id that no .xtb file lists, so every locale
would fall back to English for them. So GRIT, from the same tree, hashes
each message before and after, and every .xtb file is moved to the new ids
with the name rewritten in its translations as well.
"""

import glob
import re
import sys

sys.path.insert(0, "tools/grit")
from grit import grd_reader  # noqa: E402

# Chromium as the product, not the project: ChromiumOS, the updater and the
# copyright holder keep their names. Translations often join "Chromium OS"
# with a no-break space, which \s matches.
NAME = re.compile(r"Chromium(?!OS|\s+OS|Updater|\s+Authors)")

# Each string table that names the product, with the .grdp parts it
# includes and the translations it reads.
TABLES = [
    (
        "chrome/app/chromium_strings.grd",
        ["chrome/app/settings_chromium_strings.grdp"],
        "chrome/app/resources/chromium_strings_*.xtb",
    ),
    (
        "components/components_chromium_strings.grd",
        [],
        "components/strings/components_chromium_strings_*.xtb",
    ),
]


def rename(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    renamed = NAME.sub("Uranium", text)
    with open(path, "w", encoding="utf-8") as f:
        f.write(renamed)
    return text != renamed


def message_ids(grd):
    # Every message, whatever its <if> conditions: the ids only need to line
    # up between the two parses, and validation would want the build's
    # GRIT defines.
    root = grd_reader.Parse(grd, debug=False, skip_validation_checks=True)
    return [
        node.GetCliques()[0].GetId() for node in root.Preorder() if node.name == "message"
    ]


for grd, parts, xtbs in TABLES:
    before = message_ids(grd)
    for path in [grd] + parts:
        if not rename(path):
            sys.exit(f"rebrand: no Chromium in {path}; the string tables moved")
    after = message_ids(grd)
    # Renaming changes text, never which messages there are or their order.
    if len(before) != len(after):
        sys.exit(f"rebrand: {grd} parsed to a different number of messages")
    new_id = {old: new for old, new in zip(before, after) if old != new}

    xtb_files = sorted(glob.glob(xtbs))
    if not xtb_files:
        sys.exit(f"rebrand: no translations at {xtbs}")
    for xtb in xtb_files:
        with open(xtb, encoding="utf-8") as f:
            text = f.read()
        text = re.sub(
            r'(<translation id=")(\d+)(")',
            lambda m: m.group(1) + new_id.get(m.group(2), m.group(2)) + m.group(3),
            text,
        )
        with open(xtb, "w", encoding="utf-8") as f:
            f.write(NAME.sub("Uranium", text))
    print(f"rebrand: {grd}: {len(new_id)} messages renamed, {len(xtb_files)} translations moved")

# The product name the build stamps into its version info. The company line
# stays The Chromium Authors.
rename("chrome/app/theme/chromium/BRANDING")
