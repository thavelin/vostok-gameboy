//! The command: `ctl` for the JSON command line, otherwise the Vostok Boy
//! terminal UI (src/tui) over the backend library.

// Shared with the other Vostok Rust apps; kept as they are there.
#[rustfmt::skip]
#[allow(dead_code)]
#[path = "tui/chrome.rs"]
mod chrome;
#[rustfmt::skip]
#[path = "tui/pixels.rs"]
mod pixels;
#[rustfmt::skip]
#[path = "tui/tokens.rs"]
mod tokens;

#[path = "tui/app.rs"]
mod app;
#[path = "tui/art.rs"]
mod art;
#[path = "tui/layout.rs"]
mod layout;
#[path = "tui/screen.rs"]
mod screen;
#[path = "tui/terminal.rs"]
mod terminal;
#[path = "tui/view.rs"]
mod view;

use app::App;
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, buffer::Buffer, layout::Rect, style::Color, Terminal};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vostok_gameboy::frame::Palette;
use vostok_gameboy::session::Command;
use vostok_gameboy::settings::Config;
use vostok_gameboy::{ctl, paths};

fn print_usage() {
    println!("Vostok Gameboy {}", vostok_gameboy::VERSION);
    println!();
    println!("Usage: vostok-gameboy                      pick a game from the library");
    println!("       vostok-gameboy ROM [OPTIONS]        play a .gb, .gbc or .zip");
    println!("       vostok-gameboy ctl COMMAND          JSON command line (ctl help)");
    println!();
    println!("Options:");
    println!("  --mute            no sound");
    println!("  --palette NAME    grey, dmg, pocket, phosphor, amber (four-shade games)");
    println!("  --theme NAME      phosphor or amber");
    println!("  --pixels          draw the picture with Kitty graphics");
    println!("  --text            draw the picture with half-blocks");
    println!();
    println!("Keys (the buttons can be changed in config.toml; ? in the app lists all):");
    println!("  Arrows / WASD     D-pad            Z / K   A        X / J   B");
    println!("  Enter             Start            Backspace / Space    Select");
    println!("  P / Esc           pause menu       F5 / F8   save / load state");
    println!("  Tab               turbo            F2  view   F3  theme   Q  quit");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("ctl") => ctl::run(&args[1..]),
        Some("--help" | "-h") => {
            print_usage();
            0
        }
        Some("--version" | "-V") => {
            println!("vostok-gameboy {}", vostok_gameboy::VERSION);
            0
        }
        _ => match play(&args) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("vostok-gameboy: {error}");
                1
            }
        },
    };
    std::process::exit(code);
}

fn play(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
    };
    let takes_value = ["--palette", "--theme", "--html", "--frames"];
    let rom = args
        .iter()
        .enumerate()
        .find(|(at, arg)| {
            !arg.starts_with("--") && !(*at > 0 && takes_value.contains(&args[at - 1].as_str()))
        })
        .map(|(_, arg)| PathBuf::from(arg));
    let palette = match value("--palette") {
        Some(name) => Some(Palette::parse(name).ok_or(format!("unknown palette `{name}`"))?),
        None => None,
    };

    let app_dir = paths::app_dir();
    let mut config = Config::load(&app_dir)?;
    if flag("--mute") {
        config.audio.enabled = false;
    }
    let env = terminal::Env::from_process();
    let explicit = if flag("--pixels") {
        Some(true)
    } else {
        flag("--text").then_some(false)
    };
    let mut app = App::new(app_dir, config, terminal::pixels(explicit, &env));
    if let Some(name) = value("--theme") {
        app.theme = tokens::ThemeName::parse(name).ok_or(format!("unknown theme `{name}`"))?;
    }
    if let Some(path) = rom {
        app.start(&path, None)?;
        app.direct = true;
        if let (Some(palette), Some(game)) = (palette, &app.game) {
            game.session.send(Command::Palette(palette));
        }
    }
    if let Some(size) = value("--html") {
        return snapshot(
            &mut app,
            size,
            value("--frames")
                .and_then(|n| n.parse().ok())
                .unwrap_or(120),
        );
    }
    let outcome = run(&mut app, env.tmux);
    if app.game.is_some() {
        app.eject();
    }
    outcome.map_err(|error| error.to_string())
}

