// Gives the landing page (src/pages/index.js) every page of ../docs, grouped
// as sidebars.js groups them, each with the title its first heading gives it
// (as wiki.js names wiki pages) and the route it is served at.

const fs = require('node:fs');
const path = require('node:path');

const docs = path.join(__dirname, '..', 'docs');

function title(id) {
  const heading = fs.readFileSync(path.join(docs, `${id}.md`), 'utf8').match(/^# (.+)$/m);
  if (!heading) throw new Error(`docs/${id}.md has no "# " heading to name it`);
  return heading[1].trim();
}

// index.md is served at /overview, beside the landing page at the root
// (docusaurus.config.js), and its title is the site's own name, which on the
// landing page would read as a link to itself.
const page = (id) =>
  id === 'index' ? { id, title: 'Overview', to: '/overview' } : { id, title: title(id), to: `/${id}` };

module.exports = () => ({
  name: 'landing-data',
  async contentLoaded({ actions }) {
    const { docs: sidebar } = require('./sidebars.js');
    const groups = [{ label: 'Start here', pages: [] }];
    for (const item of sidebar) {
      if (typeof item === 'string') groups[0].pages.push(page(item));
      else groups.push({ label: item.label, pages: item.items.map(page) });
    }
    actions.setGlobalData({ groups });
  },
});
