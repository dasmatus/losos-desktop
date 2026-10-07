# Signing in

A person proves who they are to PAM, and only to PAM. derisk's login screen
relays whatever the stack asks, its lock screen asks the same stack through
a `derisk auth` process, and Settings, Sign-in sets up what a person brings:
a security key, fingerprints, verification codes, a new password.
`nixos/modules/sign-in.nix` writes the stacks.

What each method can do is bounded by how accounts are stored (the `home`
partition in [the disk layout](layout.md)): a person is a
systemd-homed record whose home area is a LUKS volume, so only something that
yields the volume's key can log them in.

| Method | Login screen | Lock screen | polkit, run0 |
| --- | --- | --- | --- |
| Password | yes | yes | yes |
| Security key (FIDO2) | yes, instead of the password | yes | yes |
| Fingerprint | no | yes, beside the field | yes |
| Verification code | after the password, when turned on | no | no |

## Security keys

A FIDO2 key, the kind a passkey lives on, is enrolled into the homed record
(`homectl update --fido2-device=auto`, which Settings runs with the person's
password and the key's PIN in its environment). homed then wraps the home
area's key with the key's hmac-secret, so the key alone opens the home:
pam_systemd_home asks for the PIN ("Security token PIN") and a touch, and
derisk shows both. Setting up a key replaces the one set up before, and the
password keeps working. Passkeys held by a phone or a laptop's TPM cannot do
this, since homed has no way to get a key out of them, and are not offered.

## Fingerprints

fprintd reads the reader. NixOS would put pam_fprintd first in every
service, which would let a finger "log in" without opening the home area, so
`login`, `derisk-login`, `derisk-setup`, `derisk-greeter` and the lock
screen's field (`derisk`) turn it off. The lock screen listens to the reader
through a service of its own, `derisk-fingerprint`, with pam_fprintd alone, in
a second conversation beside the field: touch the reader or type, and
whichever PAM accepts first unlocks. derisk offers the reader only when that
service file exists. polkit keeps NixOS's default, so a finger answers an
administrator prompt. Fingerprints are enrolled from Settings with
`fprintd-enroll`.

## Verification codes

A person turns codes on in Settings by scanning a QR code into an
authenticator app and typing the first code back, which is checked before
anything is saved. The secret and five one-time recovery codes go in
`~/.google_authenticator`, read-only to its owner, the format
pam_google_authenticator reads. `derisk-login` runs the module with
`nullok`, so only people with the file are asked, after the password or
security key. NixOS then runs pam_systemd_home once early in the stack, so
the home area holding the file is open when the module reads it; pam_unix
is left out of that service, since it would ask a homed user for their
password a second time.

## Password age

`losos.signIn.passwordMaxAgeDays` (365 by default, 0 for never) and
`losos.signIn.passwordWarnDays` (14) go in `/etc/derisk/sign-in.conf`.
derisk writes them into homed records, as `passwordChangeMaxUSec` and
`passwordChangeWarnUSec`: first-boot setup creates the first account under
them, and the display manager updates any record that says something else
when its owner logs in, so changing the option reaches every account. Only
an administrator may change those fields, so a person cannot opt out by
editing their own record.

From the date on, pam_systemd_home warns at each login ("Password will
expire soon"), and after it, the login screen asks for a new password in the
same conversation (`pam_chauthtok`) before the session starts. The lock
screen still unlocks a running session with an expired password, with a
reminder, so nobody is locked out of their own session over a date. Settings
shows when the password expires and changes it with `homectl passwd`.
