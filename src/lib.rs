//! Vostok Gameboy backend.
//!
//! Everything a front end needs to list, start and drive a game, with no
//! drawing code in it. The usual path:
//!
//! ```no_run
//! use vostok_gameboy::{cart, paths, session::{Session, SessionOptions}, settings::Config};
//!
//! let dir = paths::app_dir();
//! let config = Config::load(&dir).unwrap_or_default();
//! let rom = cart::load("Tetris.zip".as_ref()).unwrap();
//! let session = Session::start(rom, SessionOptions::from_config(&config, &dir)).unwrap();
//! // each UI tick: session.copy_frame(..), session.status(), session.send(..)
//! session.stop();
//! ```
//!
//! Module map (see `MAP.md` for the longer version):
//!
//! - [`cart`]     ROM files and zips, cartridge header
//! - [`machine`]  one emulated Game Boy, stepped a frame at a time (no threads, no clock)
//! - [`session`]  a machine on its own thread: pacing, audio, autosave, commands
//! - [`frame`]    the 160x144 picture and the DMG palettes
//! - [`input`]    key names, key map, press/release tracking
//! - [`saves`]    battery saves (`.sav`, with the real-time-clock footer)
//! - [`states`]   save-state slots (snapshot + preview picture)
//! - [`library`]  scanning ROM folders, play statistics
//! - [`settings`] `config.toml`
//! - [`audio`]    sample ring buffer and the sound output
//! - [`ctl`]      the JSON command line (`vostok-gameboy ctl ...`)

pub mod audio;
pub mod cart;
pub mod ctl;
pub mod frame;
pub mod input;
pub mod library;
pub mod machine;
pub mod paths;
pub mod saves;
pub mod session;
pub mod settings;
pub mod states;

pub const APP_NAME: &str = "vostok-gameboy";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
