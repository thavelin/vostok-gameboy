use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, SetBackgroundColor, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::thread;

use crate::config;
use crate::renderer::RenderMode;

/// Configuration selected from the menu
pub struct MenuConfig {
    pub rom_path: PathBuf,
    pub render_mode: RenderMode,
    pub audio_enabled: bool,
}

/// Menu state
struct MenuState {
    // Settings
    render_mode: RenderMode,
    audio_enabled: bool,
    roms_dir: PathBuf,
    
    // Available ROMs
    rom_files: Vec<PathBuf>,
    selected_rom_index: usize,
    
    // UI state
    current_section: MenuSection,
    scroll_offset: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum MenuSection {
    RenderMode,
    Audio,
    RomsDirectory,
    GameList,
}

impl MenuSection {
    fn next(self) -> Self {
        match self {
            Self::RenderMode => Self::Audio,
            Self::Audio => Self::RomsDirectory,
            Self::RomsDirectory => Self::GameList,
            Self::GameList => Self::RenderMode,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::RenderMode => Self::GameList,
            Self::Audio => Self::RenderMode,
            Self::RomsDirectory => Self::Audio,
            Self::GameList => Self::RomsDirectory,
        }
    }
}

// Colors - Navy/Blue theme inspired by GBC
const HIGHLIGHT_BG: Color = Color::Rgb { r: 30, g: 60, b: 114 };   // Navy blue
const HIGHLIGHT_FG: Color = Color::Rgb { r: 130, g: 180, b: 255 }; // Light blue
const DIM_COLOR: Color = Color::Rgb { r: 80, g: 100, b: 140 };     // Muted blue-gray
const TEXT_COLOR: Color = Color::Rgb { r: 200, g: 210, b: 230 };   // Light blue-white
const ACCENT_COLOR: Color = Color::Rgb { r: 100, g: 200, b: 255 }; // Cyan accent
const BORDER_COLOR: Color = Color::Rgb { r: 60, g: 90, b: 150 };   // Border blue

// Logo lines (without border - we draw that separately)
const LOGO_LINES: [&str; 12] = [
    "  ████████╗███████╗██████╗ ███╗   ███╗██╗███╗   ██╗ █████╗ ██╗     ",
    "  ╚══██╔══╝██╔════╝██╔══██╗████╗ ████║██║████╗  ██║██╔══██╗██║     ",
    "     ██║   █████╗  ██████╔╝██╔████╔██║██║██╔██╗ ██║███████║██║     ",
    "     ██║   ██╔══╝  ██╔══██╗██║╚██╔╝██║██║██║╚██╗██║██╔══██║██║     ",
    "     ██║   ███████╗██║  ██║██║ ╚═╝ ██║██║██║ ╚████║██║  ██║███████╗",
    "     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝╚══════╝",
    "   ██████╗  █████╗ ███╗   ███╗███████╗██████╗  ██████╗ ██╗   ██╗   ",
    "  ██╔════╝ ██╔══██╗████╗ ████║██╔════╝██╔══██╗██╔═══██╗╚██╗ ██╔╝   ",
    "  ██║  ███╗███████║██╔████╔██║█████╗  ██████╔╝██║   ██║ ╚████╔╝    ",
    "  ██║   ██║██╔══██║██║╚██╔╝██║██╔══╝  ██╔══██╗██║   ██║  ╚██╔╝     ",
    "  ╚██████╔╝██║  ██║██║ ╚═╝ ██║███████╗██████╔╝╚██████╔╝   ██║      ",
    "   ╚═════╝ ╚═╝  ╚═╝╚═╝     ╚═╝╚══════╝╚═════╝  ╚═════╝    ╚═╝      ",
];

const LOGO_WIDTH: usize = 70;

// GBC startup colors (matching the actual GBC boot sequence)
const GBC_BLUE: Color = Color::Rgb { r: 0, g: 0, b: 255 };
const GBC_GREEN: Color = Color::Rgb { r: 0, g: 255, b: 0 };
const GBC_MAGENTA: Color = Color::Rgb { r: 255, g: 0, b: 255 };
const GBC_RED: Color = Color::Rgb { r: 255, g: 0, b: 0 };
const GBC_YELLOW: Color = Color::Rgb { r: 255, g: 255, b: 0 };

// The color cycle order for the sweep (yellow leads, blue trails)
const GBC_CYCLE: [Color; 5] = [GBC_YELLOW, GBC_RED, GBC_MAGENTA, GBC_GREEN, GBC_BLUE];

// How many characters each color spans in the rainbow trail
// GBC shows ~85% rainbow coverage when sweep reaches end: 70 * 0.85 / 5 colors ≈ 12
const COLOR_SPAN: usize = 12;

/// Play the GBC-style startup animation on the actual title position
fn play_startup_animation(stdout: &mut io::Stdout) -> io::Result<()> {
    execute!(stdout, Clear(ClearType::All))?;
    
    // Draw the border first (same position as menu)
    execute!(stdout, SetForegroundColor(BORDER_COLOR))?;
    execute!(stdout, MoveTo(2, 0), Print(format!("╔{}╗", "═".repeat(LOGO_WIDTH))))?;
    execute!(stdout, MoveTo(2, (LOGO_LINES.len() + 1) as u16), Print(format!("╚{}╝", "═".repeat(LOGO_WIDTH))))?;
    
    // Draw side borders
    for i in 0..LOGO_LINES.len() {
        execute!(stdout, MoveTo(2, (i + 1) as u16), SetForegroundColor(BORDER_COLOR), Print("║"))?;
        execute!(stdout, MoveTo((3 + LOGO_WIDTH) as u16, (i + 1) as u16), SetForegroundColor(BORDER_COLOR), Print("║"))?;
    }
    stdout.flush()?;
    
    thread::sleep(Duration::from_millis(200));
    
    // Animation: Color sweep from left to right across all logo lines
    // Need extra width for the full rainbow trail to sweep through and settle
    let total_width = LOGO_WIDTH + (GBC_CYCLE.len() * COLOR_SPAN);
    
    for sweep_col in 0..total_width {
        // Draw all logo lines with the color sweep
        for (line_idx, line) in LOGO_LINES.iter().enumerate() {
            execute!(stdout, MoveTo(3, (line_idx + 1) as u16))?;
            
            for (char_idx, ch) in line.chars().enumerate() {
                if char_idx > sweep_col {
                    // Not yet reached by sweep - invisible (print space to blend with background)
                    execute!(stdout, Print(' '))?;
                } else {
                    // Calculate which color in the cycle based on distance from sweep
                    // Each color spans COLOR_SPAN characters for a longer rainbow trail
                    let dist = sweep_col - char_idx;
                    let color_index = dist / COLOR_SPAN;
                    let color = if color_index < GBC_CYCLE.len() {
                        GBC_CYCLE[color_index]
                    } else {
                        // After sweep passes, settle to blue
                        GBC_BLUE
                    };
                    
                    execute!(stdout, SetForegroundColor(color), Print(ch))?;
                }
            }
        }
        
        stdout.flush()?;
        thread::sleep(Duration::from_millis(8));
    }
    
    // Final frame: everything in solid blue
    for (line_idx, line) in LOGO_LINES.iter().enumerate() {
        execute!(
            stdout,
            MoveTo(3, (line_idx + 1) as u16),
            SetForegroundColor(GBC_BLUE),
            Print(line)
        )?;
    }
    stdout.flush()?;
    
    // Brief hold before showing full menu
    thread::sleep(Duration::from_millis(600));
    
    Ok(())
}

impl MenuState {
    fn new() -> Self {
        // Load persisted default; fall back to cwd if none saved or path no longer exists
        let saved = config::load();
        let roms_dir = saved
            .roms_dir
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            });
        let rom_files = scan_for_roms(&roms_dir);
        
