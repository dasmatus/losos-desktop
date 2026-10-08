//! The window: the page, and nothing of a browser's chrome. No toolbar,
//! address bar or tab strip: Danube is the web view, and derisk's command
//! palette and top-bar menus are its address bar and tabs, built from
//! what derisk.rs registers. Ctrl+L asks derisk for the palette, and the
//! window's title names the active tab. What the window still draws is
//! drawn with mcsapi's components in the desktop's theme: a progress
//! line while a page loads, a permission request's bar, a crashed page's
//! notice, the link under the pointer, and toasts.
//!
//! The page is WebKit's last frame for the active tab, painted as a
//! texture; input over it goes back to WebKit (input.rs).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use eframe::egui::{self, Key, Modifiers};
use mcsapi_components::{
    toast, Alert, Button, ButtonSize, ButtonVariant, Checkbox, Progress, Toaster, Tokens,
};

use crate::config;
use crate::derisk::Update;
use crate::input;
use crate::shared::{Command, Commands, Event, PermissionAsk, Tab, TabId};

/// The window's short side below which it is laid out for a phone.
const PHONE: f32 = 600.0;

pub struct Window {
    commands: Commands,
    events: Receiver<Event>,
    /// The title and tabs, for derisk's menus (derisk.rs), whenever they
    /// change.
    updates: Sender<Update>,
    /// What WebKit has said so far.
    state: State,
    /// The last update did not fit the channel; send it again.
    derisk_behind: bool,
    /// Times Ctrl+L asked for derisk's palette, carried in each update.
    palette: u64,
    /// Captive portal sign-in: one page, no tabs.
    captive: bool,
    /// A button went down over the page and is still held.
    dragging: bool,
    touch_active: bool,
    textures: HashMap<TabId, (egui::TextureHandle, Arc<egui::ColorImage>)>,
    last_size: Option<(i32, i32, f64)>,
    phone: Option<bool>,
    focused: bool,
    title: String,
    remember: bool,
    theme: ThemeWatch,
}

/// The window's picture of WebKit, kept from its events.
#[derive(Default)]
struct State {
    /// In opening order, which derisk's menus list them in.
    tabs: Vec<Tab>,
    active: Option<TabId>,
    /// The last frame each tab rendered.
    frames: HashMap<TabId, Arc<egui::ColorImage>>,
    hovered_link: Option<String>,
    permissions: VecDeque<PermissionAsk>,
    /// How many content blocker rule sets are in force, for the Page menu.
    filters: usize,
    /// The window's title, as sent to derisk.
    title: String,
}

impl State {
    fn tab(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Applies one event; `true` for one that changes the menus.
    fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Opened(tab) => self.tabs.push(tab),
            Event::Changed(tab) => {
                if let Some(t) = self.tab(tab.id) {
                    let crashed = t.crashed;
                    *t = Tab { crashed, ..tab };
                }
            }
            Event::LoadStarted(id) => {
                if let Some(t) = self.tab(id) {
                    t.crashed = false;
                }
            }
            Event::Crashed(id) => {
                if let Some(t) = self.tab(id) {
                    t.crashed = true;
                    t.loading = false;
                }
            }
            Event::Activated(id) => self.active = Some(id),
            Event::Closed(id) => {
                self.tabs.retain(|t| t.id != id);
                self.frames.remove(&id);
                self.permissions.retain(|p| p.tab != id);
                if self.active == Some(id) {
                    self.active = None;
                }
            }
            Event::Frame(id, image) => {
                self.frames.insert(id, image);
                return false;
            }
            Event::HoveredLink(link) => {
                self.hovered_link = link;
                return false;
            }
            Event::Permission(ask) => {
                self.permissions.push_back(ask);
                return false;
            }
            Event::PermissionDone(id) => {
                self.permissions.retain(|p| p.id != id);
                return false;
            }
            Event::Filters(n) => self.filters = n,
            // Handled by the window itself before they get here.
            Event::Copied(_) | Event::Notice(_) | Event::Quit => return false,
        }
        true
    }
}

/// The desktop theme, read again whenever derisk republishes it.
struct ThemeWatch {
    checked: Option<Instant>,
    id: Option<String>,
    tokens: Tokens,
}

