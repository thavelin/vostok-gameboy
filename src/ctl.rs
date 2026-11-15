//! `vostok-gameboy ctl ...`: the backend from the command line, as JSON.
//!
//! For scripts, agents and tests. Every command prints one JSON document to
//! stdout and exits 0; on failure it prints `{"ok": false, "error": "..."}`
//! and exits 1. Nothing here opens the sound device or needs a terminal.

use crate::cart;
use crate::frame::{Frame, Palette};
use crate::input::Button;
use crate::library::{self, Stats};
use crate::machine::Machine;
use crate::paths;
use crate::saves;
use crate::settings::Config;
use crate::states;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
vostok-gameboy ctl <command>

  list [--dir DIR]... [--sort name|recent|played] [--search TEXT]
                               games in the configured ROM folders (or DIR)
  info ROM                     cartridge header, save file, save states, statistics
  states ROM                   the four save-state slots of a game
  states ROM clear N           empty slot N
  states ROM thumb N FILE.png  write slot N's preview picture
  palette ROM NAME [--all]     remember a palette for this game (or for every game)
  config                       settings, folders and the choices they accept
  dirs add DIR | remove DIR    change the ROM folders in config.toml
  run ROM [options]            play without a screen, as fast as possible
      --frames N               frames to emulate (default 60)
      --press SPEC             buttons, e.g. start@120,a@200+10
                               (button@first-frame[+frames held], default 6)
      --shot FILE.png          save the last frame as a PNG
      --palette NAME           grey, dmg, pocket, phosphor, amber
      --with-save              start from the save file (never written back)
      --load-state N           start from save-state slot N
      --save-state N           write the machine to slot N when done

Output is JSON. ROM is a .gb, .gbc or .zip file.";

/// Runs a `ctl` command (the arguments after `ctl`) and returns the exit code.
pub fn run(args: &[String]) -> i32 {
    match dispatch(args, &paths::app_dir()) {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            0
        }
        Err(error) => {
            println!("{}", json!({ "ok": false, "error": error }));
            1
        }
    }
}