        Self {
            render_mode: RenderMode::Ascii,
            audio_enabled: true,
            roms_dir,
            rom_files,
            selected_rom_index: 0,
            current_section: MenuSection::RenderMode,
            scroll_offset: 0,
        }
    }
    
    fn refresh_roms(&mut self) {
        self.rom_files = scan_for_roms(&self.roms_dir);
        self.selected_rom_index = 0;
        self.scroll_offset = 0;
    }
}

/// Scan a directory for .gb and .gbc files
fn scan_for_roms(dir: &Path) -> Vec<PathBuf> {
    let mut roms = Vec::new();
    
    // Also check for a "roms" subdirectory
    // Note: only use lowercase "roms" to avoid duplicates on case-insensitive filesystems (macOS)
    let dirs_to_scan = vec![
        dir.to_path_buf(),
        dir.join("roms"),
        dir.join("games"),
    ];
    
    for scan_dir in dirs_to_scan {
        if let Ok(entries) = std::fs::read_dir(&scan_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    let ext = ext.to_string_lossy().to_lowercase();
                    if ext == "gb" || ext == "gbc" {
                        // Canonicalize to avoid duplicates from symlinks or case differences
                        let canonical = path.canonicalize().unwrap_or(path);
                        if !roms.iter().any(|p| p == &canonical) {
                            roms.push(canonical);
                        }
                    }
                }
            }
        }
    }
    
    // Sort by filename
    roms.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
    });
    
    roms
}

