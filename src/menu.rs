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
    StartGame,
}

impl MenuSection {
    fn next(self) -> Self {
        match self {
            Self::RenderMode => Self::Audio,
            Self::Audio => Self::RomsDirectory,
            Self::RomsDirectory => Self::GameList,
            Self::GameList => Self::StartGame,
            Self::StartGame => Self::RenderMode,
        }
    }
    
    fn prev(self) -> Self {
        match self {
            Self::RenderMode => Self::StartGame,
            Self::Audio => Self::RenderMode,
            Self::RomsDirectory => Self::Audio,
            Self::GameList => Self::RomsDirectory,
            Self::StartGame => Self::GameList,
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
        let roms_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let rom_files = scan_for_roms(&roms_dir);
        
        Self {
            render_mode: RenderMode::Ascii,
            audio_enabled: true,
            roms_dir,
            rom_files,
            selected_rom_index: 0,
            current_section: MenuSection::GameList,
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
                    if state.current_section == MenuSection::GameList {
                        state.current_section = MenuSection::RenderMode;
                    } else {
                        return Ok(None);
                    }
                }
                KeyCode::Up => {
                    if state.current_section == MenuSection::GameList {
                        if state.selected_rom_index > 0 {
                            state.selected_rom_index -= 1;
                            if state.selected_rom_index < state.scroll_offset {
                                state.scroll_offset = state.selected_rom_index;
                            }
                        }
                    } else {
                        state.current_section = state.current_section.prev();
                    }
                }
                KeyCode::Down => {
                    if state.current_section == MenuSection::GameList {
                        if state.selected_rom_index + 1 < state.rom_files.len() {
                            state.selected_rom_index += 1;
                            // Adjust scroll (show max 8 items)
                            if state.selected_rom_index >= state.scroll_offset + 8 {
                                state.scroll_offset = state.selected_rom_index.saturating_sub(7);
                            }
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
                        MenuSection::StartGame | MenuSection::GameList => {
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
                            // Toggle between current dir and roms subdir
                            let roms_subdir = state.roms_dir.join("roms");
                            if state.roms_dir.ends_with("roms") || state.roms_dir.ends_with("ROMs") {
                                state.roms_dir = state.roms_dir.parent()
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or_else(|| PathBuf::from("."));
                            } else if roms_subdir.exists() {
                                state.roms_dir = roms_subdir;
                            }
                            state.refresh_roms();
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
    
    // ROMs Directory
    let dir_display = state.roms_dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| state.roms_dir.to_string_lossy().to_string());
    draw_option(
        stdout, 
        4, 
        menu_start_y + 4, 
        "Directory  ", 
        &format!("./{:<20}", truncate_str(&dir_display, 20)),
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
    
    // Start button
    let button_y = games_start_y + 10;
    execute!(stdout, MoveTo(4, button_y))?;
    
    let can_start = !state.rom_files.is_empty();
    if state.current_section == MenuSection::StartGame && can_start {
        execute!(
            stdout,
            SetBackgroundColor(HIGHLIGHT_BG),
            SetForegroundColor(HIGHLIGHT_FG),
            Print("  ╔════════════════════╗  "),
            ResetColor
        )?;
        execute!(stdout, MoveTo(4, button_y + 1))?;
        execute!(
            stdout,
            SetBackgroundColor(HIGHLIGHT_BG),
            SetForegroundColor(HIGHLIGHT_FG),
            Print("  ║   ▶ START GAME ◀   ║  "),
            ResetColor
        )?;
        execute!(stdout, MoveTo(4, button_y + 2))?;
        execute!(
            stdout,
            SetBackgroundColor(HIGHLIGHT_BG),
            SetForegroundColor(HIGHLIGHT_FG),
            Print("  ╚════════════════════╝  "),
            ResetColor
        )?;
    } else {
        let fg = if can_start { TEXT_COLOR } else { DIM_COLOR };
        execute!(
            stdout,
            SetForegroundColor(fg),
            Print("  ╔════════════════════╗  "),
            ResetColor
        )?;
        execute!(stdout, MoveTo(4, button_y + 1))?;
        execute!(
            stdout,
            SetForegroundColor(fg),
            Print("  ║     START GAME     ║  "),
            ResetColor
        )?;
        execute!(stdout, MoveTo(4, button_y + 2))?;
        execute!(
            stdout,
            SetForegroundColor(fg),
            Print("  ╚════════════════════╝  "),
            ResetColor
        )?;
    }
    
    // Help text
    let help_y = button_y + 5;
    execute!(stdout, MoveTo(4, help_y), SetForegroundColor(DIM_COLOR))?;
    execute!(stdout, Print("Controls: ↑↓←→ Navigate  Z Confirm  X Back  Enter Start  Space Toggle  R Refresh"))?;
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

