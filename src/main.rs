mod audio;
mod config;
mod framebuffer;
mod input;
mod menu;
mod renderer;
mod save;

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use gameboy_core::{Gameboy, RTC, StepResult};
use rodio::{OutputStream, Sink};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use audio::{AudioBuffer, GameboyAudioSource};
use framebuffer::{FrameBuffer, GB_HEIGHT, GB_WIDTH};
use input::map_key_to_button;
use menu::show_menu;
use renderer::{RenderConfig, RenderMode, Renderer};
use save::{get_save_path, load_save, save_game};

// Button count and index mapping (faster than HashMap for 8 buttons)
const BUTTON_COUNT: usize = 8;

#[inline]
fn button_index(button: gameboy_core::Button) -> usize {
    use gameboy_core::Button;
    match button {
        Button::A => 0,
        Button::B => 1,
        Button::Start => 2,
        Button::Select => 3,
        Button::Up => 4,
        Button::Down => 5,
        Button::Left => 6,
        Button::Right => 7,
    }
}

// Colors
const FPS_COLOR: &str = "\x1b[38;2;80;200;80m";
const RESET: &str = "\x1b[0m";

struct SimpleRTC;

impl RTC for SimpleRTC {
    fn get_current_time(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}


fn print_usage(program: &str) {
    eprintln!("ASCII GameBoy Emulator");
    eprintln!();
    eprintln!("Usage: {} <rom_file.gb> [OPTIONS]", program);
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --ascii       ASCII art mode with brightness characters (default)");
    eprintln!("  --block       Unicode block mode with solid half-blocks");
    eprintln!("  --mute        Disable audio output");
    eprintln!();
    eprintln!("Controls:");
    eprintln!("  Arrow keys    D-Pad");
    eprintln!("  Z             A button");
    eprintln!("  X             B button");
    eprintln!("  Enter         Start");
    eprintln!("  Space         Select");
    eprintln!("  Q / Esc       Quit");
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    
    // Determine ROM path, mode, and audio from either CLI args or interactive menu
    let (rom_path, mode, audio_enabled) = if args.len() < 2 {
        // No arguments: show interactive menu
        match show_menu()? {
            Some(config) => (config.rom_path, config.render_mode, config.audio_enabled),
            None => {
                // User quit the menu
                return Ok(());
            }
        }
    } else if args.iter().any(|a| a == "--help" || a == "-h") {
        print_usage(&args[0]);
        std::process::exit(0);
    } else {
        // CLI arguments provided
        let rom_path = Path::new(&args[1]).to_path_buf();
        let mode = if args.iter().any(|a| a == "--block") {
            RenderMode::Block
        } else {
            RenderMode::Ascii
        };
        let audio_enabled = !args.iter().any(|a| a == "--mute");
        (rom_path, mode, audio_enabled)
    };

    let rom = std::fs::read(&rom_path).expect("Failed to read ROM file");

    let mut gameboy =
        Gameboy::from_rom(rom, Box::new(SimpleRTC)).expect("Failed to load ROM");

    // Load existing save file if present
    if let Ok(true) = load_save(&mut gameboy, &rom_path) {
        eprintln!("Loaded save file: {}", get_save_path(&rom_path).display());
        std::thread::sleep(Duration::from_millis(500)); // Brief pause to show message
    }

    // Setup audio output (lock-free buffer)
    // Larger buffer (16K samples = ~180ms stereo) for more headroom
    let audio_buffer = Arc::new(AudioBuffer::new(16384));
    let (_stream, stream_handle) =
        OutputStream::try_default().expect("Failed to open audio output");
    let sink = Sink::try_new(&stream_handle).expect("Failed to create audio sink");

    // Pre-buffer some frames before starting audio to avoid initial underruns
    let mut pre_buffer_frames = if audio_enabled { 3 } else { 0 };

    let mut stdout = io::stdout();

    // Setup terminal
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;

    let mut framebuffer = FrameBuffer::new(GB_WIDTH, GB_HEIGHT);

    let config = RenderConfig { mode };
    let mut renderer = Renderer::new(config);

    // Fit the game to the current terminal size
    if let Ok((cols, rows)) = terminal::size() {
        renderer.update_dimensions(cols, rows);
    }

    // Use precise floating-point duration for accurate 60 FPS (16.667ms, not 16ms)
    let frame_duration = Duration::from_secs_f64(1.0 / 60.0);

    // FPS tracking
    let mut frame_count = 0u32;
    let mut fps_timer = Instant::now();

    let mut running = true;
    // Track when each button was last seen (press or repeat event)
    // Using fixed array instead of HashMap - faster for 8 buttons
    let mut button_last_seen: [Option<Instant>; BUTTON_COUNT] = [None; BUTTON_COUNT];
    // Detect if terminal supports release events (Ghostty, kitty, etc.)
    let mut terminal_supports_release = false;
    // Fallback timeout for terminals without release support (Terminal.app, etc.)
    // Needs to be long enough to span keyboard repeat gaps (~150ms is safe)
    let button_timeout = Duration::from_millis(150);

    while running {
        let frame_start = Instant::now();

        // Only use timeout-based release for terminals that don't support release events
        if !terminal_supports_release {
            use gameboy_core::Button;
            const ALL_BUTTONS: [Button; BUTTON_COUNT] = [
                Button::A, Button::B, Button::Start, Button::Select,
                Button::Up, Button::Down, Button::Left, Button::Right,
            ];
            for button in ALL_BUTTONS {
                let idx = button_index(button);
                if let Some(last_seen) = button_last_seen[idx] {
                    if last_seen.elapsed() > button_timeout {
                        button_last_seen[idx] = None;
                        gameboy.release_button(button);
                    }
                }
            }
        }

        // Process input events
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(cols, rows) => {
                    // Terminal was resized (e.g., zoom in/out) — refit and clear
                    renderer.update_dimensions(cols, rows);
                }
                Event::Key(key_event) => {
                    // Handle quit keys
                    if key_event.code == KeyCode::Char('q')
                        || key_event.code == KeyCode::Char('Q')
                        || key_event.code == KeyCode::Esc
                    {
                        running = false;
                        break;
                    }

                    // Map to gameboy button and handle press/repeat
                    if let Some(button) = map_key_to_button(key_event.code) {
                    let idx = button_index(button);
                    match key_event.kind {
                        KeyEventKind::Press | KeyEventKind::Repeat => {
                            // Update last seen time (used for fallback timeout)
                            let is_new = button_last_seen[idx].is_none();
                            button_last_seen[idx] = Some(Instant::now());

                            // Only call press_button if this is a new press
                            if is_new {
                                gameboy.press_button(button);
                            }
                        }
                        KeyEventKind::Release => {
                            // Terminal supports release events! Use them directly.
                            terminal_supports_release = true;
                            if button_last_seen[idx].take().is_some() {
                                gameboy.release_button(button);
                            }
                        }
                    }
                }
                }
                _ => {} // Ignore other events (mouse, focus, etc.)
            }
        }