/// Display the menu and return the selected configuration
pub fn show_menu() -> io::Result<Option<MenuConfig>> {
    let mut stdout = io::stdout();
    let mut state = MenuState::new();
    
    // Setup terminal
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
    
    // Play the GBC-style startup animation
    play_startup_animation(&mut stdout)?;
    
    let result = run_menu_loop(&mut stdout, &mut state);
    
    // Cleanup terminal
    execute!(stdout, Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    
    result
}

fn run_menu_loop(stdout: &mut io::Stdout, state: &mut MenuState) -> io::Result<Option<MenuConfig>> {
    loop {
        draw_menu(stdout, state)?;
        
        // Wait for input
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            

            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                    return Ok(None);
                }
                KeyCode::Char('x') | KeyCode::Char('X') => {
                    return Ok(None);
                }
                KeyCode::Up => {
                    if state.current_section == MenuSection::GameList && !state.rom_files.is_empty() {
                        if state.selected_rom_index > 0 {
                            state.selected_rom_index -= 1;
                            if state.selected_rom_index < state.scroll_offset {
                                state.scroll_offset = state.selected_rom_index;
                            }
                        } else {
                            // At the top of the game list — step back into settings
                            state.current_section = state.current_section.prev();
                        }
                    } else {
                        state.current_section = state.current_section.prev();
                    }
                }
                KeyCode::Down => {
                    if state.current_section == MenuSection::GameList && !state.rom_files.is_empty() {
                        if state.selected_rom_index + 1 < state.rom_files.len() {
                            state.selected_rom_index += 1;
                            if state.selected_rom_index >= state.scroll_offset + 8 {
                                state.scroll_offset = state.selected_rom_index.saturating_sub(7);
                            }
                        } else {
                            // At the bottom of the game list — wrap back to top of settings
                            state.current_section = state.current_section.next();
                        }
                    } else {
                        state.current_section = state.current_section.next();
                    }
                }
                KeyCode::Tab => {
                    state.current_section = state.current_section.next();
                }
                KeyCode::BackTab => {
                    state.current_section = state.current_section.prev();
                }
                // Left/Right and Space (Select): Toggle options
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    match state.current_section {
                        MenuSection::RenderMode => {
                            state.render_mode = match state.render_mode {
                                RenderMode::Ascii => RenderMode::Block,
                                RenderMode::Block => RenderMode::Ascii,
                            };
                        }
                        MenuSection::Audio => {
                            state.audio_enabled = !state.audio_enabled;
                        }
                        _ => {}
                    }
                }
                // Z (A button) or Enter (Start): Select/Confirm
                KeyCode::Char('z') | KeyCode::Char('Z') | KeyCode::Enter => {
                    match state.current_section {
                        MenuSection::GameList => {
                            if !state.rom_files.is_empty() {
                                let rom_path = state.rom_files[state.selected_rom_index].clone();
                                return Ok(Some(MenuConfig {
                                    rom_path,
                                    render_mode: state.render_mode,
                                    audio_enabled: state.audio_enabled,
                                }));
                            }
                        }
                        MenuSection::RomsDirectory => {
                            if let Some(chosen) = pick_directory(stdout, &state.roms_dir)? {
                                state.roms_dir = chosen.clone();
                                state.refresh_roms();
                                // Persist the choice
                                let _ = config::save(&config::Config {
                                    roms_dir: Some(chosen),
                                });
                            }
                        }
                        MenuSection::RenderMode => {
                            state.render_mode = match state.render_mode {
                                RenderMode::Ascii => RenderMode::Block,
                                RenderMode::Block => RenderMode::Ascii,
                            };
                        }
                        MenuSection::Audio => {
                            state.audio_enabled = !state.audio_enabled;
                        }
                    }
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    state.refresh_roms();
                }
                _ => {}
            }
        }
    }
}

