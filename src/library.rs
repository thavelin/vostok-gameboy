//! The game shelf: scanning ROM folders and remembering what was played.
//!
//! Scanning reads every ROM (to check its header and compute its id), so run
//! it off the UI thread for big folders. Play statistics are kept per ROM id
//! in `<app dir>/stats.json`, so they follow a game when its file is renamed.

use crate::cart::{self, Header};
use crate::states::{self, SlotInfo, SLOTS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STATS_FILE: &str = "stats.json";
/// Folders below a ROM folder that are looked into.
const MAX_DEPTH: usize = 2;

#[derive(Debug, Clone, Serialize)]
pub struct RomEntry {
    /// CRC-32 of the ROM as eight hex digits.
    pub id: String,
    /// The file to hand to [`cart::load`].
    pub path: PathBuf,
    /// Name of the ROM inside the zip, when `path` is a zip.
    pub member: Option<String>,
    /// File name without extension: the name to show.
    pub name: String,
    pub header: Header,
    /// False when the emulator core cannot run it (`header.unsupported` says why).
    pub playable: bool,
    pub save: Option<PathBuf>,
    /// The save-state slots, index 0 being slot 1.
    pub states: [Option<SlotInfo>; SLOTS as usize],
    /// The newest save state: the one "resume" goes back to.
    pub resume: Option<SlotInfo>,
    pub stats: PlayStats,
}

/// How to order the shelf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Name,
    /// Most recently played first; never-played games after, by name.
    Recent,
    /// Longest played first.
    Played,
}

impl Scan {
    pub fn sort(&mut self, order: Sort) {
        self.entries.sort_by_key(|entry| entry.name.to_lowercase());
        match order {
            Sort::Name => {}
            Sort::Recent => self
                .entries
                .sort_by_key(|entry| std::cmp::Reverse(entry.stats.last_played)),
            Sort::Played => self
                .entries
                .sort_by_key(|entry| std::cmp::Reverse(entry.stats.seconds)),
        }
    }

    /// The entries whose name or cartridge title contains `text` (any case).
    pub fn search(&self, text: &str) -> Vec<&RomEntry> {
        let wanted = text.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&wanted)
                    || entry.header.title.to_lowercase().contains(&wanted)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Scan {
    pub entries: Vec<RomEntry>,
    /// Files that looked like ROMs but could not be read, with the reason.
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

/// Lists the games in `dirs`, sorted by name. The same file reached through
/// two folders is listed once.
pub fn scan(dirs: &[PathBuf], saves_dir: &Path, states_dir: &Path, stats: &Stats) -> Scan {
    let mut files = Vec::new();
    for dir in dirs {
        collect(dir, 0, &mut files);
    }
    files.sort();
    files.dedup();

    let mut result = Scan::default();
    for path in files {
        match cart::load(&path) {
            Ok(rom) => {
                let id = rom.id();
                let slots = states::list(states_dir, &id);
                result.entries.push(RomEntry {
                    resume: slots
                        .iter()
                        .flatten()
                        .max_by_key(|slot| slot.saved_at)
                        .copied(),
                    states: slots,
                    stats: stats.get(&id),
                    id,
                    name: rom.stem(),
                    member: rom.member,
                    playable: rom.header.unsupported.is_none(),
                    save: crate::saves::find(saves_dir, &path),
                    header: rom.header,
                    path,
                });
            }
            // A zip of something else is not worth a warning.
            Err(_) if cart::is_zip_name(&path) => {}
            Err(error) => result.skipped.push(Skipped {
                path,
                reason: error.to_string(),
            }),
        }
    }
    result
        .entries
        .sort_by_key(|entry| entry.name.to_lowercase());
    result
}

fn collect(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !hidden && depth < MAX_DEPTH {
                collect(&path, depth + 1, files);
            }
        } else if cart::is_rom_name(&path) || cart::is_zip_name(&path) {
            files.push(path.canonicalize().map(strip_verbatim).unwrap_or(path));
        }
    }
}

