#!/usr/bin/env node
// Writes ../docs as GitHub wiki pages into the directory named on the command
// line, for the docs workflow to commit to the repository's wiki.
//
// The wiki is a copy, never the source: every run replaces all of it, so a
// page edited in the wiki is lost at the next push to docs/. _Footer.md says
// so on every page.
//
// A wiki names a page by its file, so each page takes its first heading as
// its name ("Halium" -> Halium.md, shown as "Halium") and loses that heading,
// which the wiki would otherwise draw twice. index.md becomes Home.md, the
// wiki's front page. Links between pages are rewritten to those names; the
// sidebar follows sidebars.js, the same order the Pages site uses.

const fs = require('fs');
const path = require('path');

const out = process.argv[2];
if (!out) {
  console.error('usage: wiki.js <output directory>');
  process.exit(2);
}

const docs = path.join(__dirname, '..', 'docs');
const sidebar = require('./sidebars.js').docs;

const pages = new Map();
for (const file of fs.readdirSync(docs).filter((f) => f.endsWith('.md'))) {
  const id = file.replace(/\.md$/, '');
  const text = fs.readFileSync(path.join(docs, file), 'utf8');
  const heading = text.match(/^# (.+)$/m);
  if (!heading) throw new Error(`${file} has no "# " heading to name its page`);
  const title = heading[1].trim();
  // A wiki file name can hold neither a slash nor most punctuation.
  const name = id === 'index' ? 'Home' : title.replace(/[^\p{L}\p{N} -]+/gu, '').replace(/ /g, '-');
  pages.set(id, { title, name, body: text.replace(heading[0], '').replace(/^\s+/, '') });
}

const listed = new Set();
function walk(items) {
  for (const item of items) {
    if (typeof item === 'string') listed.add(item);
    else walk(item.items);
  }
}
walk(sidebar);
for (const id of pages.keys()) {
  if (!listed.has(id)) throw new Error(`docs/${id}.md is not in sidebars.js`);
}

const link = (id) => {
  const page = pages.get(id);
  if (!page) throw new Error(`a link names docs/${id}.md, which does not exist`);
  return page.name;
};

fs.mkdirSync(out, { recursive: true });
for (const old of fs.readdirSync(out).filter((f) => f.endsWith('.md'))) {
  fs.unlinkSync(path.join(out, old));
}

for (const [id, page] of pages) {
  const body = page.body.replace(
    /\]\(([\w-]+)\.md(#[^)]*)?\)/g,
    (_, target, anchor) => `](${link(target)}${anchor || ''})`,
  );
  fs.writeFileSync(path.join(out, `${page.name}.md`), body);
}

const lines = [];
function side(items, depth) {
  for (const item of items) {
    const indent = '  '.repeat(depth);
    if (typeof item === 'string') {
      const page = pages.get(item);
      lines.push(`${indent}- [${page.title}](${page.name})`);
    } else {
      lines.push(`${indent}- **${item.label}**`);
      side(item.items, depth + 1);
    }
  }
}
side(sidebar, 0);
fs.writeFileSync(path.join(out, '_Sidebar.md'), lines.join('\n') + '\n');

const source = process.env.WIKI_SOURCE_URL;
fs.writeFileSync(
  path.join(out, '_Footer.md'),
  `Generated from ${source ? `[docs/](${source})` : '`docs/`'} in the repository at every change; edit the pages there, since an edit made here is overwritten.\n`,
);
