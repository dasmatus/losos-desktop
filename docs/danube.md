# Danube

Danube is the OS's web browser and the default for links and web pages
(`nixos/modules/danube.nix`). It is [WPE WebKit](https://wpewebkit.org),
the WebKit port with no toolkit of its own, drawn inside a window built
from mcsapi's components in derisk's theme (`src/danube`). It also opens
the sign-in page of a network that wants one first (a captive portal), and
it loads the content blockers that [system-wide ad blocking](adblock.md)
compiles.

## How it is put together

WPE's headless platform renders each page into a buffer and hands it over
instead of putting it on screen. Danube copies the active tab's frame into
an egui texture and draws it under its own toolbar, so the browser's
interface is ordinary mcsapi code: the same buttons, inputs, alerts and
toasts as derisk's apps, recoloured whenever derisk publishes a new theme
(`$XDG_RUNTIME_DIR/derisk/theme.json`). Pointer, wheel, touch and key input
over the page goes back to WebKit as WPE events; typed text goes as one key
press per character, and Ctrl+C, Ctrl+X and Ctrl+V cross between WebKit's
clipboard and the desktop's.

WebKit runs on the process's main thread under GLib's main loop, the window
on a second thread under winit's (`engine.rs` and `ui.rs`); the window sends
commands, and WebKit keeps the tabs' state current and asks for a repaint.

nixpkgs has only WebKitGTK, so `nixos/pkgs/wpewebkit.nix` builds the WPE
port from the WPE release of the same WebKit version, with nixpkgs'
WebKitGTK patches. It builds only the WPEPlatform API (headless and Wayland
platforms) and nothing upstream substitutes it, so CI compiles it once per
architecture and keeps it in the project's cache.

## The window

On a desktop the tabs run along the top, above Back, Forward, Reload, the
address bar and the ad blocking badge. On a phone, a window whose short
side is under 600 logical pixels as in [GTK and Qt on a phone](gtk-qt-phone.md),
the toolbar and address bar sit at the bottom, under the thumb, and the
tabs are a switcher behind a button that shows their count. A phone window
also sends a mobile User-Agent string ("Android", "Mobile"), so sites serve
their phone version from the next page loaded.

The address bar opens what looks like an address and searches for
anything else, with the desktop's search engine: derisk's
`defaults.search`, set by the choice screen and Settings, Default apps
([choice screens](choice-screens.md)), DuckDuckGo until one is chosen.
`losos.danube.search` or `search =` in `~/.config/danube/settings.conf`
picks another.

Tabs, Back, Forward, Reload and Close Tab are also derisk global menus: in
the top bar and in the command palette, where each tab is listed by its
title. Danube registers them over derisk's agent socket
(`$XDG_RUNTIME_DIR/derisk/agent.sock`), finding its window by app id
(`org.losos.Danube`) and title.

Keys: Ctrl+L (or Alt+D) the address bar, Ctrl+T a new tab, Ctrl+W close,
Ctrl+Tab the next tab, Ctrl+R or F5 reload, Alt+Left and Alt+Right back and
forward; a middle click on a link opens it in a tab behind. A second
`danube URL` opens the URL in a new tab of the running browser.

Downloads go to `~/Downloads`, never overwriting a file there, and a toast
says when one finished.

## Sites' permissions

A page's request for the camera, microphone, screen, location,
notifications, clipboard, pointer lock, the list of devices, protected
(DRM) media or its cookies on other sites is denied unless the person
allows it. Danube asks in a bar over the page, for that site, once or
remembered; remembered answers are lines in `~/.config/danube/permissions`
(`host kind allow|deny`), which can be edited or deleted.

## Captive portals

`danube captive-watch`, a user service in every session, rechecks the
network whenever systemd-networkd's state changes: it fetches
`losos.captivePortal.checkUrl` over plain HTTP, which a portal intercepts,
and compares the answer with `losos.captivePortal.expect`. Anything else
means a portal, and it opens `danube --captive-portal`: one window titled
"Sign in to network", no tabs, a profile that ends with the window, and no
content blockers, since a portal's page is often built from what they
block. That window checks again every three seconds and closes itself once
the network lets traffic through. The desktop entry's "Sign in to network"
action opens it by hand.

## Sandbox and hardening

Two sandboxes, both built with the [Hakoniwa](https://github.com/souk4711/hakoniwa)
crate (`src/danube/crates/danube-sandbox`).

- **The browser.** Started outside it, `danube` builds a container and runs
  itself again inside: its own user, mount, PID, IPC and UTS namespaces;
  the Nix store, `/etc`, the GPU driver and sysfs read-only; `/dev/dri`;
  an empty home with only `~/.local/share/danube`, `~/.cache/danube`,
  `~/.config/danube` and `~/Downloads` writable and the user's fonts,
  icons and derisk's settings read-only; an empty runtime directory with
  only the Wayland, PipeWire and PulseAudio sockets and derisk's directory;
  and the session bus through xdg-dbus-proxy, which lets through only the
  desktop portals, notifications and the accessibility bus. Landlock
  restricts the same paths again, and a seccomp filter denies the system
  calls Flatpak denies (keyctl, ptrace's relatives, kexec, bpf and the
  rest), except the namespace calls the next sandbox needs. The network
  is the host's, since WebKit's network process needs it.
- **Each web process.** WebKit puts every web process in a sandbox of its
  own, built with bwrap upstream. One patch
  (`nixos/pkgs/patches/wpewebkit`) lets the embedder name a launcher that
  takes bwrap's arguments; Danube names `danube-sandbox`, which reads the
  arguments WebKit gives and builds the same sandbox with Hakoniwa:
  WebKit's own filesystem layout and its seccomp filter, loaded last, with
  the descriptors WebKit hands over carried in. An argument it does not
  know is an error, so a WebKit update that asks for more cannot run a web
  process with less.

WebKit is built with level-3 fortify checks, stack clash protection,
automatic variables zeroed, eager binding (the whole GOT read-only), and
control-flow protection where the CPU has it (CET on x86_64, BTI and PAC on
aarch64). WebDriver, the remote inspector and WebKit's unshipped
(experimental) features, WebXR among them, are not built. JavaScriptCore's
JIT compilers are off by default (`JSC_useJIT=false`): scripts and
WebAssembly run in its interpreters, slower, and a page can no longer have
the browser write and run machine code, which most WebKit exploits rely on.
`losos.danube.jit` or `javascript.jit = true` in the user's settings turns
them back on. WebKit keeps its own allocator (libpas, with its Gigacage)
for JavaScript and DOM objects; everything else it allocates through
`malloc`, which is hardened_malloc on a PC ([the allocator](allocator.md)).
Tracking prevention (WebKit's ITP) is on, and there is no keyring for
WebKit to save passwords in.

The sandboxes need unprivileged user namespaces, which NixOS's kernel
allows. A phone's own kernel may not: see [what is not done](not-done.md).

## Settings

`/etc/danube/settings.conf` (from `losos.danube.*`), then the user's
`~/.config/danube/settings.conf`, `key = value` per line:

| Key | Meaning |
| --- | --- |
| `home` | What a new tab opens (an empty page by default) |
| `search` | A derisk engine name or an https URL the query is appended to |
| `javascript.jit` | `true` turns JavaScriptCore's JIT compilers on |

## What it replaced

Danube replaced Uranium, which was ungoogled-chromium under the OS's name:
a launcher that turned it into Chrome for Android on a phone, five patches
and a rebranding script for a build CI could only compile in rounds
(`uranium.yml`), a Flatpak of it served from the proxy, and `uranium-tabs`, a
native messaging host that mirrored its tabs into derisk's menus. Chromium
draws its own interface with its own toolkit, so it could never be an
mcsapi window or follow derisk's theme, and it is not coming back. Profiles
do not carry over: Danube starts with a new one in `~/.local/share/danube`,
and a user's own `~/.config/mimeapps.list` that names `uranium.desktop`
has to name `org.losos.Danube.desktop` instead.
