//! ROM files and the cartridge header.
//!
//! A ROM is a `.gb` / `.gbc` file, or a `.zip` holding one. The header is
//! checked here, before the emulator core sees the bytes, because the core
//! indexes into the ROM without checking and would panic on a short or odd file.

use serde::Serialize;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest ROM a Game Boy cartridge can hold (MBC5, 8 MiB).
pub const MAX_ROM_BYTES: u64 = 8 * 1024 * 1024;
const HEADER_END: usize = 0x150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartError {
    Io(String),
    /// Not a Game Boy ROM, or too damaged to read.
    Invalid(String),
    /// A real cartridge, but one the emulator core cannot run.
    Unsupported(String),
}

impl fmt::Display for CartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartError::Io(text) | CartError::Invalid(text) | CartError::Unsupported(text) => {
                f.write_str(text)
            }
        }
    }
}

impl std::error::Error for CartError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    /// Original Game Boy game: four shades, drawn with the chosen palette.
    Mono,
    /// Runs on both; in colour here.
    Enhanced,
    /// Game Boy Color only.
    Only,
}

/// The fields of the cartridge header a front end may want to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Header {
    pub title: String,
    pub color: Color,
    pub super_gameboy: bool,
    /// Raw cartridge type byte (0x147).
    pub cart_type: u8,
    /// Mapper chip name, e.g. `MBC3`.
    pub mapper: &'static str,
    pub battery: bool,
    pub clock: bool,
    pub rumble: bool,
    /// ROM size the header declares, in bytes.
    pub rom_bytes: usize,
    /// Cartridge RAM size the header declares, in bytes.
    pub ram_bytes: usize,
    pub version: u8,
    pub header_checksum_ok: bool,
    pub global_checksum: u16,
    /// `None` when the core can run it, otherwise the reason it cannot.
    pub unsupported: Option<String>,
}

impl Header {
    /// Bytes of cartridge RAM worth keeping in a save file.
    pub fn save_ram_bytes(&self) -> usize {
        if !self.battery {
            0
        } else if self.mapper == "MBC2" {
            // Built into the chip: 512 half-bytes, no size in the header.
            0x200
        } else {
            self.ram_bytes
        }
    }

    /// True when there is something to write to a save file at all.
    pub fn saves(&self) -> bool {
        self.battery && (self.save_ram_bytes() > 0 || self.clock)
    }
}

/// `(mapper, battery, clock, rumble)` for a cartridge type byte.
fn cart_type(byte: u8) -> Option<(&'static str, bool, bool, bool)> {
    Some(match byte {
        0x00 => ("ROM", false, false, false),
        0x01 | 0x02 => ("MBC1", false, false, false),
        0x03 => ("MBC1", true, false, false),
        0x05 => ("MBC2", false, false, false),
        0x06 => ("MBC2", true, false, false),
        0x08 => ("ROM", false, false, false),
        0x09 => ("ROM", true, false, false),
        0x0B | 0x0C => ("MMM01", false, false, false),
        0x0D => ("MMM01", true, false, false),
        0x0F | 0x10 => ("MBC3", true, true, false),
        0x11 | 0x12 => ("MBC3", false, false, false),
        0x13 => ("MBC3", true, false, false),
        0x19 | 0x1A => ("MBC5", false, false, false),
        0x1B => ("MBC5", true, false, false),
        0x1C | 0x1D => ("MBC5", false, false, true),
        0x1E => ("MBC5", true, false, true),
        0x20 => ("MBC6", false, false, false),
        0x22 => ("MBC7", true, false, true),
        0xFC => ("Pocket Camera", false, false, false),
        0xFD => ("TAMA5", true, false, false),
        0xFE => ("HuC3", false, false, false),
        0xFF => ("HuC1", true, false, false),
        _ => return None,
    })
}

/// Cartridge types `gameboy_core` 0.3 has a mapper for.
fn core_runs(byte: u8) -> bool {
    matches!(
        byte,
        0x00 | 0x08 | 0x09 | 0x01..=0x03 | 0x05 | 0x06 | 0x0F..=0x13 | 0x19..=0x1E
    )
}

