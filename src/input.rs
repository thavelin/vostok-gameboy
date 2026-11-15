//! Buttons, key names, the key map and press/release tracking.
//!
//! Terminals differ: some report a key being let go, many only repeat it while
//! it is held. [`InputTracker`] hides that. Feed it every press, repeat and
//! release with the time; it answers with button edges to pass to the session.

use crossterm::event::KeyCode;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    Start,
    Select,
}

impl Button {
    pub const ALL: [Button; 8] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::A,
        Button::B,
        Button::Start,
        Button::Select,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Button::Up => "up",
            Button::Down => "down",
            Button::Left => "left",
            Button::Right => "right",
            Button::A => "a",
            Button::B => "b",
            Button::Start => "start",
            Button::Select => "select",
        }
    }

    pub fn parse(name: &str) -> Option<Button> {
        let name = name.trim().to_ascii_lowercase();
        Button::ALL.into_iter().find(|button| button.name() == name)
    }
}

impl From<Button> for gameboy_core::Button {
    fn from(button: Button) -> Self {
        match button {
            Button::Up => gameboy_core::Button::Up,
            Button::Down => gameboy_core::Button::Down,
            Button::Left => gameboy_core::Button::Left,
            Button::Right => gameboy_core::Button::Right,
            Button::A => gameboy_core::Button::A,
            Button::B => gameboy_core::Button::B,
            Button::Start => gameboy_core::Button::Start,
            Button::Select => gameboy_core::Button::Select,
        }
    }
}

/// A key as written in `config.toml`: `"up"`, `"enter"`, `"z"`, `"f5"`.
/// Letters are stored lower-case, so Shift and Caps Lock do not matter.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Space,
    Backspace,
    Tab,
    Esc,
    F(u8),
}

impl Key {
    pub fn parse(name: &str) -> Option<Key> {
        let lower = name.trim().to_lowercase();
        Some(match lower.as_str() {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "enter" | "return" => Key::Enter,
            "space" => Key::Space,
            "backspace" | "bksp" => Key::Backspace,
            "tab" => Key::Tab,
            "esc" | "escape" => Key::Esc,
            _ => {
                let mut chars = lower.chars();
                match (chars.next(), chars.as_str()) {
                    (Some(only), "") if !only.is_control() && only != ' ' => Key::Char(only),
                    (Some('f'), digits) => {
                        Key::F(digits.parse().ok().filter(|n| (1..=24).contains(n))?)
                    }
                    _ => return None,
                }
            }
        })
    }

    pub fn name(self) -> String {
        match self {
            Key::Char(c) => c.to_string(),
            Key::Up => "up".into(),
            Key::Down => "down".into(),
            Key::Left => "left".into(),
            Key::Right => "right".into(),
            Key::Enter => "enter".into(),
            Key::Space => "space".into(),
            Key::Backspace => "backspace".into(),
            Key::Tab => "tab".into(),
            Key::Esc => "esc".into(),
            Key::F(n) => format!("f{n}"),
        }
    }

    pub fn from_code(code: KeyCode) -> Option<Key> {
        Some(match code {
            KeyCode::Char(' ') => Key::Space,
            KeyCode::Char(c) => Key::Char(c.to_lowercase().next().unwrap_or(c)),
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Tab => Key::Tab,
            KeyCode::Esc => Key::Esc,
            KeyCode::F(n) => Key::F(n),
            _ => return None,
        })
    }
}

/// The `[keys]` table of `config.toml`: key names for each button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyNames {
    pub up: Vec<String>,
    pub down: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub a: Vec<String>,
    pub b: Vec<String>,
    pub start: Vec<String>,
    pub select: Vec<String>,
}

impl Default for KeyNames {
    fn default() -> Self {
        let keys = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        Self {
            up: keys(&["up", "w"]),
            down: keys(&["down", "s"]),
            left: keys(&["left", "a"]),
            right: keys(&["right", "d"]),
            a: keys(&["z", "k"]),
            b: keys(&["x", "j"]),
            start: keys(&["enter"]),
            select: keys(&["backspace", "space"]),
        }
    }
}

impl KeyNames {
    pub fn for_button(&self, button: Button) -> &[String] {
        match button {
            Button::Up => &self.up,
            Button::Down => &self.down,
            Button::Left => &self.left,
            Button::Right => &self.right,
            Button::A => &self.a,
            Button::B => &self.b,
            Button::Start => &self.start,
            Button::Select => &self.select,
        }
    }
}

/// Key to button lookup built from [`KeyNames`].
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    bindings: Vec<(Key, Button)>,
    /// Names in the config that are not keys, as `button: name`.
    pub unknown: Vec<String>,
}

impl KeyMap {
    pub fn new(names: &KeyNames) -> Self {
        let mut map = KeyMap::default();
        for button in Button::ALL {
            for name in names.for_button(button) {
                match Key::parse(name) {
                    // The first button to claim a key keeps it.
                    Some(key) if map.button(key).is_none() => map.bindings.push((key, button)),
                    Some(_) => {}
                    None => map.unknown.push(format!("{}: {name}", button.name())),
                }
            }
        }
        map
    }

    pub fn button(&self, key: Key) -> Option<Button> {
        self.bindings
            .iter()
            .find(|(bound, _)| *bound == key)
            .map(|(_, button)| *button)
    }

    pub fn lookup(&self, code: KeyCode) -> Option<Button> {
        self.button(Key::from_code(code)?)
    }

