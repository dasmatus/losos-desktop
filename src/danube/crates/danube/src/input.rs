//! The window's input, turned from egui's events into what WPE's view
//! takes: positions relative to the page, buttons by number, and keys as
//! X keysyms, which is how WPE names a key whatever platform it runs on.
//!
//! egui reports typed text and keys apart. Text goes to the page as one
//! key press per character, keysym `0x0100_0000 + code point` as WPE
//! expects for Unicode; a key goes only when it types nothing (arrows,
//! Enter, Backspace) or carries Ctrl or Alt, so a shortcut reaches the page
//! and a letter is never typed twice.

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect, TouchPhase};

/// One input event for the page, in logical pixels from its top left.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Move {
        x: f64,
        y: f64,
    },
    Button {
        x: f64,
        y: f64,
        button: u32,
        pressed: bool,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        precise: bool,
    },
    Key {
        keyval: u32,
        pressed: bool,
    },
    Touch {
        id: u32,
        x: f64,
        y: f64,
        phase: Touch,
    },
    Leave,
    /// The keyboard modifiers now held; carried by every event after it.
    Modifiers(Mods),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Down,
    Move,
    Up,
    Cancel,
}

/// Held modifiers, in WPE's bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub u32);

impl From<Modifiers> for Mods {
    fn from(m: Modifiers) -> Self {
        use crate::ffi::*;
        let mut bits = 0;
        if m.ctrl || m.command {
            bits |= WPE_MODIFIER_KEYBOARD_CONTROL;
        }
        if m.shift {
            bits |= WPE_MODIFIER_KEYBOARD_SHIFT;
        }
        if m.alt {
            bits |= WPE_MODIFIER_KEYBOARD_ALT;
        }
        if m.mac_cmd {
            bits |= WPE_MODIFIER_KEYBOARD_META;
        }
        Mods(bits)
    }
}

/// The keysym for a key that types nothing by itself, or for a letter or
/// digit pressed with Ctrl or Alt.
pub fn keysym(key: Key) -> Option<u32> {
    let sym = match key {
        Key::Enter => 0xff0d,
        Key::Tab => 0xff09,
        Key::Backspace => 0xff08,
        Key::Escape => 0xff1b,
        Key::Delete => 0xffff,
        Key::Insert => 0xff63,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::ArrowLeft => 0xff51,
        Key::ArrowUp => 0xff52,
        Key::ArrowRight => 0xff53,
        Key::ArrowDown => 0xff54,
        Key::Space => 0x20,
        Key::F1 => 0xffbe,
        Key::F2 => 0xffbf,
        Key::F3 => 0xffc0,
        Key::F4 => 0xffc1,
        Key::F5 => 0xffc2,
        Key::F6 => 0xffc3,
        Key::F7 => 0xffc4,
        Key::F8 => 0xffc5,
        Key::F9 => 0xffc6,
        Key::F10 => 0xffc7,
        Key::F11 => 0xffc8,
        Key::F12 => 0xffc9,
        _ => {
            // Letters, digits and punctuation: their keysym is their
            // Latin-1 code, lower case for letters.
            let name = key.symbol_or_name();
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() || !c.is_ascii() {
                return None;
            }
            c.to_ascii_lowercase() as u32
        }
    };
    Some(sym)
}

/// The keysym WPE takes for a typed character.
pub fn char_keysym(c: char) -> u32 {
    let code = c as u32;
    // Latin-1 keysyms are the code itself; everything else is Unicode's
    // plane offset by 0x0100_0000.
    if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        code
    } else {
        0x0100_0000 + code
    }
}

/// Whether a key press should go to the page as a key rather than wait for
/// the text it types.
fn sends_key(key: Key, modifiers: Modifiers) -> bool {
    let types_text = matches!(key.symbol_or_name().chars().count(), 1) || key == Key::Space;
    !types_text || modifiers.ctrl || modifiers.command || modifiers.alt
}

