# Context

State of the project. Update it when the state changes.

## What this is

A fork of [dquigles/terminal_gameboy](https://github.com/dquigles/terminal_gameboy) (MIT) being turned into
Vostok Gameboy. The emulation itself is `gameboy_core` 0.3.3 (Apache-2.0), copied into `core/` and changed there
(save states added); the rest of the repo is everything around it.

## State (2026-11-14)

- **Backend: built.** The library half (see `MAP.md`) is in place with 44 offline tests.
- **Save states: built**, in the core and in the backend (four slots per game, preview picture, session
  commands, `ctl`). This is what the Figma design's F5 / F8 / slot grid needs.
- **Front end: built** (`src/tui`, see `MAP.md`): the green handheld shell from Troy's Figma file
  (`hBPyEE64P7SysCpPEKxaj2`, page "Vostok Game Boy · Layouts", the "v2 Filled" frames) at 200x56, 140x39, 70x60,
  100x28 and 80x24, the pause panel with the slot grid, the library, help, phosphor and amber themes. The
  original `menu.rs`, `renderer.rs` and `config.rs` are gone, and with them the ASCII mode.
- The package and command are `vostok-gameboy` (was `terminal_gameboy`). Launchers: `run.cmd` / `run.sh`,
  `setup.cmd` / `setup.sh`, and on Troy's PC `~\.local\bin\vostok-gameboy.cmd`.

## Checked

- `cargo test` (44 library + 30 front-end tests + 1 doc test) and `cargo clippy --all-targets -- -D warnings` pass.
- Every screen drawn with `--html` (real Tetris zip running) and compared by eye with the Figma frames:
  140x39, 200x56 panels, 70x60, 100x28, 80x24, too small, pause panel, library, amber.
- Headless against the real ROM zips in `..\Games` (`ctl list`, `ctl run --shot`): all seven are read and
  playable; Tetris reaches its menu through scripted Start presses; Pokemon Yellow draws in colour.
- Save states on all seven real games: 700 frames, save to a slot, load it in a new process, 500 more frames
  with button presses gives the same final picture as 1200 frames straight through.
- A real-time session with the sound device open (muted) holds ~60 fps; turbo and stop work.

## Not checked

- **The app in a real terminal.** Nobody has played it yet: the loop in `main.rs` (keys, key releases, Kitty
  graphics placement and clean-up, resize, sound out loud) has only been compiled. Everything under it is
  tested through `App::key` and `view::render`.
- The quarter-block shell in fonts other than the browser's (seams between cells depend on the font).
- Battery saves written by a real game (Pokemon) and loaded again. Covered by tests with the test ROM only.
- A save state taken with the real-time clock running (no MBC3 clock game in the test set).
- Linux / the Omarchy laptop.

## What the design asks for and where it is

| Design | Backend |
| --- | --- |
| F5 save / F8 load, slots 1-4, del clear | `Command::SaveState(n)`, `LoadState(n)`, `ClearState(n)`; `Status::slots` |
| Slot time and thumbnail | `SlotInfo::saved_at`, `states::read(..).thumbnail` (80x72) |
| Library "SAVE" count, "r resume slot N" | `RomEntry::states`, `RomEntry::resume` |
| Palette saved per ROM, "a all roms" | `Config::set_palette_for`, `set_palette_all`, `SessionOptions::for_game` |
| Palette names | `Palette::label()` (GB GREEN, PHOSPHOR, AMBER, POCKET GREY, GREY) |
| Keys A z/k, B x/j, Select backspace/space | defaults in `KeyNames` |
| Tab turbo 4x, m mute, p pause, reset | `Command::Speed(4)`, `Mute`, `TogglePause`, `Reset` |
| "frame 16.7 ms · cpu 9%", fps | `Status::frame_ms`, `Status::load`, `Status::fps` (target `machine::FRAME_RATE`) |
| Library sort recent / search | `Scan::sort(Sort::Recent)`, `Scan::search` |
| Scale, half-block / Kitty, F2 view, F3 theme | front end (`tui/app.rs`); view, theme and sort are kept in the `[ui]` config table. No Sixel. |

Different from the design text: sound is 44151 Hz stereo (`machine::SAMPLE_RATE`), not 48 kHz.

## Limits that come from the emulator core

- **No gamepad, link cable, Super Game Boy borders or rumble.**
- Mappers: ROM only, MBC1, MBC2, MBC3 (with clock), MBC5. Others are listed as not playable with the reason.
- While a game has the display off the core reports no frames; `Machine::run_frame` returns after about two
  frames' worth of CPU steps so nothing hangs.
- A save state holds no button state: buttons held while it is loaded stay as the player holds them.

## Decisions

- Settings and data live in `~/.config/vostok-gameboy/` on every OS (`VOSTOK_GAMEBOY_HOME` overrides):
  `config.toml`, `stats.json`, `saves/`, `states/`.
- Battery saves are standard `.sav` files so they move between emulators. Autosave: two seconds after the game
  stops writing its save RAM, on pause, and on stop. The previous file is kept as `.sav.bak`.
- Save states are this app's own format (`states/<rom id>/slot<N>.state`). A state refuses to load into a
  different game or after the format version changes, and a failed load leaves the running game untouched.
  Loading a state replaces the cartridge RAM, so the next autosave writes that into the `.sav`.
- A game is identified by the CRC-32 of its ROM, so statistics, slots and palette survive renaming the file.
- ROMs stay zipped; the first `.gb` / `.gbc` inside a zip is used.
- No ROMs, saves or copyrighted game data in the repo. Tests use the home-made ROM in `cart::testing`.
- No console maker's names or logos in the UI (the shell says VOSTOK BOY).

## Rules for agents

- `src/tui/tokens.rs`, `chrome.rs` and `pixels.rs` are copies of the files in vostok-files: do not edit or
  reformat them here (`main.rs` marks them `#[rustfmt::skip]`). Format with
  `rustfmt --edition 2021 src/main.rs src/lib.rs`.
- `assets/shell/*.txt` are generated by `tools/cellart.py` from the Figma SVG export; fix the drawing in Figma
  or the converter, not the text file. Figma is the spec: read it, do not change it.
- Changing what the core keeps in a struct means updating that module's `state_fields!` list and bumping
  `state::VERSION` in `core/src/state.rs`; the round-trip test in `machine.rs` catches a missed field only if
  the test ROM exercises it.
- Tests stay offline and write only to temporary folders. For manual runs set `VOSTOK_GAMEBOY_HOME` to a
  scratch folder so the owner's settings and saves are not touched.
- Commit or push only when asked.
