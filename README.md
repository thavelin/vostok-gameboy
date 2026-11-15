# Vostok Gameboy

A Game Boy and Game Boy Color emulator for the terminal, drawn as a handheld: the game runs in the screen of a
green **VOSTOK BOY** shell whose d-pad and buttons light up as you press the keys.

> A fork of [terminal_gameboy](https://github.com/dquigles/terminal_gameboy) by Dillon Quigley, rebuilt as a Vostok
> terminal app. How the code is laid out is in [MAP.md](MAP.md); current state and limits are in
> [CONTEXT.md](CONTEXT.md). The emulator itself is [gameboy_core](https://crates.io/crates/gameboy_core), kept in
> `core/` with save states added (see `core/README.md`).

## Features

- **The shell** — five drawings, picked to fit the terminal: 140x39 (wide, key lists at the sides), 70x60 (tall),
  100x28, 80x24, and 200x56 with information panels (F2). Too small for any of them, the game pauses and says
  so; F2 switches to the screen alone, which fits down to 20x10.
- **The picture** — real pixels in terminals with Kitty graphics (Kitty, Ghostty, WezTerm, Konsole, herdr);
  half-blocks everywhere else.
- **Library** — every `.gb`, `.gbc` and `.zip` in your ROM folders, with search, sort, play time and save slots.
- **Save states** — four slots per game with a preview picture; battery saves as standard `.sav` files.
- **Sound**, turbo, five palettes for four-shade games (kept per game), two themes (phosphor, amber).
- **JSON command line** for scripts and agents (`vostok-gameboy ctl`).

## Install

Needs [Rust](https://rustup.rs).

```bash
./setup.sh
```

On Windows run `setup.cmd`. Then start it with `./run.sh` or `run.cmd`; both rebuild first when the source changed.

## Use

```bash
vostok-gameboy
```

opens the library. The first time, press `d` and type the folder your games are in.

```bash
vostok-gameboy Tetris.zip
```

plays one game directly.

| Option | |
| --- | --- |
| `--mute` | No sound |
| `--palette NAME` | grey, dmg, pocket, phosphor, amber (four-shade games) |
| `--theme NAME` | phosphor or amber |
| `--pixels` / `--text` | Force Kitty graphics / half-blocks (`VOSTOK_GAMEBOY_RENDER=pixels` or `text` does the same) |

## Keys

Playing:

| Key | |
| --- | --- |
| Arrows / WASD | D-pad |
| Z / K | A |
| X / J | B |
| Enter | Start |
| Backspace / Space | Select |
| P / Esc | Pause menu (resume, save, load, palette, controls, reset, quit) |
| F5 / F8 | Save / load the selected slot |
| Tab | Turbo x4: tap to switch, hold to use while held |
| M | Mute |
| F6 | Next palette |
| F2 | View: shell, shell + panels, screen only |
| - / + | Smaller / larger shell |
| F3 | Theme |
| ? | All keys |
| Q | Back to the library |

In the pause menu: up / down choose, left / right or 1-4 pick the slot, Enter chooses, Del clears the slot.

Library: up / down, Enter play, `r` resume the newest save state, `/` search, `s` sort, `d` add a folder,
`p` palette, `i` cartridge details, `q` quit.

The eight buttons are set in the `[keys]` table of `~/.config/vostok-gameboy/config.toml`.

## JSON command line

For scripts and agents; see `vostok-gameboy ctl help`.

```bash
vostok-gameboy ctl dirs add ~/Games
vostok-gameboy ctl list
vostok-gameboy ctl run Tetris.zip --frames 600 --press start@300 --shot tetris.png
vostok-gameboy ctl run Tetris.zip --frames 600 --save-state 1
vostok-gameboy ctl run Tetris.zip --load-state 1 --frames 60
vostok-gameboy ctl states Tetris.zip
vostok-gameboy ctl palette Tetris.zip amber
```

## Terminal

- 24-bit colour and a monospace font. The shell is drawn with quarter-block characters, so a font that draws
  those edge to edge (most do) looks best.
- 140x39 or larger for the full shell; it steps down to smaller drawings by itself.
- Terminals that report key releases (Kitty, Ghostty, WezTerm, Windows Terminal) give exact button holds;
  elsewhere a held key is released a moment after its last repeat.

## ROM files

You provide your own legally obtained `.gb` / `.gbc` files, loose or inside a `.zip`.

## Saves

Games with a battery save to `~/.config/vostok-gameboy/saves/<rom name>.sav` (a standard `.sav`). The save is
written shortly after the game saves, when you pause, and on exit; the previous file is kept as `.sav.bak`.
Saves from the original app (`.saves/` next to the ROM) are picked up automatically.

Save states are separate: `~/.config/vostok-gameboy/states/<rom id>/slot<N>.state`, four slots per game. They
only load into this app, and only into the game they were made with.

## License

MIT License — see [LICENSE](LICENSE) for details.

## Acknowledgments

- [terminal_gameboy](https://github.com/dquigles/terminal_gameboy) by Dillon Quigley — the app this started from
- [gameboy_core](https://crates.io/crates/gameboy_core) by Ben Konz — Game Boy emulation (Apache-2.0, copied into `core/`)
- [ratatui](https://crates.io/crates/ratatui) and [crossterm](https://crates.io/crates/crossterm) — terminal drawing and input
- [rodio](https://crates.io/crates/rodio) — sound
