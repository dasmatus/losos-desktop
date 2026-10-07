// In the offline copy's hash routes a section is index.html#/page#section, and
// Docusaurus scrolls to `section` when a link inside the site is followed but
// not when the whole URL is opened from outside, which is how a desktop
// error's "Learn more" arrives (offline-sections.js). This scrolls there once
// the page has drawn. On a first load something resets the scroll position
// after this hook runs, so it waits a moment first; scrolling at once was
// measured to leave the page at its top.
export function onRouteDidUpdate({ location }) {
  const id = decodeURIComponent((location.hash || '').replace(/^#/, ''));
  if (!id) return;
  setTimeout(() => document.getElementById(id)?.scrollIntoView(), 50);
}
