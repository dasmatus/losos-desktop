# When something goes wrong

derisk's apps, the installer and first-boot setup show a failure in an
alert dialog: what failed as its title, red, the causes under it, what to do
about it when the app knows, and its code. A warning, something that went
only partly wrong, has the accent color instead. "Copy details" copies the
whole report as plain text, for a bug report; "Learn more" opens the section
of this page the error names; "OK" or Escape closes it.

Those sections are read from the copy of this documentation the image
carries, `/run/current-system/sw/share/doc/losos` (`nixos/modules/docs.nix`),
so they open without a network, and from the published site when the copy
has no such page. The installer and first-boot setup run before there is a
session or a browser, so their dialogs print the section's address on the
published site instead, for reading on another device. The dialogs are mcsapi's `ErrorDialog` drawing an
`mcsapi_ui::Error`, a miette report that knows its section; the theme colors
them like every other component.

## Files could not open or change something

Files reports what the filesystem refused: a folder that is gone, a name
that is taken, a file you are not allowed to read or change. The cause under
the message is the system's own reason, such as "Permission denied" or "Not
a directory". Files on a disk you did not mount yourself, and anything
outside your home directory, usually belong to root. Files opens a file by
handing it to `xdg-open`, so a file it could not open at all means
`xdg-open` itself did not start; a file that opens in the wrong application,
or in none, is `xdg-open`'s choice.

## The editor could not open or save a file

The Text Editor saves to the path in its location field, which can differ
from the file it opened: that is how it saves a copy. An empty field has
nowhere to save to. A file it could not open is usually unreadable to you or
not text; a path that does not exist yet opens as a new file instead, and is
created on the first save.

## Settings could not be read or saved

Settings keeps everything in `$XDG_CONFIG_HOME/derisk/settings.conf`, or
`~/.config/derisk/settings.conf` without the variable, and writes it whole
each time you press Save, through a temporary file beside it. "No config
directory" means neither variable is set, so the changes last only until
Settings closes. A file it could not read or write is the filesystem
refusing, as in Files above; a home directory that is full or read-only
does it too.

## Settings skipped some lines

A line Settings did not understand, such as `desktop.gaps = lots`, is left
out and that setting keeps its default; the warning names the first one. The
next Save writes the file again from what Settings shows, without the
skipped lines.

## The default browser did not change

The browser you pick under Default apps is written to
`$XDG_CONFIG_HOME/mimeapps.list` (`~/.config/mimeapps.list`), the file every
desktop reads, for web links and HTML files. Settings saved its own file but
could not write that one, so links still open in the old browser. The cause
says why; it is the filesystem again. See [the choice screens](choice-screens.md)
for where the list of browsers comes from.

## The installer stopped

The message is the installer's backend's own, from the step that failed,
and "Show details" is its log. A disk it refused was not in its list or is
the medium the installer booted from. A download that does not verify, in a
network that re-signs TLS, is what a release disk is for. [The
installer](installer.md) describes each step. Nothing is written before
"Erase and install"; after it, a failed install leaves the disk without a
working system, and the install can be run again from the start. tty2 has a
root shell for looking around first.

## Setup could not finish

First-boot setup applies its pages one by one with `localectl`,
`timedatectl` and `homectl`, and the message is the tool's. The step that
fails is usually the account: homed refuses a user name that is taken or not
a valid name, and a password its quality check rejects. Back returns to the
account page. Setup runs again on every boot
until an account exists ([First-boot setup](first-boot.md)).