/// Raw mode and the alternate screen, undone on every way out.
struct TerminalGuard {
    enhanced: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        // Ask for key-release events where the terminal can send them (kitty
        // protocol); Windows consoles send them without being asked.
        let enhanced = supports_keyboard_enhancement().unwrap_or(false)
            && execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
            )
            .is_ok();
        Ok(Self { enhanced })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        if self.enhanced {
            let _ = execute!(stdout, PopKeyboardEnhancementFlags);
        }
        let _ = execute!(stdout, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// How much to enlarge the picture before sending it: enough that the
/// terminal never has to stretch it far (cells are about nine pixels wide).
fn enlarge(area: Rect) -> usize {
    (usize::from(area.width) * 9 / 160).clamp(2, 4)
}

fn run(app: &mut App, tmux: bool) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut out = io::stdout();
    let mut presenter = pixels::Presenter::default();
    let mut gallery = pixels::Gallery::default();
    let mut thumbs: HashMap<u32, String> = HashMap::new();
    let mut placed: Option<Rect> = None;
    let mut clock = String::new();
    let size = terminal.size()?;
    app.size = (size.width, size.height);

    while !app.quit {
        app.tick(Instant::now());
        // Waiting here (not sleeping) keeps key presses snappy between frames.
        let mut wait = Duration::from_millis(if app.game.is_some() { 2 } else { 40 });
        while event::poll(wait)? {
            wait = Duration::ZERO;
            match event::read()? {
                Event::Key(key) => app.key(key),
                Event::FocusLost => app.blur(),
                Event::Resize(cols, rows) => {
                    app.size = (cols, rows);
                    presenter.cleanup(&mut out, tmux)?;
                    gallery.clear(&mut out, tmux)?;
                    placed = None;
                    app.dirty = true;
                }
                _ => {}
            }
        }
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        if now != clock {
            clock = now;
            app.dirty = true;
        }
        if !app.dirty {
            continue;
        }
        app.dirty = false;
        let mut pictures = view::Pictures::default();
        terminal.draw(|frame| pictures = view::render(frame.buffer_mut(), app, &clock))?;

        match (pictures.game, &mut app.game) {
            (Some(area), Some(game)) => {
                if game.fresh || placed != Some(area) {
                    presenter.present(
                        &mut out,
                        &screen::png(&game.frame, enlarge(area)),
                        area,
                        tmux,
                    )?;
                    game.fresh = false;
                    placed = Some(area);
                }
            }
            _ => {
                presenter.cleanup(&mut out, tmux)?;
                placed = None;
            }
        }
        thumbs.retain(|id, _| pictures.thumbs.iter().any(|thumb| thumb.0 == *id));
        for (id, _, frame) in &pictures.thumbs {
            thumbs.entry(*id).or_insert_with(|| screen::png(frame, 2));
        }
        let wanted: Vec<(u32, &str, Rect)> = pictures
            .thumbs
            .iter()
            .map(|(id, area, _)| (*id, thumbs[id].as_str(), *area))
            .collect();
        gallery.sync(&mut out, &wanted, tmux)?;
    }
    presenter.cleanup(&mut out, tmux)?;
    gallery.clear(&mut out, tmux)?;
    out.flush()
}

/// `--html COLSxROWS`: draw one screen and print it as HTML (for comparing
/// with the design without a terminal). With a ROM it runs `--frames` first.
fn snapshot(app: &mut App, size: &str, frames: u64) -> Result<(), String> {
    let (cols, rows) = size
        .split_once('x')
        .ok_or("--html takes COLSxROWS, for example 140x39")?;
    let (cols, rows): (u16, u16) = (
        cols.parse().map_err(|_| "bad width")?,
        rows.parse().map_err(|_| "bad height")?,
    );
    app.size = (cols, rows);
    let deadline = Instant::now() + Duration::from_secs(20);
    while app
        .game
        .as_ref()
        .is_some_and(|game| game.status.frames < frames)
        && Instant::now() < deadline
    {
        app.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(5));
    }
    for key in std::env::var("VOSTOK_GAMEBOY_KEYS")
        .unwrap_or_default()
        .split_whitespace()
    {
        let code = match key {
            "esc" => event::KeyCode::Esc,
            "enter" => event::KeyCode::Enter,
            "down" => event::KeyCode::Down,
            "right" => event::KeyCode::Right,
            "f2" => event::KeyCode::F(2),
            "f3" => event::KeyCode::F(3),
            "f5" => event::KeyCode::F(5),
            other => event::KeyCode::Char(other.chars().next().unwrap_or(' ')),
        };
        app.key(event::KeyEvent::new(code, event::KeyModifiers::NONE));
        std::thread::sleep(Duration::from_millis(150));
        if let Some(game) = &mut app.game {
            game.session.send(Command::Volume(0.0));
        }
        app.tick(Instant::now() + Duration::from_millis(250));
    }
    app.tick(Instant::now() + Duration::from_millis(250));
    let mut buf = Buffer::empty(Rect::new(0, 0, cols, rows));
    view::render(
        &mut buf,
        app,
        &chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    );
    let hex = |color: Color, fallback: &str| match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_string(),
    };
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><body style='margin:0;background:#0b0d0b'>\
         <pre style=\"margin:8px;font:11.9px/14.2px 'Fira Code','Cascadia Mono',Consolas,monospace\">",
    );
    for y in 0..rows {
        for x in 0..cols {
            let cell = &buf[(x, y)];
            let weight = if cell.modifier.contains(ratatui::style::Modifier::BOLD) {
                ";font-weight:700"
            } else {
                ""
            };
            let symbol = cell.symbol().replace('&', "&amp;").replace('<', "&lt;");
            html += &format!(
                "<span style='color:{};background:{}{weight}'>{symbol}</span>",
                hex(cell.fg, "#b4f5c6"),
                hex(cell.bg, "#020502")
            );
        }
        html.push('\n');
    }
    println!("{html}</pre>");
    if app.game.is_some() {
        app.eject();
    }
    Ok(())
}