/// Windows' `canonicalize` answers with a `\\?\` prefix that other programs
/// and people do not expect; drop it for ordinary drive paths.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayStats {
    pub launches: u32,
    pub seconds: u64,
    /// Unix time the last session ended.
    pub last_played: Option<u64>,
}

/// Play statistics for every game, keyed by ROM id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Stats {
    games: BTreeMap<String, PlayStats>,
}

impl Stats {
    /// A missing or unreadable file starts empty: statistics are not worth
    /// refusing to start over.
    pub fn load(app_dir: &Path) -> Stats {
        std::fs::read_to_string(app_dir.join(STATS_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app_dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::paths::write_atomic(&app_dir.join(STATS_FILE), text.as_bytes())
    }

    pub fn get(&self, id: &str) -> PlayStats {
        self.games.get(id).copied().unwrap_or_default()
    }

    pub fn record(&mut self, id: &str, seconds: u64, now: u64) {
        let stats = self.games.entry(id.to_string()).or_default();
        stats.launches += 1;
        stats.seconds += seconds;
        stats.last_played = Some(now);
    }

    /// Reads the file, adds one finished session and writes it back.
    pub fn record_session(app_dir: &Path, id: &str, seconds: u64) -> std::io::Result<()> {
        let mut stats = Stats::load(app_dir);
        stats.record(id, seconds, crate::paths::unix_now());
        stats.save(app_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::testing;

    #[test]
    fn scan_finds_roms_and_zips_and_reports_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let roms = dir.path().join("roms");
        let saves = dir.path().join("saves");
        std::fs::create_dir_all(roms.join("sub").join(".hidden")).unwrap();
        std::fs::create_dir_all(&saves).unwrap();

        std::fs::write(roms.join("beta.gb"), testing::rom()).unwrap();
        let mut other = testing::rom();
        other[0x200] = 1; // a different game: different id
        testing::write_zip(&roms.join("sub").join("Alpha.zip"), "alpha.gbc", &other);
        std::fs::write(roms.join("broken.gb"), b"not a rom").unwrap();
        testing::write_zip(&roms.join("photos.zip"), "cat.txt", b"meow");
        std::fs::write(
            roms.join("sub").join(".hidden").join("no.gb"),
            testing::rom(),
        )
        .unwrap();
        std::fs::write(roms.join("notes.txt"), b"hi").unwrap();
        std::fs::write(saves.join("beta.sav"), [0u8; 4]).unwrap();

        let mut stats = Stats::default();
        let beta_id = format!("{:08x}", crc32fast::hash(&testing::rom()));
        stats.record(&beta_id, 90, 1_000);

        // Listing the folder twice must not list the games twice.
        let states = dir.path().join("states");
        let scan = scan(&[roms.clone(), roms.clone()], &saves, &states, &stats);
        let names: Vec<&str> = scan
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["Alpha", "beta"]);
        assert_eq!(scan.entries[0].member.as_deref(), Some("alpha.gbc"));
        assert_eq!(scan.entries[0].save, None);
        assert!(scan.entries[1].save.is_some());
        assert_eq!(scan.entries[1].id, beta_id);
        assert_eq!(scan.entries[1].stats.seconds, 90);
        assert!(scan.entries.iter().all(|entry| entry.playable));
        assert_eq!(scan.skipped.len(), 1);
        assert!(scan.skipped[0].path.ends_with("broken.gb"));
    }

    #[test]
    fn stats_accumulate_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        Stats::record_session(dir.path(), "abcd0123", 30).unwrap();
        Stats::record_session(dir.path(), "abcd0123", 12).unwrap();
        let stats = Stats::load(dir.path());
        let game = stats.get("abcd0123");
        assert_eq!((game.launches, game.seconds), (2, 42));
        assert!(game.last_played.is_some());
        assert_eq!(stats.get("ffffffff"), PlayStats::default());
    }
}