fn draw_menu(stdout: &mut io::Stdout, state: &MenuState) -> io::Result<()> {
    execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
    
    // Draw top border
    execute!(stdout, SetForegroundColor(BORDER_COLOR))?;
    execute!(stdout, MoveTo(2, 0), Print(format!("╔{}╗", "═".repeat(LOGO_WIDTH))))?;
    
    // Draw logo in solid blue (matching the GBC startup end state)
    for (i, line) in LOGO_LINES.iter().enumerate() {
        execute!(stdout, MoveTo(2, (i + 1) as u16), SetForegroundColor(BORDER_COLOR), Print("║"))?;
        execute!(stdout, MoveTo(3, (i + 1) as u16), SetForegroundColor(GBC_BLUE), Print(line))?;
        execute!(stdout, MoveTo((3 + LOGO_WIDTH) as u16, (i + 1) as u16), SetForegroundColor(BORDER_COLOR), Print("║"))?;
    }
    
    // Draw bottom border
    execute!(stdout, SetForegroundColor(BORDER_COLOR))?;
    execute!(stdout, MoveTo(2, (LOGO_LINES.len() + 1) as u16), Print(format!("╚{}╝", "═".repeat(LOGO_WIDTH))))?;
    
    let menu_start_y = LOGO_LINES.len() as u16 + 3;
    
    // Draw settings section
    execute!(stdout, MoveTo(4, menu_start_y))?;
    execute!(stdout, SetForegroundColor(ACCENT_COLOR), Print("═══ Settings ═══"), ResetColor)?;
    
    // Render Mode
    draw_option(
        stdout, 
        4, 
        menu_start_y + 2, 
        "Render Mode", 
        &format!("◄ {} ►", match state.render_mode {
            RenderMode::Ascii => "ASCII ",
            RenderMode::Block => "Block ",
        }),
        state.current_section == MenuSection::RenderMode
    )?;
    
    // Audio
    draw_option(
        stdout, 
        4, 
        menu_start_y + 3, 
        "Audio      ", 
        &format!("◄ {}  ►", if state.audio_enabled { "On " } else { "Off" }),
        state.current_section == MenuSection::Audio
    )?;
    
    // ROMs Directory — show full path, indicate if it's the persisted default
    let saved_roms_dir = config::load().roms_dir;
    let is_saved = saved_roms_dir.as_deref() == Some(state.roms_dir.as_path());
    let dir_display = state.roms_dir.to_string_lossy().to_string();
    let dir_label = if is_saved {
        format!("{} [saved]", truncate_str(&dir_display, 32))
    } else {
        truncate_str(&dir_display, 40)
    };
    draw_option(
        stdout,
        4,
        menu_start_y + 4,
        "Directory  ",
        &dir_label,
        state.current_section == MenuSection::RomsDirectory
    )?;
    
    // Games section
    execute!(stdout, MoveTo(4, menu_start_y + 6))?;
    execute!(stdout, SetForegroundColor(ACCENT_COLOR), Print("═══ Games ═══"), ResetColor)?;
    
    let games_start_y = menu_start_y + 8;
    
    if state.rom_files.is_empty() {
        execute!(
            stdout, 
            MoveTo(4, games_start_y),
            SetForegroundColor(DIM_COLOR),
            Print("No ROM files found (.gb, .gbc)"),
            ResetColor
        )?;
        execute!(
            stdout, 
            MoveTo(4, games_start_y + 1),
            SetForegroundColor(DIM_COLOR),
            Print("Press [R] to refresh"),
            ResetColor
        )?;
    } else {
        let visible_count = 8.min(state.rom_files.len());
        let is_game_list_selected = state.current_section == MenuSection::GameList;
        
        for (i, rom) in state.rom_files.iter()
            .skip(state.scroll_offset)
            .take(visible_count)
            .enumerate() 
        {
            let actual_index = state.scroll_offset + i;
            let is_selected = actual_index == state.selected_rom_index;
            let rom_name = rom.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            
            execute!(stdout, MoveTo(4, games_start_y + i as u16))?;
            
            if is_selected && is_game_list_selected {
                execute!(
                    stdout,
                    SetBackgroundColor(HIGHLIGHT_BG),
                    SetForegroundColor(HIGHLIGHT_FG),
                    Print(format!(" ▶ {:<40} ", truncate_str(&rom_name, 40))),
                    ResetColor
                )?;
            } else if is_selected {
                execute!(
                    stdout,
                    SetForegroundColor(TEXT_COLOR),
                    Print(format!(" ▶ {:<40} ", truncate_str(&rom_name, 40))),
                    ResetColor
                )?;
            } else {
                execute!(
                    stdout,
                    SetForegroundColor(DIM_COLOR),
                    Print(format!("   {:<40} ", truncate_str(&rom_name, 40))),
                    ResetColor
                )?;
            }
        }
        
        // Scroll indicators
        if state.scroll_offset > 0 {
            execute!(
                stdout,
                MoveTo(48, games_start_y),
                SetForegroundColor(ACCENT_COLOR),
                Print("▲"),
                ResetColor
            )?;
        }
        if state.scroll_offset + visible_count < state.rom_files.len() {
            execute!(
                stdout,
                MoveTo(48, games_start_y + visible_count as u16 - 1),
                SetForegroundColor(ACCENT_COLOR),
                Print("▼"),
                ResetColor
            )?;
        }
    }
    
    // Help text
    let help_y = games_start_y + 10;
    execute!(stdout, MoveTo(4, help_y), SetForegroundColor(DIM_COLOR))?;
    execute!(stdout, Print("Controls: ↑↓ Navigate  Z/Enter Confirm  X/Esc Back  Space Toggle  R Refresh"))?;
    execute!(stdout, MoveTo(4, help_y + 1), SetForegroundColor(DIM_COLOR))?;
    execute!(stdout, Print("Directory: Enter Browse  P Type path  S Set & save default"))?;
    execute!(stdout, ResetColor)?;
    
    stdout.flush()?;
    Ok(())
}

