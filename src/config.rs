use std::io;
use std::path::{Path, PathBuf};

/// Returns `~/.config/terminal_gameboy/` using $HOME; None if HOME is unset.
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("terminal_gameboy")
    })
}

/// Returns the path to the config file itself.
fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config"))
}

pub struct Config {
    pub roms_dir: Option<PathBuf>,
}

/// Load config from disk. Returns defaults (roms_dir: None) on any error or
/// if the file doesn't exist yet.
pub fn load() -> Config {
    let Some(path) = config_file() else {
        return Config { roms_dir: None };
    };

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Config { roms_dir: None };
    };

    let mut roms_dir = None;
    for line in contents.lines() {
        if let Some(val) = line.strip_prefix("roms_dir=") {
            let p = PathBuf::from(val.trim());
            if p.is_dir() {
                roms_dir = Some(p);
            }
        }
    }

    Config { roms_dir }
}

/// Persist config to disk, creating the config directory if needed.
pub fn save(cfg: &Config) -> io::Result<()> {
    let Some(dir) = config_dir() else {
        return Ok(()); // No HOME set — skip silently
    };
    let Some(file) = config_file() else {
        return Ok(());
    };

    std::fs::create_dir_all(&dir)?;

    let mut contents = String::new();
    if let Some(ref p) = cfg.roms_dir {
        contents.push_str(&format!("roms_dir={}\n", p.display()));
    }
    std::fs::write(file, contents)?;
    Ok(())
}

/// Expand a leading `~` to the home directory. Returns None if `~` is used
/// but HOME is unset.
pub fn expand_tilde(path: &str) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var_os("HOME")?;
        Some(Path::new(&home).join(rest))
    } else if path == "~" {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home))
    } else {
        Some(PathBuf::from(path))
    }
}