impl ThemeWatch {
    fn new() -> Self {
        Self {
            checked: None,
            id: None,
            tokens: Tokens::from_spec(&mcsapi_theme::Theme::dark()),
        }
    }

    /// The theme derisk last published (`$XDG_RUNTIME_DIR/derisk/theme.json`,
    /// whose `id` names a built-in theme or a file in derisk's theme
    /// library), checked once a second, as derisk's own portal does.
    fn tokens(&mut self) -> Tokens {
        if self
            .checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return self.tokens;
        }
        self.checked = Some(Instant::now());
        let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
            return self.tokens;
        };
        let path = std::path::Path::new(&dir).join("derisk/theme.json");
        let id = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|json| json["id"].as_str().map(str::to_owned));
        if id.is_some() && id != self.id {
            let id_str = id.as_deref().unwrap();
            let theme = mcsapi_theme::Theme::builtin(id_str).or_else(|| {
                mcsapi_theme::Library::xdg("derisk")
                    .load(id_str)
                    .ok()
                    .map(|p| p.theme)
            });
            if let Some(theme) = theme {
                self.tokens = Tokens::from_spec(&theme);
            }
            self.id = id;
        }
        self.tokens
    }
}

impl Window {
    pub fn new(
        commands: Commands,
        events: Receiver<Event>,
        updates: Sender<Update>,
        captive: bool,
    ) -> Self {
        Self {
            commands,
            events,
            updates,
            state: State::default(),
            derisk_behind: false,
            palette: 0,
            captive,
            dragging: false,
            touch_active: false,
            textures: HashMap::new(),
            last_size: None,
            phone: None,
            focused: true,
            title: String::new(),
            remember: false,
            theme: ThemeWatch::new(),
        }
    }

    fn send(&self, command: Command) {
        self.commands.send(command);
    }

    /// Takes everything WebKit sent since the last frame; `false` when
    /// WebKit is gone and the window should close.
    fn drain(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        loop {
            match self.events.try_recv() {
                Ok(Event::Quit) | Err(TryRecvError::Disconnected) => return false,
                Ok(Event::Copied(text)) => ctx.copy_text(text),
                Ok(Event::Notice(text)) => toast(ctx, text, None),
                Ok(event) => changed |= self.state.apply(event),
                Err(TryRecvError::Empty) => break,
            }
        }
        if changed || self.derisk_behind {
            self.tell_derisk();
        }
        true
    }

    /// Hands derisk the title and tabs its menus are built from. The
    /// channel is small and the window never waits on it: when derisk has
    /// fallen behind, the update goes with the next frame instead.
    fn tell_derisk(&mut self) {
        let update = Update {
            title: self.state.title.clone(),
            tabs: self.state.tabs.clone(),
            active: self.state.active,
            filters: self.state.filters,
            palette: self.palette,
        };
        self.derisk_behind = self.updates.try_send(update).is_err();
    }

    /// Asks derisk for its command palette, the address bar. The captive
    /// portal window has no derisk thread, and its page is the one it was
    /// opened for.
    fn open_palette(&mut self) {
        if self.captive {
            return;
        }
        self.palette += 1;
        self.tell_derisk();
    }

    /// Browser shortcuts, taken before the page sees the keys.
    fn shortcuts(&mut self, ctx: &egui::Context, active: Option<&Tab>, tabs: &[Tab]) {
        let ctrl = |key| egui::KeyboardShortcut::new(Modifiers::COMMAND, key);
        let alt = |key| egui::KeyboardShortcut::new(Modifiers::ALT, key);
        let pressed = |shortcut| ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed(ctrl(Key::L)) || pressed(alt(Key::D)) {
            self.open_palette();
        }
        if !self.captive && pressed(ctrl(Key::T)) {
            // The new tab is empty, and the palette is where its address
            // is typed.
            self.send(Command::NewTab {
                uri: None,
                activate: true,
            });
            self.open_palette();
        }
        let Some(tab) = active else { return };
        if !self.captive && pressed(ctrl(Key::W)) {
            self.send(Command::Close(tab.id));
        }
        if pressed(ctrl(Key::R)) || pressed(egui::KeyboardShortcut::new(Modifiers::NONE, Key::F5)) {
            self.send(Command::Reload(tab.id));
        }
        if pressed(alt(Key::ArrowLeft)) {
            self.send(Command::Back(tab.id));
        }
        if pressed(alt(Key::ArrowRight)) {
            self.send(Command::Forward(tab.id));
        }
        if self.captive || tabs.is_empty() {
            return;
        }
        let ctrl_shift =
            |key| egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        if pressed(ctrl_shift(Key::Tab)) {
            if let Some(next) = crate::derisk::neighbour(tabs, tab.id, false) {
                self.send(Command::Activate(next));
            }
        } else if pressed(ctrl(Key::Tab)) {
            if let Some(next) = crate::derisk::neighbour(tabs, tab.id, true) {
                self.send(Command::Activate(next));
            }
        }
    }

