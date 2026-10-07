// The offline copy's entry points: sections/<page>.html for every page in
// ../docs and sections/<page>/<heading>.html for every heading on it, each a
// redirect into the hash route that shows it. A desktop error links here
// (mcsapi's `Docs`), because these are plain files: xdg-open hands a file://
// URL to the browser without its #fragment, and the fragment is where a
// hash-routed page lives.
//
// Headings are named as Docusaurus names their anchors, with its own slugger,
// after the inline Markdown a heading may carry is reduced to its text.

const fs = require('node:fs');
const path = require('node:path');
const { createSlugger } = require('@docusaurus/utils');

const docs = path.join(__dirname, '..', 'docs');

// "## Building `.#image`" and "## [pm](pm.md)" have the anchors of their text.
const text = (heading) =>
  heading
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]/g, '')
    .trim();

function headings(markdown) {
  const slugger = createSlugger();
  const found = [];
  let fenced = false;
  for (const line of markdown.split('\n')) {
    if (/^(```|~~~)/.test(line)) fenced = !fenced;
    const match = !fenced && line.match(/^#{2,6} (.+?)\s*#*\s*$/);
    if (match) found.push(slugger.slug(text(match[1])));
  }
  return found;
}

function redirect(file, target) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    `<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=${target}">` +
      `<a href="${target}">${target}</a>\n`,
  );
}

module.exports = () => ({
  name: 'offline-sections',
  async postBuild({ outDir }) {
    const sections = path.join(outDir, 'sections');
    for (const file of fs.readdirSync(docs).filter((f) => f.endsWith('.md'))) {
      const id = file.replace(/\.md$/, '');
      // The index page is the site's root route.
      const route = id === 'index' ? '/' : `/${id}`;
      redirect(path.join(sections, `${id}.html`), `../index.html#${route}`);
      for (const anchor of headings(fs.readFileSync(path.join(docs, file), 'utf8'))) {
        redirect(path.join(sections, id, `${anchor}.html`), `../../index.html#${route}#${anchor}`);
      }
    }
  },
});

module.exports.headings = headings;
