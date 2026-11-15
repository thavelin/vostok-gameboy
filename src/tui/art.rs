//! The Game Boy body, drawn in terminal cells.
//!
//! The designer's "cell art" (`assets/shell/*.txt`, made by `tools/cellart.py`
//! from the Figma file) is a grid of sub-cells, two across and two down per
//! terminal cell, one letter per colour layer. Each cell is drawn as one
//! quadrant glyph with two colours. A second grid names the parts that move
//! (d-pad arms, buttons) so they can be drawn pressed.

use crate::tokens::ThemeName;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier},
};
use std::sync::LazyLock;
use vostok_gameboy::input::Button;

pub struct Art {
    w: usize,
    h: usize,
    cells: Vec<u8>,
    parts: Vec<u8>,
    hole: Rect,
}

macro_rules! art {
    ($name:ident, $file:literal) => {
        pub static $name: LazyLock<Art> = LazyLock::new(|| {
            Art::parse(include_str!(concat!("../../assets/shell/", $file, ".txt")))
        });
    };
}
art!(LARGE, "large");
art!(WIDE, "wide");
art!(TALL, "tall");
art!(COMPACT, "compact");
art!(SMALL, "small");
art!(MINI, "mini");
art!(PAUSE, "pause");
art!(CARTRIDGE, "cartridge");

impl Art {
    pub fn parse(text: &str) -> Art {
        let mut lines = text.lines();
        let mut size = lines
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .skip(1)
            .map(|v| v.parse().unwrap_or(0));
        let (w, h) = (size.next().unwrap_or(0), size.next().unwrap_or(0));
        let grid = |lines: &mut std::str::Lines| {
            let mut out = Vec::with_capacity(w * h);
            for _ in 0..h {
                let row = lines.next().unwrap_or_default().as_bytes();
                out.extend((0..w).map(|x| row.get(x).copied().unwrap_or(b'.')));
            }
            out
        };
        let cells = grid(&mut lines);
        lines.next(); // "parts"
        let parts = grid(&mut lines);
        let mut art = Art {
            w,
            h,
            cells,
            parts,
            hole: Rect::default(),
        };
        art.hole = art.find_hole();
        art
    }

    /// Size in terminal cells.
    #[cfg(test)]
    pub fn cols(&self) -> u16 {
        (self.w / 2) as u16
    }
    #[cfg(test)]
    pub fn rows(&self) -> u16 {
        (self.h / 2) as u16
    }

    fn deep(&self, cx: usize, cy: usize) -> bool {
        (0..4).all(|q| self.cells[(cy * 2 + q / 2) * self.w + cx * 2 + q % 2] == b'd')
    }

    /// Where the picture goes: the largest block of cells that are all "deep"
    /// (the dark glass), in cells from the art's corner.
    pub fn hole(&self) -> Rect {
        self.hole
    }

    fn find_hole(&self) -> Rect {
        let (cols, rows) = (self.w / 2, self.h / 2);
        let mut seen = vec![false; cols * rows];
        let mut best = (0usize, Rect::default());
        for start in 0..cols * rows {
            if seen[start] || !self.deep(start % cols, start / cols) {
                continue;
            }
            let (mut x0, mut x1, mut y0, mut y1, mut count) = (cols, 0, rows, 0, 0);
            let mut stack = vec![start];
            seen[start] = true;
            while let Some(at) = stack.pop() {
                let (x, y) = (at % cols, at / cols);
                (x0, x1, y0, y1, count) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y), count + 1);
                let near = [
                    (x + 1, y),
                    (x.wrapping_sub(1), y),
                    (x, y + 1),
                    (x, y.wrapping_sub(1)),
                ];
                for (nx, ny) in near {
                    if nx < cols && ny < rows && !seen[ny * cols + nx] && self.deep(nx, ny) {
                        seen[ny * cols + nx] = true;
                        stack.push(ny * cols + nx);
                    }
                }
            }
            if count > best.0 {
                best = (
                    count,
                    Rect::new(
                        x0 as u16,
                        y0 as u16,
                        (x1 - x0 + 1) as u16,
                        (y1 - y0 + 1) as u16,
                    ),
                );
            }
        }
        best.1
    }
}

