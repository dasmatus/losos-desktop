//! The window: tabs, the address bar and the page, drawn with mcsapi's
//! components in the desktop's theme. On a desktop the tabs run along the
//! top above the toolbar; on a phone (the window's short side under
//! 600 px) the address bar sits at the bottom, under the thumb, and the
//! tabs are a switcher behind a button that shows their count.
//!
//! The page is WebKit's last frame for the active tab, painted as a
//! texture; input over it goes back to WebKit (input.rs).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers};
use mcsapi_components::{
    Alert, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Checkbox, Input, Progress,
    Toaster, Tokens, toast,
};

use crate::config::{self, Settings};
use crate::input;
use crate::shared::{Command, Shared, Tab, TabId};

/// The window's short side below which it is laid out for a phone.
const PHONE: f32 = 600.0;

pub struct Window {
    shared: Arc<Shared>,
    settings: Settings,
    /// Captive portal sign-in: one page, no tabs.
    captive: bool,
    /// What is typed in the address bar while it has focus.
    typing: Option<String>,
    address_id: egui::Id,
    /// The page took the last click, so keys go to it.
    page_focused: bool,
    /// A button went down over the page and is still held.
    dragging: bool,
    touch_active: bool,
    textures: HashMap<TabId, (egui::TextureHandle, Arc<egui::ColorImage>)>,
    last_size: Option<(i32, i32, f64)>,
    phone: Option<bool>,
    focused: bool,
    title: String,
    switcher: bool,
    remember: bool,
    theme: ThemeWatch,
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
        if self.checked.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
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
            let theme = mcsapi_theme::Theme::builtin(id_str)
                .or_else(|| mcsapi_theme::Library::xdg("derisk").load(id_str).ok().map(|p| p.theme));
            if let Some(theme) = theme {
                self.tokens = Tokens::from_spec(&theme);
            }
            self.id = id;
        }
        self.tokens
    }
}

impl Window {
    pub fn new(shared: Arc<Shared>, settings: Settings, captive: bool) -> Self {
        Self {
            shared,
            settings,
            captive,
            typing: None,
            address_id: egui::Id::new("danube-address"),
            page_focused: true,
            dragging: false,
            touch_active: false,
            textures: HashMap::new(),
            last_size: None,
            phone: None,
            focused: true,
            title: String::new(),
            switcher: false,
            remember: false,
            theme: ThemeWatch::new(),
        }
    }

    fn send(&self, command: Command) {
        self.shared.send(command);
    }

    /// Browser shortcuts, taken before the page sees the keys.
    fn shortcuts(&mut self, ctx: &egui::Context, active: Option<&Tab>, tabs: &[Tab]) {
        let ctrl = |key| egui::KeyboardShortcut::new(Modifiers::COMMAND, key);
        let alt = |key| egui::KeyboardShortcut::new(Modifiers::ALT, key);
        let pressed = |shortcut| ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        if pressed(ctrl(Key::L)) || pressed(alt(Key::D)) {
            ctx.memory_mut(|m| m.request_focus(self.address_id));
            self.page_focused = false;
        }
        if !self.captive && pressed(ctrl(Key::T)) {
            self.send(Command::NewTab { uri: None, activate: true });
            ctx.memory_mut(|m| m.request_focus(self.address_id));
            self.page_focused = false;
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
        if !self.captive && pressed(ctrl(Key::Tab)) && !tabs.is_empty() {
            let at = tabs.iter().position(|t| t.id == tab.id).unwrap_or(0);
            self.send(Command::Activate(tabs[(at + 1) % tabs.len()].id));
        }
    }

    fn address_bar(&mut self, ui: &mut egui::Ui, tab: Option<&Tab>, tokens: &Tokens) {
        let Some(tab) = tab else { return };
        if !tab.uri.is_empty() {
            let (text, color) = if tab.secure {
                ("Secure", tokens.muted_foreground)
            } else if tab.uri.starts_with("http://") {
                ("Not secure", tokens.destructive)
            } else {
                ("", tokens.muted_foreground)
            };
            if !text.is_empty() {
                ui.label(egui::RichText::new(text).small().color(color))
                    .on_hover_text(config::host(&tab.uri));
            }
        }
        let has_focus = ui.memory(|m| m.has_focus(self.address_id));
        let mut text = match (&self.typing, has_focus) {
            (Some(t), true) => t.clone(),
            _ => tab.uri.clone(),
        };
        let response = ui.add(Input::new(&mut text).placeholder("Search or enter address"));
        // The field's id, for Ctrl+L to focus it; stable while the toolbar's
        // layout is.
        self.address_id = response.id;
        if response.has_focus() {
            self.page_focused = false;
            self.typing = Some(text.clone());
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                if let Some(uri) = config::address(&text, &self.settings.search) {
                    self.send(Command::Load(tab.id, uri));
                }
                self.page_focused = true;
            }
            self.typing = None;
        }
    }