    /// The prompt for the oldest pending permission request.
    fn permission(&mut self, ui: &mut egui::Ui, ask: &crate::shared::PermissionAsk) {
        let host = if ask.host.is_empty() {
            "This page"
        } else {
            ask.host.as_str()
        };
        ui.add(Alert::new(format!("{host} wants to {}", ask.kind.describe())).description(
            "Nothing is shared unless you allow it. A remembered answer is kept in ~/.config/danube/permissions.",
        ));
        ui.horizontal(|ui| {
            if ui.add(Button::new("Allow").size(ButtonSize::Sm)).clicked() {
                self.send(Command::Permission {
                    id: ask.id,
                    allow: true,
                    remember: self.remember,
                });
            }
            if ui
                .add(
                    Button::new("Deny")
                        .variant(ButtonVariant::Outline)
                        .size(ButtonSize::Sm),
                )
                .clicked()
            {
                self.send(Command::Permission {
                    id: ask.id,
                    allow: false,
                    remember: self.remember,
                });
            }
            ui.add(Checkbox::new(&mut self.remember).label("Remember for this site"));
        });
    }

    /// Draws the active tab's page and sends it the input over it.
    fn page(
        &mut self,
        ui: &mut egui::Ui,
        tab: Option<&Tab>,
        frame: Option<Arc<egui::ColorImage>>,
        tokens: &Tokens,
    ) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ctx = ui.ctx().clone();

        let scale = ctx.pixels_per_point() as f64;
        let size = (
            rect.width().round() as i32,
            rect.height().round() as i32,
            scale,
        );
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            self.send(Command::Resize {
                width: size.0,
                height: size.1,
                scale,
            });
        }

        let Some(tab) = tab else {
            ui.painter().rect_filled(rect, 0.0, tokens.background);
            return;
        };
        match frame {
            Some(image) => {
                let entry = self.textures.entry(tab.id);
                let texture = match entry {
                    std::collections::hash_map::Entry::Occupied(mut slot) => {
                        if !Arc::ptr_eq(&slot.get().1, &image) {
                            slot.get_mut().0.set(
                                egui::ImageData::Color(image.clone()),
                                egui::TextureOptions::NEAREST,
                            );
                            slot.get_mut().1 = image;
                        }
                        slot.get().0.id()
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        let handle = ctx.load_texture(
                            format!("tab-{}", tab.id),
                            egui::ImageData::Color(image.clone()),
                            egui::TextureOptions::NEAREST,
                        );
                        let id = handle.id();
                        slot.insert((handle, image));
                        id
                    }
                };
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
                ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
            }
        }
        if tab.crashed {
            ui.painter().rect_filled(rect, 0.0, tokens.background);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(32.0)));
            child.add(Alert::new("This page stopped working").description(
                "Its web process ended, so nothing on it runs. Reload to start it again.",
            ));
            if child.add(Button::new("Reload")).clicked() {
                self.send(Command::Reload(tab.id));
            }
            return;
        }

        if ctx.input(|i| i.pointer.any_pressed()) && response.hovered() {
            self.dragging = true;
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.dragging = false;
        }
        // Keys go to the page unless a component of the window's own (the
        // permission bar's checkbox, say) has the keyboard.
        let keys = !ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        let hovered = response.hovered() || self.dragging;
        let events: Vec<egui::Event> = events
            .into_iter()
            .filter(|e| !matches!(e, egui::Event::MouseWheel { .. }) || hovered)
            .collect();
        let mut inputs =
            input::translate(&events, rect, keys, self.dragging, &mut self.touch_active);
        if keys {
            for event in &events {
                match event {
                    egui::Event::Copy => inputs.extend(ctrl_key('c')),
                    egui::Event::Cut => inputs.extend(ctrl_key('x')),
                    egui::Event::Paste(text) => self.send(Command::Paste(tab.id, text.clone())),
                    _ => {}
                }
            }
        }
        if !inputs.is_empty() {
            self.send(Command::Input(tab.id, inputs));
        }
        if response.hovered() {
            if let Some(link) = &self.state.hovered_link {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                let galley_pos = egui::pos2(rect.min.x + 6.0, rect.max.y - 22.0);
                let text = egui::RichText::new(link).small().color(tokens.foreground);
                let mut status = ui.new_child(egui::UiBuilder::new().max_rect(
                    egui::Rect::from_min_size(galley_pos, egui::vec2(rect.width() * 0.6, 20.0)),
                ));
                egui::Frame::new()
                    .fill(tokens.card)
                    .corner_radius(tokens.control_radius())
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(&mut status, |ui| {
                        ui.add(egui::Label::new(text).truncate());
                    });
            }
        }
    }
}