/// The command without the printing, for tests. `app_dir` is where
/// `config.toml` and the saves live.
pub fn dispatch(args: &[String], app_dir: &Path) -> Result<Value, String> {
    let args: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--json")
        .collect();
    let Some((&command, rest)) = args.split_first() else {
        return Err(format!("no command given\n{USAGE}"));
    };
    match command {
        "list" => list(rest, app_dir),
        "info" => info(rest, app_dir),
        "states" => slots(rest, app_dir),
        "palette" => palette(rest, app_dir),
        "config" => config(app_dir),
        "dirs" => dirs(rest, app_dir),
        "run" => headless(rest, app_dir),
        "help" | "--help" | "-h" => Ok(json!({ "ok": true, "usage": USAGE })),
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn list(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let config = Config::load(app_dir)?;
    let mut dirs = Vec::new();
    let mut sort = library::Sort::Name;
    let mut search = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        match arg {
            "--dir" => dirs.push(PathBuf::from(rest.next().ok_or("--dir needs a folder")?)),
            "--sort" => {
                sort = match *rest.next().ok_or("--sort needs name, recent or played")? {
                    "name" => library::Sort::Name,
                    "recent" => library::Sort::Recent,
                    "played" => library::Sort::Played,
                    other => return Err(format!("unknown order `{other}`")),
                }
            }
            "--search" => search = Some(*rest.next().ok_or("--search needs some text")?),
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    if dirs.is_empty() {
        dirs = config.rom_dirs.clone();
    }
    let mut scan = library::scan(
        &dirs,
        &config.saves_dir(app_dir),
        &config.states_dir(app_dir),
        &Stats::load(app_dir),
    );
    scan.sort(sort);
    let games = scan.search(search.unwrap_or(""));
    Ok(json!({
        "ok": true,
        "dirs": dirs,
        "count": games.len(),
        "games": to_value(&games)?,
        "skipped": to_value(&scan.skipped)?,
    }))
}

fn info(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let [path] = args else {
        return Err("usage: ctl info ROM".into());
    };
    let config = Config::load(app_dir)?;
    let rom = cart::load(Path::new(path)).map_err(|error| error.to_string())?;
    Ok(json!({
        "ok": true,
        "id": rom.id(),
        "path": rom.path,
        "member": rom.member,
        "name": rom.stem(),
        "bytes": rom.bytes.len(),
        "header": to_value(&rom.header)?,
        "playable": rom.header.unsupported.is_none(),
        "save": saves::find(&config.saves_dir(app_dir), &rom.path),
        "states": to_value(&states::list(&config.states_dir(app_dir), &rom.id()))?,
        "palette": config.palette_for(&rom.id()).name(),
        "stats": to_value(&Stats::load(app_dir).get(&rom.id()))?,
    }))
}

fn slot_number(text: &str) -> Result<u8, String> {
    match text.parse::<u8>() {
        Ok(slot) if (1..=states::SLOTS).contains(&slot) => Ok(slot),
        _ => Err(format!(
            "`{text}` is not a slot: use 1 to {}",
            states::SLOTS
        )),
    }
}

fn slots(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let Some((path, action)) = args.split_first() else {
        return Err("usage: ctl states ROM [clear N | thumb N FILE.png]".into());
    };
    let config = Config::load(app_dir)?;
    let dir = config.states_dir(app_dir);
    let rom = cart::load(Path::new(path)).map_err(|error| error.to_string())?;
    let id = rom.id();
    let mut done = json!(null);
    match action {
        [] => {}
        ["clear", slot] => {
            let cleared =
                states::clear(&dir, &id, slot_number(slot)?).map_err(|error| error.to_string())?;
            done = json!({ "cleared": cleared });
        }
        ["thumb", slot, file] => {
            let slot = slot_number(slot)?;
            let saved = states::read(&dir, &id, slot)
                .map_err(|error| error.to_string())?
                .ok_or(format!("slot {slot} is empty"))?;
            write_png(Path::new(file), &saved.thumbnail)
                .map_err(|error| format!("cannot write {file}: {error}"))?;
            done = json!({ "thumb": file });
        }
        _ => return Err("usage: ctl states ROM [clear N | thumb N FILE.png]".into()),
    }
    Ok(json!({
        "ok": true,
        "id": id,
        "dir": dir.join(&id),
        "done": done,
        "slots": to_value(&states::list(&dir, &id))?,
        "resume": to_value(&states::latest(&dir, &id))?,
    }))
}

fn palette(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let (path, name, all) = match args {
        [path, name] => (path, name, false),
        [path, name, "--all"] => (path, name, true),
        _ => return Err("usage: ctl palette ROM NAME [--all]".into()),
    };
    let chosen = Palette::parse(name).ok_or(format!("unknown palette `{name}`"))?;
    let rom = cart::load(Path::new(path)).map_err(|error| error.to_string())?;
    let mut config = Config::load(app_dir)?;
    if all {
        config.set_palette_all(chosen);
    } else {
        config.set_palette_for(&rom.id(), chosen);
    }
    config
        .save(app_dir)
        .map_err(|error| format!("cannot write config.toml: {error}"))?;
    Ok(json!({
        "ok": true,
        "id": rom.id(),
        "palette": config.palette_for(&rom.id()).name(),
        "default": config.palette().name(),
        "all": all,
    }))
}

fn config(app_dir: &Path) -> Result<Value, String> {
    let config = Config::load(app_dir)?;
    Ok(json!({
        "ok": true,
        "version": crate::VERSION,
        "app_dir": app_dir,
        "config_file": app_dir.join(crate::settings::FILE_NAME),
        "saves_dir": config.saves_dir(app_dir),
        "states_dir": config.states_dir(app_dir),
        "config": to_value(&config)?,
        "palettes": Palette::ALL.map(Palette::name),
        "palette_labels": Palette::ALL.map(Palette::label),
        "state_slots": states::SLOTS,
        "buttons": Button::ALL.map(Button::name),
    }))
}

fn dirs(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let [action, dir] = args else {
        return Err("usage: ctl dirs add DIR | remove DIR".into());
    };
    let mut config = Config::load(app_dir)?;
    let dir = paths::expand_tilde(dir);
    let changed = match *action {
        "add" => {
            if !dir.is_dir() {
                return Err(format!("{} is not a folder", dir.display()));
            }
            // Store the full path so it works from any working folder.
            let dir = std::path::absolute(&dir).unwrap_or(dir);
            config.add_rom_dir(dir)
        }
        "remove" => {
            let absolute = std::path::absolute(&dir).unwrap_or_else(|_| dir.clone());
            config.remove_rom_dir(&dir) | config.remove_rom_dir(&absolute)
        }
        other => return Err(format!("unknown action `{other}`: use add or remove")),
    };
    if changed {
        config
            .save(app_dir)
            .map_err(|error| format!("cannot write config.toml: {error}"))?;
    }
    Ok(json!({ "ok": true, "changed": changed, "rom_dirs": config.rom_dirs }))
}

/// One `button@frame+held` item of `--press`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub button: Button,
    pub from: u64,
    pub held: u64,
}

pub fn parse_presses(spec: &str) -> Result<Vec<Press>, String> {
    spec.split(',')
        .filter(|item| !item.trim().is_empty())
        .map(|item| {
            let bad =
                || format!("bad --press item `{item}`: expected button@frame or button@frame+held");
            let (name, when) = item.trim().split_once('@').ok_or_else(bad)?;
            let (from, held) = when.split_once('+').unwrap_or((when, "6"));
            Ok(Press {
                button: Button::parse(name).ok_or_else(bad)?,
                from: from.trim().parse().map_err(|_| bad())?,
                held: held.trim().parse::<u64>().map_err(|_| bad())?.max(1),
            })
        })
        .collect()
}

fn headless(args: &[&str], app_dir: &Path) -> Result<Value, String> {
    let mut path = None;
    let mut frames = 60u64;
    let mut presses = Vec::new();
    let mut shot = None;
    let mut palette = None;
    let mut with_save = false;
    let mut load_state = None;
    let mut save_state = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        let mut value = |name: &str| rest.next().copied().ok_or(format!("{name} needs a value"));
        match arg {
            "--frames" => {
                frames = value("--frames")?
                    .parse()
                    .map_err(|_| "--frames needs a number")?;
            }
            "--press" => presses.extend(parse_presses(value("--press")?)?),
            "--shot" => shot = Some(PathBuf::from(value("--shot")?)),
            "--palette" => {
                let name = value("--palette")?;
                palette = Some(Palette::parse(name).ok_or(format!("unknown palette `{name}`"))?);
            }
            "--with-save" => with_save = true,
            "--load-state" => load_state = Some(slot_number(value("--load-state")?)?),
            "--save-state" => save_state = Some(slot_number(value("--save-state")?)?),
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    let path = path.ok_or("usage: ctl run ROM [options]")?;
    if frames > 1_000_000 {
        return Err("--frames is limited to 1000000".into());
    }

    let config = Config::load(app_dir)?;
    let rom = cart::load(&path).map_err(|error| error.to_string())?;
    let id = rom.id();
    let states_dir = config.states_dir(app_dir);
    let mut machine = Machine::new(&rom, palette.unwrap_or(config.palette_for(&id)))?;
    let mut save_loaded = false;
    if with_save && rom.header.saves() {
        let header = &rom.header;
        let read = saves::read(
            &config.saves_dir(app_dir),
            &rom.path,
            header.save_ram_bytes(),
            header.clock,
        );
        if let Some(save) = read.map_err(|error| format!("cannot read the save: {error}"))? {
            machine.load_save(&save);
            save_loaded = true;
        }
    }

    if let Some(slot) = load_state {
        let saved = states::read(&states_dir, &id, slot)
            .map_err(|error| error.to_string())?
            .ok_or(format!("slot {slot} is empty"))?;
        machine.load_state(&saved.state)?;
    }
    let first_frame = machine.frames();

    let started = std::time::Instant::now();
    let mut samples = 0u64;
    for frame in 0..frames {
        for press in &presses {
            if press.from == frame {
                machine.press(press.button);
            }
            if press.from + press.held == frame {
                machine.release(press.button);
            }
        }
        machine.run_frame(|block| samples += block.len() as u64);
    }
    let seconds = started.elapsed().as_secs_f64();
    let saved_state = match save_state {
        Some(slot) => Some(
            states::write(
                &states_dir,
                &id,
                slot,
                machine.frame(),
                machine.frames(),
                &machine.save_state(),
            )
            .map_err(|error| format!("cannot write slot {slot}: {error}"))?,
        ),
        None => None,
    };

    if let Some(shot) = &shot {
        write_png(shot, machine.frame())
            .map_err(|error| format!("cannot write {}: {error}", shot.display()))?;
    }
    let distinct = {
        let mut colours: Vec<_> = machine
            .frame()
            .pixels
            .iter()
            .map(|p| (p.r, p.g, p.b))
            .collect();
        colours.sort_unstable();
        colours.dedup();
        colours.len()
    };
    Ok(json!({
        "ok": true,
        "id": id,
        "title": rom.header.title,
        "frames": machine.frames(),
        "started_at_frame": first_frame,
        "saved_state": to_value(&saved_state)?,
        "emulated_seconds": frames as f64 / crate::machine::FRAME_RATE,
        "wall_seconds": seconds,
        "speed": if seconds > 0.0 { frames as f64 / crate::machine::FRAME_RATE / seconds } else { 0.0 },
        "audio_samples": samples,
        "frame_crc32": format!("{:08x}", machine.frame().checksum()),
        "frame_colours": distinct,
        "ram_written": machine.take_ram_dirty(),
        "save_loaded": save_loaded,
        "shot": shot,
    }))
}

pub fn write_png(path: &Path, frame: &Frame) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, frame.width as u32, frame.height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(&frame.to_rgb_bytes())
        .map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::testing;

    fn call(app_dir: &Path, args: &[&str]) -> Result<Value, String> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        dispatch(&args, app_dir)
    }

    #[test]
    fn press_specs_parse() {
        assert_eq!(
            parse_presses("start@120, a@200+10").unwrap(),
            [
                Press {
                    button: Button::Start,
                    from: 120,
                    held: 6
                },
                Press {
                    button: Button::A,
                    from: 200,
                    held: 10
                },
            ]
        );
        assert!(parse_presses("").unwrap().is_empty());
        for bad in ["jump@3", "a", "a@x", "a@3+y"] {
            assert!(parse_presses(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn dirs_list_info_and_run_work_together() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let roms = dir.path().join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        let rom = roms.join("Test Game.zip");
        testing::write_zip(&rom, "test.gb", &testing::rom());
        let roms_arg = roms.to_str().unwrap();
        let rom_arg = rom.to_str().unwrap();

        assert_eq!(call(&app, &["list"]).unwrap()["count"], 0);
        assert_eq!(
            call(&app, &["dirs", "add", roms_arg]).unwrap()["changed"],
            true
        );
        assert_eq!(
            call(&app, &["dirs", "add", roms_arg]).unwrap()["changed"],
            false
        );
        assert!(call(&app, &["dirs", "add", "no/such/folder"]).is_err());

        let listed = call(&app, &["list", "--json"]).unwrap();
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["games"][0]["name"], "Test Game");
        assert_eq!(listed["games"][0]["header"]["mapper"], "MBC1");

        let info = call(&app, &["info", rom_arg]).unwrap();
        assert_eq!(info["header"]["title"], "VOSTOKTEST");
        assert_eq!(info["playable"], true);
        assert_eq!(info["save"], Value::Null);

        let shot = dir.path().join("out").join("shot.png");
        let ran = call(
            &app,
            &[
                "run",
                rom_arg,
                "--frames",
                "30",
                "--press",
                "a@5",
                "--shot",
                shot.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(ran["frames"], 30);
        assert_eq!(ran["ram_written"], true);
        assert_eq!(&std::fs::read(&shot).unwrap()[..4], b"\x89PNG");
        // A headless run never leaves a save behind.
        assert!(!app.join("saves").exists());
        assert!(!app.join("states").exists());

        // Save states: 30 frames into slot 2, 20 more from there, same as 50 straight.
        let saved = call(
            &app,
            &["run", rom_arg, "--frames", "30", "--save-state", "2"],
        )
        .unwrap();
        assert_eq!(saved["saved_state"]["frames"], 30);
        let resumed = call(
            &app,
            &["run", rom_arg, "--frames", "20", "--load-state", "2"],
        )
        .unwrap();
        let straight = call(&app, &["run", rom_arg, "--frames", "50"]).unwrap();
        assert_eq!(resumed["started_at_frame"], 30);
        assert_eq!(resumed["frames"], 50);
        assert_eq!(resumed["frame_crc32"], straight["frame_crc32"]);

        let slots = call(&app, &["states", rom_arg]).unwrap();
        assert_eq!(slots["slots"][1]["slot"], 2);
        assert_eq!(slots["slots"][0], Value::Null);
        assert_eq!(slots["resume"]["slot"], 2);
        assert_eq!(
            call(&app, &["info", rom_arg]).unwrap()["states"][1]["frames"],
            30
        );
        assert_eq!(
            call(&app, &["list"]).unwrap()["games"][0]["resume"]["slot"],
            2
        );
        let thumb = dir.path().join("thumb.png");
        call(
            &app,
            &["states", rom_arg, "thumb", "2", thumb.to_str().unwrap()],
        )
        .unwrap();
        assert_eq!(&std::fs::read(&thumb).unwrap()[..4], b"\x89PNG");
        assert!(call(&app, &["run", rom_arg, "--load-state", "1"]).is_err());
        assert!(call(&app, &["states", rom_arg, "clear", "7"]).is_err());
        let cleared = call(&app, &["states", rom_arg, "clear", "2"]).unwrap();
        assert_eq!(cleared["done"]["cleared"], true);
        assert_eq!(cleared["resume"], Value::Null);

        // Palette per game, then for all games.
        assert_eq!(call(&app, &["info", rom_arg]).unwrap()["palette"], "dmg");
        let chosen = call(&app, &["palette", rom_arg, "amber"]).unwrap();
        assert_eq!(
            (&chosen["palette"], &chosen["default"]),
            (&json!("amber"), &json!("dmg"))
        );
        let chosen = call(&app, &["palette", rom_arg, "pocket", "--all"]).unwrap();
        assert_eq!(chosen["default"], "pocket");
        assert!(call(&app, &["palette", rom_arg, "neon"]).is_err());
        assert_eq!(
            call(&app, &["list", "--search", "nothing like it"]).unwrap()["count"],
            0
        );
        assert_eq!(
            call(&app, &["list", "--sort", "recent", "--search", "TEST"]).unwrap()["count"],
            1
        );

        assert_eq!(
            call(&app, &["config"]).unwrap()["config"]["rom_dirs"][0],
            listed["dirs"][0]
        );
        assert_eq!(
            call(&app, &["dirs", "remove", roms_arg]).unwrap()["rom_dirs"],
            json!([])
        );
    }

    #[test]
    fn mistakes_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            &[][..],
            &["dance"],
            &["info"],
            &["run"],
            &["run", "missing.gb"],
            &["run", "x.gb", "--frames"],
        ] {
            assert!(call(dir.path(), args).is_err(), "{args:?}");
        }
    }
}
