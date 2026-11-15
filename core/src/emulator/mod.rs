pub mod step_result;
pub mod traits;

use self::step_result::StepResult;
use self::traits::{PixelMapper, RTC};
use crate::cpu::Cpu;
use crate::gpu::GPU;
use crate::joypad::Controller;
use crate::mmu::cartridge::Cartridge;
use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;
use crate::state::{self, StateError, StateResult, Stateful, Writer};
use crate::timer::Timer;

pub struct Emulator {
    cpu: Cpu,
    gpu: GPU,
    timer: Timer,
    memory: Memory,
    /// The frame ended on the same step a sound block filled: report the
    /// frame on the next call instead of losing it.
    pending_vblank: bool,
}

impl Emulator {
    pub fn from_cartridge(cartridge: Cartridge, rtc: Box<dyn RTC>) -> Emulator {
        let is_cgb = cartridge.is_cgb();
        Emulator {
            cpu: Cpu::new(is_cgb),
            gpu: GPU::new(is_cgb),
            timer: Timer::new(),
            memory: Memory::from_cartridge(cartridge, rtc, is_cgb),
            pending_vblank: false,
        }
    }

    pub fn emulate(
        &mut self,
        system: &mut impl PixelMapper,
        controller: &mut Controller,
    ) -> StepResult {
        if self.pending_vblank {
            self.pending_vblank = false;
            return StepResult::VBlank;
        }
        let cycles = self.cpu.step(&mut self.memory);
        self.timer.update(cycles, &mut self.memory);
        let audio_buffer_full = self.memory.get_sound_mut().step(cycles);
        let vblank = self.gpu.step(cycles, &mut self.memory, system);
        controller.update(&mut self.memory);
        self.handle_interrupts();

        if audio_buffer_full {
            self.pending_vblank = vblank;
            StepResult::AudioBufferFull
        } else if vblank {
            StepResult::VBlank
        } else {
            StepResult::Nothing
        }
    }

    fn handle_interrupts(&mut self) {
        if let Some(interrupt) = self.memory.get_interrupts() {
            self.process_interrupt(interrupt);
        }
    }
    fn process_interrupt(&mut self, interrupt: Interrupt) {
        if self.cpu.are_interrupts_enabled() {
            self.cpu.disable_interrupts();
            match interrupt {
                Interrupt::Vblank => self.cpu.rst_40(&mut self.memory),
                Interrupt::Lcd => self.cpu.rst_48(&mut self.memory),
                Interrupt::Timer => self.cpu.rst_50(&mut self.memory),
                Interrupt::Serial => self.cpu.rst_58(&mut self.memory),
                Interrupt::Joypad => self.cpu.rst_60(&mut self.memory),
            }
            self.memory.remove_interrupt(interrupt);
        }
        self.cpu.unhalt();
    }

    pub fn save_state(&self) -> Vec<u8> {
        let mut w = Writer::default();
        self.identity().save(&mut w);
        Stateful::save(self, &mut w);
        state::seal(w)
    }

    pub fn load_state(&mut self, data: &[u8]) -> StateResult {
        let mut r = state::open(data)?;
        let mut identity = [0u8; 6];
        identity.load(&mut r)?;
        if identity != self.identity() {
            return Err(StateError::WrongGame);
        }
        // Fields are overwritten as they are read, so keep a way back.
        let before = self.save_state();
        let result = Stateful::load(self, &mut r).and_then(|_| state::finished(&r));
        if result.is_err() {
            let mut undo = state::open(&before).expect("own snapshot");
            identity.load(&mut undo).expect("own snapshot");
            Stateful::load(self, &mut undo).expect("own snapshot");
        }
        result
    }

    /// Enough of the cartridge header to tell games apart: colour mode,
    /// cartridge type, RAM size, header checksum and global checksum.
    fn identity(&self) -> [u8; 6] {
        let rom = self.memory.get_cartridge().get_rom();
        [
            self.memory.get_cartridge().is_cgb() as u8,
            rom[0x147],
            rom[0x149],
            rom[0x14D],
            rom[0x14E],
            rom[0x14F],
        ]
    }

    pub fn get_cartridge(&self) -> &Cartridge {
        &self.memory.get_cartridge()
    }

    pub fn set_ram_change_callback(&mut self, f: Box<dyn FnMut(usize, u8)>) {
        self.memory.set_ram_change_callback(f);
    }

    pub fn get_cartridge_mut(&mut self) -> &mut Cartridge {
        self.memory.get_cartridge_mut()
    }

    pub fn get_audio_buffer(&self) -> &[f32] {
        self.memory.get_sound().get_audio_buffer()
    }
}

state_fields!(Emulator { cpu, gpu, memory, pending_vblank });
