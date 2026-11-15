//! The command: `ctl` for the JSON command line, otherwise the original
//! full-screen player (menu.rs + renderer.rs) driven by the backend library.
//! The player below is a stand-in until the Vostok TUI replaces it; it shows
//! the whole front-end contract in one loop.

mod config;
mod menu;
mod renderer;

// The original renderer asks for `crate::framebuffer`.
use vostok_gameboy::frame as framebuffer;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{
        self, Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use menu::show_menu;
use renderer::{RenderConfig, RenderMode, Renderer};
use vostok_gameboy::frame::{Frame, Palette};
use vostok_gameboy::input::{InputTracker, KeyMap};
use vostok_gameboy::session::{Command, Session, SessionOptions, State};
use vostok_gameboy::settings::Config;
use vostok_gameboy::{cart, ctl, paths};

const STATUS_COLOR: &str = "\x1b[38;2;80;200;80m";
const RESET: &str = "\x1b[0m";

fn print_usage() {
    eprintln!("Vostok Gameboy {}", vostok_gameboy::VERSION);
    eprintln!();
    eprintln!("Usage: vostok-gameboy                      pick a game from the menu");
    eprintln!("       vostok-gameboy ROM [OPTIONS]        play a .gb, .gbc or .zip");
    eprintln!("       vostok-gameboy ctl COMMAND          JSON command line (ctl help)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --ascii           ASCII art (default)");
    eprintln!("  --block           Unicode half-blocks");
    eprintln!("  --mute            no sound");
    eprintln!("  --palette NAME    grey, dmg, pocket, phosphor, amber (four-shade games)");
    eprintln!();
    eprintln!("Keys (change them in config.toml):");
    eprintln!("  Arrows / WASD     D-pad");
    eprintln!("  Z / X             A / B");
    eprintln!("  Enter / Backspace Start / Select");
    eprintln!("  P                 pause        Tab   turbo on/off");
    eprintln!("  Q / Esc           quit (the game is saved)");
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
    let app_dir = paths::app_dir();
    let config = Config::load(&app_dir)?;
    let mut options = SessionOptions::from_config(&config, &app_dir);
    let mut palette_chosen = false;

    let (rom_path, mode) = match args.iter().find(|arg| !arg.starts_with("--")) {
        Some(path) => {
            let mode = if args.iter().any(|arg| arg == "--block") { RenderMode::Block } else { RenderMode::Ascii };
            if args.iter().any(|arg| arg == "--mute") {
                options.audio = false;
            }
            if let Some(index) = args.iter().position(|arg| arg == "--palette") {
                let name = args.get(index + 1).map(String::as_str).unwrap_or("");
                options.palette = Palette::parse(name).ok_or(format!("unknown palette `{name}`"))?;
                palette_chosen = true;
            }
            (PathBuf::from(path), mode)
        }
        None => match show_menu().map_err(|error| error.to_string())? {
            Some(choice) => {
                options.audio &= choice.audio_enabled;
                (choice.rom_path, choice.render_mode)
            }
            None => return Ok(()),
        },
    };

    let rom = cart::load(&rom_path).map_err(|error| error.to_string())?;
    if !palette_chosen {
        options.palette = config.palette_for(&rom.id());
    }
    let session = Session::start(rom, options)?;
    let keys = KeyMap::new(&config.keys);

    let outcome = run_screen(&session, &keys, mode);
    let last = session.stop();
    outcome.map_err(|error| error.to_string())?;

    if let Some(note) = last.note {
        eprintln!("vostok-gameboy: {note}");
    }
    if last.saved_at.is_some() {
        println!("Saved to {}", saved_to(&config, &app_dir, &rom_path).display());
    }
    Ok(())
}

fn saved_to(config: &Config, app_dir: &Path, rom_path: &Path) -> PathBuf {
    vostok_gameboy::saves::save_path(&config.saves_dir(app_dir), rom_path)
}

/// Raw mode and the alternate screen, undone on every way out.
struct TerminalGuard {
    enhanced: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        // Ask for key-release events where the terminal can send them (kitty
        // protocol); Windows consoles send them without being asked.
        let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false)
            && execute!(stdout, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)).is_ok();
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
        let _ = terminal::disable_raw_mode();
    }
}

fn run_screen(session: &Session, keys: &KeyMap, mode: RenderMode) -> io::Result<()> {
    let _terminal = TerminalGuard::enter()?;
    let mut stdout = io::stdout();
    let mut renderer = Renderer::new(RenderConfig { mode });
    if let Ok((cols, rows)) = terminal::size() {
        renderer.update_dimensions(cols, rows);
    }

    let mut frame = Frame::default();
    let mut seen = 0u64;
    let mut input = InputTracker::default();
    let mut turbo = false;
    let mut status_due = Instant::now();

    loop {
        for edge in input.expire(Instant::now()) {
            session.send(edge.into());
        }

        // Waiting here (not sleeping) keeps key presses snappy between frames.
        while event::poll(Duration::from_millis(2))? {
            match event::read()? {
                Event::Resize(cols, rows) => {
                    renderer.update_dimensions(cols, rows);
                    seen = 0; // redraw even while paused
                }
                Event::FocusLost => {
                    for edge in input.release_all() {
                        session.send(edge.into());
                    }
                }
                Event::Key(key) => {
                    let pressed = key.kind == KeyEventKind::Press;
                    match key.code {
                        KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('p' | 'P') if pressed => session.send(Command::TogglePause),
                        KeyCode::Tab if pressed => {
                            turbo = !turbo;
                            session.send(Command::Speed(if turbo { 4 } else { 1 }));
                        }
                        code => {
                            if let Some(button) = keys.lookup(code) {
                                let edge = match key.kind {
                                    KeyEventKind::Press | KeyEventKind::Repeat => input.press(button, Instant::now()),
                                    KeyEventKind::Release => input.release(button),
                                };
                                if let Some(edge) = edge {
                                    session.send(edge.into());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if session.copy_frame(&mut frame, &mut seen) {
            renderer.render(&frame, &mut stdout)?;
        }

        if Instant::now() >= status_due {
            status_due = Instant::now() + Duration::from_millis(500);
            let status = session.status();
            if !matches!(status.state, State::Running | State::Paused) {
                return Ok(());
            }
            let state = match status.state {
                State::Paused => "PAUSED".to_string(),
                _ if status.speed > 1 => format!("x{}", status.speed),
                _ => format!("{:4.1} fps", status.fps),
            };
            let save = if status.unsaved { "unsaved" } else { "" };
            execute!(stdout, MoveTo(0, renderer.fps_row()))?;
            write!(stdout, "{STATUS_COLOR}{state:<10} {save:<8}{RESET}")?;
            stdout.flush()?;
        }
    }
}
