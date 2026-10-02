//! The LosOS Desktop installer, on tty1 of the installer ISO.
//!
//! Three steps, in order: get online, pick a disk and type `erase`, watch the
//! install. Getting online is skipped when a cable already did it. Wi-Fi is
//! wpa_supplicant's, driven over its control socket (`wpa.rs`); networkd runs
//! DHCP on whatever link comes up, so joining a network is all this has to
//! do. The install is systemd-repart and then systemd-sysupdate (`install.rs`)
//! from definitions the OS generated, so this program decides nothing about
//! what lands on the disk.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use losos_installer::disks::{self, Disk};
use losos_installer::install::{self, Plan, Step};
use losos_installer::network;
use losos_installer::wpa::{self, Control, Network};

const USAGE: &str = "usage: losos-installer --repart-definitions DIR --sysupdate-definitions DIR \
[--esp DIR] [--medium DIR] [--work DIR] [--source URL]";

/// How long a scan is given before its results are final. Results are shown
/// as they arrive; this only decides when the screen stops saying "scanning".
const SCAN_TIME: Duration = Duration::from_secs(4);
/// Association, the handshake and DHCP together. A wrong passphrase looks
/// like a handshake that never completes, so this is also how long it takes
/// to say so.
const CONNECT_TIME: Duration = Duration::from_secs(30);
/// Enough of sysupdate's output to read back after a failure.
const LOG_LINES: usize = 1000;

struct Args {
    repart_definitions: PathBuf,
    sysupdate_templates: PathBuf,
    esp: PathBuf,
    medium: String,
    work: PathBuf,
    source: Option<String>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let mut repart = None;
        let mut sysupdate = None;
        let mut parsed = Args {
            repart_definitions: PathBuf::new(),
            sysupdate_templates: PathBuf::new(),
            esp: PathBuf::from("/run/losos-installer/esp"),
            medium: "/iso".into(),
            work: PathBuf::from("/run/losos-installer"),
            source: None,
        };
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))?;
            match flag.as_str() {
                "--repart-definitions" => repart = Some(PathBuf::from(value)),
                "--sysupdate-definitions" => sysupdate = Some(PathBuf::from(value)),
                "--esp" => parsed.esp = PathBuf::from(value),
                "--medium" => parsed.medium = value,
                "--work" => parsed.work = PathBuf::from(value),
                "--source" => parsed.source = Some(value),
                _ => return Err(format!("unknown argument {flag}\n{USAGE}")),
            }
        }
        parsed.repart_definitions = repart.ok_or(USAGE)?;
        parsed.sysupdate_templates = sysupdate.ok_or(USAGE)?;
        Ok(parsed)
    }
}

enum Phase {
    Idle,
    Scanning(Instant),
    Passphrase {
        network: Network,
        input: String,
    },
    Connecting {
        network: Network,
        id: u32,
        since: Instant,
    },
}

struct Wifi {
    interface: Option<String>,
    control: Option<Control>,
    networks: Vec<Network>,
    list: ListState,
    phase: Phase,
    message: Option<String>,
    /// Going online moves on to the disks by itself at start-up and after
    /// joining a network, but not when someone came back here on purpose.
    advance: bool,
}

impl Wifi {
    fn new(advance: bool) -> Self {
        Wifi {
            interface: None,
            control: None,
            networks: Vec::new(),
            list: ListState::default().with_selected(Some(0)),
            phase: Phase::Idle,
            message: None,
            advance,
        }
    }

    fn scan(&mut self) {
        let Some(control) = &self.control else { return };
        // A reply of FAIL-BUSY means a scan is already running, which serves
        // as well, so the reply is not checked.
        match control.request("SCAN") {
            Ok(_) => self.phase = Phase::Scanning(Instant::now()),
            Err(e) => self.message = Some(format!("Scanning failed: {e}")),
        }
    }