pub fn parse_header(rom: &[u8]) -> Result<Header, CartError> {
    if rom.len() < HEADER_END {
        return Err(CartError::Invalid(format!(
            "too small to be a Game Boy ROM ({} bytes)",
            rom.len()
        )));
    }
    let type_byte = rom[0x147];
    let Some((mapper, battery, clock, rumble)) = cart_type(type_byte) else {
        return Err(CartError::Invalid(format!(
            "unknown cartridge type 0x{type_byte:02X}; this does not look like a Game Boy ROM"
        )));
    };
    let rom_code = rom[0x148];
    if rom_code > 8 {
        return Err(CartError::Invalid(format!(
            "unknown ROM size code 0x{rom_code:02X}"
        )));
    }
    let ram_code = rom[0x149];
    let ram_bytes = match ram_code {
        0 => 0,
        1 => 2 * 1024,
        2 => 8 * 1024,
        3 => 32 * 1024,
        4 => 128 * 1024,
        5 => 64 * 1024,
        _ => {
            return Err(CartError::Invalid(format!(
                "unknown RAM size code 0x{ram_code:02X}"
            )))
        }
    };

    let color = match rom[0x143] {
        0x80 => Color::Enhanced,
        0xC0 => Color::Only,
        _ => Color::Mono,
    };
    // Colour-era headers give the last title byte to the colour flag, and later
    // ones also the four before it to a maker code (capitals and digits).
    let maker_code = rom[0x13F..0x143]
        .iter()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
    let title_end = match color {
        Color::Mono => 0x144,
        _ if maker_code => 0x13F,
        _ => 0x143,
    };
    let title: String = rom[0x134..title_end]
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| {
            if (0x20..0x7F).contains(&byte) {
                byte as char
            } else {
                ' '
            }
        })
        .collect();

    let computed = rom[0x134..=0x14C]
        .iter()
        .fold(0u8, |sum, &byte| sum.wrapping_sub(byte).wrapping_sub(1));

    let unsupported = if !core_runs(type_byte) {
        Some(format!(
            "{mapper} cartridges are not supported by the emulator core"
        ))
    } else if ram_code > 4 {
        Some("64 KiB cartridge RAM is not supported by the emulator core".to_string())
    } else {
        None
    };

    Ok(Header {
        title: title.trim().to_string(),
        color,
        super_gameboy: rom[0x146] == 0x03,
        cart_type: type_byte,
        mapper,
        battery,
        clock,
        rumble,
        rom_bytes: 0x8000usize << rom_code,
        ram_bytes,
        version: rom[0x14C],
        header_checksum_ok: computed == rom[0x14D],
        global_checksum: u16::from_be_bytes([rom[0x14E], rom[0x14F]]),
        unsupported,
    })
}

/// A ROM read into memory, checked and ready for [`crate::machine::Machine`].
#[derive(Debug, Clone)]
pub struct RomImage {
    /// The file on disk: the ROM itself, or the zip it came from.
    pub path: PathBuf,
    /// Name of the ROM inside the zip, when `path` is a zip.
    pub member: Option<String>,
    pub bytes: Vec<u8>,
    pub header: Header,
    /// CRC-32 of the ROM bytes: the stable id used for play statistics.
    pub crc32: u32,
}

impl RomImage {
    pub fn from_bytes(
        path: PathBuf,
        member: Option<String>,
        mut bytes: Vec<u8>,
    ) -> Result<Self, CartError> {
        let header = parse_header(&bytes)?;
        let crc32 = crc32fast::hash(&bytes);
        // The core indexes whole 16 KiB banks: pad a short dump rather than let it panic.
        let wanted = header.rom_bytes.max(0x8000);
        if bytes.len() < wanted {
            bytes.resize(wanted, 0xFF);
        }
        Ok(Self {
            path,
            member,
            bytes,
            header,
            crc32,
        })
    }

