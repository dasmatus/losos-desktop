# How people prove who they are, beyond the password: a fingerprint, a
# verification code, a security key, and a password that has to be renewed.
# docs/sign-in.md is the long version.
#
# Everything goes through PAM, so the system's stack stays the one source of
# truth and derisk only holds the conversation: its login screen relays what
# the stack asks, and its lock screen asks the same stack through
# `derisk auth`. What each method can do is bounded by the home area being a
# LUKS volume (accounts.nix): only something that yields the volume's key can
# log someone in, so
#
# - a security key (FIDO2, a passkey on a key) is enrolled into the homed
#   record itself (Settings, Sign-in; `homectl update --fido2-device=auto`),
#   which makes it a key slot of the home area: it logs in and unlocks;
# - a fingerprint yields no key, so it unlocks the lock screen and answers
#   polkit, but never logs anyone in;
# - a verification code is a second step after the password at login.
{ config, lib, ... }:

let
  cfg = config.losos.signIn;
  fprintd = config.services.fprintd.package;
in
{
  # fprintd and its PAM module. NixOS then puts pam_fprintd first in every
  # service's auth stack, which is right for polkit and wrong for anything
  # that opens a home area: a finger accepted there ends authentication
  # before pam_systemd_home is given a secret, the login "succeeds", and the
  # home stays locked. The services below turn it back off.
  services.fprintd.enable = true;

  security.pam.services = {
    # Logins: the console's and derisk's. derisk-setup and the greeter's own
    # session authenticate nobody.
    login.fprintAuth = false;
    derisk-login.fprintAuth = false;
    derisk-setup.fprintAuth = false;
    derisk-greeter.fprintAuth = false;

    # The lock screen's field. The reader has a conversation of its own
    # beside it (derisk-fingerprint), so the field never waits on a finger
    # before it asks for the password.
    derisk.fprintAuth = false;

    # The lock screen's reader: pam_fprintd alone, so a missing reader or a
    # timeout ends this conversation instead of falling through to a password
    # prompt nobody is shown. derisk offers the reader only when this file
    # exists. Account checks are the lock screen's own.
    derisk-fingerprint.text = ''
      auth     required  ${fprintd}/lib/security/pam_fprintd.so
      account  include   derisk
    '';

    # Verification codes at login, for whoever turned them on in Settings
    # (~/.google_authenticator); nobody else is asked (allowNullOTP). NixOS
    # runs pam_systemd_home once early when this is on, so the home area is
    # open by the time the module reads the file inside it. Unlocking a
    # session already running does not ask again.
    derisk-login.googleAuthenticator = {
      enable = true;
      allowNullOTP = true;
    };
    # With codes on, NixOS also runs pam_unix early, without try_first_pass,
    # and it would ask a homed user for their password a second time. Every
    # person here is a homed record, which pam_systemd_home authenticates;
    # pam_unix has nobody left to log in.
    derisk-login.unixAuth = false;
  };

  # The password age derisk writes into homed records: at first-boot setup
  # for the first account, and at each login for any account whose record
  # says something else (derisk's password_age). homed then warns before the
  # date and makes the next login after it ask for a new password.
  environment.etc."derisk/sign-in.conf".text = ''
    password.max_age_days = ${toString cfg.passwordMaxAgeDays}
    password.warn_days = ${toString cfg.passwordWarnDays}
  '';
}
