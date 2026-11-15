//! What the player is doing and what each key does. No drawing here: `view`
//! reads this and draws it.

use crate::layout::{self, Kind, View};
use crate::tokens::ThemeName;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vostok_gameboy::cart::{self, Color};
use vostok_gameboy::frame::Frame;
use vostok_gameboy::input::{InputTracker, KeyMap};
use vostok_gameboy::library::{self, RomEntry, Scan, Sort, Stats};
use vostok_gameboy::session::{Command, Session, SessionOptions, State, Status};
use vostok_gameboy::settings::Config;
use vostok_gameboy::{paths, states};

/// The pause menu, top to bottom.
pub const MENU: [&str; 7] = [
    "RESUME",
    "SAVE STATE",
    "LOAD STATE",
    "PALETTE",
    "CONTROLS",
    "RESET",
    "QUIT TO LIBRARY",
];

/// A game in the machine.
pub struct Game {
    pub session: Session,
    pub name: String,
    pub id: String,
    pub frame: Frame,
    seen: u64,
    /// A new picture has arrived since the last one was shown.
    pub fresh: bool,
    pub status: Status,
    status_due: Instant,
    pub input: InputTracker,
    pub turbo: bool,
    turbo_at: Instant,
    /// The pause menu is open on this item.
    pub menu: Option<usize>,
    /// The save-state slot F5 and F8 use, 1..=4.
    pub slot: u8,
    /// Preview pictures of the slots, with the save they were read for.
    pub thumbs: [Option<((u64, u64), Frame)>; 4],
    /// Paused because the terminal is too small to show anything.
    held: bool,
}

impl Game {
    pub fn paused(&self) -> bool {
        self.menu.is_some() || self.status.state == State::Paused
    }
    pub fn mono(&self) -> bool {
        self.session.header().color == Color::Mono
    }
}

pub enum Typing {
    Search,
    /// A ROM folder to add.
    Folder(String),
}

pub struct Library {
    pub scan: Scan,
    pub sort: Sort,
    pub search: String,
    pub typing: Option<Typing>,
    pub selected: usize,
    /// The cartridge header dialog.
    pub info: bool,
    /// The newest save-state picture of the selected game: (rom id, picture).
    pub preview: Option<(String, Option<Frame>)>,
}

pub struct App {
    pub app_dir: PathBuf,
    pub config: Config,
    pub keys: KeyMap,
    pub theme: ThemeName,
    pub view: View,
    /// How many shell sizes to step down from the largest that fits.
    pub shrink: u8,
    /// The terminal draws pictures (Kitty graphics); otherwise half-blocks.
    pub pixels: bool,
    pub library: Library,
    pub game: Option<Game>,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    /// Started with a ROM on the command line: leaving the game leaves the app.
    pub direct: bool,
    pub quit: bool,
    pub dirty: bool,
    pub size: (u16, u16),
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Name => "name",
        Sort::Recent => "recent",
        Sort::Played => "played",
    }
}

impl App {
    pub fn new(app_dir: PathBuf, config: Config, pixels: bool) -> App {
        let text = |key: &str| {
            config
                .ui
                .get(key)
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let sort = [Sort::Recent, Sort::Name, Sort::Played]
            .into_iter()
            .find(|s| sort_name(*s) == text("sort"));
        let mut app = App {
            keys: KeyMap::new(&config.keys),
            theme: ThemeName::parse(&text("theme")).unwrap_or_default(),
            view: View::parse(&text("view")).unwrap_or_default(),
            shrink: 0,
            pixels,
            library: Library {
                scan: Scan::default(),
                sort: sort.unwrap_or(Sort::Recent),
                search: String::new(),
                typing: None,
                selected: 0,
                info: false,
                preview: None,
            },
            game: None,
            help: false,
            toast: None,
            direct: false,
            quit: false,
            dirty: true,
            size: (0, 0),
            app_dir,
            config,
        };
        app.rescan();
        app
    }

    pub fn sort_label(&self) -> &'static str {
        sort_name(self.library.sort)
    }

