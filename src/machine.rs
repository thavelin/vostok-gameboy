//! One emulated Game Boy, stepped a frame at a time.
//!
//! No threads, no clock, no files: give it a ROM, press buttons, call
//! [`Machine::run_frame`], read the picture. That makes it the piece to use
//! for tests and headless runs; [`crate::session::Session`] wraps it for play.

use crate::cart::{Header, RomImage};
use crate::frame::{Frame, Palette, Screen};
use crate::input::Button;
use crate::saves::{ClockStamp, SaveData};
use gameboy_core::{Gameboy, Rtc, StepResult, RTC};
use std::cell::Cell;
use std::rc::Rc;

/// Frames per second of the real hardware (4194304 Hz / 70224 cycles).
pub const FRAME_RATE: f64 = 4_194_304.0 / 70_224.0;
/// Sample rate of the core's sound: it takes one stereo sample every 95 CPU
/// cycles, which is 44151 Hz rather than a round 44100. Playing it at the true
/// rate keeps sound and picture from drifting apart.
pub const SAMPLE_RATE: u32 = 4_194_304 / 95;

/// A frame is at most ~17.6k CPU steps. The core reports no frame end while a
/// game has the display switched off, so stop after about two frames' worth
/// rather than spin forever.
const MAX_STEPS_PER_FRAME: u32 = 40_000;

struct SystemClock;

impl RTC for SystemClock {
    fn get_current_time(&self) -> u64 {
        crate::paths::unix_now()
    }
}

pub struct Machine {
    gameboy: Gameboy,
    screen: Screen,
    header: Header,
    rom: Vec<u8>,
    ram_dirty: Rc<Cell<bool>>,
    frames: u64,
}

