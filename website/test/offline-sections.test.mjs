// The offline copy's section files (offline-sections.js) are named after the
// anchors Docusaurus gives headings, which derisk's error alerts link to by
// name. These are the cases that differ from the plain heading text; the
// whole set was compared with the anchors of a built site when it was written.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const { headings } = createRequire(import.meta.url)("../offline-sections.js");

test("headings become Docusaurus's anchors", () => {
  const page = [
    "# Title is the page, not a section",
    "## Which devices",
    "## Building `.#image`",
    "## [pm](pm.md) on a phone",
    "```sh",
    "## not a heading inside a fence",
    "```",
    "### Which devices",
  ].join("\n");
  assert.deepEqual(headings(page), [
    "which-devices",
    "building-image",
    "pm-on-a-phone",
    "which-devices-1",
  ]);
});
