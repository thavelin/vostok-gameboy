//! `config.toml` in the app folder (see [`crate::paths::app_dir`]).
//!
//! Every field has a default, so a missing or partial file is fine. The `[ui]`
//! table belongs to the front end: the backend keeps whatever is in it and
//! writes it back untouched.

use crate::frame::Palette;
use crate::input::KeyNames;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "config.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Folders scanned for ROMs (`.gb`, `.gbc`, `.zip`).
    pub rom_dirs: Vec<PathBuf>,
    /// Where saves go; empty means `<app dir>/saves`.
    pub saves_dir: Option<PathBuf>,
    /// Colours for original four-shade games: grey, dmg, pocket, phosphor, amber.
    pub palette: String,
    /// Seconds between a game writing to its save RAM and the file being
    /// written; 0 saves only on exit.
    pub autosave_secs: u64,
    pub audio: Audio,
    pub keys: KeyNames,
    /// Palette chosen for one game, by ROM id; games not listed use `palette`.
    pub palettes: BTreeMap<String, String>,
    /// Free space for the front end (theme, render mode, layout, ...).
    pub ui: toml::Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub enabled: bool,
    /// 0.0 to 1.0
    pub volume: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 0.8,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rom_dirs: Vec::new(),
            saves_dir: None,
            palette: Palette::Dmg.name().to_string(),
            autosave_secs: 2,
            audio: Audio::default(),
            keys: KeyNames::default(),
            palettes: BTreeMap::new(),
            ui: toml::Table::new(),
        }
    }
}

impl Config {
    /// A missing file gives the defaults; a file that does not parse is an
    /// error with the line in it, so a typo is not silently ignored.
    pub fn load(dir: &Path) -> Result<Config, String> {
        let path = dir.join(FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::paths::write_atomic(&dir.join(FILE_NAME), text.as_bytes())
    }

    pub fn palette(&self) -> Palette {
        Palette::parse(&self.palette).unwrap_or(Palette::Dmg)
    }

    /// The palette for one game: its own choice, else the general one.
    pub fn palette_for(&self, rom_id: &str) -> Palette {
        self.palettes
            .get(rom_id)
            .and_then(|name| Palette::parse(name))
            .unwrap_or_else(|| self.palette())
    }

    /// Remembers a palette for one game only.
    pub fn set_palette_for(&mut self, rom_id: &str, palette: Palette) {
        self.palettes
            .insert(rom_id.to_string(), palette.name().to_string());
    }

    /// Uses a palette for every game, forgetting the per-game choices.
    pub fn set_palette_all(&mut self, palette: Palette) {
        self.palette = palette.name().to_string();
        self.palettes.clear();
    }

    /// Where save-state slots are kept.
    pub fn states_dir(&self, app_dir: &Path) -> PathBuf {
        app_dir.join("states")
    }

    pub fn saves_dir(&self, app_dir: &Path) -> PathBuf {
        self.saves_dir
            .clone()
            .unwrap_or_else(|| app_dir.join("saves"))
    }

    /// Adds a ROM folder; false when it is already listed.
    pub fn add_rom_dir(&mut self, dir: PathBuf) -> bool {
        if self.rom_dirs.contains(&dir) {
            return false;
        }
        self.rom_dirs.push(dir);
        true
    }

    pub fn remove_rom_dir(&mut self, dir: &Path) -> bool {
        let before = self.rom_dirs.len();
        self.rom_dirs.retain(|listed| listed != dir);
        self.rom_dirs.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_defaults_and_a_broken_one_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), Config::default());
        std::fs::write(dir.path().join(FILE_NAME), "palette = [").unwrap();
        assert!(Config::load(dir.path())
            .unwrap_err()
            .contains("config.toml"));
    }

    #[test]
    fn round_trips_and_keeps_the_front_ends_table() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.add_rom_dir("E:/games".into());
        assert!(!config.add_rom_dir("E:/games".into()));
        config.palette = "amber".into();
        config.audio.volume = 0.25;
        config.set_palette_for("0badf00d", Palette::Phosphor);
        config
            .ui
            .insert("theme".into(), toml::Value::String("phosphor".into()));
        config.save(dir.path()).unwrap();

        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded, config);
        assert_eq!(loaded.palette(), Palette::Amber);
        assert_eq!(loaded.saves_dir(dir.path()), dir.path().join("saves"));
        assert_eq!(loaded.states_dir(dir.path()), dir.path().join("states"));
        assert_eq!(loaded.palette_for("0badf00d"), Palette::Phosphor);
        assert_eq!(loaded.palette_for("another"), Palette::Amber);

        let mut all = loaded;
        all.set_palette_all(Palette::Pocket);
        assert_eq!(all.palette_for("0badf00d"), Palette::Pocket);
    }

    #[test]
    fn a_partial_file_fills_in_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE_NAME),
            "palette = \"neon\"\n[keys]\na = [\"k\"]\n[ui]\nscale = 2\n",
        )
        .unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.palette(), Palette::Dmg); // unknown name falls back
        assert_eq!(config.keys.a, ["k"]);
        assert_eq!(config.keys.b, ["x", "j"]);
        assert!(config.audio.enabled);
        assert_eq!(config.ui["scale"].as_integer(), Some(2));
    }
}