fn ctrl_key(c: char) -> Vec<input::Input> {
    let keyval = c as u32;
    vec![
        input::Input::Modifiers(input::Mods(crate::ffi::WPE_MODIFIER_KEYBOARD_CONTROL)),
        input::Input::Key {
            keyval,
            pressed: true,
        },
        input::Input::Key {
            keyval,
            pressed: false,
        },
        input::Input::Modifiers(input::Mods(0)),
    ]
}

fn tab_title(tab: &Tab) -> String {
    if !tab.title.is_empty() {
        tab.title.clone()
    } else if !tab.uri.is_empty() && tab.uri != "about:blank" {
        config::host(&tab.uri)
    } else {
        "New tab".into()
    }
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let tokens = self.theme.tokens();
        tokens.install(&ctx);

        if !self.drain(&ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let tabs = self.state.tabs.clone();
        let active = self.state.active;
        let frame = active.and_then(|id| self.state.frames.get(&id).cloned());
        let permission = self.state.permissions.front().cloned();
        self.textures
            .retain(|id, _| tabs.iter().any(|t| t.id == *id));
        let tab = active
            .and_then(|id| tabs.iter().find(|t| t.id == id))
            .cloned();

        let focused = ctx.input(|i| i.focused);
        if focused != self.focused {
            self.focused = focused;
            self.send(Command::Focus(focused));
        }
        let screen = ctx.content_rect();
        let phone = screen.width().min(screen.height()) < PHONE;
        if self.phone != Some(phone) {
            self.phone = Some(phone);
            self.send(Command::Phone(phone));
        }

        let title = if self.captive {
            "Sign in to network".to_owned()
        } else {
            match &tab {
                Some(t) => format!("{} — Danube", tab_title(t)),
                None => "Danube".to_owned(),
            }
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.state.title = title.clone();
            self.title = title;
            self.tell_derisk();
        }

        self.shortcuts(&ctx, tab.as_ref(), &tabs);

        let bar = egui::Frame::new()
            .fill(tokens.background)
            .inner_margin(egui::Margin::symmetric(8, 6));
        // A loading page's only sign, a line along the top edge: the
        // window has no toolbar to put a spinner in.
        if let Some(t) = tab.as_ref().filter(|t| t.loading) {
            egui::Panel::top("progress")
                .frame(egui::Frame::new().fill(tokens.background))
                .show(ui, |ui| {
                    ui.add(Progress::new(t.progress as f32).width(ui.available_width()));
                });
        }
        if let Some(ask) = &permission {
            egui::Panel::top("permission")
                .frame(bar)
                .show(ui, |ui| self.permission(ui, ask));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(tokens.background))
            .show(ui, |ui| {
                self.page(ui, tab.as_ref(), frame, &tokens);
            });
        Toaster::show(&ctx);
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.send(Command::Quit);
    }
}
