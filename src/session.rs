//! A running game.
//!
//! [`Session::start`] puts a [`Machine`] on its own thread, which keeps real
//! Game Boy speed, plays the sound, loads the save and writes it back (soon
//! after the game saves, and on stop). The front end stays on its own thread
//! and only does three things each tick:
//!
//! - [`Session::copy_frame`] to get the picture when there is a new one,
//! - [`Session::status`] for the numbers to show,
//! - [`Session::send`] for buttons and controls.

use crate::audio::AudioOut;
use crate::cart::{Header, RomImage};
use crate::frame::{Frame, Palette};
use crate::input::{Button, Edge};
use crate::machine::{Machine, FRAME_RATE};
use crate::saves::{self, SaveData};
use crate::settings::Config;
use crate::states::{self, SlotInfo, SLOTS};
use serde::Serialize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const MAX_SPEED: u8 = 8;

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub palette: Palette,
    /// Open the sound device. Off for tests and headless use.
    pub audio: bool,
    pub volume: f32,
    pub muted: bool,
    pub saves_dir: PathBuf,
    /// Where save-state slots are kept (see [`crate::states`]).
    pub states_dir: PathBuf,
    /// Load the existing save before the first frame.
    pub load_save: bool,
    /// Write the save file (autosave and on stop).
    pub write_saves: bool,
    /// How long cartridge RAM must be quiet before it is written; zero saves
    /// only on stop.
    pub autosave: Duration,
    /// Where `stats.json` lives; `None` records no play statistics.
    pub stats_dir: Option<PathBuf>,
    pub start_paused: bool,
}

impl SessionOptions {
    /// The options for normal play, from the user's settings.
    pub fn from_config(config: &Config, app_dir: &Path) -> Self {
        Self {
            palette: config.palette(),
            audio: config.audio.enabled,
            volume: config.audio.volume,
            muted: false,
            saves_dir: config.saves_dir(app_dir),
            states_dir: config.states_dir(app_dir),
            load_save: true,
            write_saves: true,
            autosave: Duration::from_secs(config.autosave_secs),
            stats_dir: Some(app_dir.to_path_buf()),
            start_paused: false,
        }
    }

    /// [`Self::from_config`] with the palette remembered for this game.
    pub fn for_game(config: &Config, app_dir: &Path, rom_id: &str) -> Self {
        Self {
            palette: config.palette_for(rom_id),
            ..Self::from_config(config, app_dir)
        }
    }

