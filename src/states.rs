//! Save-state slots.
//!
//! A save state is a snapshot of the whole machine, taken at any moment, kept
//! in one of [`SLOTS`] numbered slots per game. Each slot is one file,
//! `<states dir>/<rom id>/slot<N>.state`, holding when it was made, an 80x72
//! preview picture and the snapshot itself. Slots are filed under the ROM id,
//! so they follow a game when its file is renamed.
//!
//! This is separate from the battery save ([`crate::saves`]): that is the
//! game's own save file; this is the emulator freezing time.

use crate::frame::{Frame, Rgb};
use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};

/// Slots are numbered 1..=SLOTS.
pub const SLOTS: u8 = 4;
pub const THUMB_WIDTH: usize = 80;
pub const THUMB_HEIGHT: usize = 72;

const MAGIC: &[u8; 4] = b"VSLT";
const HEADER: usize = 4 + 8 + 8;
const THUMB_BYTES: usize = THUMB_WIDTH * THUMB_HEIGHT * 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SlotInfo {
    pub slot: u8,
    /// Unix time the state was made.
    pub saved_at: u64,
    /// Frames the game had run when it was made.
    pub frames: u64,
}

#[derive(Debug, Clone)]
pub struct Slot {
    pub info: SlotInfo,
    pub thumbnail: Frame,
    /// The machine snapshot, for [`crate::machine::Machine::load_state`].
    pub state: Vec<u8>,
}

fn check(slot: u8) -> io::Result<()> {
    if (1..=SLOTS).contains(&slot) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("there is no slot {slot}: slots are 1 to {SLOTS}"),
        ))
    }
}

pub fn path(states_dir: &Path, rom_id: &str, slot: u8) -> PathBuf {
    states_dir.join(rom_id).join(format!("slot{slot}.state"))
}

/// The picture at half size (each preview pixel is the average of four).
pub fn thumbnail(frame: &Frame) -> Frame {
    let mut small = Frame::new(frame.width / 2, frame.height / 2);
    for y in 0..small.height {
        for x in 0..small.width {
            let mut sum = [0u32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let pixel = frame.get_pixel(x * 2 + dx, y * 2 + dy);
                sum[0] += u32::from(pixel.r);
                sum[1] += u32::from(pixel.g);
                sum[2] += u32::from(pixel.b);
            }
            let average = Rgb::new((sum[0] / 4) as u8, (sum[1] / 4) as u8, (sum[2] / 4) as u8);
            small.set_pixel(y * small.width + x, average);
        }
    }
    small
}

/// Writes a slot, replacing what was in it. `frame` is the full picture at
/// the moment of the snapshot.
pub fn write(
    states_dir: &Path,
    rom_id: &str,
    slot: u8,
    frame: &Frame,
    frames: u64,
    state: &[u8],
) -> io::Result<SlotInfo> {
    check(slot)?;
    let info = SlotInfo {
        slot,
        saved_at: crate::paths::unix_now(),
        frames,
    };
    let mut bytes = Vec::with_capacity(HEADER + THUMB_BYTES + state.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&info.saved_at.to_le_bytes());
    bytes.extend_from_slice(&info.frames.to_le_bytes());
    let small = thumbnail(frame);
    if small.width != THUMB_WIDTH || small.height != THUMB_HEIGHT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the picture is not 160x144",
        ));
    }
    bytes.extend_from_slice(&small.to_rgb_bytes());
    bytes.extend_from_slice(state);
    crate::paths::write_atomic(&path(states_dir, rom_id, slot), &bytes)?;
    Ok(info)
}

fn parse_info(slot: u8, bytes: &[u8]) -> Option<SlotInfo> {
    if bytes.len() < HEADER || &bytes[..4] != MAGIC {
        return None;
    }
    let number = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    Some(SlotInfo {
        slot,
        saved_at: number(4),
        frames: number(12),
    })
}

