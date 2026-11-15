//! Bridge for the original menu (`menu.rs`), which knows one ROM folder.
//! The real settings are `vostok_gameboy::settings::Config`.

use std::io;
use std::path::PathBuf;
use vostok_gameboy::{paths, settings};

pub struct Config {
    pub roms_dir: Option<PathBuf>,
}

/// The first configured ROM folder that still exists.
pub fn load() -> Config {
    let config = settings::Config::load(&paths::app_dir()).unwrap_or_default();
    Config { roms_dir: config.rom_dirs.into_iter().find(|dir| dir.is_dir()) }
}

/// Puts the menu's folder first in the list of ROM folders.
pub fn save(cfg: &Config) -> io::Result<()> {
    let Some(dir) = &cfg.roms_dir else { return Ok(()) };
    let app_dir = paths::app_dir();
    let mut config = settings::Config::load(&app_dir).map_err(io::Error::other)?;
    config.remove_rom_dir(dir);
    config.rom_dirs.insert(0, dir.clone());
    config.save(&app_dir)
}

pub fn expand_tilde(path: &str) -> Option<PathBuf> {
    Some(paths::expand_tilde(path))
}
