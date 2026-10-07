# Danube

Danube is the OS's web browser and the default for links and web pages
(`nixos/modules/danube.nix`). It is [WPE WebKit](https://wpewebkit.org),
the WebKit port with no toolkit of its own, drawn inside a window built
from mcsapi's components in derisk's theme (`src/danube`). It is the web
view and nothing more: derisk's command palette is its address bar and
its tab switcher, and derisk's top bar carries its menus. It also opens
the sign-in page of a network that wants one first (a captive portal), and
it loads the content blockers that [system-wide ad blocking](adblock.md)
compiles.

## How it is put together

WPE's headless platform renders each page into a buffer and hands it over
instead of putting it on screen. Danube copies the active tab's frame into
an egui texture and draws it, so what little interface the window has (a
progress line, a permission request's bar, a crashed page's notice,
toasts) is ordinary mcsapi code: the same components as derisk's apps,
recoloured whenever derisk publishes a new theme
(`$XDG_RUNTIME_DIR/derisk/theme.json`). Pointer, wheel, touch and key input
over the page goes back to WebKit as WPE events; typed text goes as one key
press per character, and Ctrl+C, Ctrl+X and Ctrl+V cross between WebKit's
clipboard and the desktop's.

WebKit runs on the process's main thread under GLib's main loop, the window
on a second thread under winit's (`engine.rs` and `ui.rs`). Two bounded
channels (crossbeam's) join them and nothing is locked: the window sends
commands, WebKit sends events (a tab opened, its title, a frame, a
permission asked) and asks for a repaint, and the window keeps its own
picture of the tabs from them. A frame that does not fit the channel is
dropped for the next. The threads beside them, derisk's menus, later
`danube` runs handing over URLs, the captive portal's recheck, are scoped
to the same `run` and end with WebKit.

There are no Rust bindings for WPE's 2.0 API, so the crate makes its own:
`build.rs` runs bindgen over the installed `wpe/webkit.h` and
`wpe/headless/wpe-headless.h` at build time, allowlisted to WebKit's and
WPE's symbols and the handful of GLib calls used, and `ffi.rs` includes the
result. Every signature and enum value is the library's own, so a WebKit
update that changes one fails to compile rather than at runtime. `webkit.rs`
wraps the objects in types that own one reference each (`Display`,
`WebView`, `View`, `Toplevel`, `Settings`, `NetworkSession`, `FilterStore`
and so on), give it back when dropped, offer the calls as methods, and take
closures for signals, which GObject frees with the connection. `engine.rs`
holds only the tab logic on top of them and has no `unsafe` in it.

nixpkgs has only WebKitGTK, so `nixos/pkgs/wpewebkit.nix` builds the WPE
port from the WPE release of the same WebKit version, with nixpkgs'
WebKitGTK patches. It builds only the WPEPlatform API (headless and Wayland
platforms) and nothing upstream substitutes it, so CI compiles it once per
architecture and keeps it in the project's cache.

## The window

The window is the page, and nothing else: no toolbar, no address bar, no
tab strip. Danube is the web view; addresses, tabs and (later)
extensions are derisk's command palette's to manage, and derisk's top bar
shows its menus. The window's title names the tab in front. Over the page
the window draws only what has to be there: a progress line along the top
while a page loads, a bar for a site's permission request, a notice on a
page whose web process died, and the link under the pointer. A phone
window (short side under 600 logical pixels, as in
[GTK and Qt on a phone](gtk-qt-phone.md)) sends a mobile User-Agent
string ("Android", "Mobile"), so sites serve their phone version from the
next page loaded.

The palette is the address bar. Ctrl+L (or Alt+D) in Danube asks derisk
to show it, and what is typed there that reads as a web address
(`example.com`, `localhost:8080`, a full URL) opens in Danube, in a new
tab. Anything else the palette searches for with the desktop's search
engine, chosen by the choice screen and Settings, Default apps
([choice screens](choice-screens.md)). Danube has no search setting of
its own.

The palette's Tabs and Extensions headings are derisk's bundled browser
palette plugin (a WebAssembly plugin of the palette, like its other
sources) showing what Danube registers: `register_palette` on the agent
socket, with the tabs (id, title, URL, which is active) and the
extensions (none until the extension runtime exists), sent again
whenever they change. What is picked there comes back as a `palette`
event with a command: `activate_tab`, `close_tab`, `new_tab`, or `open`
with the http(s) URL derisk made of the typed text (`https://` for a
bare host, `http://` for localhost), which Danube opens in a new tab. The extension commands (`enable_extension`, `disable_extension`,
`extension_options`) Danube only logs for now. Without the plugin (an
older derisk) the registration is refused and the menus below still list
the tabs.

The menus are derisk global menus, in the top bar and in the command
palette, where "Tabs › Wikipedia" switches to that tab and "Close Tab ›
Wikipedia" closes it:

- **Page**: the address, "Secure" or "Not secure", and how many of
  [system-wide ad blocking](adblock.md)'s filter sets are in force, as
  lines the top bar shows and the palette leaves out; then Open Address
  (the palette) and Copy Address.
- **Tabs**: each by its title, New Tab, Next Tab, Previous Tab.
- **Navigate**: Back, Forward, Reload, Stop.
- **Close Tab**: each by its title.

Danube registers them over derisk's agent socket
(`$XDG_RUNTIME_DIR/derisk/agent.sock`), finding its window by app id
(`org.losos.Danube`) and title, and sends them again whenever a tab
opens, closes or changes its title, address or security. Outside a derisk
session there is no palette and no menus, and only the keyboard reaches
the tabs.

Keys: Ctrl+L (or Alt+D) the palette, Ctrl+T a new tab (and the palette
for its address), Ctrl+W close, Ctrl+Tab and Ctrl+Shift+Tab the next and
previous tab, Ctrl+R or F5 reload, Alt+Left and Alt+Right back and
forward; a middle click on a link opens it in a tab behind.

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
| `javascript.jit` | `true` turns JavaScriptCore's JIT compilers on |

There is no `search` key, and `losos.danube.search` is gone with the
address bar: the palette searches with the desktop's engine.

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
