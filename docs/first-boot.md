# First-boot setup

A fresh install has no user. `derisk setup` (`nixos/modules/setup.nix`) runs
on tty1 as a logind session of its own, after homed and before the display
manager, and draws derisk's pages on the seat: language, keyboard layout,
time zone, network, then the first account (full name, user name, password).
It saves them with `localectl set-locale`, `localectl set-x11-keymap`,
`timedatectl set-timezone` and `homectl create --member-of=wheel`, the
password passed in `NEWPASSWORD` and never on a command line, and exits; the
login screen then takes the seat. The keyboard layout switches live as it is
picked, and the display manager hands localed's saved layout to the login
screen and the session as xkbcommon's defaults.

It runs on every boot and exits at once when `userdbctl` lists a regular
user, so a machine switched off halfway through asks again. If derisk cannot
draw at all, homed's console wizard asks for the user instead. homed's own
first-boot unit is kept for `home.create.*` credentials, without its prompt.
The languages offered are the locales built into the archive, the thirty-odd
derisk has names for, and `i18n.imperativeLocale` lets localed's choices
stick. On a phone-sized screen the pages fill the screen and the on-screen
keyboard opens under a focused field; on a PC the card floats and the
keyboard is a button in the corner.
