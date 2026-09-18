//! The wire shape GNOME already knows how to render.
//!
//! gnome-control-center's Privacy & Security panel reads a list of
//! `a{sv}` dictionaries from fwupd and renders each as a row. Rather than patch
//! that rendering to understand a second, private format, this emits the
//! *same* dictionary -- so the panel change needed to show OS-level checks
//! alongside firmware ones is "ask a second bus name and concatenate", not a
//! new widget.
//!
//! The keys and the enum values below are taken from
//! `panels/privacy/firmware-security/cc-firmware-security-utils.{c,h}` in
//! gnome-control-center 47.2, which is the consumer. Anything it does not
//! recognise is ignored, and anything it expects and does not get renders as
//! unknown, so this is a contract worth writing down rather than inferring.

use std::collections::HashMap;

use serde::Serialize;
use zbus::zvariant::{OwnedValue, Value};

/// `FwupdSecurityAttrResult`, in declaration order because the wire carries the
/// discriminant rather than the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttrResult {
    Unknown = 0,
    Enabled = 1,
    NotEnabled = 2,
    Valid = 3,
    NotValid = 4,
    Locked = 5,
    NotLocked = 6,
    Encrypted = 7,
    NotEncrypted = 8,
    Tainted = 9,
    NotTainted = 10,
    Found = 11,
    NotFound = 12,
    Supported = 13,
    NotSupported = 14,
}

impl AttrResult {
    /// Whether this result is the good outcome for the check that produced it.
    ///
    /// Not derivable from the value: `Locked` is good for a bootloader and
    /// `NotFound` is good for a debug interface, so each check says which of
    /// its outcomes counts as success rather than this guessing from the enum.
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// `FWUPD_SECURITY_ATTR_FLAG_SUCCESS`. The panel uses it to colour the row, so
/// a check that reports a result without it reads as a failure however
/// reassuring its text.
pub const FLAG_SUCCESS: u64 = 1 << 0;
/// `..._ACTION_CONFIG_OS`: tells the reader this is fixable in the OS rather
/// than being a property of the hardware they cannot change.
pub const FLAG_ACTION_CONFIG_OS: u64 = 1 << 13;

#[derive(Clone, Debug, Serialize)]
pub struct Attr {
    /// Reverse-DNS id. Ours are under io.losos so they cannot collide with
    /// fwupd's org.fwupd.hsi.* even if both end up in one list.
    pub appstream_id: String,
    /// One line, shown as the row title.
    pub summary: String,
    /// The paragraph shown when the row is opened.
    pub description: String,
    pub result: AttrResult,
    /// Which HSI level this contributes to. fwupd's scale is 0-4; OS-level
    /// checks are reported at the level whose definition they match rather
    /// than inventing a sixth.
    pub hsi_level: u32,
    pub success: bool,
    /// Where the check looked, so a disagreement can be settled by looking
    /// there too. Not part of fwupd's schema -- it rides in Description.
    pub evidence: String,
}

impl Attr {
    pub fn to_dbus(&self) -> HashMap<String, OwnedValue> {
        let mut map: HashMap<String, OwnedValue> = HashMap::new();
        let mut insert = |key: &str, value: Value<'_>| {
            if let Ok(owned) = OwnedValue::try_from(value) {
                map.insert(key.to_string(), owned);
            }
        };

        insert("AppstreamId", Value::from(self.appstream_id.clone()));
        insert("Summary", Value::from(self.summary.clone()));
        // The evidence is appended to the description rather than dropped: a
        // report that says "kernel lockdown is not enabled" and does not say
        // where it looked is not checkable by the person reading it.
        insert(
            "Description",
            Value::from(format!("{}\n\n{}", self.description, self.evidence)),
        );
        insert("HsiResult", Value::from(self.result.as_u32()));
        insert("HsiLevel", Value::from(self.hsi_level));

        let mut flags = FLAG_ACTION_CONFIG_OS;
        if self.success {
            flags |= FLAG_SUCCESS;
        }
        insert("Flags", Value::from(flags));
        map
    }
}