fn draw_option(stdout: &mut io::Stdout, x: u16, y: u16, label: &str, value: &str, selected: bool) -> io::Result<()> {
    execute!(stdout, MoveTo(x, y))?;
    
    if selected {
        execute!(
            stdout,
            SetBackgroundColor(HIGHLIGHT_BG),
            SetForegroundColor(HIGHLIGHT_FG),
            Print(format!(" {} ", label)),
            SetForegroundColor(ACCENT_COLOR),
            Print(format!("{} ", value)),
            ResetColor
        )?;
    } else {
        execute!(
            stdout,
            SetForegroundColor(TEXT_COLOR),
            Print(format!(" {} ", label)),
            SetForegroundColor(DIM_COLOR),
            Print(format!("{} ", value)),
            ResetColor
        )?;
    }
    
    Ok(())
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len - 1])
    }
}

/// Count .gb/.gbc files directly inside a directory (non-recursive).
fn count_roms_in(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| {
                            let x = x.to_string_lossy().to_lowercase();
                            x == "gb" || x == "gbc"
                        })
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

/// Read the immediate subdirectories of `dir`, sorted case-insensitively.
fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    // Skip hidden directories (start with '.')
                    let name = p.file_name()?.to_string_lossy().to_string();
                    if name.starts_with('.') {
                        return None;
                    }
                    if p.is_dir() { Some(p) } else { None }
                })
                .collect()
        })
        .unwrap_or_default();

    dirs.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .cmp(&b.file_name().unwrap_or_default().to_string_lossy().to_lowercase())
    });
    dirs
}

