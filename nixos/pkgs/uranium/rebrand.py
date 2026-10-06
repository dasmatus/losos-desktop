"""Rename Chromium to Uranium and take Google out of the browser's strings.

Run from the root of a Chromium source tree, after its patches and
ungoogled-chromium's. The name is in about 700 strings and Google in a few
thousand, not in one place a patch could change, and a patch over all of
them would stop applying at every Chromium release, so this rewrites them
instead.

Rewriting the English text alone would lose every translation of those
strings: GRIT finds a translation by an id hashed from the English text,
and a changed text has a new id that no .xtb file lists, so every locale
would fall back to English for them. So GRIT, from the same tree, hashes
each message before and after, and every .xtb file is moved to the new ids
with the same rewrite applied to its translations.
"""

import os
import re
import sys

sys.path.insert(0, "tools/grit")
from grit import grd_reader  # noqa: E402

# Chromium as the product, not the project: ChromiumOS, the updater and the
# copyright holder keep their names. Translations often join "Chromium OS"
# with a no-break space, which \s matches.
NAME = re.compile(r"Chromium(?!OS|\s+OS|Updater|\s+Authors)")

# The two string tables that name the product, where Chromium becomes
# Uranium. Everywhere else Chromium names the project and stays.
PRODUCT_TABLES = {
    "chrome/app/chromium_strings.grd",
    "components/components_chromium_strings.grd",
}
PRODUCT_PART = "chrome/app/settings_chromium_strings.grdp"

# Google comes out of every string, as Matus asked: "Google Account" reads
# "Account", "sent to Google" reads "sent to". Most of these strings belong
# to features ungoogled-chromium already removes or the policies switch off;
# what is left on screen no longer names a company the browser does not talk
# to. Only the word with a capital G: google.com, IDS_GOOGLE_* and the
# lower-case file names in the tables stay as they are, and so does a
# domain such as Google.com, which names a site rather than speaking for it.
#
# A preposition before Google alone goes with it ("Sign in to Google"
# becomes "Sign in"), unless Google starts a product's name ("Save to Google
# Drive" becomes "Save to Drive"). English only: a translation keeps its
# own grammar around the gap.
GOOGLE_OBJECT = re.compile(
    r"[ \u00a0]+(?:by|from|with|to|on|at|for|in|into|of|about|via|through|and|or)"
    r"[ \u00a0]+Google(?:'s|\u2019s)?\b(?![ \u00a0\-]+[A-Z]|\.[a-z])"
)
GOOGLE_WORD = re.compile(r"\bGoogle(?:'s|\u2019s)?(?!\.[a-z])(?:[ \u00a0\-]+|\b)")
GAP = "\x00"

# The directories whose string tables the browser shows. third_party holds
# other projects' tables, and the rest of the tree has none on screen.
STRING_DIRS = ["chrome", "components", "content", "extensions", "ui", "device", "services"]


# A placeholder's example (<ex>Google</ex>) is never shown and GRIT refuses
# an empty one, so examples keep their text, as do comments, which nobody
# sees and which a removal could leave holding "--", which XML forbids.
EXAMPLE = re.compile(r"(<ex>.*?</ex>|<!--.*?-->)", re.S)


def degoogle_text(text):
    text = GOOGLE_OBJECT.sub(GAP, text)
    text = GOOGLE_WORD.sub(GAP, text)
    # Close up the space a removed word leaves before punctuation or a tag,
    # and after a space or a tag, so no two spaces are left behind.
    text = re.sub(r"[ \u00a0]*" + GAP + r"+(?=[.,;:!?)<\n]|$)", "", text)
    text = re.sub(r"(?<=[ \u00a0(>\n])" + GAP + r"+[ \u00a0]*", "", text)
    text = re.sub(r"^" + GAP + r"+", "", text)
    return text.replace(GAP, "")


def degoogle(text):
    return "".join(
        part if EXAMPLE.fullmatch(part) else degoogle_text(part)
        for part in EXAMPLE.split(text)
    )


def rewrite(text, product):
    text = degoogle(text)
    return NAME.sub("Uranium", text) if product else text


def rewrite_file(path, product):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    rewritten = rewrite(text, product)
    if rewritten != text:
        with open(path, "w", encoding="utf-8") as f:
            f.write(rewritten)
    return rewritten != text