/// The body colour variables of the Figma file ("Vostok Boy Body"), in the
/// order of `LETTERS`, then label.
const LETTERS: &[u8] = b"sbhlzdABLpPmnNoSk";
const PHOSPHOR: [u32; 18] = [
    0x010A04, 0x1F7A3D, 0x4FC274, 0x145A2B, 0x0E3A1C, 0x031508, 0x3DFF7A, 0x2A9E50, 0xFF4D4D,
    0x07210F, 0x2E6A40, 0x020E05, 0x2BC45C, 0x9CFFBC, 0x126B31, 0x08301A, 0x062612, 0x6FE896,
];
const AMBER: [u32; 18] = [
    0x060300, 0xA86A00, 0xEDB04A, 0x764A00, 0x3D2600, 0x140C00, 0xFFB000, 0xB87400, 0xFF5A1F,
    0x1E1200, 0x6A4300, 0x0D0800, 0xF09A00, 0xFFD98F, 0x8A5500, 0x2E1C00, 0x241600, 0xFFC966,
];

fn table(theme: ThemeName) -> &'static [u32; 18] {
    match theme {
        ThemeName::Phosphor => &PHOSPHOR,
        ThemeName::Amber => &AMBER,
    }
}

fn rgb(value: u32) -> Color {
    Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// The colour of one layer letter (`b'b'` body, `b'k'` ink, ...).
pub fn color(theme: ThemeName, letter: u8) -> Color {
    let index = LETTERS.iter().position(|l| *l == letter).unwrap_or(1);
    rgb(table(theme)[index])
}

/// Text printed on the bezel ("DOT MATRIX", "BATT").
pub fn label(theme: ThemeName) -> Color {
    rgb(table(theme)[17])
}

/// What changes from frame to frame.
pub struct Paint {
    pub theme: ThemeName,
    /// Buttons held right now, by `Button::index()`.
    pub down: [bool; 8],
    /// The power light.
    pub led: bool,
}

fn part_button(part: u8) -> Option<Button> {
    Some(match part {
        b'U' => Button::Up,
        b'D' => Button::Down,
        b'L' => Button::Left,
        b'R' => Button::Right,
        b'A' => Button::A,
        b'B' => Button::B,
        b'E' => Button::Select,
        b'T' => Button::Start,
        _ => return None,
    })
}

/// A held part: the dark rubber of the d-pad and pills lights up, the bright
/// A and B caps sink into shade.
fn pressed(letter: u8, part: u8) -> u8 {
    match (part, letter) {
        (b'A' | b'B', b'n') => b'o',
        (b'A' | b'B', b'N') => b'n',
        (b'A' | b'B', b'o') => b'm',
        (_, b'p') => b'P',
        (_, b'P') => b'n',
        (_, b'm') => b'p',
        _ => letter,
    }
}

/// Quadrant glyphs by which sub-cells are in the foreground colour:
/// bit 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
const QUADRANTS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

fn distance(a: Color, b: Color) -> u32 {
    match (a, b) {
        (Color::Rgb(r, g, b), Color::Rgb(x, y, z)) => [(r, x), (g, y), (b, z)]
            .iter()
            .map(|(p, q)| (i32::from(*p) - i32::from(*q)).pow(2) as u32)
            .sum(),
        _ => u32::from(a != b),
    }
}

/// Draw `art` with its corner at (`ox`, `oy`), which may be off screen; only
/// cells inside `clip` are touched, and cells with no art are left alone.
pub fn draw(
    buf: &mut Buffer,
    art: &Art,
    ox: i32,
    oy: i32,
    clip: Rect,
    paint: &Paint,
    empty: Color,
) {
    let clip = clip.intersection(buf.area);
    for cy in 0..art.h / 2 {
        let y = oy + cy as i32;
        if y < i32::from(clip.y) || y >= i32::from(clip.bottom()) {
            continue;
        }
        for cx in 0..art.w / 2 {
            let x = ox + cx as i32;
            if x < i32::from(clip.x) || x >= i32::from(clip.right()) {
                continue;
            }
            let mut blank = true;
            let colors: [Color; 4] = std::array::from_fn(|q| {
                let at = (cy * 2 + q / 2) * art.w + cx * 2 + q % 2;
                let mut letter = art.cells[at];
                if letter == b'.' {
                    return empty;
                }
                blank = false;
                if letter == b'L' && !paint.led {
                    letter = b'z';
                }
                if part_button(art.parts[at]).is_some_and(|button| paint.down[button.index()]) {
                    letter = pressed(letter, art.parts[at]);
                }
                color(paint.theme, letter)
            });
            if blank {
                continue;
            }
            // Two colours per cell: the most common is the background, the
            // next the foreground, and any third joins whichever is nearer.
            let count = |c: Color| colors.iter().filter(|other| **other == c).count();
            let bg = *colors.iter().max_by_key(|c| count(**c)).unwrap_or(&empty);
            let fg = colors
                .iter()
                .filter(|c| **c != bg)
                .max_by_key(|c| count(**c))
                .copied();
            let cell = &mut buf[(x as u16, y as u16)];
            cell.reset();
            cell.set_bg(bg);
            if let Some(fg) = fg {
                let mask = (0..4)
                    .filter(|q| {
                        colors[*q] != bg && distance(colors[*q], fg) <= distance(colors[*q], bg)
                    })
                    .fold(0, |mask, q| mask | 1 << q);
                cell.set_symbol(QUADRANTS[mask]).set_fg(fg);
            } else {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Print on the body: the glyph takes the cell, and the cell keeps whichever
/// of its two colours covered more of it as the background.
pub fn print(buf: &mut Buffer, x: i32, y: i32, text: &str, fg: Color, bold: bool, clip: Rect) {
    let clip = clip.intersection(buf.area);
    for (index, ch) in text.chars().enumerate() {
        let x = x + index as i32;
        if x < i32::from(clip.x)
            || x >= i32::from(clip.right())
            || y < i32::from(clip.y)
            || y >= i32::from(clip.bottom())
        {
            continue;
        }
        let cell = &mut buf[(x as u16, y as u16)];
        let covered = QUADRANTS
            .iter()
            .position(|glyph| *glyph == cell.symbol())
            .map_or(0, |mask| mask.count_ones());
        let bg = if covered > 2 { cell.fg } else { cell.bg };
        cell.reset();
        cell.set_char(ch).set_fg(fg).set_bg(bg);
        if bold {
            cell.modifier = Modifier::BOLD;
        }
    }
}

/// Fade everything in `area` towards `to`, keeping `keep` (0..1) of each colour.
pub fn dim(buf: &mut Buffer, area: Rect, to: Color, keep: f32) {
    let Color::Rgb(tr, tg, tb) = to else { return };
    let mix = |c: Color| match c {
        Color::Rgb(r, g, b) => {
            let blend = |from: u8, to: u8| {
                (f32::from(from) * keep + f32::from(to) * (1.0 - keep)).round() as u8
            };
            Color::Rgb(blend(r, tr), blend(g, tg), blend(b, tb))
        }
        Color::Reset => to,
        other => other,
    };
    let area = area.intersection(buf.area);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = mix(cell.fg);
            cell.bg = mix(cell.bg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_planner_knows_where_each_body_is() {
        use crate::layout::{bodies, Kind};
        for (kind, body) in bodies() {
            let art: &Art = match kind {
                Kind::Large => &LARGE,
                Kind::Wide => &WIDE,
                Kind::Tall => &TALL,
                Kind::Compact => &COMPACT,
                _ => &SMALL,
            };
            let filled: Vec<usize> = (0..art.w * art.h)
                .filter(|at| !matches!(art.cells[*at], b'.' | b' '))
                .map(|at| at % art.w / 2)
                .collect();
            let found = (
                *filled.iter().min().unwrap() as u16,
                *filled.iter().max().unwrap() as u16,
            );
            assert_eq!(found, body, "{kind:?}");
        }
    }
    use crate::chrome::buffer_text;

    fn paint() -> Paint {
        Paint {
            theme: ThemeName::Phosphor,
            down: [false; 8],
            led: true,
        }
    }

    #[test]
    fn every_shell_has_a_picture_hole_of_the_designed_size() {
        // 320x288 px at x2 is 45x21 cells of 7.14x14.2 px; x1 at 6x12 px is 26x12.
        for (art, cols, rows, hole) in [
            (&*WIDE, 140, 39, (45, 21)),
            (&*LARGE, 200, 56, (45, 21)),
            (&*TALL, 70, 60, (45, 21)),
            (&*COMPACT, 100, 28, (54, 25)),
            (&*SMALL, 80, 24, (27, 13)),
        ] {
            assert_eq!((art.cols(), art.rows()), (cols, rows));
            let found = art.hole();
            assert!(
                found.width.abs_diff(hole.0) <= 2 && found.height.abs_diff(hole.1) <= 2,
                "{found:?} for {hole:?}"
            );
        }
        assert!(MINI.hole().width >= 4);
    }

    #[test]
    fn a_cell_gets_one_glyph_and_two_colours() {
        let art = Art::parse("size 4 2\nbb.h\nbbhh\nparts\n....\n....\n");
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        let empty = Color::Rgb(2, 5, 2);
        draw(&mut buf, &art, 1, 0, area, &paint(), empty);
        assert_eq!(buf[(1, 0)].symbol(), " ");
        assert_eq!(buf[(1, 0)].bg, color(ThemeName::Phosphor, b'b'));
        // Top-left is empty, the other three are highlight.
        assert_eq!(buf[(2, 0)].symbol(), "▘");
        assert_eq!(
            (buf[(2, 0)].fg, buf[(2, 0)].bg),
            (empty, color(ThemeName::Phosphor, b'h'))
        );
        // Off the right edge and untouched cells stay as they were.
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
        draw(&mut buf, &art, 3, 1, area, &paint(), empty);
        assert_eq!(buf[(3, 1)].bg, color(ThemeName::Phosphor, b'b'));
    }

    #[test]
    fn a_held_button_is_drawn_differently_and_only_that_button() {
        let draw_with = |down: &[Button]| {
            let area = Rect::new(0, 0, WIDE.cols(), WIDE.rows());
            let mut buf = Buffer::empty(area);
            let mut paint = paint();
            for button in down {
                paint.down[button.index()] = true;
            }
            draw(&mut buf, &WIDE, 0, 0, area, &paint, Color::Black);
            buf
        };
        let idle = draw_with(&[]);
        for button in Button::ALL {
            let held = draw_with(&[button]);
            let changed = idle
                .content
                .iter()
                .zip(&held.content)
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                (2..120).contains(&changed),
                "{button:?}: {changed} cells changed"
            );
        }
        assert!(buffer_text(&idle).contains('▀'));
    }

    #[test]
    fn print_keeps_the_body_colour_behind_the_text() {
        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        let art = Art::parse("size 12 2\nbbbbbbbbbbbb\nbbbbbbbbbbbb\nparts\n\n\n");
        draw(&mut buf, &art, 0, 0, area, &paint(), Color::Black);
        print(
            &mut buf,
            4,
            0,
            "START",
            color(ThemeName::Phosphor, b'k'),
            true,
            area,
        );
        assert_eq!(buffer_text(&buf).trim(), "ST");
        assert_eq!(buf[(5, 0)].bg, color(ThemeName::Phosphor, b'b'));
        dim(&mut buf, area, Color::Rgb(0, 0, 0), 0.5);
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(0x10, 0x3D, 0x1F));
    }
}
