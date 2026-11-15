//! Battery saves.
//!
//! A save is the cartridge RAM as a plain `.sav`, the format other emulators
//! read. Cartridges with a real-time clock get the usual 48-byte clock footer
//! after the RAM. Saves live in one folder (`<app dir>/saves/`), named after
//! the ROM file; the original app's `.saves/` folder beside the ROM is still
//! read, so old saves carry over.

use std::io;
use std::path::{Path, PathBuf};

const CLOCK_FOOTER: usize = 48;

/// The cartridge's clock registers and the moment they were read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockStamp {
    /// seconds, minutes, hours, day low, day high + flags
    pub registers: [u8; 5],
    pub unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveData {
    pub ram: Vec<u8>,
    pub clock: Option<ClockStamp>,
}

pub fn encode(data: &SaveData) -> Vec<u8> {
    let mut bytes = data.ram.clone();
    if let Some(clock) = data.clock {
        // Current registers, then the latched copy, each as a 32-bit word.
        for _ in 0..2 {
            for register in clock.registers {
                bytes.extend_from_slice(&u32::from(register).to_le_bytes());
            }
        }
        bytes.extend_from_slice(&clock.unix.to_le_bytes());
    }
    bytes
}

/// Reads a save for a cartridge with `ram_len` bytes of RAM. A file that is
/// shorter is padded with `0xFF`; a longer one (the original app wrote the
/// core's whole 32 KiB buffer) is cut to size.
pub fn decode(bytes: &[u8], ram_len: usize, has_clock: bool) -> SaveData {
    let extra = bytes.len().saturating_sub(ram_len);
    // 44 bytes is the same footer with a 32-bit timestamp.
    let clock = if has_clock && (extra == CLOCK_FOOTER || extra == 44) {
        let footer = &bytes[ram_len..];
        let mut registers = [0u8; 5];
        for (index, register) in registers.iter_mut().enumerate() {
            *register = footer[index * 4];
        }
        let mut stamp = [0u8; 8];
        stamp[..extra - 40].copy_from_slice(&footer[40..]);
        Some(ClockStamp {
            registers,
            unix: u64::from_le_bytes(stamp),
        })
    } else {
        None
    };
    let mut ram = bytes[..ram_len.min(bytes.len())].to_vec();
    ram.resize(ram_len, 0xFF);
    SaveData { ram, clock }
}

/// `<saves dir>/<rom file name without extension>.sav`
pub fn save_path(saves_dir: &Path, rom_path: &Path) -> PathBuf {
    saves_dir.join(format!("{}.sav", crate::cart::stem(rom_path)))
}

/// Where the original app kept the save: `.saves/` beside the ROM.
pub fn legacy_path(rom_path: &Path) -> PathBuf {
    rom_path
        .parent()
        .unwrap_or(Path::new(""))
        .join(".saves")
        .join(format!("{}.sav", crate::cart::stem(rom_path)))
}

/// The save file that would be loaded for this ROM, if any exists.
pub fn find(saves_dir: &Path, rom_path: &Path) -> Option<PathBuf> {
    [save_path(saves_dir, rom_path), legacy_path(rom_path)]
        .into_iter()
        .find(|path| path.is_file())
}

pub fn read(
    saves_dir: &Path,
    rom_path: &Path,
    ram_len: usize,
    has_clock: bool,
) -> io::Result<Option<SaveData>> {
    match find(saves_dir, rom_path) {
        Some(path) => Ok(Some(decode(&std::fs::read(path)?, ram_len, has_clock))),
        None => Ok(None),
    }
}

/// Writes the save, keeping the previous one as `<name>.sav.bak`.
pub fn write(path: &Path, data: &SaveData) -> io::Result<()> {
    if path.is_file() {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".bak");
        // A failed backup must not stop the save itself.
        let _ = std::fs::copy(path, PathBuf::from(backup));
    }
    crate::paths::write_atomic(path, &encode(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> ClockStamp {
        ClockStamp {
            registers: [59, 58, 23, 200, 1],
            unix: 1_800_000_000,
        }
    }

    #[test]
    fn ram_and_clock_round_trip() {
        let data = SaveData {
            ram: (0..=255).collect(),
            clock: Some(clock()),
        };
        let bytes = encode(&data);
        assert_eq!(bytes.len(), 256 + 48);
        assert_eq!(decode(&bytes, 256, true), data);
        // Without a clock on the cartridge the footer is not looked for.
        assert_eq!(decode(&bytes, 256, false).clock, None);
    }

    #[test]
    fn clock_only_cartridges_save_just_the_footer() {
        let data = SaveData {
            ram: Vec::new(),
            clock: Some(clock()),
        };
        assert_eq!(decode(&encode(&data), 0, true), data);
    }

    #[test]
    fn odd_sized_files_are_fitted_not_trusted() {
        // The original app's 32 KiB dump for an 8 KiB cartridge.
        let mut old = vec![0xFFu8; 0x8000];
        old[0] = 7;
        let fitted = decode(&old, 0x2000, true);
        assert_eq!(fitted.ram.len(), 0x2000);
        assert_eq!(fitted.ram[0], 7);
        assert_eq!(fitted.clock, None);

        let short = decode(&[1, 2, 3], 8, false);
        assert_eq!(short.ram, [1, 2, 3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn write_keeps_a_backup_and_read_prefers_the_saves_folder() {
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("roms").join("Game.zip");
        let saves = dir.path().join("saves");
        assert_eq!(read(&saves, &rom, 4, false).unwrap(), None);

        // Only the old location exists: it is used.
        let legacy = legacy_path(&rom);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, [9, 9, 9, 9]).unwrap();
        assert_eq!(read(&saves, &rom, 4, false).unwrap().unwrap().ram, [9; 4]);

        let path = save_path(&saves, &rom);
        write(
            &path,
            &SaveData {
                ram: vec![1; 4],
                clock: None,
            },
        )
        .unwrap();
        write(
            &path,
            &SaveData {
                ram: vec![2; 4],
                clock: None,
            },
        )
        .unwrap();
        assert_eq!(read(&saves, &rom, 4, false).unwrap().unwrap().ram, [2; 4]);
        assert_eq!(std::fs::read(saves.join("Game.sav.bak")).unwrap(), [1; 4]);
        // The old file is left alone.
        assert_eq!(std::fs::read(&legacy).unwrap(), [9; 4]);
    }
}