    fn save_ui(&mut self) {
        let theme = if self.theme == ThemeName::Amber {
            "amber"
        } else {
            "phosphor"
        };
        for (key, value) in [
            ("theme", theme),
            ("view", self.view.name()),
            ("sort", self.sort_label()),
        ] {
            self.config
                .ui
                .insert(key.into(), toml::Value::String(value.into()));
        }
        self.save_config();
    }

    fn save_config(&mut self) {
        if let Err(error) = self.config.save(&self.app_dir) {
            self.say(format!("settings not saved: {error}"));
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now() + Duration::from_secs(3)));
        self.dirty = true;
    }

    /// Read the ROM folders again, keeping the selection on the same game.
    pub fn rescan(&mut self) {
        let keep = self.selected().map(|entry| entry.id.clone());
        let (saves, states) = (
            self.config.saves_dir(&self.app_dir),
            self.config.states_dir(&self.app_dir),
        );
        self.library.scan = library::scan(
            &self.config.rom_dirs,
            &saves,
            &states,
            &Stats::load(&self.app_dir),
        );
        self.library.scan.sort(self.library.sort);
        let shown = self.shown();
        let at = keep.and_then(|id| shown.iter().position(|entry| entry.id == id));
        self.library.selected = at.unwrap_or(0).min(shown.len().saturating_sub(1));
        self.library.preview = None;
        self.dirty = true;
    }

    /// The games in the list right now (all, or those matching the search).
    pub fn shown(&self) -> Vec<&RomEntry> {
        if self.library.search.is_empty() {
            self.library.scan.entries.iter().collect()
        } else {
            self.library.scan.search(&self.library.search)
        }
    }

    pub fn selected(&self) -> Option<&RomEntry> {
        self.shown().get(self.library.selected).copied()
    }

    /// Put a cartridge in and switch on. `resume` loads that save-state slot.
    pub fn start(&mut self, path: &Path, resume: Option<u8>) -> Result<(), String> {
        let rom = cart::load(path).map_err(|error| error.to_string())?;
        rom.check_supported().map_err(|error| error.to_string())?;
        let (id, name) = (rom.id(), rom.stem());
        let options = SessionOptions::for_game(&self.config, &self.app_dir, &id);
        let session = Session::start(rom, options)?;
        if let Some(slot) = resume {
            session.send(Command::LoadState(slot));
        }
        let now = Instant::now();
        self.game = Some(Game {
            status: session.status(),
            session,
            name,
            id,
            frame: Frame::default(),
            seen: 0,
            fresh: false,
            status_due: now,
            input: InputTracker::default(),
            turbo: false,
            turbo_at: now,
            menu: None,
            slot: resume.unwrap_or(1),
            thumbs: [None, None, None, None],
            held: false,
        });
        self.dirty = true;
        Ok(())
    }

    /// Switch off: the session writes the battery save and the play time.
    pub fn eject(&mut self) {
        if let Some(game) = self.game.take() {
            let last = game.session.stop();
            if let Some(note) = last.note {
                self.say(note);
            }
        }
        if self.direct {
            self.quit = true;
        }
        self.rescan();
    }

    /// Called every pass of the main loop.
    pub fn tick(&mut self, now: Instant) {
        if self.toast.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        let too_small =
            layout::plan(self.size.0, self.size.1, self.view, self.shrink).kind == Kind::TooSmall;
        let states_dir = self.config.states_dir(&self.app_dir);
        let mut ended = false;
        if let Some(game) = &mut self.game {
            for edge in game.input.expire(now) {
                game.session.send(edge.into());
                self.dirty = true;
            }
            if game.session.copy_frame(&mut game.frame, &mut game.seen) {
                game.fresh = true;
                self.dirty = true;
            }
            if too_small != game.held && game.menu.is_none() {
                game.held = too_small;
                game.session.send(Command::Pause(too_small));
            }
            if now >= game.status_due {
                game.status_due = now + Duration::from_millis(200);
                game.status = game.session.status();
                for (index, info) in game.status.slots.iter().enumerate() {
                    let key = info.as_ref().map(|info| (info.saved_at, info.frames));
                    if key != game.thumbs[index].as_ref().map(|(key, _)| *key) {
                        let slot = key.and_then(|_| {
                            states::read(&states_dir, &game.id, index as u8 + 1)
                                .ok()
                                .flatten()
                        });
                        game.thumbs[index] = key.zip(slot.map(|slot| slot.thumbnail));
                    }
                }
                ended = !matches!(game.status.state, State::Running | State::Paused);
                self.dirty = true;
            }
        } else if let Some((id, resume)) = self.selected().map(|entry| {
            (
                entry.id.clone(),
                entry.resume.as_ref().map(|info| info.slot),
            )
        }) {
            if self
                .library
                .preview
                .as_ref()
                .is_none_or(|(shown, _)| *shown != id)
            {
                let slot =
                    resume.and_then(|slot| states::read(&states_dir, &id, slot).ok().flatten());
                self.library.preview = Some((id, slot.map(|slot| slot.thumbnail)));
                self.dirty = true;
            }
        }
        if ended {
            self.eject();
        }
    }

    /// The window lost focus: let go of every button.
    pub fn blur(&mut self) {
        if let Some(game) = &mut self.game {
            for edge in game.input.release_all() {
                game.session.send(edge.into());
            }
            self.dirty = true;
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        self.dirty = true;
        let pressed = key.kind != KeyEventKind::Release;
        if pressed
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('c')
        {
            self.quit = true;
        } else if self.help {
            self.help = !pressed;
        } else if pressed && key.code == KeyCode::F(3) {
            self.theme = self.theme.toggled();
            self.save_ui();
        } else if self.game.is_none() {
            if pressed {
                self.library_key(key.code);
            }
        } else if self.game.as_ref().is_some_and(|game| game.menu.is_some()) {
            if key.kind == KeyEventKind::Press {
                self.menu_key(key.code);
            }
        } else {
            self.play_key(key);
        }
    }

    fn play_key(&mut self, key: KeyEvent) {
        let Some(game) = &mut self.game else { return };
        let now = Instant::now();
        // Buttons first, so a key the player mapped to one is never a command.
        let control = matches!(key.code, KeyCode::Tab | KeyCode::Esc | KeyCode::F(_));
        if let Some(button) = self.keys.lookup(key.code).filter(|_| !control) {
            let edge = match key.kind {
                KeyEventKind::Release => game.input.release(button),
                _ => game.input.press(button, now),
            };
            if let Some(edge) = edge {
                game.session.send(edge.into());
            }
            return;
        }
        if key.kind == KeyEventKind::Release {
            // Tab held down is turbo while held; a tap switches it on or off.
            if key.code == KeyCode::Tab
                && game.turbo
                && now.duration_since(game.turbo_at) > Duration::from_millis(300)
            {
                game.turbo = false;
                game.session.send(Command::Speed(1));
            }
            return;
        }
        let repeat = key.kind == KeyEventKind::Repeat;
        match key.code {
            KeyCode::Tab if !repeat => {
                game.turbo = !game.turbo;
                game.turbo_at = now;
                game.session
                    .send(Command::Speed(if game.turbo { 4 } else { 1 }));
            }
            KeyCode::Char('p' | 'P') | KeyCode::Esc if !repeat => self.open_menu(),
            KeyCode::Char('m' | 'M') if !repeat => {
                let muted = !game.status.muted;
                game.status.muted = muted;
                game.session.send(Command::Mute(muted));
                self.say(if muted { "sound off" } else { "sound on" });
            }
            KeyCode::F(5) if !repeat => self.save_state(),
            KeyCode::F(8) if !repeat => self.load_state(),
            KeyCode::F(6) if !repeat => self.next_palette(),
            KeyCode::F(2) if !repeat => {
                self.view = self.view.next();
                // Panels need 200x56: where they do not fit, skip to the next view.
                if self.view == View::Panels
                    && layout::plan(self.size.0, self.size.1, self.view, 0).kind != Kind::Large
                {
                    self.view = self.view.next();
                }
                self.shrink = 0;
                self.save_ui();
            }
            KeyCode::Char('h' | 'H') if !repeat => {
                self.view = View::Screen;
                self.save_ui();
            }
            KeyCode::Char('-' | '_') => {
                let most = layout::plan(self.size.0, self.size.1, self.view, 0).smaller;
                self.shrink = (self.shrink + 1).min(most);
            }
            KeyCode::Char('+' | '=') => self.shrink = self.shrink.saturating_sub(1),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('q' | 'Q') if !repeat => self.eject(),
            _ => {}
        }
    }

    fn open_menu(&mut self) {
        if let Some(game) = &mut self.game {
            for edge in game.input.release_all() {
                game.session.send(edge.into());
            }
            game.session.send(Command::Pause(true));
            game.menu = Some(0);
        }
    }

    fn resume(&mut self) {
        if let Some(game) = &mut self.game {
            game.session.send(Command::Pause(false));
            game.menu = None;
            game.held = false;
        }
    }

    fn save_state(&mut self) {
        let Some(game) = &mut self.game else { return };
        game.session.send(Command::SaveState(game.slot));
        game.status_due = Instant::now() + Duration::from_millis(60);
        let slot = game.slot;
        self.say(format!("saved to slot {slot}"));
    }

    fn load_state(&mut self) {
        let Some(game) = &mut self.game else { return };
        let slot = game.slot;
        if game.status.slots[usize::from(slot) - 1].is_none() {
            self.say(format!("slot {slot} is empty"));
            return;
        }
        game.session.send(Command::LoadState(slot));
        self.resume();
        self.say(format!("loaded slot {slot}"));
    }

    fn next_palette(&mut self) {
        let Some(game) = &mut self.game else { return };
        if !game.mono() {
            self.say("a colour game brings its own colours");
            return;
        }
        let palette = game.status.palette.next();
        game.status.palette = palette;
        game.session.send(Command::Palette(palette));
        let id = game.id.clone();
        self.config.set_palette_for(&id, palette);
        self.save_config();
        self.say(format!("palette: {}", palette.label()));
    }

    fn menu_key(&mut self, code: KeyCode) {
        let Some(game) = &mut self.game else { return };
        let item = game.menu.unwrap_or(0);
        match code {
            KeyCode::Up | KeyCode::Char('w' | 'k') => {
                game.menu = Some((item + MENU.len() - 1) % MENU.len())
            }
            KeyCode::Down | KeyCode::Char('s' | 'j') => game.menu = Some((item + 1) % MENU.len()),
            KeyCode::Left | KeyCode::Char('a') => game.slot = (game.slot + 2) % 4 + 1,
            KeyCode::Right | KeyCode::Char('d') => game.slot = game.slot % 4 + 1,
            KeyCode::Char(digit @ '1'..='4') => game.slot = digit as u8 - b'0',
            KeyCode::Esc | KeyCode::Char('p' | 'P') => self.resume(),
            KeyCode::F(5) => self.save_state(),
            KeyCode::F(8) => self.load_state(),
            KeyCode::F(6) => self.next_palette(),
            KeyCode::Delete => {
                game.session.send(Command::ClearState(game.slot));
                game.status_due = Instant::now() + Duration::from_millis(60);
                let slot = game.slot;
                self.say(format!("slot {slot} cleared"));
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('q' | 'Q') => self.eject(),
            KeyCode::Enter | KeyCode::Char('z' | ' ') => match item {
                0 => self.resume(),
                1 => self.save_state(),
                2 => self.load_state(),
                3 => self.next_palette(),
                4 => self.help = true,
                5 => {
                    game.session.send(Command::Reset);
                    self.resume();
                }
                _ => self.eject(),
            },
            _ => {}
        }
    }

    /// Keys while a line is being typed (the search, or a folder to add).
    fn typing_key(&mut self, mut typing: Typing, code: KeyCode) {
        let keep = match (&mut typing, code) {
            (Typing::Search, KeyCode::Esc) => {
                self.library.search.clear();
                self.library.selected = 0;
                false
            }
            (_, KeyCode::Esc) | (Typing::Search, KeyCode::Enter) => false,
            (Typing::Folder(text), KeyCode::Enter) => {
                let dir = paths::expand_tilde(text.trim());
                if !dir.is_dir() {
                    self.say(format!("not a folder: {}", dir.display()));
                } else if self.config.add_rom_dir(dir) {
                    self.save_config();
                    self.rescan();
                }
                false
            }
            (Typing::Folder(text), KeyCode::Char(ch)) => {
                text.push(ch);
                true
            }
            (Typing::Folder(text), KeyCode::Backspace) => {
                text.pop();
                true
            }
            (Typing::Search, KeyCode::Char(ch)) => {
                self.library.search.push(ch);
                self.library.selected = 0;
                true
            }
            (Typing::Search, KeyCode::Backspace) => {
                self.library.search.pop();
                self.library.selected = 0;
                true
            }
            _ => true,
        };
        if keep {
            self.library.typing = Some(typing);
        }
    }

    fn library_key(&mut self, code: KeyCode) {
        let count = self.shown().len();
        let step = |selected: usize, by: isize| {
            (selected as isize + by).clamp(0, count.max(1) as isize - 1) as usize
        };
        match code {
            KeyCode::Up => return self.library.selected = step(self.library.selected, -1),
            KeyCode::Down => return self.library.selected = step(self.library.selected, 1),
            KeyCode::PageUp => return self.library.selected = step(self.library.selected, -10),
            KeyCode::PageDown => return self.library.selected = step(self.library.selected, 10),
            _ => {}
        }
        if let Some(typing) = self.library.typing.take() {
            return self.typing_key(typing, code);
        }
        match code {
            KeyCode::Char('k') => self.library.selected = step(self.library.selected, -1),
            KeyCode::Char('j') => self.library.selected = step(self.library.selected, 1),
            KeyCode::Home => self.library.selected = 0,
            KeyCode::End => self.library.selected = count.saturating_sub(1),
            KeyCode::Char('/') => self.library.typing = Some(Typing::Search),
            KeyCode::Char('d') => self.library.typing = Some(Typing::Folder(String::new())),
            KeyCode::Char('i') => self.library.info = !self.library.info,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('q') | KeyCode::Esc if self.library.info => self.library.info = false,
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('s') => {
                self.library.sort = match self.library.sort {
                    Sort::Recent => Sort::Name,
                    Sort::Name => Sort::Played,
                    Sort::Played => Sort::Recent,
                };
                self.save_ui();
                self.rescan();
            }
            KeyCode::Char('p') => {
                let Some((id, mono)) = self
                    .selected()
                    .map(|entry| (entry.id.clone(), entry.header.color == Color::Mono))
                else {
                    return;
                };
                if mono {
                    let palette = self.config.palette_for(&id).next();
                    self.config.set_palette_for(&id, palette);
                    self.save_config();
                    self.say(format!("palette: {}", palette.label()));
                } else {
                    self.say("a colour game brings its own colours");
                }
            }
            KeyCode::Enter | KeyCode::Char('r') => {
                let Some((path, newest)) = self.selected().map(|entry| {
                    (
                        entry.path.clone(),
                        entry.resume.as_ref().map(|info| info.slot),
                    )
                }) else {
                    return;
                };
                let resume = if code == KeyCode::Enter { None } else { newest };
                if code != KeyCode::Enter && resume.is_none() {
                    self.say("no save state to resume");
                } else if let Err(error) = self.start(&path, resume) {
                    self.say(error);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    /// An app on a scratch folder with one home-made ROM in its library.
    pub fn app_with_rom(dir: &Path) -> App {
        let roms = dir.join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(roms.join("Kometka Run.gb"), cart::testing::rom()).unwrap();
        let mut config = Config::default();
        config.audio.enabled = false;
        config.add_rom_dir(roms);
        let mut app = App::new(dir.to_path_buf(), config, false);
        app.size = (140, 39);
        app
    }

    pub fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn release(app: &mut App, code: KeyCode) {
        app.key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
    }

    /// Let the session thread catch up, then read its status.
    pub fn settle(app: &mut App) {
        std::thread::sleep(Duration::from_millis(120));
        if let Some(game) = &mut app.game {
            game.status_due = Instant::now();
        }
        app.tick(Instant::now());
    }

    #[test]
    fn a_game_starts_from_the_library_and_q_brings_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        assert_eq!(app.shown().len(), 1);
        press(&mut app, KeyCode::Char('r'));
        assert!(app.game.is_none(), "nothing to resume yet");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.game.as_ref().unwrap().name, "Kometka Run");
        settle(&mut app);
        assert!(app.game.as_ref().unwrap().fresh, "a picture arrived");
        press(&mut app, KeyCode::Char('q'));
        assert!(app.game.is_none() && !app.quit);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn buttons_are_held_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('z'));
        press(&mut app, KeyCode::Left);
        let down = |app: &App, button| app.game.as_ref().unwrap().input.is_down(button);
        use vostok_gameboy::input::Button;
        assert!(down(&app, Button::A) && down(&app, Button::Left) && !down(&app, Button::B));
        release(&mut app, KeyCode::Char('z'));
        assert!(!down(&app, Button::A) && down(&app, Button::Left));
        // Opening the menu lets go of everything.
        press(&mut app, KeyCode::Char('p'));
        assert!(!down(&app, Button::Left));
        assert_eq!(app.game.as_ref().unwrap().menu, Some(0));
    }

    #[test]
    fn the_pause_menu_saves_loads_and_clears_slots() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        settle(&mut app);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter); // SAVE STATE
        settle(&mut app);
        let game = app.game.as_ref().unwrap();
        assert_eq!(game.status.state, State::Paused);
        assert!(game.status.slots[2].is_some() && game.thumbs[2].is_some());
        assert_eq!(
            game.thumbs[2].as_ref().unwrap().1.width,
            states::THUMB_WIDTH
        );
        // An empty slot does not load and the menu stays open.
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::F(8));
        assert_eq!(app.game.as_ref().unwrap().menu, Some(1));
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::F(8));
        settle(&mut app);
        assert_eq!(app.game.as_ref().unwrap().menu, None);
        assert_eq!(app.game.as_ref().unwrap().status.state, State::Running);
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Delete);
        settle(&mut app);
        assert!(app.game.as_ref().unwrap().status.slots[2].is_none());
        // QUIT TO LIBRARY is the last item; Up from the top wraps to it.
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert!(app.game.is_none());
    }

    #[test]
    fn resume_starts_from_the_newest_slot_and_the_library_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        settle(&mut app);
        press(&mut app, KeyCode::F(5));
        settle(&mut app);
        press(&mut app, KeyCode::Char('q'));
        app.tick(Instant::now());
        assert_eq!(app.selected().unwrap().resume.as_ref().unwrap().slot, 1);
        assert!(app.library.preview.as_ref().unwrap().1.is_some());
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.game.as_ref().unwrap().slot, 1);
    }

    #[test]
    fn theme_view_sort_and_palette_are_kept_in_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::F(3));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('p'));
        let id = app.selected().unwrap().id.clone();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::F(2)); // panels do not fit 140x39: straight to the screen
        assert_eq!(app.view, View::Screen);
        let config = Config::load(dir.path()).unwrap();
        let again = App::new(dir.path().to_path_buf(), config, false);
        assert_eq!(
            (again.theme, again.view, again.library.sort),
            (ThemeName::Amber, View::Screen, Sort::Name)
        );
        assert_eq!(
            again.config.palette_for(&id),
            Config::default().palette().next()
        );
    }

    #[test]
    fn search_and_a_new_folder_change_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Char('/'));
        for ch in "zzz".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert!(app.shown().is_empty() && app.game.is_none());
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.shown().len(), 1);
        let more = dir.path().join("more");
        std::fs::create_dir_all(&more).unwrap();
        let mut rom = cart::testing::rom();
        rom[0x200] ^= 0xFF; // a different game
        std::fs::write(more.join("Station 9.gb"), rom).unwrap();
        press(&mut app, KeyCode::Char('d'));
        for ch in more.to_string_lossy().chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.shown().len(), 2);
        assert!(Config::load(dir.path()).unwrap().rom_dirs.contains(&more));
    }

    #[test]
    fn a_terminal_too_small_pauses_the_game_until_it_grows() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        app.size = (30, 8);
        settle(&mut app);
        settle(&mut app);
        assert_eq!(app.game.as_ref().unwrap().status.state, State::Paused);
        app.size = (80, 24);
        settle(&mut app);
        settle(&mut app);
        assert_eq!(app.game.as_ref().unwrap().status.state, State::Running);
    }
}