/// Reads a slot. `Ok(None)` is an empty slot; a file that is not a slot file
/// is an error.
pub fn read(states_dir: &Path, rom_id: &str, slot: u8) -> io::Result<Option<Slot>> {
    check(slot)?;
    let bytes = match std::fs::read(path(states_dir, rom_id, slot)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let damaged = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("slot {slot} is damaged"),
        )
    };
    let info = parse_info(slot, &bytes).ok_or_else(damaged)?;
    if bytes.len() < HEADER + THUMB_BYTES {
        return Err(damaged());
    }
    let mut thumbnail = Frame::new(THUMB_WIDTH, THUMB_HEIGHT);
    for (pixel, rgb) in thumbnail
        .pixels
        .iter_mut()
        .zip(bytes[HEADER..HEADER + THUMB_BYTES].as_chunks::<3>().0)
    {
        *pixel = Rgb::new(rgb[0], rgb[1], rgb[2]);
    }
    Ok(Some(Slot {
        info,
        thumbnail,
        state: bytes[HEADER + THUMB_BYTES..].to_vec(),
    }))
}

/// What is in each slot, index 0 being slot 1. Reads only the file headers.
pub fn list(states_dir: &Path, rom_id: &str) -> [Option<SlotInfo>; SLOTS as usize] {
    let mut slots = [None; SLOTS as usize];
    for slot in 1..=SLOTS {
        use std::io::Read;
        let mut header = [0u8; HEADER];
        let read = std::fs::File::open(path(states_dir, rom_id, slot))
            .and_then(|mut file| file.read_exact(&mut header));
        if read.is_ok() {
            slots[usize::from(slot) - 1] = parse_info(slot, &header);
        }
    }
    slots
}

/// The most recently written slot, for "resume".
pub fn latest(states_dir: &Path, rom_id: &str) -> Option<SlotInfo> {
    list(states_dir, rom_id)
        .into_iter()
        .flatten()
        .max_by_key(|info| info.saved_at)
}

/// Empties a slot; false when it was already empty.
pub fn clear(states_dir: &Path, rom_id: &str, slot: u8) -> io::Result<bool> {
    check(slot)?;
    match std::fs::remove_file(path(states_dir, rom_id, slot)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> Frame {
        let mut frame = Frame::default();
        // Left half white, right half black.
        for y in 0..frame.height {
            for x in 0..frame.width / 2 {
                frame.set_pixel(y * frame.width + x, Rgb::new(255, 255, 255));
            }
        }
        frame
    }

    #[test]
    fn thumbnail_is_half_size_and_averaged() {
        let mut frame = picture();
        frame.set_pixel(0, Rgb::new(55, 55, 55));
        let small = thumbnail(&frame);
        assert_eq!((small.width, small.height), (THUMB_WIDTH, THUMB_HEIGHT));
        assert_eq!(small.get_pixel(0, 0), Rgb::new(205, 205, 205));
        assert_eq!(small.get_pixel(39, 10), Rgb::new(255, 255, 255));
        assert_eq!(small.get_pixel(40, 10), Rgb::new(0, 0, 0));
    }

    #[test]
    fn slots_write_list_read_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let id = "abcd0123";
        assert_eq!(list(dir.path(), id), [None; 4]);
        assert!(read(dir.path(), id, 2).unwrap().is_none());
        assert_eq!(latest(dir.path(), id), None);

        let info = write(dir.path(), id, 2, &picture(), 777, b"snapshot bytes").unwrap();
        assert_eq!((info.slot, info.frames), (2, 777));
        let listed = list(dir.path(), id);
        assert_eq!(listed[1], Some(info));
        assert!(listed[0].is_none() && listed[2].is_none() && listed[3].is_none());
        assert_eq!(latest(dir.path(), id), Some(info));
        // Another game's slots are separate.
        assert_eq!(list(dir.path(), "ffffffff"), [None; 4]);

        let slot = read(dir.path(), id, 2).unwrap().unwrap();
        assert_eq!(slot.info, info);
        assert_eq!(slot.state, b"snapshot bytes");
        assert_eq!(slot.thumbnail, thumbnail(&picture()));

        assert!(clear(dir.path(), id, 2).unwrap());
        assert!(!clear(dir.path(), id, 2).unwrap());
        assert_eq!(list(dir.path(), id), [None; 4]);
    }

    #[test]
    fn bad_slot_numbers_and_damaged_files_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        for slot in [0, 5] {
            assert!(write(dir.path(), "id", slot, &picture(), 0, b"").is_err());
            assert!(read(dir.path(), "id", slot).is_err());
            assert!(clear(dir.path(), "id", slot).is_err());
        }
        let file = path(dir.path(), "id", 1);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"VSLT too short").unwrap();
        assert!(read(dir.path(), "id", 1).is_err());
        assert_eq!(list(dir.path(), "id")[0], None);
    }
}