# Two messages can come out of the rewrite as one: "Sign in to Google" and
# "Sign in" are both "Sign in" afterwards, so both hash to one id and the
# moved .xtb lists that id twice. GRIT asserts on a second translation of a
# message into the same language (clique.py, "assert not (language, gender)
# in self.clique"), which stopped the build at components_strings_af.xtb.
# The first translation of an id stays; a later one would only repeat it.
TRANSLATION = re.compile(r'[ \t]*<translation id="(\d+)"[^>]*>.*?</translation>\n?', re.S)


def dedupe(text):
    seen = set()

    def first(m):
        if m.group(1) in seen:
            return ""
        seen.add(m.group(1))
        return m.group(0)

    return TRANSLATION.sub(first, text)


def tables():
    for top in STRING_DIRS:
        for root, dirs, files in os.walk(top):
            dirs[:] = [d for d in dirs if d not in ("third_party", "test", "tests")]
            for name in files:
                if name.endswith((".grd", ".grdp")):
                    yield os.path.join(root, name)


def message_ids(grd):
    # Every message, whatever its <if> conditions: the ids only need to line
    # up between the two parses, and validation would want the build's
    # GRIT defines.
    root = grd_reader.Parse(grd, debug=False, skip_validation_checks=True)
    return [
        node.GetCliques()[0].GetId() for node in root.Preorder() if node.name == "message"
    ]


def translations(grd):
    with open(grd, encoding="utf-8") as f:
        text = f.read()
    base = os.path.dirname(grd)
    return [
        os.path.join(base, path)
        for path in re.findall(r'<file\s+path="([^"]+\.xtb)"', text)
    ]


files = sorted(tables())
grds = [path for path in files if path.endswith(".grd")]
for grd in PRODUCT_TABLES:
    if grd not in grds:
        sys.exit(f"rebrand: {grd} is gone; the string tables moved")

# Only tables GRIT can read on its own are rewritten: a .grd whose ids
# cannot be taken before and after would lose its translations, so it, and
# the parts it includes, are left alone.
before = {}
for grd in grds:
    try:
        before[grd] = message_ids(grd)
    except Exception as e:  # noqa: BLE001
        print(f"rebrand: {grd} left alone, GRIT cannot read it alone: {e}")

changed = 0
for path in files:
    owner = path if path.endswith(".grd") else None
    if owner is not None and owner not in before:
        continue
    if rewrite_file(path, path in PRODUCT_TABLES or path == PRODUCT_PART):
        changed += 1
print(f"rebrand: {changed} string tables rewritten")

moved = 0
for grd, ids in before.items():
    after = message_ids(grd)
    # Rewriting changes text, never which messages there are or their order.
    if len(ids) != len(after):
        sys.exit(f"rebrand: {grd} parsed to a different number of messages")
    new_id = {old: new for old, new in zip(ids, after) if old != new}
    if not new_id:
        continue
    product = grd in PRODUCT_TABLES
    xtbs = translations(grd)
    if product and not xtbs:
        sys.exit(f"rebrand: no translations for {grd}")
    for xtb in xtbs:
        if not os.path.exists(xtb):
            continue
        with open(xtb, encoding="utf-8") as f:
            text = f.read()
        text = re.sub(
            r'(<translation id=")(\d+)(")',
            lambda m: m.group(1) + new_id.get(m.group(2), m.group(2)) + m.group(3),
            text,
        )
        with open(xtb, "w", encoding="utf-8") as f:
            f.write(rewrite(dedupe(text), product))
        moved += 1
    print(f"rebrand: {grd}: {len(new_id)} messages rewritten, {len(xtbs)} translations moved")
print(f"rebrand: {moved} translation files moved")

# The product name the build stamps into its version info. The company line
# stays The Chromium Authors.
rewrite_file("chrome/app/theme/chromium/BRANDING", True)

# ungoogled-chromium's own flags say whose they are in chrome://flags; in
# Uranium they are Uranium's. Its copyright lines keep the name.
for header in ["chrome/browser/ungoogled_flag_entries.h", "chrome/browser/bromite_flag_entries.h"]:
    with open(header, encoding="utf-8") as f:
        lines = f.readlines()
    with open(header, "w", encoding="utf-8") as f:
        for line in lines:
            if not line.lstrip().startswith("//"):
                line = line.replace("ungoogled-chromium", "Uranium")
            f.write(line)
