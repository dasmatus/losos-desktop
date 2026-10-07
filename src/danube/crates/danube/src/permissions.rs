//! Per-site permissions. Every request a page makes (camera, microphone,
//! location, notifications, ...) is denied unless the person allows it for
//! that site, either once or for good. The lasting answers are kept in
//! `~/.config/danube/permissions`, one `host kind allow|deny` line each,
//! which the person can read and edit.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// What a site asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Camera,
    Microphone,
    CameraAndMicrophone,
    Screen,
    Location,
    Notifications,
    Clipboard,
    PointerLock,
    /// Listing the cameras and microphones without using them.
    Devices,
    /// Encrypted media (DRM).
    MediaKeys,
    /// A third-party frame asking for its cookies.
    StorageAccess,
}

impl Kind {
    /// The request's kind from its GObject type name (WebKit has one
    /// request class per kind); camera and microphone are told apart by the
    /// caller, which asks the request.
    pub fn from_type_name(name: &str) -> Option<Self> {
        Some(match name {
            "WebKitGeolocationPermissionRequest" => Self::Location,
            "WebKitNotificationPermissionRequest" => Self::Notifications,
            "WebKitClipboardPermissionRequest" => Self::Clipboard,
            "WebKitPointerLockPermissionRequest" => Self::PointerLock,
            "WebKitDeviceInfoPermissionRequest" => Self::Devices,
            "WebKitMediaKeySystemPermissionRequest" => Self::MediaKeys,
            "WebKitWebsiteDataAccessPermissionRequest" => Self::StorageAccess,
            "WebKitUserMediaPermissionRequest" => Self::CameraAndMicrophone,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::CameraAndMicrophone => "camera+microphone",
            Self::Screen => "screen",
            Self::Location => "location",
            Self::Notifications => "notifications",
            Self::Clipboard => "clipboard",
            Self::PointerLock => "pointer-lock",
            Self::Devices => "devices",
            Self::MediaKeys => "media-keys",
            Self::StorageAccess => "storage-access",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [
            Self::Camera,
            Self::Microphone,
            Self::CameraAndMicrophone,
            Self::Screen,
            Self::Location,
            Self::Notifications,
            Self::Clipboard,
            Self::PointerLock,
            Self::Devices,
            Self::MediaKeys,
            Self::StorageAccess,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// What the prompt says the site wants.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Camera => "use your camera",
            Self::Microphone => "use your microphone",
            Self::CameraAndMicrophone => "use your camera and microphone",
            Self::Screen => "see your screen",
            Self::Location => "know your location",
            Self::Notifications => "show notifications",
            Self::Clipboard => "read your clipboard",
            Self::PointerLock => "hide and capture your pointer",
            Self::Devices => "list your cameras and microphones",
            Self::MediaKeys => "play protected (DRM) media",
            Self::StorageAccess => "use its cookies on other sites",
        }
    }
}

/// The remembered answers.
#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    answers: HashMap<(String, Kind), bool>,
}

impl Store {
    /// The answers in `path`; none when it is missing or unreadable. A
    /// `None` path remembers nothing past this run (captive portal).
    pub fn open(path: Option<PathBuf>) -> Self {
        let text = path.as_ref().and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
        Self {
            path,
            answers: parse(&text),
        }
    }

    pub fn get(&self, host: &str, kind: Kind) -> Option<bool> {
        self.answers.get(&(host.to_owned(), kind)).copied()
    }

    pub fn set(&mut self, host: &str, kind: Kind, allow: bool) {
        self.answers.insert((host.to_owned(), kind), allow);
        let Some(path) = &self.path else { return };
        let mut lines: Vec<String> = self
            .answers
            .iter()
            .map(|((h, k), a)| format!("{h} {} {}", k.as_str(), if *a { "allow" } else { "deny" }))
            .collect();
        lines.sort();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(error) = fs::write(path, lines.join("\n") + "\n") {
            tracing::warn!(%error, path = %path.display(), "cannot save the permission");
        }
    }
}

fn parse(text: &str) -> HashMap<(String, Kind), bool> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let host = words.next()?;
            let kind = Kind::parse(words.next()?)?;
            let allow = match words.next()? {
                "allow" => true,
                "deny" => false,
                _ => return None,
            };
            Some(((host.to_owned(), kind), allow))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let path = std::env::temp_dir().join(format!("danube-permissions-{}", std::process::id()));
        let mut store = Store::open(Some(path.clone()));
        assert_eq!(store.get("meet.example", Kind::Camera), None);
        store.set("meet.example", Kind::Camera, true);
        store.set("ads.example", Kind::Notifications, false);
        let store = Store::open(Some(path.clone()));
        assert_eq!(store.get("meet.example", Kind::Camera), Some(true));
        assert_eq!(store.get("ads.example", Kind::Notifications), Some(false));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn ignores_bad_lines() {
        let answers = parse("a.example camera maybe\nb.example teleport allow\nc.example location deny\n");
        assert_eq!(answers.len(), 1);
    }
}
