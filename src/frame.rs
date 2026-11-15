//! The picture: 160x144 RGB pixels, plus the palettes used for original
//! (four-shade) Game Boy games. Colour games bring their own colours.

use gameboy_core::{CGBColor, Color as Shade, PixelMapper};
use serde::{Deserialize, Serialize};

pub const GB_WIDTH: usize = 160;
pub const GB_HEIGHT: usize = 144;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Brightness 0..=255 (integer form of 0.299 R + 0.587 G + 0.114 B).
    #[inline]
    pub fn to_grey(self) -> u8 {
        ((self.r as u32 * 77 + self.g as u32 * 150 + self.b as u32 * 29) >> 8) as u8
    }
}

/// One finished frame. `pixels` is row-major, `width * height` long.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
}

/// The name the original renderer uses.
pub type FrameBuffer = Frame;

impl Default for Frame {
    fn default() -> Self {
        Self::new(GB_WIDTH, GB_HEIGHT)
    }
}

impl Frame {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![Rgb::default(); width * height],
        }
    }

    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> Rgb {
        self.pixels[y * self.width + x]
    }

    #[inline]
    pub fn set_pixel(&mut self, index: usize, color: Rgb) {
        if let Some(pixel) = self.pixels.get_mut(index) {
            *pixel = color;
        }
    }

    /// `r, g, b` bytes, row by row: the layout image encoders want.
    pub fn to_rgb_bytes(&self) -> Vec<u8> {
        self.pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
    }

    /// CRC-32 of the pixels: a cheap way to tell two frames apart in tests.
    pub fn checksum(&self) -> u32 {
        crc32fast::hash(&self.to_rgb_bytes())
    }
}

/// Colours for the four shades of an original Game Boy game, lightest first.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Palette {
    /// Plain greys.
    #[default]
    Grey,
    /// The pea-green of the 1989 screen.
    Dmg,
    /// Game Boy Pocket: cooler, higher contrast.
    Pocket,
    /// Green phosphor terminal.
    Phosphor,
    /// Amber monitor.
    Amber,
}

impl Palette {
    pub const ALL: [Palette; 5] = [
        Palette::Grey,
        Palette::Dmg,
        Palette::Pocket,
        Palette::Phosphor,
        Palette::Amber,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Palette::Grey => "grey",
            Palette::Dmg => "dmg",
            Palette::Pocket => "pocket",
            Palette::Phosphor => "phosphor",
            Palette::Amber => "amber",
        }
    }

    /// The name to show in a menu.
    pub fn label(self) -> &'static str {
        match self {
            Palette::Grey => "GREY",
            Palette::Dmg => "GB GREEN",
            Palette::Pocket => "POCKET GREY",
            Palette::Phosphor => "PHOSPHOR",
            Palette::Amber => "AMBER",
        }
    }

    pub fn parse(name: &str) -> Option<Palette> {
        let name = name.trim().to_ascii_lowercase();
        Palette::ALL
            .into_iter()
            .find(|palette| palette.name() == name)
            .or(match name.as_str() {
                "gray" => Some(Palette::Grey),
                "green" => Some(Palette::Dmg),
                _ => None,
            })
    }

    pub fn next(self) -> Palette {
        let index = Palette::ALL
            .iter()
            .position(|&palette| palette == self)
            .unwrap_or(0);
        Palette::ALL[(index + 1) % Palette::ALL.len()]
    }

    pub fn shades(self) -> [Rgb; 4] {
        match self {
            Palette::Grey => [
                Rgb::new(255, 255, 255),
                Rgb::new(170, 170, 170),
                Rgb::new(85, 85, 85),
                Rgb::new(0, 0, 0),
            ],
            Palette::Dmg => [
                Rgb::new(155, 188, 15),
                Rgb::new(139, 172, 15),
                Rgb::new(48, 98, 48),
                Rgb::new(15, 56, 15),
            ],
            Palette::Pocket => [
                Rgb::new(196, 207, 161),
                Rgb::new(139, 149, 109),
                Rgb::new(77, 83, 60),
                Rgb::new(31, 31, 31),
            ],
            Palette::Phosphor => [
                Rgb::new(120, 255, 150),
                Rgb::new(51, 190, 90),
                Rgb::new(16, 104, 48),
                Rgb::new(2, 20, 10),
            ],
            Palette::Amber => [
                Rgb::new(255, 196, 64),
                Rgb::new(204, 136, 24),
                Rgb::new(112, 64, 8),
                Rgb::new(22, 12, 2),
            ],
        }
    }
}

/// What the emulator core draws into.
pub struct Screen {
    pub frame: Frame,
    shades: [Rgb; 4],
    palette: Palette,
}

impl Screen {
    pub fn new(palette: Palette) -> Self {
        Self {
            frame: Frame::default(),
            shades: palette.shades(),
            palette,
        }
    }

    pub fn palette(&self) -> Palette {
        self.palette
    }

    /// Takes effect from the next frame drawn.
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        self.shades = palette.shades();
    }
}

impl PixelMapper for Screen {
    fn map_pixel(&mut self, pixel: usize, shade: Shade) {
        let index = match shade {
            Shade::White => 0,
            Shade::LightGray => 1,
            Shade::DarkGray => 2,
            Shade::Black => 3,
        };
        self.frame.set_pixel(pixel, self.shades[index]);
    }

    fn cgb_map_pixel(&mut self, pixel: usize, color: CGBColor) {
        self.frame
            .set_pixel(pixel, Rgb::new(color.red, color.green, color.blue));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_parse_by_name_and_cycle() {
        for palette in Palette::ALL {
            assert_eq!(Palette::parse(palette.name()), Some(palette));
            // Lightest first, darkest last.
            let shades = palette.shades();
            assert!(shades[0].to_grey() > shades[3].to_grey());
        }
        assert_eq!(Palette::parse(" GRAY "), Some(Palette::Grey));
        assert_eq!(Palette::parse("neon"), None);
        assert_eq!(Palette::Amber.next(), Palette::Grey);
    }

    #[test]
    fn screen_maps_shades_through_the_palette() {
        let mut screen = Screen::new(Palette::Dmg);
        screen.map_pixel(0, Shade::Black);
        screen.map_pixel(usize::MAX, Shade::Black); // out of range: ignored
        assert_eq!(screen.frame.get_pixel(0, 0), Palette::Dmg.shades()[3]);
        assert_eq!(screen.frame.to_rgb_bytes().len(), GB_WIDTH * GB_HEIGHT * 3);
    }
}
