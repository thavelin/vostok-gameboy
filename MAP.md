# Map

One crate, two halves: a **library** (`vostok_gameboy`, the backend) and a **binary**
(`vostok-gameboy`, the front end). The front end never touches the emulator core directly.

```
front end (src/main.rs + UI modules)
   │  Session::start / send / copy_frame / status / stop
   ▼
session.rs ── its own thread: speed, sound, autosave, commands
   ▼
machine.rs ── one Game Boy, one frame per call (no threads, no clock, no files)
   ▼
core/ (gameboy_core, kept in this repo) ── CPU, picture, sound, mappers, snapshots
```

## Backend (library)

| File | What it owns |
| --- | --- |
| `cart.rs` | Reading `.gb` / `.gbc` / `.zip`, the cartridge header (`Header`), `RomImage`, the ROM id (CRC-32). Rejects files that would crash the core. `cart::testing::rom()` is the home-made test ROM. |
| `machine.rs` | `Machine`: `run_frame`, `press`/`release`, `frame`, `save_data`/`load_save`, `save_state`/`load_state`, `reset`. `FRAME_RATE`, `SAMPLE_RATE`. |
| `session.rs` | `Session`, `SessionOptions`, `Command`, `Status`, `State`. The only thing a player screen needs. |
| `frame.rs` | `Frame` (160x144 `Rgb`), `Palette` (grey, dmg, pocket, phosphor, amber), `Screen` (what the core draws into). |
| `input.rs` | `Button`, `Key`, `KeyNames` (the `[keys]` config table), `KeyMap`, `InputTracker` (press/release for terminals with and without key-release events). |
| `saves.rs` | `.sav` files: plain cartridge RAM plus the 48-byte clock footer. `<app dir>/saves/<rom name>.sav`, previous copy kept as `.sav.bak`; the old `.saves/` beside the ROM is still read. |
| `states.rs` | Save-state slots: four per game, `<app dir>/states/<rom id>/slot<N>.state` = time made + 80x72 preview + machine snapshot. `write`, `read`, `list`, `latest`, `clear`, `thumbnail`. |
| `library.rs` | `scan` (ROM folders → `RomEntry` list, with each game's slots and its newest one as `resume`), `Scan::sort` / `Scan::search`, `Stats` (`stats.json`: launches, seconds, last played, per ROM id). |
| `settings.rs` | `Config` ⇄ `config.toml`. Palette per game (`palette_for`, `set_palette_for`, `set_palette_all`). The `[ui]` table is the front end's to use. |
| `audio.rs` | `AudioBuffer` (lock-free ring), `AudioOut` (the sound device). Used by the session; a front end does not need it. |
| `paths.rs` | `app_dir()` = `~/.config/vostok-gameboy` or `VOSTOK_GAMEBOY_HOME`; `write_atomic`. |
| `ctl.rs` | `vostok-gameboy ctl …`: `list`, `info`, `config`, `dirs add/remove`, `states`, `palette`, `run` (headless, `--press`, `--shot`, `--save-state`, `--load-state`). JSON out. |

`core/` is its own crate (`gameboy_core`, Apache-2.0). Its `README.md` lists what was changed; the rule there: a new
field that changes during play goes into that module's `state_fields!` list and `state::VERSION` goes up.

## Front end (binary)

`src/main.rs` plus `src/tui/`. Everything that draws goes through one function, `view::render(buffer, app, clock)`,
so a screen can be drawn into a plain buffer in a test.

| File | What it is |
| --- | --- |
| `main.rs` | Arguments, raw mode / alternate screen (`TerminalGuard`), the loop: tick, read keys, redraw when `app.dirty`, send the picture. |
| `tui/app.rs` | `App`: all state and every key. `Library` (list, search, sort, add folder), `Game` (the running `Session`, pause menu, slot, turbo). No drawing. |
| `tui/layout.rs` | `plan(cols, rows, view, shrink)` -> which shell (`Kind`) and where. Pure. |
| `tui/view.rs` | Draws a `Plan`: the shell, labels, side lists, panels, pause panel, library, help, status line and footer. Returns `Pictures`: where the game picture and thumbnails go. |
| `tui/art.rs` | The shell drawings: parses `assets/shell/*.txt`, colours each layer from the theme, lights held buttons, draws with quarter-block characters. |
| `tui/screen.rs` | The game picture as PNG (pixel terminals) or half-blocks. |
| `tui/terminal.rs` | Pixels or half-blocks, decided from environment variables only. |
| `tui/tokens.rs`, `chrome.rs`, `pixels.rs` | **Shared with the other Vostok Rust apps (vostok-files): keep them byte-identical**, change them there first. Themes, header / footer / frame helpers, Kitty graphics. |

### The shell drawings

`assets/shell/<name>.txt` is generated, not hand-edited: `python tools/cellart.py IN.svg SUBCELL_W SUBCELL_H OUT.txt`
turns the designer's SVG (one path per colour layer, exported from the Figma file) into a grid of layer letters,
two by two per terminal cell, followed by a `parts` grid that says which cells belong to which button
(U D L R A B, E select, T start). `art.rs` lists what each letter means. The picture goes in the largest
all-`d` (deep) block of the drawing, at the size `view::shell` gives for that drawing.

To change a colour: `art::color`. To move a label: the `*_LABELS` tables in `view.rs`. To add a size: an asset,
a row in `layout::SHELLS`, an arm in `view::shell`.

### Looking at a screen without a terminal

`vostok-gameboy [ROM] --html 140x39 [--frames N] > out.html` draws one screen and prints it as HTML (not in
`--help`; a development aid). `VOSTOK_GAMEBOY_KEYS="esc down f2"` presses keys first (esc, enter, down, right,
f2, f3, f5 or a character).

## A player screen, in short

```rust
let app_dir = paths::app_dir();
let config = Config::load(&app_dir)?;                 // Err = config.toml has a typo
let mut games = library::scan(&config.rom_dirs, &config.saves_dir(&app_dir), &config.states_dir(&app_dir), &Stats::load(&app_dir));
games.sort(Sort::Recent);                             // games.search("tet") for the search box
let game = &games.entries[0];                         // game.states = the 4 slots, game.resume = newest slot
let session = Session::start(cart::load(&game.path)?, SessionOptions::for_game(&config, &app_dir, &game.id))?;
// "r resume slot N": session.send(Command::LoadState(game.resume.unwrap().slot)) right after start
let keys = KeyMap::new(&config.keys);
let mut input = InputTracker::default();
let (mut frame, mut seen) = (Frame::default(), 0u64);

// every tick (a few ms):
for edge in input.expire(Instant::now()) { session.send(edge.into()); }
// on a key event:  keys.lookup(code) → input.press(button, now) / input.release(button) → session.send(edge.into())
if session.copy_frame(&mut frame, &mut seen) { /* draw `frame` */ }
let status = session.status();                        // fps, frame_ms, load, frames, speed, palette, muted, unsaved, saved_at, slots, note, state
// controls: session.send(Command::TogglePause | Speed(4) | Palette(p) | Mute(true) | Volume(0.5) | Reset | Save)
// save states: Command::SaveState(n) | LoadState(n) | ClearState(n), n = 1..=4; status.slots shows the result
// slot preview picture: states::read(&config.states_dir(&app_dir), session.id(), n)?.unwrap().thumbnail (an 80x72 Frame)
// palette menu: Palette::ALL + .label(); keep the choice with config.set_palette_for(id, p) or set_palette_all(p), then config.save
let last = session.stop();                            // writes the save; last.note holds any problem
```

## Where things go

- **A new control for a running game** → a `Command` variant and its arm in `Worker::apply` (`session.rs`),
  a `Status` field if it has state to show, and a test.
- **Something to show per game in the list** → a field on `RomEntry` (`library.rs`); it appears in `ctl list` for free.
- **A new setting** → `Config` in `settings.rs` with a default. Front-end-only settings go in `[ui]` instead.
- **A new palette** → `Palette` in `frame.rs` (`ALL`, `name`, `shades`).
- **Anything that draws** → the binary, never the library.
