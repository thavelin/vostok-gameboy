# gameboy_core (Vostok Gameboy copy)

The emulator core: CPU, picture, sound, memory and cartridge mappers.

This is [gameboy_core](https://crates.io/crates/gameboy_core) 0.3.3 by Ben Konz
(<https://github.com/benkonz/gameboy_emulator>), licensed under Apache-2.0 (see `LICENSE`),
copied into this repository so it can be changed.

## Changes from the original

- **Save states** (`src/state.rs`, `Gameboy::save_state` / `Gameboy::load_state`): a snapshot of the whole
  machine. Each module lists its saved fields with `state_fields!` at the end of its file. A new field that
  changes during play must be added to that list, and `state::VERSION` bumped.
- **No lost frames**: when a frame ended on the same CPU step that a sound block filled, the original reported
  only the sound block and the frame ran on into the next one. The frame end is now reported on the next call.
