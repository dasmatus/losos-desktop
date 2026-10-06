// The documentation site. Its pages are ../docs, the same Markdown files
// GitHub renders in the repository and the docs workflow copies to the wiki,
// so there is one copy of the text and this directory only lays it out.
//
// Where it is served from comes from the environment, never from this file:
// the workflow takes the URL and base path from actions/configure-pages and
// the repository from GITHUB_SERVER_URL and GITHUB_REPOSITORY, so a fork or a
// renamed repository publishes to its own Pages site unchanged. Without them,
// as in `npm start`, it serves at the root of localhost.

const { themes } = require('prism-react-renderer');

const repository =
  process.env.GITHUB_SERVER_URL && process.env.GITHUB_REPOSITORY
    ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}`
    : undefined;
// Edit links point at the branch the docs live on, which the workflow reads
// from the repository's own default branch.
const editBranch = process.env.DOCS_EDIT_BRANCH;

/** @type {import('@docusaurus/types').Config} */
module.exports = {
  title: 'LosOS Desktop',
  tagline: 'A derisk desktop OS built as a NixOS configuration',
  url: process.env.DOCS_URL || 'http://localhost:3000',
  // configure-pages gives the base path without its trailing slash, and an
  // empty one for a site served from a custom domain's root.
  baseUrl: `${(process.env.DOCS_BASE_PATH || '').replace(/\/+$/, '')}/`,
  trailingSlash: false,

  onBrokenLinks: 'throw',
  // The pages are plain CommonMark, written to read the same on GitHub and
  // in the wiki. MDX would parse every `<arch>` and `{` in the prose as JSX.
  markdown: {
    format: 'md',
    hooks: { onBrokenMarkdownLinks: 'throw' },
  },

  // The web flasher (static/flasher/) is plain files, and the one thing it
  // needs from the build is where the proxy is, which CI has as the
  // repository variable LOSOS_PROXY_URL and nothing here names. Without it
  // the flasher still installs from files the person already has.
  plugins: [
    () => ({
      name: 'flasher-config',
      async postBuild({ outDir }) {
        const fs = require('node:fs/promises');
        const config = { proxy: process.env.LOSOS_PROXY_URL || null, channel: 'nightly' };
        await fs.writeFile(`${outDir}/flasher/config.json`, `${JSON.stringify(config)}\n`);
      },
    }),
  ],

  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          path: '../docs',
          routeBasePath: '/',
          sidebarPath: require.resolve('./sidebars.js'),
          editUrl:
            repository && editBranch
              ? ({ docPath }) => `${repository}/edit/${editBranch}/docs/${docPath}`
              : undefined,
        },
        blog: false,
      }),
    ],
  ],

  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      colorMode: { respectPrefersColorScheme: true },
      navbar: {
        title: 'LosOS Desktop',
        items: [
          // A page of its own outside the docs, so not a route Docusaurus
          // knows; pathname:// links to it as a file.
          { href: 'pathname:///flasher/', label: 'Flasher', position: 'left' },
          ...(repository
            ? [
              { href: `${repository}/wiki`, label: 'Wiki', position: 'right' },
              { href: repository, label: 'GitHub', position: 'right' },
            ]
            : []),
        ],
      },
      footer: {
        style: 'dark',
        copyright: 'LosOS Desktop is licensed AGPL-3.0-or-later.',
      },
      prism: {
        theme: themes.github,
        darkTheme: themes.dracula,
        additionalLanguages: ['nix', 'bash', 'yaml'],
      },
    }),
};