/// Interactive directory browser. Returns the chosen PathBuf if the user
/// pressed S (Set & Save), or None if they cancelled.
///
/// Runs inside the already-active alternate screen / raw mode.
fn pick_directory(stdout: &mut io::Stdout, start_dir: &Path) -> io::Result<Option<PathBuf>> {
    let mut current = start_dir.to_path_buf();
    let mut subdirs = list_subdirs(&current);
    // Index 0 = ".." (go up), indices 1.. = subdirs
    let mut selected: usize = 0;
    let mut scroll: usize = 0;
    let max_visible: usize = 16;

    // Input mode state
    let mut typing_path = false;
    let mut typed_input = String::new();
    let mut status_msg: Option<(String, bool)> = None; // (msg, is_error)

    loop {
        // ── Draw the browser ──────────────────────────────────────────────
        execute!(stdout, Clear(ClearType::All))?;

        // Title bar
        execute!(
            stdout,
            MoveTo(2, 0),
            SetForegroundColor(ACCENT_COLOR),
            Print("╔══ Browse ROM Directory ══╗"),
            ResetColor
        )?;

        // Current path
        execute!(
            stdout,
            MoveTo(2, 1),
            SetForegroundColor(TEXT_COLOR),
            Print("Path: "),
            SetForegroundColor(HIGHLIGHT_FG),
            Print(truncate_str(&current.to_string_lossy(), 60)),
            ResetColor
        )?;

        // ROM count in current dir
        let rom_count = count_roms_in(&current);
        let rom_hint = if rom_count == 0 {
            format!("  ({} ROMs here)", rom_count)
        } else {
            format!("  ({} ROM{} here ✓)", rom_count, if rom_count == 1 { "" } else { "s" })
        };
        execute!(
            stdout,
            MoveTo(2, 2),
            SetForegroundColor(if rom_count > 0 { ACCENT_COLOR } else { DIM_COLOR }),
            Print(&rom_hint),
            ResetColor
        )?;

        execute!(
            stdout,
            MoveTo(2, 3),
            SetForegroundColor(BORDER_COLOR),
            Print("─".repeat(60)),
            ResetColor
        )?;

        // Build entry list: ".." first, then subdirs
        let total_entries = 1 + subdirs.len(); // 0 = "..", 1.. = subdirs
        let list_start_y: u16 = 4;

        // Clamp scroll so selected is always visible
        if selected < scroll {
            scroll = selected;
        } else if selected >= scroll + max_visible {
            scroll = selected.saturating_sub(max_visible - 1);
        }

        for i in 0..max_visible {
            let entry_idx = scroll + i;
            if entry_idx >= total_entries {
                break;
            }
            let y = list_start_y + i as u16;
            let is_selected = entry_idx == selected;

            if entry_idx == 0 {
                // ".." entry
                execute!(stdout, MoveTo(2, y))?;
                if is_selected {
                    execute!(
                        stdout,
                        SetBackgroundColor(HIGHLIGHT_BG),
                        SetForegroundColor(HIGHLIGHT_FG),
                        Print(format!(" ▶ ../  (go up) {:>35} ", "")),
                        ResetColor
                    )?;
                } else {
                    execute!(
                        stdout,
                        SetForegroundColor(DIM_COLOR),
                        Print("   ../  (go up)"),
                        ResetColor
                    )?;
                }
            } else {
                let sub = &subdirs[entry_idx - 1];
                let name = sub
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "?".to_string());
                let roms_here = count_roms_in(sub);
                let suffix = if roms_here > 0 {
                    format!("  [{} ROM{}]", roms_here, if roms_here == 1 { "" } else { "s" })
                } else {
                    String::new()
                };

                execute!(stdout, MoveTo(2, y))?;
                if is_selected {
                    execute!(
                        stdout,
                        SetBackgroundColor(HIGHLIGHT_BG),
                        SetForegroundColor(HIGHLIGHT_FG),
                        Print(format!(" ▶ {:<44}{}", truncate_str(&name, 44), truncate_str(&suffix, 14))),
                        ResetColor
                    )?;
                } else {
                    execute!(
                        stdout,
                        SetForegroundColor(TEXT_COLOR),
                        Print(format!("   {:<44}", truncate_str(&name, 44))),
                        SetForegroundColor(ACCENT_COLOR),
                        Print(truncate_str(&suffix, 14)),
                        ResetColor
                    )?;
                }
            }
        }

        // Scroll indicators
        let vis_end_y = list_start_y + max_visible.min(total_entries) as u16;
        if scroll > 0 {
            execute!(stdout, MoveTo(64, list_start_y), SetForegroundColor(ACCENT_COLOR), Print("▲"), ResetColor)?;
        }
        if scroll + max_visible < total_entries {
            execute!(stdout, MoveTo(64, vis_end_y - 1), SetForegroundColor(ACCENT_COLOR), Print("▼"), ResetColor)?;
        }

        // Separator
        let sep_y = list_start_y + max_visible as u16 + 1;
        execute!(
            stdout,
            MoveTo(2, sep_y),
            SetForegroundColor(BORDER_COLOR),
            Print("─".repeat(60)),
            ResetColor
        )?;

        // Status / path-input line
        let status_y = sep_y + 1;
        if typing_path {
            execute!(
                stdout,
                MoveTo(2, status_y),
                SetForegroundColor(ACCENT_COLOR),
                Print("Path: "),
                SetForegroundColor(TEXT_COLOR),
                Print(&typed_input),
                Print("█"),  // cursor
                ResetColor
            )?;
        } else if let Some((ref msg, is_err)) = status_msg {
            execute!(
                stdout,
                MoveTo(2, status_y),
                SetForegroundColor(if is_err {
                    Color::Rgb { r: 255, g: 80, b: 80 }
                } else {
                    ACCENT_COLOR
                }),
                Print(msg),
                ResetColor
            )?;
        }

        // Help footer
        let help_y = status_y + 2;
        execute!(
            stdout,
            MoveTo(2, help_y),
            SetForegroundColor(DIM_COLOR),
            Print("↑↓ Move  Enter Navigate  S Set&Save  P Type path  Esc/X Cancel"),
            ResetColor
        )?;

        stdout.flush()?;

        // ── Handle input ──────────────────────────────────────────────────
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            status_msg = None; // clear status on next keypress

            if typing_path {
                match key.code {
                    KeyCode::Esc => {
                        typing_path = false;
                        typed_input.clear();
                    }
                    KeyCode::Enter => {
                        // Confirm the typed path
                        let expanded = if typed_input.is_empty() {
                            None
                        } else {
                            config::expand_tilde(typed_input.trim())
                        };
                        if let Some(p) = expanded {
                            if p.is_dir() {
                                current = p;
                                subdirs = list_subdirs(&current);
                                selected = 0;
                                scroll = 0;
                                typing_path = false;
                                typed_input.clear();
                                status_msg = Some(("Directory set.".to_string(), false));
                            } else {
                                status_msg = Some(("Path not found or not a directory.".to_string(), true));
                            }
                        } else {
                            status_msg = Some(("Invalid path.".to_string(), true));
                        }
                        if !typing_path { typed_input.clear(); }
                    }
                    KeyCode::Backspace => {
                        typed_input.pop();
                    }
                    KeyCode::Char(c) => {
                        typed_input.push(c);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('x') | KeyCode::Char('X') => {
                        return Ok(None);
                    }
                    KeyCode::Up => {
                        if selected > 0 { selected -= 1; }
                    }
                    KeyCode::Down => {
                        if selected + 1 < total_entries { selected += 1; }
                    }
                    KeyCode::Enter | KeyCode::Char('z') | KeyCode::Char('Z') => {
                        if selected == 0 {
                            // Go up
                            if let Some(parent) = current.parent().map(|p| p.to_path_buf()) {
                                current = parent;
                                subdirs = list_subdirs(&current);
                                selected = 0;
                                scroll = 0;
                            }
                        } else {
                            // Enter subdirectory
                            let target = subdirs[selected - 1].clone();
                            current = target;
                            subdirs = list_subdirs(&current);
                            selected = 0;
                            scroll = 0;
                        }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        // Set & Save current directory
                        return Ok(Some(current));
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        // Switch to manual path entry mode
                        typing_path = true;
                        typed_input.clear();
                    }
                    _ => {}
                }
            }
        }
    }
}

