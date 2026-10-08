// The site's front page: what LosOS Desktop is, how to get it running, what it
// looks like, and every page of the documentation. The pages themselves are
// ../docs; this only points into them, and docs/index.md, the overview, is
// served at /overview. It draws in the OS's own dark navy and lime whatever
// the site's colour mode, like the screenshots it shows.
import React from 'react';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import { usePluginData } from '@docusaurus/useGlobalData';

import overview from '../../../docs/images/overview.png';
import firstBoot from '../../../docs/images/first-boot-pages.png';
import signIn from '../../../docs/images/sign-in-login-screen.png';
import choice from '../../../docs/images/choice-screens-default-apps.png';
import errorDialog from '../../../docs/images/error-dialog.png';
// An SVG imported into a page becomes a React component, which would put the
// diagram's own <style> into the page's document and style the whole page with
// it. Loading it as a file keeps it an image, as it is on the docs pages.
const layout = require('!!file-loader?name=assets/images/[name]-[contenthash].[ext]!../../../docs/images/layout.svg').default;
const halium = require('!!file-loader?name=assets/images/[name]-[contenthash].[ext]!../../../docs/images/halium.svg').default;

import styles from './index.module.css';

const features = [
  {
    title: 'An image, not a pile of packages',
    text: 'The OS is the Nix store on a dm-verity /usr. An update writes the slot not in use, and a version that does not come up is given up for the one before.',
    to: '/layout',
    link: 'Where the OS lives',
  },
  {
    title: 'Every home is encrypted',
    text: 'Each person is a systemd-homed account whose home is a LUKS volume. A password or a security key opens it, and a fingerprint unlocks the lock screen.',
    to: '/sign-in',
    link: 'Signing in',
  },
  {
    title: 'One desktop, PC and phone',
    text: 'derisk is the desktop, the login screen and the setup. The same OS builds as one generic image for Treble phones, over Halium.',
    to: '/halium',
    link: 'Halium GSI',
  },
  {
    title: 'Apps and the web',
    text: 'Flatpaks from Flathub through Bazaar, Danube as the browser, and ad blocking from filter lists for every app on the system.',
    to: '/danube',
    link: 'Danube',
  },
];

const ways = [
  { title: 'The installer ISO', text: 'Boot it from a stick and install the newest release onto a disk.', to: '/installer' },
  { title: 'Beside Windows', text: 'One program shrinks C: and adds LosOS, no stick needed.', to: '/windows-installer' },
  { title: 'On a phone', text: 'Flash the GSI from a browser over USB.', to: '/web-flasher' },
  { title: 'Build it yourself', text: 'nix build makes the image, the ISO and a release.', to: '/building' },
];

const gallery = [
  { src: firstBoot, alt: "First-boot setup's Time zone, Network and Your account pages", caption: 'First-boot setup', to: '/first-boot' },
  { src: signIn, alt: 'The login screen, and the same screen after a wrong password', caption: 'Signing in', to: '/sign-in' },
  { src: choice, alt: "Settings' Default apps page with the browser and search engine choices", caption: 'The choice screens', to: '/choice-screens' },
  { src: errorDialog, alt: 'An error dialog with its cause, what to do, and Copy details, Learn more and OK', caption: 'When something goes wrong', to: '/troubleshooting' },
];

function Hero({ offline }) {
  return (
    <header className={styles.hero}>
      <div className={styles.heroInner}>
        <div className={styles.heroText}>
          <p className={styles.kicker}>A derisk desktop OS, built as a NixOS configuration</p>
          <h1 className={styles.title}>LosOS Desktop</h1>
          <p className={styles.lead}>
            It installs as an image, uses systemd for everything it can, updates itself into a
            second slot, and rolls back when an update does not start.
          </p>
          <div className={styles.actions}>
            <Link className={styles.primary} to="/overview">Read the overview</Link>
            <Link className={styles.secondary} to="/installer">Install it</Link>
            {/* The flasher is a page outside the docs, and the offline copy has none. */}
            {!offline && <Link className={styles.secondary} to="pathname:///flasher/">Flash a phone</Link>}
          </div>
        </div>
        <img className={styles.heroShot} src={overview} alt="The derisk overview: open windows as tiles, workspaces along the top, and the clock, suggestions, calendar and notes beside them" />
      </div>
    </header>
  );
}

function Section({ title, children, className }) {
  return (
    <section className={`${styles.section} ${className || ''}`}>
      <h2 className={styles.sectionTitle}>{title}</h2>
      {children}
    </section>
  );
}

export default function Home() {
  const { siteConfig } = useDocusaurusContext();
  const { groups } = usePluginData('landing-data');
  return (
    <Layout title="Home" description={siteConfig.tagline}>
      <Hero offline={siteConfig.customFields.offline} />
      <main className={styles.main}>
        <Section title="What it is">
          <div className={styles.features}>
            {features.map((f) => (
              <Link key={f.to} className={styles.card} to={f.to}>
                <h3>{f.title}</h3>
                <p>{f.text}</p>
                <span className={styles.more}>{f.link} →</span>
              </Link>
            ))}
          </div>
        </Section>

        <Section title="Get it running">
          <ol className={styles.ways}>
            {ways.map((w) => (
              <li key={w.to}>
                <Link className={styles.way} to={w.to}>
                  <strong>{w.title}</strong>
                  <span>{w.text}</span>
                </Link>
              </li>
            ))}
          </ol>
        </Section>

        <Section title="What it looks like">
          <div className={styles.gallery}>
            {gallery.map((g) => (
              <Link key={g.to} className={styles.shot} to={g.to}>
                <img src={g.src} alt={g.alt} loading="lazy" />
                <span>{g.caption}</span>
              </Link>
            ))}
          </div>
        </Section>

        <Section title="How it fits together">
          <div className={styles.diagrams}>
            <Link className={styles.diagram} to="/layout">
              <img src={layout} alt="The disk: the image's ESP and slot A, and slot B, root, home and swap made on the first boot" loading="lazy" />
              <span>The disk, and what each partition holds</span>
            </Link>
            <Link className={styles.diagram} to="/halium">
              <img src={halium} alt="A phone's partitions and how it starts the Halium GSI" loading="lazy" />
              <span>How a phone starts the same OS</span>
            </Link>
          </div>
        </Section>

        <Section title="Every page">
          <div className={styles.index}>
            {groups.map((g) => (
              <nav key={g.label} className={styles.group} aria-label={g.label}>
                <h3>{g.label}</h3>
                <ul>
                  {g.pages.map((p) => (
                    <li key={p.id}><Link to={p.to}>{p.title}</Link></li>
                  ))}
                </ul>
              </nav>
            ))}
          </div>
        </Section>
      </main>
    </Layout>
  );
}