    /// No sound, no statistics, nothing written unless a save state is asked
    /// for (those go to `<saves_dir>/states`): for tests and tools.
    pub fn headless(saves_dir: PathBuf) -> Self {
        Self {
            states_dir: saves_dir.join("states"),
            palette: Palette::Grey,
            audio: false,
            volume: 0.0,
            muted: true,
            saves_dir,
            load_save: false,
            write_saves: false,
            autosave: Duration::ZERO,
            stats_dir: None,
            start_paused: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Press(Button),
    Release(Button),
    Pause(bool),
    TogglePause,
    /// Frames emulated per displayed frame: 1 is normal, up to [`MAX_SPEED`].
    /// Sound is off while above 1.
    Speed(u8),
    /// Power-cycle the console (the save RAM is kept).
    Reset,
    /// Write the save file now.
    Save,
    /// Snapshot the machine into slot 1..=4, replacing what was there.
    SaveState(u8),
    /// Go back to the snapshot in a slot. An empty slot only leaves a note.
    LoadState(u8),
    /// Empty a slot.
    ClearState(u8),
    Palette(Palette),
    Mute(bool),
    Volume(f32),
}

impl From<Edge> for Command {
    fn from(edge: Edge) -> Self {
        match edge {
            Edge::Pressed(button) => Command::Press(button),
            Edge::Released(button) => Command::Release(button),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Paused,
    /// Ended normally; the save is written.
    Stopped,
    /// The emulator core failed; `Status::note` has the reason.
    Crashed,
}

/// Everything a status panel shows. Cheap to clone; read it once per tick.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub state: State,
    /// Frames emulated since power-on.
    pub frames: u64,
    /// Displayed frames per second over the last second.
    pub fps: f32,
    /// Milliseconds spent emulating each displayed frame, averaged over the
    /// last second (the budget is 16.74 at normal speed).
    pub frame_ms: f32,
    /// `frame_ms` as a share of the frame budget: 0.09 is "cpu 9%".
    pub load: f32,
    pub speed: u8,
    pub palette: Palette,
    /// False when no sound device could be opened.
    pub audio_available: bool,
    pub muted: bool,
    pub volume: f32,
    /// Seconds played (paused time not counted).
    pub play_seconds: u64,
    /// The cartridge keeps a save file.
    pub saves: bool,
    /// Cartridge RAM has changes that are not in the save file yet.
    pub unsaved: bool,
    /// Unix time the save file was last written this session.
    pub saved_at: Option<u64>,
    /// The save-state slots, index 0 being slot 1; `None` is an empty slot.
    pub slots: [Option<SlotInfo>; SLOTS as usize],
    /// The latest problem worth showing (no sound device, save failed, ...).
    pub note: Option<String>,
}

struct Shared {
    frame: Mutex<Frame>,
    frame_number: AtomicU64,
    status: Mutex<Status>,
}

/// A poisoned lock only means another thread panicked mid-update; the data is
/// plain values, so keep going with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

enum Message {
    Command(Command),
    Stop,
}

pub struct Session {
    messages: Sender<Message>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    header: Header,
    id: String,
    path: PathBuf,
}

impl Session {
    /// Starts the game. Fails when the cartridge is unsupported or the core
    /// rejects the ROM; a missing sound device or unreadable save does not
    /// fail the start, it shows up in [`Status::note`].
    pub fn start(rom: RomImage, options: SessionOptions) -> Result<Session, String> {
        let shared = Arc::new(Shared {
            frame: Mutex::new(Frame::default()),
            frame_number: AtomicU64::new(0),
            status: Mutex::new(Status {
                state: if options.start_paused {
                    State::Paused
                } else {
                    State::Running
                },
                frames: 0,
                fps: 0.0,
                frame_ms: 0.0,
                load: 0.0,
                speed: 1,
                palette: options.palette,
                audio_available: false,
                muted: options.muted,
                volume: options.volume.clamp(0.0, 1.0),
                play_seconds: 0,
                saves: rom.header.saves(),
                unsaved: false,
                saved_at: None,
                slots: states::list(&options.states_dir, &rom.id()),
                note: None,
            }),
        });
        let header = rom.header.clone();
        let id = rom.id();
        let path = rom.path.clone();

        let (messages, inbox) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("gameboy".into())
            .spawn(move || {
                // The machine is built here because it cannot move between threads.
                let worker = match Worker::new(rom, options, thread_shared) {
                    Ok(worker) => {
                        let _ = ready.send(Ok(()));
                        worker
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                worker.run(inbox);
            })
            .map_err(|error| format!("cannot start the emulator thread: {error}"))?;

        match started.recv() {
            Ok(Ok(())) => Ok(Session {
                messages,
                shared,
                thread: Some(thread),
                header,
                id,
                path,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("the emulator thread ended before it started the game".into()),
        }
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The ROM id (see [`RomImage::id`]).
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rom_path(&self) -> &Path {
        &self.path
    }

    /// Queues a command; it takes effect before the next frame.
    pub fn send(&self, command: Command) {
        let _ = self.messages.send(Message::Command(command));
    }

    /// Copies the newest picture into `frame` when it is newer than `seen`
    /// (start `seen` at 0) and returns true; otherwise leaves both alone.
    pub fn copy_frame(&self, frame: &mut Frame, seen: &mut u64) -> bool {
        let number = self.shared.frame_number.load(Ordering::Acquire);
        if number == *seen {
            return false;
        }
        frame.clone_from(&lock(&self.shared.frame));
        *seen = number;
        true
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// True until the game has stopped or crashed.
    pub fn is_alive(&self) -> bool {
        matches!(self.status().state, State::Running | State::Paused)
    }

    /// Stops the game, writes the save and waits for the thread. Returns the
    /// final status: check `note` for a save that could not be written.
    pub fn stop(mut self) -> Status {
        self.shut_down();
        self.status()
    }

    fn shut_down(&mut self) {
        let _ = self.messages.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shut_down();
    }
}

struct Worker {
    machine: Machine,
    options: SessionOptions,
    shared: Arc<Shared>,
    audio: Option<AudioOut>,
    save_path: PathBuf,
    rom_id: String,
    paused: bool,
    speed: u8,
    muted: bool,
    /// When cartridge RAM first and last changed since the last write.
    dirty: Option<(Instant, Instant)>,
    last_written: Option<SaveData>,
    played: Duration,
}

impl Worker {
    fn new(rom: RomImage, options: SessionOptions, shared: Arc<Shared>) -> Result<Self, String> {
        let mut machine = Machine::new(&rom, options.palette)?;
        let mut note = None;
        let mut last_written = None;
        if options.load_save && rom.header.saves() {
            let header = &rom.header;
            match saves::read(
                &options.saves_dir,
                &rom.path,
                header.save_ram_bytes(),
                header.clock,
            ) {
                Ok(Some(save)) => {
                    machine.load_save(&save);
                    last_written = machine.save_data();
                }
                Ok(None) => {}
                Err(error) => note = Some(format!("could not read the save: {error}")),
            }
        }
        let audio = if options.audio {
            match AudioOut::open(if options.muted { 0.0 } else { options.volume }) {
                Ok(audio) => Some(audio),
                Err(error) => {
                    note.get_or_insert(error);
                    None
                }
            }
        } else {
            None
        };
        {
            let mut status = lock(&shared.status);
            status.audio_available = audio.is_some();
            status.note = note;
        }
        Ok(Self {
            save_path: saves::save_path(&options.saves_dir, &rom.path),
            rom_id: rom.id(),
            paused: options.start_paused,
            muted: options.muted,
            machine,
            options,
            shared,
            audio,
            speed: 1,
            dirty: None,
            last_written,
            played: Duration::ZERO,
        })
    }

    fn run(mut self, inbox: Receiver<Message>) {
        let outcome = catch_unwind(AssertUnwindSafe(|| self.play(&inbox)));
        let crashed = outcome.is_err();
        if let Err(panic) = outcome {
            let reason = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_else(|| "unknown error".into());
            lock(&self.shared.status).note = Some(format!("the emulator core failed: {reason}"));
        } else {
            // After a crash the RAM cannot be trusted; the last autosave stands.
            self.write_save();
        }
        if let Some(dir) = &self.options.stats_dir {
            let _ = crate::library::Stats::record_session(dir, &self.rom_id, self.played.as_secs());
        }
        let mut status = lock(&self.shared.status);
        status.state = if crashed {
            State::Crashed
        } else {
            State::Stopped
        };
        status.play_seconds = self.played.as_secs();
    }

    fn play(&mut self, inbox: &Receiver<Message>) {
        let frame_time = Duration::from_secs_f64(1.0 / FRAME_RATE);
        let mut next_frame = Instant::now();
        let mut second_start = Instant::now();
        let mut shown_this_second = 0u32;
        let mut fps = 0.0f32;
        let mut busy_this_second = Duration::ZERO;
        let mut frame_ms = 0.0f32;

        loop {
            // While paused, wait here instead of spinning.
            if self.paused {
                match inbox.recv_timeout(Duration::from_millis(100)) {
                    Ok(Message::Command(command)) => self.apply(command),
                    Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if !self.paused {
                    next_frame = Instant::now();
                    second_start = next_frame;
                    shown_this_second = 0;
                    busy_this_second = Duration::ZERO;
                }
            }
            loop {
                match inbox.try_recv() {
                    Ok(Message::Command(command)) => self.apply(command),
                    Ok(Message::Stop) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }

            if !self.paused {
                let audio = if self.speed == 1 && !self.muted {
                    self.audio.as_ref()
                } else {
                    None
                };
                let began = Instant::now();
                for _ in 0..self.speed {
                    self.machine.run_frame(|samples| {
                        if let Some(audio) = audio {
                            audio.push(samples);
                        }
                    });
                }
                busy_this_second += began.elapsed();
                self.publish_frame();
                self.played += frame_time;
                shown_this_second += 1;
            }

            let now = Instant::now();
            if self.machine.take_ram_dirty() {
                self.dirty = Some((self.dirty.map_or(now, |(first, _)| first), now));
            }
            self.autosave(now);

            let elapsed = now.duration_since(second_start);
            if elapsed >= Duration::from_secs(1) {
                fps = shown_this_second as f32 / elapsed.as_secs_f32();
                if shown_this_second > 0 {
                    frame_ms = busy_this_second.as_secs_f32() * 1000.0 / shown_this_second as f32;
                }
                second_start = now;
                shown_this_second = 0;
                busy_this_second = Duration::ZERO;
            }
            {
                let mut status = lock(&self.shared.status);
                status.state = if self.paused {
                    State::Paused
                } else {
                    State::Running
                };
                status.frames = self.machine.frames();
                status.fps = if self.paused { 0.0 } else { fps };
                status.frame_ms = frame_ms;
                status.load = frame_ms / (frame_time.as_secs_f32() * 1000.0);
                status.speed = self.speed;
                status.palette = self.machine.palette();
                status.muted = self.muted;
                status.play_seconds = self.played.as_secs();
                status.unsaved = self.dirty.is_some();
            }

            if !self.paused {
                next_frame += frame_time;
                let now = Instant::now();
                if next_frame > now {
                    std::thread::sleep(next_frame - now);
                } else if now - next_frame > Duration::from_millis(100) {
                    // Fell far behind (machine asleep, debugger): do not race to catch up.
                    next_frame = now;
                }
            }
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Press(button) => self.machine.press(button),
            Command::Release(button) => self.machine.release(button),
            Command::Pause(paused) => self.set_paused(paused),
            Command::TogglePause => self.set_paused(!self.paused),
            Command::Speed(speed) => {
                self.speed = speed.clamp(1, MAX_SPEED);
                self.clear_audio();
            }
            Command::Reset => {
                if let Err(error) = self.machine.reset() {
                    lock(&self.shared.status).note = Some(format!("reset failed: {error}"));
                }
                self.clear_audio();
            }
            Command::Save => {
                self.dirty = None;
                self.write_save();
            }
            Command::SaveState(slot) => self.save_state(slot),
            Command::LoadState(slot) => self.load_state(slot),
            Command::ClearState(slot) => {
                let cleared = states::clear(&self.options.states_dir, &self.rom_id, slot);
                self.slot_changed(cleared.map(|_| ()), "clear");
            }
            Command::Palette(palette) => self.machine.set_palette(palette),
            Command::Mute(muted) => {
                self.muted = muted;
                self.apply_volume();
            }
            Command::Volume(volume) => {
                lock(&self.shared.status).volume = volume.clamp(0.0, 1.0);
                self.apply_volume();
            }
        }
    }

    fn publish_frame(&self) {
        lock(&self.shared.frame).clone_from(self.machine.frame());
        self.shared.frame_number.fetch_add(1, Ordering::AcqRel);
    }

    fn save_state(&mut self, slot: u8) {
        let written = states::write(
            &self.options.states_dir,
            &self.rom_id,
            slot,
            self.machine.frame(),
            self.machine.frames(),
            &self.machine.save_state(),
        );
        self.slot_changed(written.map(|_| ()), "write");
    }

    fn load_state(&mut self, slot: u8) {
        let loaded = match states::read(&self.options.states_dir, &self.rom_id, slot) {
            Ok(Some(saved)) => self.machine.load_state(&saved.state),
            Ok(None) => Err(format!("slot {slot} is empty")),
            Err(error) => Err(error.to_string()),
        };
        match loaded {
            Ok(()) => {
                // Show the snapshot's picture at once, also while paused.
                self.publish_frame();
                self.clear_audio();
                let mut status = lock(&self.shared.status);
                status.frames = self.machine.frames();
                status.note = None;
            }
            Err(error) => {
                lock(&self.shared.status).note = Some(format!("could not load state: {error}"));
            }
        }
    }

    /// Refreshes the slot list in the status after a slot file changed.
    fn slot_changed(&mut self, result: std::io::Result<()>, action: &str) {
        let slots = states::list(&self.options.states_dir, &self.rom_id);
        let mut status = lock(&self.shared.status);
        status.slots = slots;
        status.note = result
            .err()
            .map(|error| format!("could not {action} the save state: {error}"));
    }

    fn set_paused(&mut self, paused: bool) {
        if paused && !self.paused {
            // A pause is when people close the terminal: make sure the save is on disk.
            self.dirty = None;
            self.write_save();
        }
        self.paused = paused;
        self.clear_audio();
    }

    fn clear_audio(&self) {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
    }

    fn apply_volume(&self) {
        if let Some(audio) = &self.audio {
            let volume = lock(&self.shared.status).volume;
            audio.set_volume(if self.muted { 0.0 } else { volume });
        }
    }

    /// Writes once the game has stopped touching its RAM for `autosave`, or
    /// after fifteen times that for games that never stop.
    fn autosave(&mut self, now: Instant) {
        let wait = self.options.autosave;
        if wait.is_zero() {
            return;
        }
        if let Some((first, last)) = self.dirty {
            if now.duration_since(last) >= wait || now.duration_since(first) >= wait * 15 {
                self.dirty = None;
                self.write_save();
            }
        }
    }

    fn write_save(&mut self) {
        if !self.options.write_saves {
            return;
        }
        let Some(data) = self.machine.save_data() else {
            return;
        };
        if self.last_written.as_ref() == Some(&data) {
            return;
        }
        let result = saves::write(&self.save_path, &data);
        let mut status = lock(&self.shared.status);
        match result {
            Ok(()) => {
                self.last_written = Some(data);
                status.saved_at = Some(crate::paths::unix_now());
            }
            Err(error) => {
                status.note = Some(format!(
                    "could not write {}: {error}",
                    self.save_path.display()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::testing;

    fn rom(dir: &Path) -> RomImage {
        RomImage::from_bytes(dir.join("Test Game.gb"), None, testing::rom()).unwrap()
    }

    /// Waits (up to five seconds) for `done`, so a slow machine does not fail the test.
    fn wait_for(session: &Session, done: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = session.status();
            if done(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "timed out; last status: {status:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn plays_frames_pauses_and_hands_out_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let session =
            Session::start(rom(dir.path()), SessionOptions::headless(dir.path().into())).unwrap();
        assert_eq!(session.header().title, "VOSTOKTEST");
        wait_for(&session, |status| status.frames >= 5);

        let mut frame = Frame::default();
        let mut seen = 0;
        assert!(session.copy_frame(&mut frame, &mut seen));
        assert!(seen > 0);

        session.send(Command::Pause(true));
        let paused = wait_for(&session, |status| status.state == State::Paused);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(session.status().frames, paused.frames);
        // Nothing new while paused.
        session.copy_frame(&mut frame, &mut seen);
        assert!(!session.copy_frame(&mut frame, &mut seen));

        session.send(Command::Speed(200));
        session.send(Command::Palette(Palette::Amber));
        session.send(Command::TogglePause);
        let running = wait_for(&session, |status| status.frames > paused.frames + 8);
        assert_eq!(running.speed, MAX_SPEED);
        assert_eq!(running.palette, Palette::Amber);

        let last = session.stop();
        assert_eq!(last.state, State::Stopped);
        // Headless writes nothing.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn saves_on_stop_reloads_and_records_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let saves = dir.path().join("saves");
        let options = SessionOptions {
            load_save: true,
            write_saves: true,
            stats_dir: Some(dir.path().to_path_buf()),
            ..SessionOptions::headless(saves.clone())
        };

        let session = Session::start(rom(dir.path()), options.clone()).unwrap();
        let id = session.id().to_string();
        wait_for(&session, |status| status.frames >= 5 && status.unsaved);
        let last = session.stop();
        assert!(last.saved_at.is_some(), "{last:?}");

        let file = saves.join("Test Game.sav");
        let mut bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes.len(), 0x2000);
        assert_eq!(bytes[0], 0x42);
        assert_eq!(crate::library::Stats::load(dir.path()).get(&id).launches, 1);

        // A second session starts from the file: a byte the game never writes survives.
        bytes[0x1000] = 0x77;
        std::fs::write(&file, &bytes).unwrap();
        let session = Session::start(rom(dir.path()), options).unwrap();
        wait_for(&session, |status| status.frames >= 5);
        session.stop();
        assert_eq!(std::fs::read(&file).unwrap()[0x1000], 0x77);
    }

    #[test]
    fn autosave_writes_while_the_game_is_still_running() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions {
            write_saves: true,
            autosave: Duration::from_millis(10),
            ..SessionOptions::headless(dir.path().join("saves"))
        };
        let session = Session::start(rom(dir.path()), options).unwrap();
        let status = wait_for(&session, |status| status.saved_at.is_some());
        assert_eq!(status.state, State::Running);
        assert!(dir.path().join("saves").join("Test Game.sav").is_file());
    }

    #[test]
    fn save_states_go_to_slots_and_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::headless(dir.path().join("saves"));
        let session = Session::start(rom(dir.path()), options.clone()).unwrap();
        assert_eq!(session.status().slots, [None; 4]);
        wait_for(&session, |status| status.frames >= 10);

        // Paused, so the frame count in the slot can be compared exactly.
        session.send(Command::Pause(true));
        let paused = wait_for(&session, |status| status.state == State::Paused);
        session.send(Command::SaveState(3));
        let saved = wait_for(&session, |status| status.slots[2].is_some());
        let slot = saved.slots[2].unwrap();
        assert_eq!((slot.slot, slot.frames), (3, paused.frames));
        assert!(saved.slots[0].is_none() && saved.note.is_none());

        session.send(Command::Pause(false));
        wait_for(&session, |status| status.frames >= paused.frames + 20);
        session.send(Command::Pause(true));
        wait_for(&session, |status| status.state == State::Paused);
        let mut frame = Frame::default();
        let mut seen = 0;
        session.copy_frame(&mut frame, &mut seen);
        session.send(Command::LoadState(3));
        let back = wait_for(&session, |status| status.frames == paused.frames);
        assert!(back.note.is_none(), "{back:?}");
        // The picture from the snapshot is handed out even though the game is paused.
        assert!(session.copy_frame(&mut frame, &mut seen));

        // Mistakes leave a note and change nothing.
        session.send(Command::LoadState(1));
        let note = wait_for(&session, |status| status.note.is_some())
            .note
            .unwrap();
        assert!(note.contains("slot 1 is empty"), "{note}");
        session.send(Command::SaveState(9));
        let status = wait_for(&session, |status| {
            status
                .note
                .as_deref()
                .is_some_and(|note| note.contains("no slot 9"))
        });
        assert_eq!(status.frames, paused.frames);

        session.send(Command::ClearState(3));
        wait_for(&session, |status| status.slots == [None; 4]);
        session.stop();

        // A new session of the same game sees a slot that was left behind.
        let session = Session::start(rom(dir.path()), options.clone()).unwrap();
        session.send(Command::SaveState(1));
        wait_for(&session, |status| status.slots[0].is_some());
        session.stop();
        let session = Session::start(rom(dir.path()), options).unwrap();
        assert!(session.status().slots[0].is_some());
    }

    #[test]
    fn a_cartridge_the_core_cannot_run_fails_to_start() {
        let mut bytes = testing::rom();
        bytes[0x147] = 0x22;
        let rom = RomImage::from_bytes("x.gb".into(), None, bytes).unwrap();
        let error = Session::start(rom, SessionOptions::headless("saves".into()))
            .err()
            .unwrap();
        assert!(error.contains("not supported"), "{error}");
    }
}