    fn tick(&mut self) {
        let Some(control) = &self.control else {
            self.interface = wpa::interfaces(Path::new("/sys/class/net"))
                .into_iter()
                .next();
            // wpa_supplicant makes its socket a moment after the interface
            // appears, so this is retried every tick until it is there.
            if let Some(control) = self
                .interface
                .as_deref()
                .and_then(|i| Control::open(i).ok())
            {
                self.control = Some(control);
                self.scan();
            }
            return;
        };
        // NixOS restarts wpa_supplicant whenever a wireless interface comes or
        // goes, and the restarted daemon listens on a new socket. A dead
        // connection is dropped here and the next tick opens a new one.
        if control.request("PING").is_err() {
            self.control = None;
            if let Phase::Connecting { .. } = self.phase {
                self.phase = Phase::Idle;
            }
            return;
        }
        match &self.phase {
            Phase::Scanning(since) => {
                if let Ok(reply) = control.request("SCAN_RESULTS") {
                    self.networks = wpa::parse_scan_results(&reply);
                }
                if since.elapsed() > SCAN_TIME {
                    self.phase = Phase::Idle;
                }
            }
            Phase::Connecting { network, id, since } if since.elapsed() > CONNECT_TIME => {
                let _ = control.forget(*id);
                self.message = Some(format!(
                    "Could not join {}. Check the passphrase and try again.",
                    network.name()
                ));
                self.phase = Phase::Idle;
            }
            _ => {}
        }
    }

    fn join(&mut self, network: Network, passphrase: &str) {
        let Some(control) = &self.control else { return };
        match control.connect(&network, passphrase) {
            Ok(id) => {
                self.message = None;
                self.advance = true;
                self.phase = Phase::Connecting {
                    network,
                    id,
                    since: Instant::now(),
                };
            }
            Err(e) => {
                self.message = Some(e.to_string());
                self.phase = Phase::Idle;
            }
        }
    }

    fn wpa_state(&self) -> Option<String> {
        let status = self.control.as_ref()?.status().ok()?;
        status
            .into_iter()
            .find(|(k, _)| k == "wpa_state")
            .map(|(_, v)| v)
    }
}

struct Disks {
    disks: Vec<Disk>,
    list: ListState,
    medium: Option<String>,
}

impl Disks {
    fn load(args: &Args) -> Self {
        let sys = Path::new("/sys");
        let medium = std::fs::read_to_string("/proc/self/mountinfo")
            .ok()
            .and_then(|info| disks::backing_disk(&info, sys, &args.medium));
        Disks {
            disks: disks::list(sys, medium.as_deref()),
            list: ListState::default().with_selected(Some(0)),
            medium,
        }
    }
}

struct Progress {
    disk: Disk,
    step: Option<Step>,
    log: Vec<String>,
    result: Option<Result<(), String>>,
    events: Receiver<install::Event>,
}

enum Screen {
    Network(Wifi),
    Disks(Disks),
    Confirm { disk: Disk, input: String },
    Installing(Progress),
}

struct App {
    args: Args,
    screen: Screen,
    online: bool,
    quit: bool,
}