    pub fn id(&self) -> String {
        format!("{:08x}", self.crc32)
    }

    /// File name without folder or extension: what a save file is named after.
    pub fn stem(&self) -> String {
        stem(&self.path)
    }

    /// Fails with the reason when the core cannot run this cartridge.
    pub fn check_supported(&self) -> Result<(), CartError> {
        match &self.header.unsupported {
            Some(reason) => Err(CartError::Unsupported(reason.clone())),
            None => Ok(()),
        }
    }
}

pub fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rom".to_string())
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_rom_name(path: &Path) -> bool {
    matches!(extension(path).as_str(), "gb" | "gbc")
}

pub fn is_zip_name(path: &Path) -> bool {
    extension(path) == "zip"
}

/// Reads a `.gb`, `.gbc` or `.zip` (the first ROM inside) and checks its header.
pub fn load(path: &Path) -> Result<RomImage, CartError> {
    if is_zip_name(path) {
        let (member, bytes) = read_zip(path)?;
        RomImage::from_bytes(path.to_path_buf(), Some(member), bytes)
    } else {
        let size = std::fs::metadata(path)
            .map_err(|error| CartError::Io(format!("cannot read {}: {error}", path.display())))?
            .len();
        if size > MAX_ROM_BYTES {
            return Err(CartError::Invalid(format!(
                "{} is {size} bytes, larger than any Game Boy cartridge",
                path.display()
            )));
        }
        let bytes = std::fs::read(path)
            .map_err(|error| CartError::Io(format!("cannot read {}: {error}", path.display())))?;
        RomImage::from_bytes(path.to_path_buf(), None, bytes)
    }
}