impl Machine {
    pub fn new(rom: &RomImage, palette: Palette) -> Result<Self, String> {
        rom.check_supported().map_err(|error| error.to_string())?;
        let ram_dirty = Rc::new(Cell::new(false));
        let gameboy = boot(&rom.bytes, &ram_dirty)?;
        Ok(Self {
            gameboy,
            screen: Screen::new(palette),
            header: rom.header.clone(),
            rom: rom.bytes.clone(),
            ram_dirty,
            frames: 0,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Frames emulated since power-on or the last reset.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Emulates one frame. `audio` receives interleaved stereo samples
    /// (left, right, ...) at [`SAMPLE_RATE`] whenever the core has a block ready.
    pub fn run_frame(&mut self, mut audio: impl FnMut(&[f32])) {
        for _ in 0..MAX_STEPS_PER_FRAME {
            match self.gameboy.emulate(&mut self.screen) {
                StepResult::VBlank => break,
                StepResult::AudioBufferFull => audio(self.gameboy.get_audio_buffer()),
                StepResult::Nothing => {}
            }
        }
        self.frames += 1;
    }

    pub fn press(&mut self, button: Button) {
        self.gameboy.press_button(button.into());
    }

    pub fn release(&mut self, button: Button) {
        self.gameboy.release_button(button.into());
    }

    /// The most recently finished picture.
    pub fn frame(&self) -> &Frame {
        &self.screen.frame
    }

    pub fn palette(&self) -> Palette {
        self.screen.palette()
    }

    /// Changes the four-shade colours; colour games ignore it.
    pub fn set_palette(&mut self, palette: Palette) {
        self.screen.set_palette(palette);
    }

    /// The cartridge RAM a save file would hold (empty when there is none).
    pub fn ram(&self) -> &[u8] {
        let ram = self.gameboy.get_cartridge().get_ram();
        &ram[..self.header.save_ram_bytes().min(ram.len())]
    }

    /// True once since the game last wrote to cartridge RAM; reading clears it.
    pub fn take_ram_dirty(&mut self) -> bool {
        self.ram_dirty.replace(false)
    }

    /// What to write to the save file, or `None` for a cartridge without a battery.
    pub fn save_data(&self) -> Option<SaveData> {
        if !self.header.saves() {
            return None;
        }
        let clock = self.header.clock.then(|| {
            let (registers, unix) = self.gameboy.get_cartridge().get_last_timestamp();
            ClockStamp {
                registers: registers.to_bytes(),
                unix,
            }
        });
        Some(SaveData {
            ram: self.ram().to_vec(),
            clock,
        })
    }

    /// Puts a save back into the cartridge. Call before the first frame.
    pub fn load_save(&mut self, data: &SaveData) {
        if !self.header.saves() {
            return;
        }
        let cartridge = self.gameboy.get_cartridge_mut();
        // The core sizes its buffer by mapper, not by header: keep that length.
        let mut ram = cartridge.get_ram().to_vec();
        let count = data.ram.len().min(ram.len());
        ram[..count].copy_from_slice(&data.ram[..count]);
        cartridge.set_ram(ram);
        if let (true, Some(clock)) = (self.header.clock, data.clock) {
            cartridge.set_last_timestamp(Rtc::from_bytes(&clock.registers), clock.unix);
        }
        self.ram_dirty.set(false);
    }

    /// A snapshot of the whole machine at this moment (a save state): the
    /// frame count, the picture on screen, then the core's own snapshot.
    /// Held buttons are not part of it.
    pub fn save_state(&self) -> Vec<u8> {
        let picture = self.screen.frame.to_rgb_bytes();
        let core = self.gameboy.save_state();
        let mut state = Vec::with_capacity(8 + picture.len() + core.len());
        state.extend_from_slice(&self.frames.to_le_bytes());
        state.extend_from_slice(&picture);
        state.extend_from_slice(&core);
        state
    }

    /// Puts a snapshot back. It must come from the same game; on any error
    /// the machine is left exactly as it was.
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), String> {
        let picture_bytes = self.screen.frame.pixels.len() * 3;
        if state.len() < 8 + picture_bytes {
            return Err("save state is damaged".into());
        }
        let (frames, rest) = state.split_at(8);
        let (picture, core) = rest.split_at(picture_bytes);
        self.gameboy
            .load_state(core)
            .map_err(|error| error.to_string())?;
        self.frames = u64::from_le_bytes(frames.try_into().unwrap());
        for (pixel, rgb) in self
            .screen
            .frame
            .pixels
            .iter_mut()
            .zip(picture.as_chunks::<3>().0)
        {
            *pixel = crate::frame::Rgb::new(rgb[0], rgb[1], rgb[2]);
        }
        // The cartridge RAM is now the snapshot's, not what the save file holds.
        self.ram_dirty.set(true);
        Ok(())
    }

    /// Power-cycles the console. The battery RAM survives, as on real hardware.
    pub fn reset(&mut self) -> Result<(), String> {
        let saved = self.save_data();
        self.gameboy = boot(&self.rom, &self.ram_dirty)?;
        self.frames = 0;
        if let Some(saved) = saved {
            let dirty = self.ram_dirty.get();
            self.load_save(&saved);
            self.ram_dirty.set(dirty);
        }
        Ok(())
    }
}

fn boot(rom: &[u8], ram_dirty: &Rc<Cell<bool>>) -> Result<Gameboy, String> {
    let mut gameboy = Gameboy::from_rom(rom.to_vec(), Box::new(SystemClock))?;
    let flag = Rc::clone(ram_dirty);
    gameboy.set_ram_change_callback(Box::new(move |_, _| flag.set(true)));
    Ok(gameboy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::testing;

    fn machine() -> Machine {
        let rom = RomImage::from_bytes("test.gb".into(), None, testing::rom()).unwrap();
        Machine::new(&rom, Palette::Grey).unwrap()
    }

    #[test]
    fn runs_frames_and_the_program_writes_cartridge_ram() {
        let mut machine = machine();
        assert!(!machine.take_ram_dirty());
        let mut samples = 0usize;
        for _ in 0..60 {
            machine.run_frame(|block| samples += block.len());
        }
        assert_eq!(machine.frames(), 60);
        assert_eq!(machine.ram().len(), 0x2000);
        assert_eq!(machine.ram()[0], 0x42);
        assert!(machine.take_ram_dirty());
        // About a second of stereo sound came out with it.
        assert!((70_000..110_000).contains(&samples), "{samples} samples");
    }

    #[test]
    fn buttons_reach_the_game() {
        let mut machine = machine();
        for _ in 0..5 {
            machine.run_frame(|_| {});
        }
        let idle = machine.ram()[1];
        machine.press(Button::A);
        for _ in 0..5 {
            machine.run_frame(|_| {});
        }
        let held = machine.ram()[1];
        machine.release(Button::A);
        for _ in 0..5 {
            machine.run_frame(|_| {});
        }
        assert_ne!(idle, held, "pressing A changed nothing");
        assert_eq!(machine.ram()[1], idle);
    }

    #[test]
    fn a_save_round_trips_and_survives_reset() {
        let mut first = machine();
        for _ in 0..5 {
            first.run_frame(|_| {});
        }
        let mut saved = first.save_data().unwrap();
        assert_eq!(saved.clock, None);
        saved.ram[0x100] = 0x99;

        let mut second = machine();
        second.load_save(&saved);
        assert_eq!(second.ram()[0x100], 0x99);
        second.reset().unwrap();
        assert_eq!(second.frames(), 0);
        assert_eq!(second.ram()[0x100], 0x99);
    }

    /// Runs `count` frames with a fixed button pattern and returns a checksum
    /// of every picture and of the sound.
    fn play(machine: &mut Machine, count: u64) -> (Vec<u32>, u64) {
        let mut pictures = Vec::new();
        let mut sound = 0u64;
        for step in 0..count {
            match step % 20 {
                3 => machine.press(Button::A),
                9 => machine.release(Button::A),
                12 => machine.press(Button::Left),
                17 => machine.release(Button::Left),
                _ => {}
            }
            machine.run_frame(|block| {
                for sample in block {
                    sound = sound
                        .wrapping_mul(31)
                        .wrapping_add(u64::from(sample.to_bits()));
                }
            });
            pictures.push(machine.frame().checksum());
        }
        (pictures, sound)
    }

    #[test]
    fn a_state_puts_the_machine_back_exactly() {
        let mut machine = machine();
        play(&mut machine, 40);
        let state = machine.save_state();
        let picture = machine.frame().clone();
        let ram = machine.ram().to_vec();

        let first = play(&mut machine, 60);
        assert_eq!(machine.frames(), 100);
        machine.take_ram_dirty();
        machine.load_state(&state).unwrap();
        assert_eq!(machine.frames(), 40);
        assert_eq!(machine.frame(), &picture);
        assert_eq!(machine.ram(), ram);
        assert!(machine.take_ram_dirty());
        assert_eq!(machine.save_state(), state);
        // The same input from the same state gives the same pictures and sound.
        assert_eq!(play(&mut machine, 60), first);

        // And it loads into a machine that was only just switched on.
        let mut fresh = self::machine();
        fresh.load_state(&state).unwrap();
        assert_eq!(play(&mut fresh, 60), first);
    }

    #[test]
    fn a_bad_state_is_refused_and_changes_nothing() {
        let mut machine = machine();
        play(&mut machine, 30);
        let state = machine.save_state();
        play(&mut machine, 7);
        let before = machine.save_state();

        let mut damaged = state.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0x40;
        let mut other_game = testing::rom();
        other_game[0x14D] ^= 0xFF; // a different header checksum: a different cartridge
        let other = RomImage::from_bytes("other.gb".into(), None, other_game).unwrap();
        let foreign = Machine::new(&other, Palette::Grey).unwrap().save_state();

        for (bad, why) in [
            (&damaged[..], "damaged"),
            (&state[..state.len() / 2], "damaged"),
            (&state[..5], "damaged"),
            (&foreign[..], "different game"),
        ] {
            let error = machine.load_state(bad).unwrap_err();
            assert!(error.contains(why), "{error}");
            assert_eq!(machine.save_state(), before);
        }
    }

    #[test]
    fn unsupported_cartridges_are_refused_with_a_reason() {
        let mut bytes = testing::rom();
        bytes[0x147] = 0x22;
        let rom = RomImage::from_bytes("x.gb".into(), None, bytes).unwrap();
        let error = Machine::new(&rom, Palette::Grey).err().unwrap();
        assert!(error.contains("MBC7"), "{error}");
    }
}
