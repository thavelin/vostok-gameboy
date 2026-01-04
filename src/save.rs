use gameboy_core::Gameboy;
use std::io;
use std::path::{Path, PathBuf};

/// Get the saves directory path (hidden .saves folder next to the ROM)
fn get_saves_dir(rom_path: &Path) -> PathBuf {
    rom_path
        .parent()
        .map(|p| p.join(".saves"))
        .unwrap_or_else(|| PathBuf::from(".saves"))
}

/// Get the save file path for a ROM (in .saves/ directory with .sav extension)
pub fn get_save_path(rom_path: &Path) -> PathBuf {
    let saves_dir = get_saves_dir(rom_path);
    let rom_name = rom_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    saves_dir.join(format!("{}.sav", rom_name))
}

/// Load save data from disk into the gameboy if it exists
pub fn load_save(gameboy: &mut Gameboy, rom_path: &Path) -> io::Result<bool> {
    let save_path = get_save_path(rom_path);

    // Check if this cartridge supports battery saves
    if !gameboy.get_cartridge().has_battery() {
        return Ok(false);
    }

    if save_path.exists() {
        let save_data = std::fs::read(&save_path)?;
        gameboy.get_cartridge_mut().set_ram(save_data);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Save the cartridge RAM to disk
pub fn save_game(gameboy: &Gameboy, rom_path: &Path) -> io::Result<bool> {
    // Check if this cartridge supports battery saves
    if !gameboy.get_cartridge().has_battery() {
        return Ok(false);
    }

    let save_path = get_save_path(rom_path);
    
    // Create the .saves directory if it doesn't exist
    if let Some(saves_dir) = save_path.parent() {
        std::fs::create_dir_all(saves_dir)?;
    }
    
    let ram = gameboy.get_cartridge().get_ram();
    std::fs::write(&save_path, ram)?;
    Ok(true)
}

