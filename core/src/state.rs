//! Machine snapshots (save states). Added in the Vostok Gameboy copy of this crate.
//!
//! A snapshot is every piece of state that changes while a game runs, written
//! field by field into a byte buffer. It does not hold the ROM: a snapshot is
//! loaded into a machine that already has the same cartridge in it, and the
//! header says which cartridge that must be.
//!
//! Layout: `VGBS`, format version (u16), a 64-bit hash of the body, then the
//! body. All integers are little-endian.

use std::convert::TryFrom;
use std::fmt;

pub const MAGIC: &[u8; 4] = b"VGBS";
/// Bump when the body layout changes; older files are then refused, not misread.
pub const VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// Not a snapshot file at all.
    NotAState,
    /// Written by a different version of this format.
    Version(u16),
    /// Damaged or cut short.
    Corrupt,
    /// Made with a different game in the cartridge slot.
    WrongGame,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::NotAState => f.write_str("not a save state"),
            StateError::Version(found) => write!(
                f,
                "save state is format {} and this build reads format {}",
                found, VERSION
            ),
            StateError::Corrupt => f.write_str("save state is damaged"),
            StateError::WrongGame => f.write_str("save state is from a different game"),
        }
    }
}

impl std::error::Error for StateError {}

pub type StateResult = Result<(), StateError>;

#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn put(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn take(&mut self, count: usize) -> Result<&'a [u8], StateError> {
        if self.bytes.len() < count {
            return Err(StateError::Corrupt);
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }
}

/// State that can be written to and read back from a snapshot. `load`
/// overwrites `self` in place, so sizes fixed at power-on (RAM lengths) stay
/// what the cartridge says they are.
pub trait Stateful {
    fn save(&self, w: &mut Writer);
    fn load(&mut self, r: &mut Reader) -> StateResult;
}

macro_rules! stateful_int {
    ($($ty:ty),*) => {$(
        impl Stateful for $ty {
            fn save(&self, w: &mut Writer) {
                w.put(&self.to_le_bytes());
            }
            fn load(&mut self, r: &mut Reader) -> StateResult {
                let mut raw = [0u8; std::mem::size_of::<$ty>()];
                raw.copy_from_slice(r.take(std::mem::size_of::<$ty>())?);
                *self = <$ty>::from_le_bytes(raw);
                Ok(())
            }
        }
    )*};
}

stateful_int!(u8, u16, u32, u64, i32);

impl Stateful for bool {
    fn save(&self, w: &mut Writer) {
        w.put(&[*self as u8]);
    }
    fn load(&mut self, r: &mut Reader) -> StateResult {
        *self = match r.take(1)?[0] {
            0 => false,
            1 => true,
            _ => return Err(StateError::Corrupt),
        };
        Ok(())
    }
}

/// Stored as 64 bits so a snapshot reads the same on any machine.
impl Stateful for usize {
    fn save(&self, w: &mut Writer) {
        (*self as u64).save(w);
    }
    fn load(&mut self, r: &mut Reader) -> StateResult {
        let mut wide = 0u64;
        wide.load(r)?;
        *self = usize::try_from(wide).map_err(|_| StateError::Corrupt)?;
        Ok(())
    }
}

impl<T: Stateful, const N: usize> Stateful for [T; N] {
    fn save(&self, w: &mut Writer) {
        for item in self {
            item.save(w);
        }
    }
    fn load(&mut self, r: &mut Reader) -> StateResult {
        for item in self {
            item.load(r)?;
        }
        Ok(())
    }
}

/// A memory block. Its length is part of the machine, not of the snapshot:
/// a snapshot with a different length belongs to a different cartridge.
impl Stateful for Vec<u8> {
    fn save(&self, w: &mut Writer) {
        self.len().save(w);
        w.put(self);
    }
    fn load(&mut self, r: &mut Reader) -> StateResult {
        let mut len = 0usize;
        len.load(r)?;
        if len != self.len() {
            return Err(StateError::WrongGame);
        }
        self.copy_from_slice(r.take(len)?);
        Ok(())
    }
}

/// Implements [`Stateful`] for a struct by listing the fields that change
/// while a game runs, in a fixed order.
macro_rules! state_fields {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl crate::state::Stateful for $ty {
            fn save(&self, w: &mut crate::state::Writer) {
                $( crate::state::Stateful::save(&self.$field, w); )*
            }
            fn load(&mut self, r: &mut crate::state::Reader) -> crate::state::StateResult {
                $( crate::state::Stateful::load(&mut self.$field, r)?; )*
                Ok(())
            }
        }
    };
}

/// For structs that need more than a field list: saves or loads the named
/// fields in order. `each_field!(self, save, w; a, b)` / `each_field!(self, load, r; a, b)`.
macro_rules! each_field {
    ($this:ident, save, $w:ident; $($field:ident),* $(,)?) => {
        $( crate::state::Stateful::save(&$this.$field, $w); )*
    };
    ($this:ident, load, $r:ident; $($field:ident),* $(,)?) => {
        $( crate::state::Stateful::load(&mut $this.$field, $r)?; )*
    };
}

/// FNV-1a: small, dependency-free, enough to notice a damaged file.
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Wraps a body in the file header.
pub fn seal(body: Writer) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.bytes.len() + 14);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&hash(&body.bytes).to_le_bytes());
    out.extend_from_slice(&body.bytes);
    out
}

/// Checks the header and hash and hands back a reader over the body.
pub fn open(data: &[u8]) -> Result<Reader<'_>, StateError> {
    if data.len() < 14 || &data[..4] != MAGIC {
        return Err(StateError::NotAState);
    }
    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != VERSION {
        return Err(StateError::Version(version));
    }
    let mut stored = [0u8; 8];
    stored.copy_from_slice(&data[6..14]);
    let body = &data[14..];
    if u64::from_le_bytes(stored) != hash(body) {
        return Err(StateError::Corrupt);
    }
    Ok(Reader { bytes: body })
}

/// True when every byte of the body was used: a guard against a layout drift
/// that happens to still parse.
pub fn finished(r: &Reader) -> StateResult {
    if r.bytes.is_empty() {
        Ok(())
    } else {
        Err(StateError::Corrupt)
    }
}
