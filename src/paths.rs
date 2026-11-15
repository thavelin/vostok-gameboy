//! Where files live. Settings and data sit in `~/.config/vostok-gameboy/` on
//! every OS (never `AppData\Roaming`), or wherever `VOSTOK_GAMEBOY_HOME` points.

use std::io;
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `VOSTOK_GAMEBOY_HOME` wins; otherwise `<home>/.config/vostok-gameboy`.
pub fn app_dir() -> PathBuf {
    match std::env::var_os("VOSTOK_GAMEBOY_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => home().join(".config").join(crate::APP_NAME),
    }
}

/// Expands a leading `~` to the home folder.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home().join(rest)
    } else {
        PathBuf::from(path)
    }
}

/// Writes through a temporary file and a rename, so a crash or a closed
/// terminal never leaves a half-written file behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_only_at_the_start() {
        assert_eq!(expand_tilde("~"), home());
        assert_eq!(expand_tilde("~/roms"), home().join("roms"));
        assert_eq!(expand_tilde("a/~/b"), PathBuf::from("a/~/b"));
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep").join("x.bin");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(
            std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
            1
        );
    }
}