fn read_zip(path: &Path) -> Result<(String, Vec<u8>), CartError> {
    let io = |error: &dyn fmt::Display| {
        CartError::Io(format!("cannot read {}: {error}", path.display()))
    };
    let file = std::fs::File::open(path).map_err(|error| io(&error))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| io(&error))?;
    let index = (0..archive.len())
        .find(|&index| {
            archive
                .by_index(index)
                .map(|entry| {
                    entry.is_file()
                        && entry
                            .name()
                            .is_ok_and(|name| is_rom_name(Path::new(&*name)))
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            CartError::Invalid(format!("no .gb or .gbc file inside {}", path.display()))
        })?;
    let entry = archive.by_index(index).map_err(|error| io(&error))?;
    let name = entry.name().map_err(|error| io(&error))?.into_owned();
    if entry.size() > MAX_ROM_BYTES {
        return Err(CartError::Invalid(format!(
            "{name} in {} is larger than any Game Boy cartridge",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    // `take` guards against a zip whose declared size understates the data.
    entry
        .take(MAX_ROM_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(&error))?;
    if bytes.len() as u64 > MAX_ROM_BYTES {
        return Err(CartError::Invalid(format!(
            "{name} is larger than any Game Boy cartridge"
        )));
    }
    Ok((name, bytes))
}

/// A tiny home-made ROM for tests: no game data, nothing copyrighted.
#[doc(hidden)]
pub mod testing {
    /// 32 KiB, MBC1 + RAM + battery, 8 KiB of cartridge RAM. On start it turns
    /// cartridge RAM on, writes `0x42` to its first byte, then copies the
    /// joypad's button nibble to the second byte forever.
    pub fn rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]); // NOP; JP 0x0150
        rom[0x134..0x13E].copy_from_slice(b"VOSTOKTEST");
        rom[0x147] = 0x03;
        rom[0x148] = 0x00;
        rom[0x149] = 0x02;
        #[rustfmt::skip]
        let program: &[u8] = &[
            0x3E, 0x0A,             // LD A,0x0A
            0xEA, 0x00, 0x00,       // LD (0x0000),A   cartridge RAM on
            0x3E, 0x42,             // LD A,0x42
            0xEA, 0x00, 0xA0,       // LD (0xA000),A
            0x3E, 0x10,             // LD A,0x10       select the action buttons
            0xE0, 0x00,             // LDH (0x00),A
            // loop:
            0xF0, 0x00,             // LDH A,(0x00)
            0xE6, 0x0F,             // AND 0x0F
            0xEA, 0x01, 0xA0,       // LD (0xA001),A
            0x18, 0xF7,             // JR loop
        ];
        rom[0x150..0x150 + program.len()].copy_from_slice(program);
        rom[0x14D] = rom[0x134..=0x14C]
            .iter()
            .fold(0u8, |sum, &byte| sum.wrapping_sub(byte).wrapping_sub(1));
        rom
    }

    /// Writes `bytes` as `name` inside a new zip at `path`.
    pub fn write_zip(path: &std::path::Path, name: &str, bytes: &[u8]) {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_test_header() {
        let header = parse_header(&testing::rom()).unwrap();
        assert_eq!(header.title, "VOSTOKTEST");
        assert_eq!(header.color, Color::Mono);
        assert_eq!(header.mapper, "MBC1");
        assert!(header.battery && !header.clock);
        assert_eq!(header.rom_bytes, 0x8000);
        assert_eq!(header.ram_bytes, 0x2000);
        assert_eq!(header.save_ram_bytes(), 0x2000);
        assert!(header.header_checksum_ok);
        assert_eq!(header.unsupported, None);
    }

    #[test]
    fn rejects_files_that_are_not_roms() {
        assert!(matches!(parse_header(b"hello"), Err(CartError::Invalid(_))));
        let mut rom = testing::rom();
        rom[0x147] = 0x77;
        assert!(matches!(parse_header(&rom), Err(CartError::Invalid(_))));
        rom[0x147] = 0x03;
        rom[0x149] = 0x09;
        assert!(matches!(parse_header(&rom), Err(CartError::Invalid(_))));
    }

    #[test]
    fn flags_cartridges_the_core_cannot_run() {
        let mut rom = testing::rom();
        rom[0x147] = 0x22; // MBC7
        let image = RomImage::from_bytes("x.gb".into(), None, rom).unwrap();
        assert!(matches!(
            image.check_supported(),
            Err(CartError::Unsupported(_))
        ));
    }

    #[test]
    fn colour_title_stops_before_the_maker_code() {
        let mut rom = testing::rom();
        rom[0x134..0x144].copy_from_slice(b"POKEMON_SLVAAXE\x80");
        let header = parse_header(&rom).unwrap();
        assert_eq!(header.title, "POKEMON_SLV");
        assert_eq!(header.color, Color::Enhanced);
        // An early colour game: a long title and no maker code.
        rom[0x134..0x144].copy_from_slice(b"POKEMON YELLOW\0\x80");
        assert_eq!(parse_header(&rom).unwrap().title, "POKEMON YELLOW");
    }

    #[test]
    fn short_dumps_are_padded_to_the_declared_size() {
        let mut rom = testing::rom();
        rom.truncate(0x4000);
        let image = RomImage::from_bytes("x.gb".into(), None, rom).unwrap();
        assert_eq!(image.bytes.len(), 0x8000);
    }

    #[test]
    fn loads_plain_files_and_zips_to_the_same_rom() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("Test Game.gb");
        std::fs::write(&plain, testing::rom()).unwrap();
        let zipped = dir.path().join("Test Game.zip");
        testing::write_zip(&zipped, "inner name.GB", &testing::rom());

        let a = load(&plain).unwrap();
        let b = load(&zipped).unwrap();
        assert_eq!(a.crc32, b.crc32);
        assert_eq!(a.member, None);
        assert_eq!(b.member.as_deref(), Some("inner name.GB"));
        assert_eq!(b.stem(), "Test Game");
    }

    #[test]
    fn a_zip_without_a_rom_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let zipped = dir.path().join("notes.zip");
        testing::write_zip(&zipped, "readme.txt", b"nothing here");
        assert!(matches!(load(&zipped), Err(CartError::Invalid(_))));
        assert!(matches!(
            load(&dir.path().join("missing.gb")),
            Err(CartError::Io(_))
        ));
    }
}
