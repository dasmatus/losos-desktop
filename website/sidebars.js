// The order a reader meets the pages in. Every page in ../docs is listed:
// one left out would still build but be reachable only by a link.

/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
module.exports = {
  docs: [
    'index',
    'building',
    'troubleshooting',
    {
      type: 'category',
      label: 'The system',
      collapsed: false,
      items: [
        'systemd',
        'layout',
        'boot-loaders',
        'installer',
        'windows-installer',
        'first-boot',
        'sign-in',
        'releases',
        'allocator',
        'security-report',
      ],
    },
    {
      type: 'category',
      label: 'Desktop and apps',
      collapsed: false,
      items: ['gtk-qt-phone', 'icon-theme', 'danube', 'adblock', 'choice-screens'],
    },
    {
      type: 'category',
      label: 'Building blocks',
      collapsed: false,
      items: ['trust', 'packages', 'binary-cache', 'pm'],
    },
    {
      type: 'category',
      label: 'Phones',
      collapsed: false,
      items: ['halium', 'web-flasher'],
    },
    {
      type: 'category',
      label: 'History and gaps',
      collapsed: false,
      items: ['from-the-pm-tree', 'not-done', 'nixos'],
    },
  ],
};