/// What egui's `events` mean for a page drawn at `rect`. Pointer events
/// only count over the page (or while a button pressed on it is held,
/// `dragging`); keys and text only while it has focus.
pub fn translate(
    events: &[Event],
    rect: Rect,
    focused: bool,
    dragging: bool,
    touch_active: &mut bool,
) -> Vec<Input> {
    let at = |p: Pos2| ((p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64);
    let over = |p: Pos2| dragging || rect.contains(p);
    // egui also reports the first finger of a touch as a mouse; WPE takes
    // the touch itself, which scrolls and zooms as a finger should, so the
    // mouse copies are dropped while a finger is down.
    let has_touch = events.iter().any(|e| matches!(e, Event::Touch { .. }));
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::ModifiersChanged(m) => out.push(Input::Modifiers((*m).into())),
            Event::PointerMoved(p) if !has_touch && !*touch_active => {
                if over(*p) {
                    let (x, y) = at(*p);
                    out.push(Input::Move { x, y });
                } else {
                    out.push(Input::Leave);
                }
            }
            Event::PointerGone if !has_touch && !*touch_active => out.push(Input::Leave),
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } if !has_touch && !*touch_active && (over(*pos) || !*pressed) => {
                let button = match button {
                    PointerButton::Primary => 1,
                    PointerButton::Middle => 2,
                    PointerButton::Secondary => 3,
                    // Back and forward buttons are browser commands; the
                    // window handles them.
                    _ => continue,
                };
                let (x, y) = at(*pos);
                out.push(Input::Modifiers((*modifiers).into()));
                out.push(Input::Button {
                    x,
                    y,
                    button,
                    pressed: *pressed,
                });
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } => {
                // Wheel events carry no position; egui's latest pointer
                // position is the one the window passes in as hover.
                let _ = modifiers;
                let precise = *unit == egui::MouseWheelUnit::Point;
                let scale = match unit {
                    egui::MouseWheelUnit::Page => rect.height() as f64 / 40.0,
                    _ => 1.0,
                };
                out.push(Input::Scroll {
                    x: f64::NAN,
                    y: f64::NAN,
                    dx: delta.x as f64 * scale,
                    dy: delta.y as f64 * scale,
                    precise,
                });
            }
            Event::Touch { id, phase, pos, .. } => {
                let phase = match phase {
                    TouchPhase::Start => Touch::Down,
                    TouchPhase::Move => Touch::Move,
                    TouchPhase::End => Touch::Up,
                    TouchPhase::Cancel => Touch::Cancel,
                };
                match phase {
                    Touch::Down if !rect.contains(*pos) => continue,
                    Touch::Down => *touch_active = true,
                    Touch::Up | Touch::Cancel => *touch_active = false,
                    Touch::Move => {}
                }
                let (x, y) = at(*pos);
                out.push(Input::Touch {
                    id: id.0 as u32,
                    x,
                    y,
                    phase,
                });
            }
            Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } if focused => {
                if !sends_key(*key, *modifiers) {
                    continue;
                }
                if let Some(keyval) = keysym(*key) {
                    out.push(Input::Modifiers((*modifiers).into()));
                    out.push(Input::Key {
                        keyval,
                        pressed: *pressed,
                    });
                }
            }
            Event::Text(text) if focused => push_text(&mut out, text),
            Event::Ime(egui::ImeEvent::Commit(text)) if focused => push_text(&mut out, text),
            _ => {}
        }
    }
    out
}

fn push_text(out: &mut Vec<Input>, text: &str) {
    for c in text.chars() {
        let keyval = char_keysym(c);
        out.push(Input::Key {
            keyval,
            pressed: true,
        });
        out.push(Input::Key {
            keyval,
            pressed: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 40.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn keysyms() {
        assert_eq!(keysym(Key::Enter), Some(0xff0d));
        assert_eq!(keysym(Key::A), Some('a' as u32));
        assert_eq!(keysym(Key::Num5), Some('5' as u32));
        assert_eq!(char_keysym('a'), 0x61);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('€'), 0x0100_20ac);
    }

    #[test]
    fn letters_go_as_text_and_shortcuts_as_keys() {
        let plain = Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let ctrl = Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        };
        let text = Event::Text("a".into());
        let mut touch = false;
        let out = translate(&[plain, text], page(), true, false, &mut touch);
        assert_eq!(
            out,
            vec![
                Input::Key {
                    keyval: 0x61,
                    pressed: true
                },
                Input::Key {
                    keyval: 0x61,
                    pressed: false
                }
            ]
        );
        let out = translate(&[ctrl], page(), true, false, &mut touch);
        assert_eq!(
            out.last(),
            Some(&Input::Key {
                keyval: 0x61,
                pressed: true
            })
        );
    }

    #[test]
    fn positions_are_relative_to_the_page() {
        let click = Event::PointerButton {
            pos: Pos2::new(10.0, 50.0),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        let outside = Event::PointerButton {
            pos: Pos2::new(10.0, 10.0),
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        };
        let mut touch = false;
        let out = translate(&[click, outside], page(), false, false, &mut touch);
        assert_eq!(
            out,
            vec![
                Input::Modifiers(Mods(0)),
                Input::Button {
                    x: 10.0,
                    y: 10.0,
                    button: 1,
                    pressed: true
                }
            ]
        );
    }

    #[test]
    fn touches_replace_their_mouse_copies() {
        let events = [
            Event::Touch {
                device_id: egui::TouchDeviceId(1),
                id: egui::TouchId(7),
                phase: TouchPhase::Start,
                pos: Pos2::new(5.0, 45.0),
                force: None,
            },
            Event::PointerButton {
                pos: Pos2::new(5.0, 45.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ];
        let mut touch = false;
        let out = translate(&events, page(), false, false, &mut touch);
        assert_eq!(
            out,
            vec![Input::Touch {
                id: 7,
                x: 5.0,
                y: 5.0,
                phase: Touch::Down
            }]
        );
        assert!(touch);
    }
}