    fn nav_buttons(&self, ui: &mut egui::Ui, tab: Option<&Tab>) {
        let Some(tab) = tab else { return };
        let icon = |text: &str| Button::new(text).variant(ButtonVariant::Ghost).size(ButtonSize::Icon);
        if ui.add(icon("←").enabled(tab.can_go_back)).on_hover_text("Back (Alt+Left)").clicked() {
            self.send(Command::Back(tab.id));
        }
        if ui.add(icon("→").enabled(tab.can_go_forward)).on_hover_text("Forward (Alt+Right)").clicked() {
            self.send(Command::Forward(tab.id));
        }
        if tab.loading {
            if ui.add(icon("✕")).on_hover_text("Stop").clicked() {
                self.send(Command::Stop(tab.id));
            }
        } else if ui.add(icon("⟳")).on_hover_text("Reload (Ctrl+R)").clicked() {
            self.send(Command::Reload(tab.id));
        }
    }

    fn shield(&self, ui: &mut egui::Ui, filters: usize) {
        if self.captive {
            return;
        }
        let (text, variant, hover) = if filters > 0 {
            ("Ad blocking", BadgeVariant::Secondary, format!("{filters} filter sets from losos-adblock are in force"))
        } else {
            ("No filters", BadgeVariant::Outline, "losos-adblock has compiled no filter lists yet; its DNS blocking still applies".to_owned())
        };
        ui.add(Badge::new(text).variant(variant)).on_hover_text(hover);
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui, tabs: &[Tab], active: Option<TabId>, tokens: &Tokens) {
        ui.horizontal(|ui| {
            egui::ScrollArea::horizontal().id_salt("tabs").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tab in tabs {
                        let selected = Some(tab.id) == active;
                        let fill = if selected { tokens.card } else { tokens.background };
                        egui::Frame::new()
                            .fill(fill)
                            .corner_radius(tokens.control_radius())
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.set_width(160.0);
                                ui.horizontal(|ui| {
                                    let title = tab_title(tab);
                                    let label = ui.add(
                                        egui::Label::new(egui::RichText::new(&title).color(if selected {
                                            tokens.foreground
                                        } else {
                                            tokens.muted_foreground
                                        }))
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                    );
                                    if label.on_hover_text(&title).clicked() {
                                        self.send(Command::Activate(tab.id));
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let close = Button::new("×").variant(ButtonVariant::Ghost).size(ButtonSize::Sm);
                                        if ui.add(close).on_hover_text("Close tab (Ctrl+W)").clicked() {
                                            self.send(Command::Close(tab.id));
                                        }
                                    });
                                });
                            });
                    }
                    let new = Button::new("+").variant(ButtonVariant::Ghost).size(ButtonSize::Sm);
                    if ui.add(new).on_hover_text("New tab (Ctrl+T)").clicked() {
                        self.send(Command::NewTab { uri: None, activate: true });
                    }
                });
            });
        });
    }

    /// The phone's tab switcher, in place of the page.
    fn switcher(&mut self, ui: &mut egui::Ui, tabs: &[Tab], active: Option<TabId>, tokens: &Tokens) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for tab in tabs {
                let selected = Some(tab.id) == active;
                egui::Frame::new()
                    .fill(if selected { tokens.card } else { tokens.background })
                    .stroke(tokens.border_stroke())
                    .corner_radius(tokens.card_radius())
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let open = ui.add(
                                egui::Label::new(egui::RichText::new(tab_title(tab)).strong())
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            );
                            if open.clicked() {
                                self.send(Command::Activate(tab.id));
                                self.switcher = false;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(Button::new("×").variant(ButtonVariant::Ghost).size(ButtonSize::Icon)).clicked() {
                                    self.send(Command::Close(tab.id));
                                }
                            });
                        });
                        ui.label(egui::RichText::new(config::host(&tab.uri)).small().color(tokens.muted_foreground));
                    });
                ui.add_space(6.0);
            }
            if ui.add(Button::new("New tab").variant(ButtonVariant::Outline)).clicked() {
                self.send(Command::NewTab { uri: None, activate: true });
                self.switcher = false;
            }
        });
    }

    /// The prompt for the oldest pending permission request.
    fn permission(&mut self, ui: &mut egui::Ui, ask: &crate::shared::PermissionAsk) {
        let host = if ask.host.is_empty() { "This page" } else { ask.host.as_str() };
        ui.add(Alert::new(format!("{host} wants to {}", ask.kind.describe())).description(
            "Nothing is shared unless you allow it. A remembered answer is kept in ~/.config/danube/permissions.",
        ));
        ui.horizontal(|ui| {
            if ui.add(Button::new("Allow").size(ButtonSize::Sm)).clicked() {
                self.send(Command::Permission { id: ask.id, allow: true, remember: self.remember });
            }
            if ui.add(Button::new("Deny").variant(ButtonVariant::Outline).size(ButtonSize::Sm)).clicked() {
                self.send(Command::Permission { id: ask.id, allow: false, remember: self.remember });
            }
            ui.add(Checkbox::new(&mut self.remember).label("Remember for this site"));
        });
    }

    /// Draws the active tab's page and sends it the input over it.
    fn page(&mut self, ui: &mut egui::Ui, tab: Option<&Tab>, frame: Option<Arc<egui::ColorImage>>, tokens: &Tokens) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ctx = ui.ctx().clone();

        let scale = ctx.pixels_per_point() as f64;
        let size = (rect.width().round() as i32, rect.height().round() as i32, scale);
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            self.send(Command::Resize { width: size.0, height: size.1, scale });
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
                            slot.get_mut().0.set(egui::ImageData::Color(image.clone()), egui::TextureOptions::NEAREST);
                            slot.get_mut().1 = image;
                        }
                        slot.get().0.id()
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        let handle = ctx.load_texture(format!("tab-{}", tab.id), egui::ImageData::Color(image.clone()), egui::TextureOptions::NEAREST);
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
            child.add(
                Alert::new("This page stopped working")
                    .description("Its web process ended, so nothing on it runs. Reload to start it again."),
            );
            if child.add(Button::new("Reload")).clicked() {
                self.send(Command::Reload(tab.id));
            }
            return;
        }

        if response.clicked() || response.drag_started() {
            self.page_focused = true;
            ctx.memory_mut(|m| m.surrender_focus(self.address_id));
        }
        if ctx.input(|i| i.pointer.any_pressed()) && response.hovered() {
            self.dragging = true;
        }
        if !ctx.input(|i| i.pointer.any_down()) {
            self.dragging = false;
        }
        let keys = self.page_focused && !ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        let hovered = response.hovered() || self.dragging;
        let events: Vec<egui::Event> = events
            .into_iter()
            .filter(|e| !matches!(e, egui::Event::MouseWheel { .. }) || hovered)
            .collect();
        let mut inputs = input::translate(&events, rect, keys, self.dragging, &mut self.touch_active);
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
            if let Some(link) = &self.shared.state.lock().unwrap().hovered_link {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                let galley_pos = egui::pos2(rect.min.x + 6.0, rect.max.y - 22.0);
                let text = egui::RichText::new(link).small().color(tokens.foreground);
                let mut status = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(galley_pos, egui::vec2(rect.width() * 0.6, 20.0))));
                egui::Frame::new().fill(tokens.card).corner_radius(tokens.control_radius()).inner_margin(egui::Margin::symmetric(6, 2)).show(&mut status, |ui| {
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
        input::Input::Key { keyval, pressed: true },
        input::Input::Key { keyval, pressed: false },
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
        let _ = self.shared.ctx.set(ctx.clone());
        let tokens = self.theme.tokens();
        tokens.install(&ctx);

        let (tabs, active, frame, filters, permission, copied, notices, quit) = {
            let mut s = self.shared.state.lock().unwrap();
            let active = s.active;
            (
                s.tabs.clone(),
                active,
                active.and_then(|id| s.frames.get(&id).cloned()),
                s.filters,
                s.permissions.front().cloned(),
                s.copied.take(),
                s.notices.drain(..).collect::<Vec<_>>(),
                s.quit,
            )
        };
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(text) = copied {
            ctx.copy_text(text);
        }
        for notice in notices {
            toast(&ctx, notice, None);
        }
        self.textures.retain(|id, _| tabs.iter().any(|t| t.id == *id));
        let tab = active.and_then(|id| tabs.iter().find(|t| t.id == id)).cloned();

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
            self.shared.update(|s| s.window_title = title.clone());
            self.title = title;
        }

        self.shortcuts(&ctx, tab.as_ref(), &tabs);

        let bar = egui::Frame::new().fill(tokens.background).inner_margin(egui::Margin::symmetric(8, 6));
        let toolbar = |this: &mut Self, ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                this.nav_buttons(ui, tab.as_ref());
                if phone && !this.captive {
                    let count = Button::new(tabs.len().to_string()).variant(ButtonVariant::Outline).size(ButtonSize::Icon);
                    if ui.add(count).on_hover_text("Tabs").clicked() {
                        this.switcher = !this.switcher;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    this.shield(ui, filters);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        this.address_bar(ui, tab.as_ref(), &tokens);
                    });
                });
            });
            if let Some(t) = &tab {
                if t.loading {
                    ui.add(Progress::new(t.progress as f32).width(ui.available_width()));
                }
            }
        };

        if phone {
            egui::Panel::bottom("toolbar").frame(bar).show(ui, |ui| toolbar(self, ui));
        } else {
            egui::Panel::top("toolbar").frame(bar).show(ui, |ui| {
                if !self.captive {
                    self.tab_strip(ui, &tabs, active, &tokens);
                }
                toolbar(self, ui);
            });
        }
        if let Some(ask) = &permission {
            egui::Panel::top("permission").frame(bar).show(ui, |ui| self.permission(ui, ask));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(tokens.background)).show(ui, |ui| {
            if phone && self.switcher {
                self.switcher(ui, &tabs, active, &tokens);
            } else {
                self.page(ui, tab.as_ref(), frame, &tokens);
            }
        });
        Toaster::show(&ctx);
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.shared.send(Command::Quit);
    }
}
