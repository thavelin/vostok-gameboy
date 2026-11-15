//! Vostok design tokens (Figma "Vostok Tokens": Phosphor / Amber CRT modes).
//! Shared by vostok-camera and vostok-radio; keep in sync with vostok-home tokens.py.
//! Only chrome is themed: video pixels and album art are never tinted.

// Shared file: identical in every Vostok app, so not every item is used here.
#![allow(dead_code)]

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeName {
    #[default]
    Phosphor,
    Amber,
}

impl ThemeName {
    pub fn toggled(self) -> Self {
        match self {
            Self::Phosphor => Self::Amber,
            Self::Amber => Self::Phosphor,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Phosphor => "Phosphor",
            Self::Amber => "Amber CRT",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "phosphor" | "green" => Some(Self::Phosphor),
            "amber" | "amber crt" | "amber-crt" | "crt" => Some(Self::Amber),
            _ => None,
        }
    }
}

/// Semantic colors (Figma color/* variables).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: ThemeName,
    pub bg: Color,
    pub surface: Color,
    pub panel: Color,
    pub selection: Color,
    pub highlight: Color,
    pub line_panel: Color,
    pub line_title: Color,
    pub line_focus: Color,
    pub text: Color,
    pub bright: Color,
    pub label: Color,
    pub dim: Color,
    pub muted: Color,
    pub on_accent: Color,
    pub accent: Color,
    pub focus: Color,
    pub warn: Color,
    pub danger: Color,
    pub ok: Color,
    pub level: Color,
    pub track: Color,
    /// Header star (always red, like vostok-home).
    pub star: Color,
}

const fn hex(value: u32) -> Color {
    Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Reader private-mode chrome. Identical in Phosphor and Amber; article ink
/// continues to use the selected theme (Figma Reader Deep Dive, row G).
pub mod private {
    use super::{Color, hex};
    pub const ACCENT: Color = hex(0xb38cff);
    pub const BRIGHT: Color = hex(0xd9c7ff);
    pub const LINE: Color = hex(0x9370e6);
    pub const RULE: Color = hex(0x6f52bf);
    pub const HATCH: Color = hex(0x43307a);
    pub const HEADER: Color = hex(0x22183f);
    pub const TINT: Color = hex(0x120d22);
    pub const ON_ACCENT: Color = hex(0x07050e);
}

pub const PHOSPHOR: Theme = Theme {
    name: ThemeName::Phosphor,
    bg: hex(0x020502),
    surface: hex(0x061006),
    panel: hex(0x0B1A0E),
    selection: hex(0x0C3217),
    highlight: hex(0x0D371A),
    line_panel: hex(0x165C2C),
    line_title: hex(0x2BB456),
    line_focus: hex(0xFFB000),
    text: hex(0xB4F5C6),
    bright: hex(0x3DFF7A),
    label: hex(0x259B4A),
    dim: hex(0x1F7A3D),
    muted: hex(0x808080),
    on_accent: hex(0x020502),
    accent: hex(0x3DFF7A),
    focus: hex(0xFFB000),
    warn: hex(0xFFB000),
    danger: hex(0xFF3B3B),
    ok: hex(0x3DFF7A),
    level: hex(0xB4F5C6),
    track: hex(0x0B1A0E),
    star: hex(0xFF3B3B),
};

pub const AMBER: Theme = Theme {
    name: ThemeName::Amber,
    bg: hex(0x050302),
    surface: hex(0x1A1306),
    panel: hex(0x1A1306),
    selection: hex(0x352702),
    highlight: hex(0x352702),
    line_panel: hex(0x5B4101),
    line_title: hex(0xB37D01),
    line_focus: hex(0x3DE0FF),
    text: hex(0xFFE2A8),
    bright: hex(0xFFB000),
    label: hex(0x9A6C01),
    dim: hex(0xA67300),
    muted: hex(0x808080),
    on_accent: hex(0x050302),
    accent: hex(0xFFB000),
    focus: hex(0x3DE0FF),
    warn: hex(0xFF6E00),
    danger: hex(0xFF3B3B),
    ok: hex(0xFFB000),
    level: hex(0xFFE2A8),
    track: hex(0x1A1306),
    star: hex(0xFF3B3B),
};

pub fn theme(name: ThemeName) -> &'static Theme {
    match name {
        ThemeName::Phosphor => &PHOSPHOR,
        ThemeName::Amber => &AMBER,
    }
}

/// Sparkline glyphs ▁▂▃▄▅▆▇█ for values 0..=1.
pub fn spark(value: f32) -> char {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let index = (value.clamp(0.0, 1.0) * 7.0).round() as usize;
    BARS[index]
}

/// Ten-cell block bar ▮▮▮▯▯ (Figma Block Bar 29:38).
pub fn block_bar(value: f32, cells: usize) -> String {
    let on = (value.clamp(0.0, 1.0) * cells as f32).round() as usize;
    "▮".repeat(on) + &"▯".repeat(cells - on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_match_figma_variables() {
        assert_eq!(PHOSPHOR.bright, Color::Rgb(0x3D, 0xFF, 0x7A));
        assert_eq!(PHOSPHOR.line_focus, Color::Rgb(0xFF, 0xB0, 0x00));
        assert_eq!(AMBER.line_focus, Color::Rgb(0x3D, 0xE0, 0xFF));
        assert_eq!(AMBER.warn, Color::Rgb(0xFF, 0x6E, 0x00));
        assert_eq!(theme(ThemeName::Phosphor.toggled()).name, ThemeName::Amber);
    }

    #[test]
    fn glyph_helpers_are_bounded() {
        assert_eq!(spark(-1.0), '▁');
        assert_eq!(spark(2.0), '█');
        assert_eq!(block_bar(0.7, 10), "▮▮▮▮▮▮▮▯▯▯");
        assert_eq!(block_bar(5.0, 4).chars().count(), 4);
        assert_eq!(ThemeName::parse("Amber CRT"), Some(ThemeName::Amber));
    }
}