    /// The keys bound to a button, for drawing key hints.
    pub fn keys(&self, button: Button) -> Vec<Key> {
        self.bindings
            .iter()
            .filter(|(_, bound)| *bound == button)
            .map(|(key, _)| *key)
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Pressed(Button),
    Released(Button),
}

/// Turns terminal key events into clean button presses and releases.
#[derive(Debug)]
pub struct InputTracker {
    last_seen: [Option<Instant>; 8],
    sees_releases: bool,
    hold: Duration,
}

impl Default for InputTracker {
    fn default() -> Self {
        Self::new(Duration::from_millis(150))
    }
}

impl InputTracker {
    /// `hold` is how long a button stays down after the last press or repeat
    /// in a terminal that never reports releases. It has to outlast the gap
    /// between key repeats; 150 ms does on usual settings.
    pub fn new(hold: Duration) -> Self {
        Self {
            last_seen: [None; 8],
            sees_releases: false,
            hold,
        }
    }

    /// A press or repeat event. Returns the edge when the button was up before.
    pub fn press(&mut self, button: Button, now: Instant) -> Option<Edge> {
        let was_up = self.last_seen[button.index()].replace(now).is_none();
        was_up.then_some(Edge::Pressed(button))
    }

    /// A release event. Seeing one switches the hold timer off for good.
    pub fn release(&mut self, button: Button) -> Option<Edge> {
        self.sees_releases = true;
        self.last_seen[button.index()]
            .take()
            .map(|_| Edge::Released(button))
    }

    /// Call every tick: releases buttons whose hold ran out.
    pub fn expire(&mut self, now: Instant) -> Vec<Edge> {
        if self.sees_releases {
            return Vec::new();
        }
        let hold = self.hold;
        self.release_where(|seen| now.saturating_duration_since(seen) > hold)
    }

    /// Lets go of everything, e.g. when the terminal loses focus or a menu opens.
    pub fn release_all(&mut self) -> Vec<Edge> {
        self.release_where(|_| true)
    }

    fn release_where(&mut self, due: impl Fn(Instant) -> bool) -> Vec<Edge> {
        let mut edges = Vec::new();
        for button in Button::ALL {
            let slot = &mut self.last_seen[button.index()];
            if slot.is_some_and(&due) {
                *slot = None;
                edges.push(Edge::Released(button));
            }
        }
        edges
    }

    pub fn is_down(&self, button: Button) -> bool {
        self.last_seen[button.index()].is_some()
    }

    /// True once the terminal has shown it reports key releases.
    pub fn sees_releases(&self) -> bool {
        self.sees_releases
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_round_trip() {
        for name in [
            "up",
            "enter",
            "space",
            "backspace",
            "tab",
            "esc",
            "z",
            "f5",
            "7",
            ";",
        ] {
            assert_eq!(Key::parse(name).unwrap().name(), name);
        }
        assert_eq!(Key::parse("Z"), Some(Key::Char('z')));
        assert_eq!(Key::parse("F12"), Some(Key::F(12)));
        assert_eq!(Key::parse("f"), Some(Key::Char('f')));
        for bad in ["", "f99", "ctrl+x", "zz"] {
            assert_eq!(Key::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn default_map_covers_every_button_and_ignores_case() {
        let map = KeyMap::new(&KeyNames::default());
        assert!(map.unknown.is_empty());
        for button in Button::ALL {
            assert!(!map.keys(button).is_empty(), "{button:?} has no key");
        }
        assert_eq!(map.lookup(KeyCode::Char('Z')), Some(Button::A));
        assert_eq!(map.lookup(KeyCode::Char('w')), Some(Button::Up));
        assert_eq!(map.lookup(KeyCode::Up), Some(Button::Up));
        assert_eq!(map.lookup(KeyCode::Backspace), Some(Button::Select));
        assert_eq!(map.lookup(KeyCode::Char('q')), None);
    }

    #[test]
    fn bad_names_are_reported_and_first_binding_wins() {
        let names = KeyNames {
            a: vec!["z".into(), "nope".into()],
            b: vec!["z".into(), "x".into()],
            ..Default::default()
        };
        let map = KeyMap::new(&names);
        assert_eq!(map.unknown, ["a: nope"]);
        assert_eq!(map.button(Key::Char('z')), Some(Button::A));
        assert_eq!(map.keys(Button::B), [Key::Char('x')]);
    }

    #[test]
    fn without_release_events_a_button_lets_go_after_the_hold() {
        let mut input = InputTracker::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);

        assert_eq!(
            input.press(Button::A, at(0)),
            Some(Edge::Pressed(Button::A))
        );
        assert_eq!(input.press(Button::A, at(100)), None); // a repeat
        assert!(input.expire(at(200)).is_empty());
        assert_eq!(input.expire(at(260)), [Edge::Released(Button::A)]);
        assert!(!input.is_down(Button::A));
        assert_eq!(
            input.press(Button::A, at(300)),
            Some(Edge::Pressed(Button::A))
        );
    }

    #[test]
    fn with_release_events_the_hold_timer_is_off() {
        let mut input = InputTracker::default();
        let start = Instant::now();
        input.press(Button::Left, start);
        input.press(Button::B, start);
        assert_eq!(input.release(Button::B), Some(Edge::Released(Button::B)));
        assert_eq!(input.release(Button::B), None);
        assert!(input.sees_releases());
        assert!(input.expire(start + Duration::from_secs(5)).is_empty());
        assert!(input.is_down(Button::Left));
        assert_eq!(input.release_all(), [Edge::Released(Button::Left)]);
    }
}