impl App {
    fn new(args: Args) -> Self {
        App {
            args,
            screen: Screen::Network(Wifi::new(true)),
            online: false,
            quit: false,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            self.tick();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(250))?
                && let TermEvent::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.key(key);
            }
        }
        Ok(())
    }

    fn tick(&mut self) {
        self.online = network::online(Path::new("/run/systemd/netif/state"));
        match &mut self.screen {
            Screen::Network(wifi) => {
                wifi.tick();
                let typing = matches!(wifi.phase, Phase::Passphrase { .. });
                if self.online && wifi.advance && !typing {
                    self.screen = Screen::Disks(Disks::load(&self.args));
                }
            }
            Screen::Installing(progress) => {
                while let Ok(event) = progress.events.try_recv() {
                    match event {
                        install::Event::Step(step) => progress.step = Some(step),
                        install::Event::Line(line) => {
                            progress.log.push(line);
                            if progress.log.len() > LOG_LINES {
                                progress.log.drain(..progress.log.len() - LOG_LINES);
                            }
                        }
                        install::Event::Finished(result) => progress.result = Some(result),
                    }
                }
            }
            _ => {}
        }
    }

    fn key(&mut self, key: KeyEvent) {
        let installing = matches!(&self.screen, Screen::Installing(p) if p.result.is_none());
        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !installing
        {
            // On the ISO systemd starts the installer again; this is for
            // someone running it by hand.
            self.quit = true;
            return;
        }
        let next = match &mut self.screen {
            Screen::Network(wifi) => network_key(wifi, key, self.online, &self.args),
            Screen::Disks(choice) => disks_key(choice, key, &self.args),
            Screen::Confirm { disk, input } => confirm_key(disk, input, key, &self.args),
            Screen::Installing(progress) => installing_key(progress, key, &self.args),
        };
        if let Some(screen) = next {
            self.screen = screen;
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let status = if self.online { "online" } else { "offline" };
        let lines = vec![
            Line::styled(
                "LosOS Desktop installer",
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Line::raw(match &self.args.source {
                Some(source) => format!("Network: {status}    Installing from: {source}"),
                None => format!("Network: {status}"),
            }),
        ];
        frame.render_widget(Paragraph::new(lines), header);

        let hints = match &mut self.screen {
            Screen::Network(wifi) => draw_network(frame, body, wifi, self.online),
            Screen::Disks(choice) => draw_disks(frame, body, choice),
            Screen::Confirm { disk, input } => draw_confirm(frame, body, disk, input),
            Screen::Installing(progress) => draw_installing(frame, body, progress),
        };
        // The live system logs root in on tty2, for wpa_cli and anything
        // else this screen does not cover.
        frame.render_widget(Paragraph::new(format!("{hints}    Alt+F2: shell")), footer);
    }
}

fn move_selection(list: &mut ListState, len: usize, key: KeyCode) {
    if len == 0 {
        return;
    }
    let current = list.selected().unwrap_or(0).min(len - 1);
    let next = match key {
        KeyCode::Up => current.saturating_sub(1),
        KeyCode::Down => (current + 1).min(len - 1),
        _ => current,
    };
    list.select(Some(next));
}

fn edit(input: &mut String, key: KeyEvent) {
    match key.code {
        KeyCode::Backspace => {
            input.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => input.push(c),
        _ => {}
    }
}

fn network_key(wifi: &mut Wifi, key: KeyEvent, online: bool, args: &Args) -> Option<Screen> {
    if let Phase::Passphrase { network, input } = &mut wifi.phase {
        match key.code {
            KeyCode::Esc => wifi.phase = Phase::Idle,
            KeyCode::Enter => {
                let (network, input) = (network.clone(), std::mem::take(input));
                wifi.join(network, &input);
            }
            _ => edit(input, key),
        }
        return None;
    }
    match key.code {
        KeyCode::Up | KeyCode::Down => {
            move_selection(&mut wifi.list, wifi.networks.len(), key.code)
        }
        KeyCode::Char('r') => wifi.scan(),
        KeyCode::Tab if online => return Some(Screen::Disks(Disks::load(args))),
        KeyCode::Enter => {
            let network = wifi
                .list
                .selected()
                .and_then(|i| wifi.networks.get(i))
                .cloned()?;
            wifi.message = None;
            if network.security.needs_passphrase() {
                wifi.phase = Phase::Passphrase {
                    network,
                    input: String::new(),
                };
            } else {
                wifi.join(network, "");
            }
        }
        _ => {}
    }
    None
}

fn disks_key(choice: &mut Disks, key: KeyEvent, args: &Args) -> Option<Screen> {
    match key.code {
        KeyCode::Up | KeyCode::Down => {
            move_selection(&mut choice.list, choice.disks.len(), key.code)
        }
        KeyCode::Char('r') => *choice = Disks::load(args),
        KeyCode::Esc => return Some(Screen::Network(Wifi::new(false))),
        KeyCode::Enter => {
            let disk = choice
                .list
                .selected()
                .and_then(|i| choice.disks.get(i))
                .cloned()?;
            return Some(Screen::Confirm {
                disk,
                input: String::new(),
            });
        }
        _ => {}
    }
    None
}

fn confirm_key(disk: &Disk, input: &mut String, key: KeyEvent, args: &Args) -> Option<Screen> {
    match key.code {
        KeyCode::Esc => Some(Screen::Disks(Disks::load(args))),
        KeyCode::Enter if input == "erase" => Some(start(disk.clone(), args)),
        KeyCode::Enter => {
            input.clear();
            None
        }
        _ => {
            edit(input, key);
            None
        }
    }
}

fn installing_key(progress: &mut Progress, key: KeyEvent, args: &Args) -> Option<Screen> {
    match (&progress.result, key.code) {
        (Some(Ok(())), KeyCode::Enter) => {
            let _ = Command::new("systemctl").arg("reboot").status();
            None
        }
        (Some(Err(_)), KeyCode::Enter) => Some(Screen::Disks(Disks::load(args))),
        _ => None,
    }
}

fn start(disk: Disk, args: &Args) -> Screen {
    let plan = Plan {
        disk: disk.path.clone(),
        repart_definitions: args.repart_definitions.clone(),
        sysupdate_templates: args.sysupdate_templates.clone(),
        esp: args.esp.clone(),
        work: args.work.clone(),
    };
    let (sender, events) = mpsc::channel();
    thread::spawn(move || install::run(&plan, &sender));
    Screen::Installing(Progress {
        disk,
        step: None,
        log: Vec::new(),
        result: None,
        events,
    })
}

fn describe(disk: &Disk) -> String {
    let mut text = format!("{:<10} {:>9}  {}", disk.name, disk.size_text(), disk.model);
    if disk.removable {
        text.push_str("  (removable)");
    }
    text
}

fn selectable<'a>(items: Vec<ListItem<'a>>, title: &'static str) -> List<'a> {
    // Reversed video and a marker, because the Linux console has eight
    // colours and no guarantee which of them a theme left readable.
    List::new(items)
        .block(Block::bordered().title(title))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ")
}

fn draw_network(frame: &mut Frame, area: Rect, wifi: &mut Wifi, online: bool) -> &'static str {
    let block = Block::bordered().title(" Network ");
    if let Phase::Passphrase { network, input } = &wifi.phase {
        let text = vec![
            Line::raw(format!(
                "Passphrase for {} ({}):",
                network.name(),
                network.security.label()
            )),
            Line::raw(""),
            Line::raw(format!("> {}", "*".repeat(input.chars().count()))),
        ];
        frame.render_widget(Paragraph::new(text).block(block), area);
        return "Enter: join    Esc: back";
    }

    let [top, list_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).areas(area);
    let mut lines = vec![Line::raw(match (&wifi.interface, &wifi.control, &wifi.phase) {
        (None, _, _) if online => "Online through a wired connection.".to_string(),
        (None, _, _) => {
            "No wireless device found. Plug in a network cable; the installer goes on by itself once online."
                .to_string()
        }
        (Some(iface), None, _) => format!("Waiting for wpa_supplicant on {iface}..."),
        (Some(iface), Some(_), Phase::Scanning(_)) => format!("Scanning on {iface}..."),
        (Some(_), Some(_), Phase::Connecting { network, .. }) => format!(
            "Joining {} ({})...",
            network.name(),
            wifi.wpa_state().unwrap_or_default().to_lowercase()
        ),
        (Some(iface), Some(_), _) if online => {
            format!("Online. Pick a network to move {iface} to, or press Tab to go on.")
        }
        (Some(iface), Some(_), _) => format!("Pick a network for {iface} to join."),
    })];
    if let Some(message) = &wifi.message {
        lines.push(Line::raw(message.clone()));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        top,
    );

    let items = wifi
        .networks
        .iter()
        .map(|n| {
            ListItem::new(format!(
                "{:>4} dBm  {:<10} {}",
                n.signal,
                n.security.label(),
                n.name()
            ))
        })
        .collect();
    frame.render_stateful_widget(
        selectable(items, " Wireless networks "),
        list_area,
        &mut wifi.list,
    );
    if online {
        "Up/Down: choose    Enter: join    r: scan again    Tab: continue"
    } else {
        "Up/Down: choose    Enter: join    r: scan again"
    }
}

fn draw_disks(frame: &mut Frame, area: Rect, choice: &mut Disks) -> &'static str {
    let [top, list_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).areas(area);
    let mut lines = vec![Line::raw(
        "Pick the disk to install onto. Everything on it will be erased.",
    )];
    if choice.disks.is_empty() {
        lines.push(Line::raw("No disk was found to install onto."));
    }
    if let Some(medium) = &choice.medium {
        lines.push(Line::raw(format!(
            "The installer's own medium, {medium}, is not listed."
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Disk ")),
        top,
    );
    let items = choice
        .disks
        .iter()
        .map(|d| ListItem::new(describe(d)))
        .collect();
    frame.render_stateful_widget(selectable(items, " Disks "), list_area, &mut choice.list);
    "Up/Down: choose    Enter: select    r: refresh    Esc: network"
}

fn draw_confirm(frame: &mut Frame, area: Rect, disk: &Disk, input: &str) -> &'static str {
    let text = vec![
        Line::raw(format!(
            "Everything on {} will be erased:",
            disk.path.display()
        )),
        Line::raw(""),
        Line::raw(format!("    {}", describe(disk))),
        Line::raw(""),
        Line::raw("Type erase and press Enter to install LosOS Desktop onto it."),
        Line::raw(""),
        Line::raw(format!("> {input}")),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Confirm ")),
        area,
    );
    "Enter: install    Esc: choose another disk"
}

fn draw_installing(frame: &mut Frame, area: Rect, progress: &Progress) -> &'static str {
    let steps_height = Step::ALL.len() as u16 + 4;
    let [steps_area, log_area] =
        Layout::vertical([Constraint::Length(steps_height), Constraint::Min(3)]).areas(area);
    let current = progress
        .step
        .and_then(|s| Step::ALL.iter().position(|&a| a == s));
    let failed = matches!(progress.result, Some(Err(_)));
    let done = matches!(progress.result, Some(Ok(())));
    let mut lines: Vec<Line> = Step::ALL
        .iter()
        .enumerate()
        .map(|(i, step)| {
            // ASCII, because the console font may have nothing else.
            let mark = match current {
                _ if done => "[x]",
                Some(c) if i < c => "[x]",
                Some(c) if i == c && failed => "[!]",
                Some(c) if i == c => "[>]",
                _ => "[ ]",
            };
            Line::raw(format!("{mark} {}", step.label()))
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::raw(match &progress.result {
        None => format!("Installing onto {}...", progress.disk.path.display()),
        Some(Ok(())) => {
            "Installed. Remove the installation medium and press Enter to restart.".into()
        }
        Some(Err(e)) => format!("The install failed: {e}"),
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" Installing ")),
        steps_area,
    );

    let visible = log_area.height.saturating_sub(2) as usize;
    let tail = &progress.log[progress.log.len().saturating_sub(visible)..];
    let log = Text::from(
        tail.iter()
            .map(|l| Line::raw(l.as_str()))
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(log).block(Block::bordered().title(" Log ")),
        log_area,
    );
    match progress.result {
        None => "Please wait",
        Some(Ok(())) => "Enter: restart",
        Some(Err(_)) => "Enter: choose a disk again",
    }
}

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut terminal = ratatui::init();
    let result = App::new(args).run(&mut terminal);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("losos-installer: {e}");
            ExitCode::FAILURE
        }
    }
}