        if !running {
            break;
        }

        // Run emulation until VBlank
        loop {
            let result = gameboy.emulate(&mut framebuffer);
            match result {
                StepResult::VBlank => break,
                StepResult::AudioBufferFull => {
                    // Push audio samples to the buffer (lock-free)
                    if audio_enabled {
                        let samples = gameboy.get_audio_buffer();
                        audio_buffer.push_samples(samples);
                    }
                }
                StepResult::Nothing => {}
            }
        }

        // Render the game screen
        renderer.render(&framebuffer, &mut stdout)?;

        // Start audio playback after pre-buffering
        if pre_buffer_frames > 0 {
            pre_buffer_frames -= 1;
            if pre_buffer_frames == 0 {
                let source = GameboyAudioSource::new(Arc::clone(&audio_buffer));
                sink.append(source);
            }
        }

        // Update FPS counter every second
        frame_count += 1;
        let fps_elapsed = fps_timer.elapsed();
        if fps_elapsed >= Duration::from_secs(1) {
            let fps = frame_count as f32 / fps_elapsed.as_secs_f32();
            frame_count = 0;
            fps_timer = Instant::now();

            // Draw FPS below screen
            execute!(stdout, MoveTo(0, renderer.fps_row()))?;
            write!(stdout, "{}FPS: {:5.1}  {}", FPS_COLOR, fps, RESET)?;
            stdout.flush()?;
        }

        // Frame timing
        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    }

    // Cleanup terminal
    execute!(stdout, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    // Save game on exit
    match save_game(&gameboy, &rom_path) {
        Ok(true) => println!("Game saved to: {}", get_save_path(&rom_path).display()),
        Ok(false) => {} // Game doesn't support saves
        Err(e) => eprintln!("Failed to save game: {}", e),
    }

    Ok(())
}
