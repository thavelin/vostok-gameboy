//! Draws the app into a ratatui buffer. Pure: `App` in, cells out, plus the
//! list of pictures the terminal should place on top (Kitty graphics).
//!
//! Positions are the cell coordinates of the Figma frames ("VOSTOK BOY v2 ·
//! Filled"), moved by the plan's origin.

use crate::app::{App, Game, Typing, MENU};
use crate::art::{self, Art, Paint};
use crate::chrome::{self, bold, st};
use crate::layout::{self, Kind, Plan, View};
use crate::screen;
use crate::tokens::{self, Theme};
use chrono::{Local, TimeZone};
use ratatui::{buffer::Buffer, layout::Rect, style::Style};
use vostok_gameboy::cart::{Color as CartColor, Header};
use vostok_gameboy::frame::Frame;
use vostok_gameboy::input::Button;
use vostok_gameboy::machine;
use vostok_gameboy::session::State;
use vostok_gameboy::states::SlotInfo;

/// Pictures to place over the cells once the buffer is on screen.
#[derive(Default)]
pub struct Pictures {
    /// The running game, in this rectangle.
    pub game: Option<Rect>,
    /// Save-state previews: a number that changes when the picture does.
    pub thumbs: Vec<(u32, Rect, Frame)>,
}

#[derive(Clone, Copy)]
enum Tone {
    Ink,
    InkBold,
    Bezel,
}

type Label = (i32, i32, &'static str, Tone);

const WIDE_LABELS: &[Label] = &[
    (59, 3, " DOT MATRIX · ВОСТОК ", Tone::Bezel),
    (43, 13, "BATT", Tone::Bezel),
    (93, 33, "B", Tone::InkBold),
    (101, 32, "A", Tone::InkBold),
    (65, 34, "SELECT", Tone::InkBold),
    (80, 34, "START", Tone::InkBold),
    (88, 35, "PHONES", Tone::InkBold),
];
const TALL_LABELS: &[Label] = &[
    (6, 3, "◂OFF·ON▸", Tone::Ink),
    (24, 6, " DOT MATRIX · ВОСТОК ", Tone::Bezel),
    (8, 16, "BATT", Tone::Bezel),
    (46, 45, "B", Tone::InkBold),
    (55, 44, "A", Tone::InkBold),
    (24, 53, "SELECT", Tone::InkBold),
    (34, 53, "START", Tone::InkBold),
    (26, 56, "PHONES", Tone::InkBold),
];
const LARGE_LABELS: &[Label] = &[
    (69, 2, "◂OFF·ON▸", Tone::Ink),
    (88, 5, " DOT MATRIX · ВОСТОК ", Tone::Bezel),
    (72, 15, "BATT", Tone::Bezel),
    (111, 42, "B", Tone::InkBold),
    (122, 42, "A", Tone::InkBold),
    (88, 50, "SELECT", Tone::InkBold),
    (99, 50, "START", Tone::InkBold),
    (89, 52, "PHONES", Tone::InkBold),
];
const COMPACT_LABELS: &[Label] = &[
    (85, 13, "B", Tone::InkBold),
    (92, 11, "A", Tone::InkBold),
    (80, 20, "SELECT", Tone::InkBold),
    (89, 21, "START", Tone::InkBold),
];

/// The drawing for a shell, its labels, and the size of the picture in
/// cells as designed (320x288 px at 7.14x14.2 px a cell, and so on).
fn shell(kind: Kind) -> (&'static Art, &'static [Label], (u16, u16)) {
    match kind {
        Kind::Large => (&art::LARGE, LARGE_LABELS, (45, 21)),
        Kind::Tall => (&art::TALL, TALL_LABELS, (45, 21)),
        Kind::Compact => (&art::COMPACT, COMPACT_LABELS, (53, 24)),
        _ => (&art::WIDE, WIDE_LABELS, (45, 21)),
    }
}

/// Text at a signed position, cut at `clip`. Returns the column after it.
fn put(buf: &mut Buffer, clip: Rect, x: i32, y: i32, text: &str, style: Style) -> i32 {
    let clip = clip.intersection(buf.area);
    let mut x = x;
    for ch in text.chars() {
        if x >= i32::from(clip.x)
            && x < i32::from(clip.right())
            && y >= i32::from(clip.y)
            && y < i32::from(clip.bottom())
        {
            buf[(x as u16, y as u16)].set_char(ch).set_style(style);
        }
        x += 1;
    }
    x
}

fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    buf.set_style(area.intersection(buf.area), style);
}

fn shifted(rect: Rect, by: (i32, i32)) -> Rect {
    let (x, y) = (i32::from(rect.x) + by.0, i32::from(rect.y) + by.1);
    Rect::new(x.max(0) as u16, y.max(0) as u16, rect.width, rect.height)
}

fn local(unix: u64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_opt(unix as i64, 0).single()
}

/// "today", "yesterday", "nov 12".
fn day(unix: u64) -> String {
    let Some(then) = local(unix) else {
        return "—".into();
    };
    match (Local::now().date_naive() - then.date_naive()).num_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => then
            .format("%b %e")
            .to_string()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn when(unix: u64) -> String {
    local(unix).map_or_else(
        || "—".into(),
        |then| format!("{} {}", day(unix), then.format("%H:%M")),
    )
}

fn bytes(count: usize) -> String {
    match count {
        0 => "—".into(),
        n if n >= 1 << 20 => format!("{}M", n >> 20),
        n => format!("{}K", n >> 10),
    }
}

fn played(seconds: u64) -> String {
    match seconds / 60 {
        0 if seconds == 0 => "—".into(),
        minutes if minutes >= 60 => format!("{}h {:02}m", minutes / 60, minutes % 60),
        minutes => format!("{minutes}m {:02}s", seconds % 60),
    }
}

fn tilde(path: &std::path::Path) -> String {
    let home = vostok_gameboy::paths::home();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => {
            format!("~/{}", rest.display()).replace('\\', "/")
        }
        _ => path.display().to_string(),
    }
}

fn kind_name(header: &Header) -> &'static str {
    if header.color == CartColor::Mono {
        "GB"
    } else {
        "GBC"
    }
}

/// The keys for a button as the side list shows them: "z / k".
fn key_names(app: &App, button: Button) -> String {
    let short = |name: String| match name.as_str() {
        "backspace" => "bksp".to_string(),
        _ => name,
    };
    let names: Vec<String> = app
        .keys
        .keys(button)
        .into_iter()
        .map(|key| short(key.name()))
        .collect();
    match names.as_slice() {
        [] => "—".into(),
        [one] => one.clone(),
        [one, two, ..] if one.len() + two.len() <= 4 => format!("{one} / {two}"),
        [one, ..] => one.clone(),
    }
}

fn thumb_id(place: u32, info: &SlotInfo) -> u32 {
    let mix = (info.saved_at as u32)
        .wrapping_mul(31)
        .wrapping_add(info.frames as u32)
        & 0xFFFF;
    mix << 4 | (place & 0xF)
}

/// A save-state preview: a picture where the terminal can, half-blocks elsewhere.
fn thumb(buf: &mut Buffer, app: &App, pics: &mut Pictures, id: u32, rect: Rect, frame: &Frame) {
    let rect = rect.intersection(buf.area);
    if rect.is_empty() {
        return;
    }
    screen::half_blocks(buf, rect, frame);
    if app.pixels {
        pics.thumbs.push((id, rect, frame.clone()));
    }
}

pub fn render(buf: &mut Buffer, app: &App, clock: &str) -> Pictures {
    let theme = tokens::theme(app.theme);
    let area = buf.area;
    let mut pics = Pictures::default();
    fill(buf, area, Style::new().bg(theme.bg).fg(theme.text));
    if area.height < 3 || area.width < 8 {
        return pics;
    }
    let (rom, mode) = match &app.game {
        Some(game) => player(buf, app, game, theme, &mut pics),
        None => library(buf, app, theme, &mut pics),
    };
    let top = Rect::new(0, 0, area.width, 1);
    chrome::header(
        buf,
        top,
        theme,
        "ВОСТОК-БОЙ",
        "VOSTOK BOY",
        &[rom, mode],
        clock,
    );
    if app.help {
        help(buf, app, theme);
        pics = Pictures::default();
    }
    pics
}

fn protocol(app: &App) -> &'static str {
    if app.pixels {
        "KITTY"
    } else {
        "HALF"
    }
}

/// The status line (dim, on the surface colour) and the key hints under it.
fn bottom(
    buf: &mut Buffer,
    app: &App,
    theme: &Theme,
    plan: Option<&Plan>,
    left: &str,
    right: &str,
    keys: &[chrome::Key],
) {
    let area = buf.area;
    let status = plan.is_none_or(|plan| plan.status);
    let footer = Rect::new(0, area.height - 1, area.width, 1);
    let toast = app.toast.as_ref().map(|(text, _)| text.as_str());
    if status {
        let line = Rect::new(0, area.height - 2, area.width, 1);
        fill(buf, line, Style::new().bg(theme.surface).fg(theme.dim));
        let room = usize::from(area.width).saturating_sub(right.chars().count() + 3);
        match toast {
            Some(text) => put(
                buf,
                line,
                1,
                i32::from(line.y),
                &chrome::fit(text, room),
                st(theme.text),
            ),
            None => put(
                buf,
                line,
                1,
                i32::from(line.y),
                &chrome::fit(left, room),
                st(theme.dim),
            ),
        };
        let x = i32::from(area.width) - right.chars().count() as i32 - 1;
        put(buf, line, x, i32::from(line.y), right, st(theme.dim));
        chrome::footer(buf, footer, theme, keys, "");
    } else {
        chrome::footer(buf, footer, theme, keys, toast.unwrap_or(right));
    }
}

const PLAY_KEYS: &[chrome::Key] = &[
    ("z", "A", 6),
    ("x", "B", 6),
    ("↵", "start", 7),
    ("⌫", "select", 7),
    ("p", "pause", 1),
    ("F5", "save", 3),
    ("F8", "load", 3),
    ("tab", "turbo", 5),
    ("+-", "scale", 9),
    ("F2", "view", 4),
    ("F3", "theme", 8),
    ("?", "help", 2),
    ("q", "quit", 0),
];
const PAUSE_KEYS: &[chrome::Key] = &[
    ("↑↓", "move", 1),
    ("↵", "choose", 0),
    ("1-4", "slot", 2),
    ("F5", "save", 3),
    ("F8", "load", 3),
    ("del", "clear", 5),
    ("esc", "resume", 0),
];
const LIBRARY_KEYS: &[chrome::Key] = &[
    ("↵", "insert", 0),
    ("/", "search", 2),
    ("s", "sort", 4),
    ("d", "folder", 3),
    ("r", "resume", 1),
    ("p", "palette", 6),
    ("i", "header", 7),
    ("F3", "theme", 8),
    ("?", "help", 5),
    ("q", "quit", 0),
];

fn player(
    buf: &mut Buffer,
    app: &App,
    game: &Game,
    theme: &Theme,
    pics: &mut Pictures,
) -> (String, String) {
    let area = buf.area;
    let plan = layout::plan(area.width, area.height, app.view, app.shrink);
    let paused = game.paused();
    let mut down = [false; 8];
    for button in Button::ALL {
        down[button.index()] = !paused && game.input.is_down(button);
    }
    let paint = Paint {
        theme: app.theme,
        down,
        led: true,
    };
    let (ox, oy) = plan.origin;
    let picture;
    match plan.kind {
        Kind::TooSmall => {
            too_small(buf, app, theme, &plan);
            let keys: &[chrome::Key] = &[("h", "half ½", 0), ("q", "quit", 0)];
            chrome::footer(
                buf,
                Rect::new(0, area.height - 1, area.width, 1),
                theme,
                keys,
                "",
            );
            return (format!("⏸ {}", game.name), "TOO SMALL".into());
        }
        Kind::Screen => picture = screen::fit(plan.content),
        Kind::Focus => picture = focus(buf, app, plan.content, &down),
        kind => {
            let (drawing, labels, size) = shell(kind);
            art::draw(buf, drawing, ox, oy, plan.content, &paint, theme.bg);
            for (x, y, text, tone) in labels {
                let (fg, strong) = match tone {
                    Tone::Ink => (art::color(app.theme, b'k'), false),
                    Tone::InkBold => (art::color(app.theme, b'k'), true),
                    Tone::Bezel => (art::label(app.theme), false),
                };
                art::print(buf, ox + x, oy + y, text, fg, strong, plan.content);
            }
            // The picture sits in the middle of the dark glass.
            let hole = drawing.hole();
            let (w, h) = (size.0.min(hole.width), size.1.min(hole.height));
            let glass = Rect::new(
                hole.x + (hole.width - w) / 2,
                hole.y + (hole.height - h) / 2,
                w,
                h,
            );
            picture = shifted(glass, plan.origin).intersection(plan.content);
            if plan.sides {
                sides(buf, app, theme, &plan);
            }
            if let Some(columns) = plan.panels {
                panels(buf, app, game, theme, columns, picture);
            }
        }
    }
    // A terminal that draws pictures gets the real one on top of the dark
    // glass; behind the pause menu, and everywhere else, it is drawn in cells.
    if app.pixels && game.menu.is_none() {
        pics.game = Some(picture);
    } else {
        screen::half_blocks(buf, picture, &game.frame);
    }

    let scale = if matches!(plan.kind, Kind::Screen | Kind::Focus) {
        "fit"
    } else {
        "×2"
    };
    let state = match game.status.state {
        _ if game.menu.is_some() => "⏸ PAUSED",
        State::Paused => "⏸ PAUSED",
        _ if game.turbo => "▶▶ TURBO ×4",
        _ => "▶ RUNNING",
    };
    let palette = if game.mono() {
        game.status.palette.label()
    } else {
        "COLOUR"
    };
    let sound = if !game.status.audio_available {
        "no sound"
    } else if game.status.muted {
        "muted"
    } else {
        "♪"
    };
    let left = format!(
        "{state} · {} · {scale} {} {}×{} cells · {:.1} FPS · {palette} · {sound} · slot {}{}",
        game.name.to_uppercase(),
        protocol(app),
        picture.width,
        picture.height,
        game.status.fps,
        game.slot,
        if game.status.unsaved {
            " · unsaved"
        } else {
            ""
        },
    );
    if game.menu.is_some() {
        art::dim(buf, plan.content, theme.bg, 0.18);
        pause(buf, app, game, theme, &plan, pics);
        let left = format!(
            "⏸ PAUSED · {} · slot {} selected",
            game.name.to_uppercase(),
            game.slot
        );
        bottom(buf, app, theme, None, &left, "esc resume", PAUSE_KEYS);
    } else {
        let shown = match plan.kind {
            Kind::Screen => View::Screen,
            _ if plan.panels.is_some() => View::Panels,
            _ => View::Shell,
        };
        let right = format!("F2 {}", shown.name());
        let right = if plan.status {
            right
        } else {
            format!("{scale} · {:.1} fps", game.status.fps)
        };
        bottom(buf, app, theme, Some(&plan), &left, &right, PLAY_KEYS);
    }
    let mark = if paused { "⏸" } else { "▶" };
    (
        format!("{mark} {}", game.name),
        format!("{scale} {} · {:.1} fps", protocol(app), game.status.fps),
    )
}

/// The small-terminal player: the picture as large as fits, in a slim bezel,
/// with the controls in a strip of body tucked under it. Returns where the
/// picture goes.
fn focus(buf: &mut Buffer, app: &App, content: Rect, down: &[bool; 8]) -> Rect {
    let content = content.intersection(buf.area);
    let color = |letter: u8| art::color(app.theme, letter);
    // Two columns of body and two of bezel each side, a margin of one; a row
    // of body and one of bezel above, a row of bezel, three of controls and
    // the bottom edge below.
    let room = Rect::new(
        0,
        0,
        content.width.saturating_sub(10),
        content.height.saturating_sub(7),
    );
    if room.is_empty() {
        return Rect::new(content.x, content.y, 0, 0);
    }
    let fitted = screen::fit(room);
    let (w, h) = (fitted.width + 8, fitted.height + 7);
    let x0 = content.x + (content.width - w) / 2;
    let y0 = content.y + (content.height - h) / 2;
    let picture = Rect::new(x0 + 4, y0 + 2, fitted.width, fitted.height);

    let mut block =
        |rect: Rect, symbol: &str, fg: ratatui::style::Color, bg: ratatui::style::Color| {
            for y in rect.top()..rect.bottom() {
                for x in rect.left()..rect.right() {
                    let cell = &mut buf[(x, y)];
                    cell.reset();
                    cell.set_symbol(symbol).set_fg(fg).set_bg(bg);
                }
            }
        };
    let (body, bezel) = (color(b'b'), color(b'z'));
    block(Rect::new(x0, y0, w, h), " ", body, body);
    block(Rect::new(x0, y0, w, 1), "▀", color(b'h'), body);
    block(Rect::new(x0, y0 + h - 1, w, 1), "▄", color(b'l'), body);
    // The cut corner every one of these has, bottom right.
    block(
        Rect::new(x0 + w - 2, y0 + h - 1, 2, 1),
        " ",
        body,
        color(b's'),
    );
    block(
        Rect::new(x0 + 2, y0 + 1, w - 4, picture.height + 2),
        " ",
        bezel,
        bezel,
    );
    block(picture, " ", color(b'd'), color(b'd'));
    block(
        Rect::new(x0 + 2, picture.y + picture.height / 3, 1, 1),
        "■",
        color(b'L'),
        bezel,
    );

    // Controls: d-pad left, B and A right, Select and Start between them
    // where the body is wide enough.
    let cy = y0 + picture.height + 3;
    let pad = |button: Button| color(if down[button.index()] { b'P' } else { b'p' });
    let dx = x0 + 3;
    for (button, x, y) in [
        (Button::Up, dx + 2, cy),
        (Button::Left, dx, cy + 1),
        (Button::Right, dx + 4, cy + 1),
        (Button::Down, dx + 2, cy + 2),
    ] {
        block(Rect::new(x, y, 2, 1), " ", pad(button), pad(button));
    }
    block(
        Rect::new(dx + 2, cy + 1, 2, 1),
        " ",
        color(b'p'),
        color(b'p'),
    );
    let ax = x0 + w - 7;
    for (button, x, y) in [(Button::A, ax, cy), (Button::B, ax - 6, cy + 1)] {
        let (top, face) = if down[button.index()] {
            (color(b'n'), color(b'o'))
        } else {
            (color(b'N'), color(b'n'))
        };
        block(Rect::new(x, y, 4, 1), "▀", top, face);
        block(Rect::new(x, y + 1, 4, 1), " ", face, face);
    }
    let wide = w >= 44;
    if wide {
        let mx = x0 + w / 2;
        for (button, x) in [(Button::Select, mx - 7), (Button::Start, mx + 1)] {
            block(Rect::new(x, cy + 1, 6, 1), "▄", pad(button), body);
        }
    }
    let ink = color(b'k');
    let mut text = |x: u16, y: u16, text: &str, strong: bool| {
        art::print(buf, i32::from(x), i32::from(y), text, ink, strong, content);
    };
    text(ax + 1, cy + 1, "A", true);
    text(ax - 5, cy + 2, "B", true);
    if wide {
        let mx = x0 + w / 2;
        text(mx - 5, cy, "VOSTOK BOY", true);
        text(mx - 7, cy + 2, "SELECT", false);
        text(mx + 1, cy + 2, "START", false);
    }
    picture
}

/// Key lists either side of the wide body.
fn sides(buf: &mut Buffer, app: &App, theme: &Theme, plan: &Plan) {
    let (ox, oy) = plan.origin;
    let clip = plan.content;
    let mut list = |x: i32, gap: i32, title: &str, rows: &[(i32, String, &str)]| {
        put(buf, clip, ox + x, oy + 6, title, bold(theme.label));
        for (y, key, action) in rows {
            put(buf, clip, ox + x, oy + y, key, bold(theme.accent));
            put(
                buf,
                clip,
                ox + x + gap,
                oy + y,
                action,
                st(theme.line_title),
            );
        }
    };
    let arrows = app
        .keys
        .keys(Button::Up)
        .first()
        .is_some_and(|key| key.name() == "up");
    list(
        3,
        7,
        "KEYS · КНОПКИ",
        &[
            (
                8,
                if arrows {
                    "↑↓←→".into()
                } else {
                    key_names(app, Button::Up)
                },
                "d-pad",
            ),
            (10, "wasd".into(), "d-pad alt"),
            (14, "p".into(), "pause"),
            (16, "F5 F8".into(), "save / load"),
            (18, "tab".into(), "turbo"),
            (20, "+ -".into(), "scale"),
            (22, "m".into(), "mute"),
        ],
    );
    list(
        116,
        8,
        "BUTTONS",
        &[
            (8, key_names(app, Button::A), "A"),
            (10, key_names(app, Button::B), "B"),
            (12, key_names(app, Button::Start), "START"),
            (14, key_names(app, Button::Select), "SELECT"),
            (18, "F3".into(), "theme"),
            (20, "F6".into(), "palette"),
            (22, "F2".into(), "view"),
        ],
    );
}

/// Label and value rows inside a panel.
fn rows(buf: &mut Buffer, theme: &Theme, inner: Rect, lines: &[(&str, String)]) {
    // A blank line between rows where the panel is tall enough for it.
    let (lead, step) = if lines.len() * 2 <= usize::from(inner.height) {
        (1, 2)
    } else {
        (0, 1)
    };
    for (index, (label, value)) in lines.iter().enumerate() {
        let y = i32::from(inner.y) + lead + index as i32 * step;
        put(
            buf,
            inner,
            i32::from(inner.x) + 1,
            y,
            label,
            st(theme.label),
        );
        let room = usize::from(inner.width).saturating_sub(13);
        put(
            buf,
            inner,
            i32::from(inner.x) + 12,
            y,
            &chrome::fit(value, room),
            st(theme.text),
        );
    }
}

/// A panel column cut into the three panels of the 200x56 design (19, 16
/// and 15 rows of 52), two where it is too short for three, or one.
fn stack(column: Rect) -> [Rect; 3] {
    let none = Rect::new(column.x, column.y, 0, 0);
    let piece = |y: u16, height: u16| Rect::new(column.x, column.y + y, column.width, height);
    let rows = column.height;
    if rows >= 44 {
        let (first, second) = ((rows - 2) * 19 / 50, (rows - 2) * 16 / 50);
        [
            piece(0, first),
            piece(first + 1, second),
            piece(first + second + 2, rows - first - second - 2),
        ]
    } else if rows >= 22 {
        let first = (rows - 1).div_ceil(2);
        [piece(0, first), piece(first + 1, rows - first - 1), none]
    } else {
        [piece(0, rows), none, none]
    }
}

/// The information panels beside the body: cartridge, save states and
/// library on the left; status, render and input on the right. A short
/// column keeps the first two of each side (input rather than render).
fn panels(
    buf: &mut Buffer,
    app: &App,
    game: &Game,
    theme: &Theme,
    columns: (Rect, Rect),
    picture: Rect,
) {
    let (left, right) = (stack(columns.0), stack(columns.1));
    let (render_at, input_at) = if right[2].height > 0 {
        (right[1], right[2])
    } else {
        (right[2], right[1])
    };
    let header = game.session.header();
    let status = &game.status;

    let inner = chrome::draw_panel(buf, left[0], theme, "CARTRIDGE · КАРТРИДЖ", false);
    let check = if header.header_checksum_ok {
        "✓ checksum ok"
    } else {
        "✗ checksum wrong"
    };
    let file = game
        .session
        .rom_path()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    rows(
        buf,
        theme,
        inner,
        &[
            (
                "title",
                if header.title.is_empty() {
                    game.name.clone()
                } else {
                    header.title.clone()
                },
            ),
            (
                "type",
                format!(
                    "{} · {}",
                    kind_name(header),
                    if header.battery {
                        "battery save"
                    } else {
                        "no save"
                    }
                ),
            ),
            ("mapper", header.mapper.to_string()),
            ("rom", bytes(header.rom_bytes)),
            ("ram", bytes(header.ram_bytes)),
            ("header", check.to_string()),
            ("file", file),
            ("played", played(status.play_seconds)),
        ],
    );

    let inner = chrome::draw_panel(buf, left[1], theme, "SAVE STATES · СОХРАНЕНИЯ", false);
    let step = i32::from(inner.height / 4).clamp(1, 3);
    for (index, slot) in status.slots.iter().enumerate() {
        let y = i32::from(inner.y) + i32::from(step > 1) + index as i32 * step;
        let x = i32::from(inner.x) + 1;
        let chosen = usize::from(game.slot) == index + 1;
        put(
            buf,
            inner,
            x,
            y,
            &format!("[{}]", index + 1),
            if chosen {
                bold(theme.accent)
            } else {
                st(theme.label)
            },
        );
        match slot {
            Some(info) => {
                put(buf, inner, x + 5, y, &when(info.saved_at), st(theme.text));
                put(
                    buf,
                    inner,
                    x + 24,
                    y,
                    &format!("at {} of play", played(info.frames / 60)),
                    st(theme.label),
                );
            }
            None => {
                put(buf, inner, x + 5, y, "—      empty", st(theme.muted));
                put(buf, inner, x + 24, y, "F5 to save here", st(theme.dim));
            }
        }
    }

    let inner = chrome::draw_panel(buf, left[2], theme, "LIBRARY · БИБЛИОТЕКА (recent)", false);
    for (index, entry) in app
        .library
        .scan
        .entries
        .iter()
        .take(usize::from(inner.height) / 2)
        .enumerate()
    {
        let y = i32::from(inner.y) + 1 + index as i32 * 2;
        let current = entry.id == game.id;
        let name = format!(
            "{} {}",
            if current { "▶" } else { " " },
            chrome::fit(&entry.name, 30)
        );
        put(
            buf,
            inner,
            i32::from(inner.x) + 1,
            y,
            &name,
            if current {
                bold(theme.bright)
            } else {
                st(theme.text)
            },
        );
        let about = format!(
            "{} · {}",
            kind_name(&entry.header),
            bytes(entry.header.rom_bytes)
        );
        put(
            buf,
            inner,
            i32::from(inner.x) + 36,
            y,
            &about,
            st(theme.dim),
        );
    }

    let inner = chrome::draw_panel(buf, right[0], theme, "STATUS · СОСТОЯНИЕ", false);
    let audio = if !status.audio_available {
        "no sound device".to_string()
    } else {
        format!(
            "{} · {:.1} kHz · 2 ch",
            if status.muted { "muted" } else { "♪ on" },
            f64::from(machine::SAMPLE_RATE) / 1000.0
        )
    };
    rows(
        buf,
        theme,
        inner,
        &[
            (
                "fps",
                format!("{:.1} / {:.2} target", status.fps, machine::FRAME_RATE),
            ),
            (
                "frame",
                format!(
                    "{:.1} ms · cpu {:.0}%",
                    status.frame_ms,
                    status.load * 100.0
                ),
            ),
            ("audio", audio),
            (
                "palette",
                if game.mono() {
                    status.palette.label().to_string()
                } else {
                    "from the game (colour)".into()
                },
            ),
            (
                "speed",
                if game.turbo {
                    "4× turbo".into()
                } else {
                    "1× (tab: turbo 4×)".to_string()
                },
            ),
            (
                "rtc",
                if header.clock {
                    "cartridge clock".into()
                } else {
                    "—".to_string()
                },
            ),
            (
                "save",
                if !status.saves {
                    "—".into()
                } else if status.unsaved {
                    "unsaved".into()
                } else {
                    "saved".to_string()
                },
            ),
        ],
    );

    let inner = chrome::draw_panel(buf, render_at, theme, "RENDER · ВЫВОД", false);
    rows(
        buf,
        theme,
        inner,
        &[
            (
                "protocol",
                if app.pixels {
                    "kitty graphics".into()
                } else {
                    "half-blocks ▀".to_string()
                },
            ),
            ("screen", "160×144 px".to_string()),
            ("cells", format!("{}×{}", picture.width, picture.height)),
            (
                "terminal",
                format!("{}×{}", buf.area.width, buf.area.height),
            ),
            (
                "fits",
                "200×56 · 140×39 · 70×60 · 100×28 · smaller: screen first".to_string(),
            ),
            (
                "fallback",
                "half-blocks, then the screen alone (F2)".to_string(),
            ),
        ],
    );

    let inner = chrome::draw_panel(buf, input_at, theme, "INPUT · КНОПКИ", false);
    let all = |button| {
        app.keys
            .keys(button)
            .into_iter()
            .map(|key| key.name())
            .collect::<Vec<_>>()
            .join(" / ")
    };
    rows(
        buf,
        theme,
        inner,
        &[
            ("A", all(Button::A)),
            ("B", all(Button::B)),
            ("START", all(Button::Start)),
            ("SELECT", all(Button::Select)),
            (
                "D-PAD",
                format!(
                    "{} {} {} {}",
                    all(Button::Up),
                    all(Button::Down),
                    all(Button::Left),
                    all(Button::Right)
                ),
            ),
            ("pad", "gamepad: not supported".to_string()),
        ],
    );
}

/// The pause menu with the four save-state cards.
fn pause(
    buf: &mut Buffer,
    app: &App,
    game: &Game,
    theme: &Theme,
    plan: &Plan,
    pics: &mut Pictures,
) {
    let area = buf.area;
    let item = game.menu.unwrap_or(0);
    let entry = |index: usize| match index {
        1 | 2 => format!("{} ▸ {}", MENU[index], game.slot),
        3 if game.mono() => format!("{} ▸ {}", MENU[index], game.status.palette.label()),
        _ => MENU[index].to_string(),
    };
    if area.width < 112 || area.height < 36 {
        // No room for the drawn panel: a plain dialog with the same choices.
        let rect = chrome::centered(plan.content, 46, 19);
        let inner = chrome::dialog(buf, rect, theme, "PAUSED · ПАУЗА");
        for index in 0..MENU.len() {
            let y = i32::from(inner.y) + 1 + index as i32;
            let (mark, style) = if index == item {
                ("▶ ", bold(theme.bright))
            } else {
                ("  ", st(theme.text))
            };
            put(
                buf,
                inner,
                i32::from(inner.x) + 1,
                y,
                &format!("{mark}{}", entry(index)),
                style,
            );
        }
        for (index, slot) in game.status.slots.iter().enumerate() {
            let y = i32::from(inner.y) + 10 + index as i32;
            let chosen = usize::from(game.slot) == index + 1;
            let text = match slot {
                Some(info) => format!("[{}] {}", index + 1, when(info.saved_at)),
                None => format!("[{}] — empty", index + 1),
            };
            put(
                buf,
                inner,
                i32::from(inner.x) + 1,
                y,
                &text,
                if chosen {
                    bold(theme.accent)
                } else {
                    st(theme.label)
                },
            );
        }
        return;
    }
    let (ox, oy) = (
        (i32::from(area.width) - 140) / 2,
        (i32::from(area.height) - 39) / 2,
    );
    let clip = plan.content;
    let paint = Paint {
        theme: app.theme,
        down: [false; 8],
        led: true,
    };
    art::draw(buf, &art::PAUSE, ox, oy, clip, &paint, theme.bg);
    let (ink, light, stripe) = (
        art::color(app.theme, b'k'),
        art::label(app.theme),
        art::color(app.theme, b'A'),
    );
    art::print(buf, ox + 52, oy + 5, " PAUSED · ПАУЗА ", light, true, clip);
    for index in 0..MENU.len() {
        let y = oy + 9 + index as i32 * 2;
        if index == item {
            let bar = shifted(Rect::new(19, 0, 24, 1), (ox, y)).intersection(clip);
            for x in bar.x..bar.right() {
                buf[(x, bar.y)].reset();
                buf[(x, bar.y)].set_bg(art::color(app.theme, b'n'));
            }
            art::print(
                buf,
                ox + 20,
                y,
                &format!("▶ {}", entry(index)),
                ink,
                true,
                clip,
            );
        } else {
            art::print(buf, ox + 22, y, &entry(index), light, false, clip);
        }
    }
    for (index, slot) in game.status.slots.iter().enumerate() {
        let (cx, cy) = (ox + [47, 85][index % 2], oy + [7, 20][index / 2]);
        let chosen = usize::from(game.slot) == index + 1;
        if chosen {
            // The chosen card's frame: a bright top edge and left edge.
            for step in 0..35 {
                art::print(buf, cx + step, cy, "▀", stripe, false, clip);
            }
            for step in 1..12 {
                art::print(buf, cx, cy + step, "▌", stripe, false, clip);
            }
        }
        let title = if chosen { stripe } else { light };
        art::print(
            buf,
            cx + 16,
            cy + 2,
            &format!("SLOT {}", index + 1),
            title,
            true,
            clip,
        );
        match slot {
            Some(info) => {
                art::print(
                    buf,
                    cx + 16,
                    cy + 4,
                    &when(info.saved_at),
                    light,
                    false,
                    clip,
                );
                art::print(
                    buf,
                    cx + 16,
                    cy + 6,
                    &format!("{} played", played(info.frames / 60)),
                    light,
                    false,
                    clip,
                );
                art::print(
                    buf,
                    cx + 2,
                    cy + 9,
                    "F5 save · F8 load · del clear",
                    light,
                    false,
                    clip,
                );
                if let Some((_, frame)) = &game.thumbs[index] {
                    let rect = shifted(Rect::new(2, 2, 11, 5), (cx, cy));
                    thumb(
                        buf,
                        app,
                        pics,
                        thumb_id(index as u32, info),
                        rect.intersection(clip),
                        frame,
                    );
                }
            }
            None => {
                art::print(buf, cx + 4, cy + 4, "· empty ·", theme.dim, false, clip);
                art::print(buf, cx + 16, cy + 4, "—", light, false, clip);
                art::print(buf, cx + 2, cy + 9, "F5 save here", light, false, clip);
            }
        }
    }
    let note = " a state = full machine snapshot · thumbnails are 80×72 ";
    art::print(
        buf,
        ox + 70 - note.chars().count() as i32 / 2,
        oy + 33,
        note,
        ink,
        false,
        clip,
    );
}

/// 40x12 and under: a small body and what to do about it.
fn too_small(buf: &mut Buffer, app: &App, theme: &Theme, plan: &Plan) {
    let area = buf.area;
    let clip = Rect::new(0, 1, area.width, area.height.saturating_sub(2));
    let (ox, oy) = plan.origin;
    let paint = Paint {
        theme: app.theme,
        down: [false; 8],
        led: true,
    };
    art::draw(buf, &art::MINI, ox, oy, clip, &paint, theme.bg);
    let x = ox + 12;
    put(buf, clip, x, oy + 1, "TERMINAL TOO SMALL", bold(theme.warn));
    put(buf, clip, x, oy + 2, "МАЛО МЕСТА · paused", st(theme.label));
    put(
        buf,
        clip,
        x,
        oy + 4,
        &format!("now  {}×{}", area.width, area.height),
        st(theme.text),
    );
    put(buf, clip, x, oy + 5, "need 46×24 (×1)", st(theme.text));
    let (w, h) = layout::SCREEN_MIN;
    put(
        buf,
        clip,
        x,
        oy + 6,
        &format!("     {w}×{h} half"),
        st(theme.dim),
    );
    put(buf, clip, x, oy + 8, "resize · h · q", st(theme.line_title));
}

/// The cartridge picker.
fn library(buf: &mut Buffer, app: &App, theme: &Theme, pics: &mut Pictures) -> (String, String) {
    let area = buf.area;
    let shown = app.shown();
    let full = area.width >= 140 && area.height >= 39;
    let (ox, oy) = if full {
        (
            (i32::from(area.width) - 140) / 2,
            (i32::from(area.height) - 39) / 2,
        )
    } else {
        (0, 0)
    };
    let content = Rect::new(0, 1, area.width, area.height - 3);
    let folder = app
        .config
        .rom_dirs
        .first()
        .map_or_else(|| "no folder yet".to_string(), |dir| tilde(dir));
    let more = app.config.rom_dirs.len().saturating_sub(1);
    let folders = if more > 0 {
        format!("{folder} +{more}")
    } else {
        folder
    };
    let panel = if full {
        shifted(Rect::new(1, 2, 70, 34), (ox, oy))
    } else {
        content
    };
    let inner = chrome::draw_panel(
        buf,
        panel,
        theme,
        &format!("LIBRARY · БИБЛИОТЕКА · {folders}"),
        true,
    );
    let (x, mut y) = (i32::from(inner.x), i32::from(inner.y));

    let searching = matches!(app.library.typing, Some(Typing::Search));
    if app.library.search.is_empty() && !searching {
        put(buf, inner, x, y, "/ search…", st(theme.line_title));
    } else {
        let end = put(
            buf,
            inner,
            x,
            y,
            &format!("/ {}", app.library.search),
            st(theme.text),
        );
        if searching {
            put(buf, inner, end, y, "▏", st(theme.focus));
        }
    }
    let count = format!("sort: {} ▾  ·  {} roms", app.sort_label(), shown.len());
    put(
        buf,
        inner,
        i32::from(inner.right()) - count.chars().count() as i32,
        y,
        &count,
        st(theme.dim),
    );
    y += 2;

    let name_w = usize::from(inner.width).saturating_sub(33).max(8);
    let line = |name: &str, kind: &str, size: &str, save: &str, last: &str| {
        format!(
            "{} {:<6}{:<7}{:<6}{}",
            chrome::pad(name, name_w),
            kind,
            size,
            save,
            last
        )
    };
    put(
        buf,
        inner,
        x + 2,
        y,
        &line("NAME", "TYPE", "SIZE", "SAVE", "LAST"),
        bold(theme.label),
    );
    y += 2;

    let room = usize::try_from(i32::from(inner.bottom()) - y).unwrap_or(0);
    let step = if shown.len() * 2 <= room + 1 { 2 } else { 1 };
    let visible = room.div_ceil(step).max(1);
    let first = (app.library.selected + 1).saturating_sub(visible);
    for (index, entry) in shown.iter().enumerate().skip(first).take(visible) {
        let row = y + ((index - first) * step) as i32;
        let saves = entry.states.iter().flatten().count();
        let text = line(
            &entry.name,
            kind_name(&entry.header),
            &bytes(entry.header.rom_bytes),
            &if saves > 0 {
                saves.to_string()
            } else {
                "—".into()
            },
            &entry.stats.last_played.map_or_else(|| "—".into(), day),
        );
        if index == app.library.selected {
            fill(
                buf,
                Rect::new(inner.x, row as u16, inner.width, 1),
                Style::new().bg(theme.selection),
            );
            put(buf, inner, x, row, &format!("▶ {text}"), bold(theme.bright));
        } else {
            put(
                buf,
                inner,
                x + 2,
                row,
                &text,
                st(if entry.playable {
                    theme.text
                } else {
                    theme.muted
                }),
            );
        }
    }
    if shown.is_empty() {
        let (first, second) = if app.config.rom_dirs.is_empty() {
            (
                "no rom folder yet",
                "press d and type the folder your .gb / .gbc / .zip files are in",
            )
        } else if app.library.search.is_empty() {
            ("no roms in this folder", "press d to add another folder")
        } else {
            ("nothing matches", "esc clears the search")
        };
        put(buf, inner, x + 2, y, first, st(theme.text));
        put(buf, inner, x + 2, y + 2, second, st(theme.dim));
    }

    let selected = app.selected();
    if full {
        let paint = Paint {
            theme: app.theme,
            down: [false; 8],
            led: true,
        };
        art::draw(buf, &art::CARTRIDGE, ox, oy, content, &paint, theme.bg);
        let (ink, light, stripe) = (
            art::color(app.theme, b'k'),
            art::label(app.theme),
            art::color(app.theme, b'A'),
        );
        art::print(buf, ox + 86, oy + 5, "ВОСТОК-БОЙ", light, true, content);
        art::print(
            buf,
            ox + 86,
            oy + 7,
            "GAME CARTRIDGE · КАРТРИДЖ",
            ink,
            true,
            content,
        );
        if let Some(entry) = selected {
            let header = &entry.header;
            art::print(
                buf,
                ox + 111,
                oy + 12,
                kind_name(header),
                light,
                false,
                content,
            );
            art::print(
                buf,
                ox + 111,
                oy + 14,
                &chrome::fit(header.mapper, 9),
                light,
                false,
                content,
            );
            let check = if header.header_checksum_ok {
                "✓ ok"
            } else {
                "✗ sum"
            };
            art::print(buf, ox + 111, oy + 16, check, light, false, content);
            art::print(
                buf,
                ox + 86,
                oy + 25,
                &chrome::fit(&entry.name, 33),
                stripe,
                true,
                content,
            );
            let about = format!(
                "{} · {} · played {}",
                header.mapper,
                bytes(header.rom_bytes),
                played(entry.stats.seconds)
            );
            art::print(
                buf,
                ox + 86,
                oy + 27,
                &chrome::fit(&about, 33),
                light,
                false,
                content,
            );
            let window = shifted(Rect::new(87, 12, 22, 10), (ox, oy));
            match (&app.library.preview, &entry.resume) {
                (Some((id, Some(frame))), Some(info)) if *id == entry.id => {
                    thumb(buf, app, pics, thumb_id(8, info), window, frame);
                    art::print(
                        buf,
                        ox + 87,
                        oy + 22,
                        &format!("slot {} · {}", info.slot, when(info.saved_at)),
                        light,
                        false,
                        content,
                    );
                }
                _ => {
                    art::print(
                        buf,
                        ox + 91,
                        oy + 16,
                        "no save state",
                        light,
                        false,
                        content,
                    );
                    art::print(
                        buf,
                        ox + 91,
                        oy + 18,
                        "↵ starts fresh",
                        theme.dim,
                        false,
                        content,
                    );
                }
            }
        }
        for (index, (key, action)) in [
            ("↵", "insert"),
            ("r", "resume"),
            ("p", "palette"),
            ("i", "header"),
        ]
        .iter()
        .enumerate()
        {
            let row = oy + 6 + index as i32 * 2;
            let style = if index == 0 {
                bold(theme.bright)
            } else {
                st(theme.line_title)
            };
            put(buf, content, ox + 128, row, key, bold(theme.accent));
            put(buf, content, ox + 130, row, action, style);
        }
    }

    if app.library.info {
        if let Some(entry) = selected {
            let header = &entry.header;
            let inner = chrome::dialog(
                buf,
                chrome::centered(content, 58, 21),
                theme,
                "CARTRIDGE HEADER · ЗАГОЛОВОК",
            );
            let state = match &header.unsupported {
                Some(reason) => format!("cannot run: {reason}"),
                None => "playable".into(),
            };
            rows(
                buf,
                theme,
                inner,
                &[
                    ("title", header.title.clone()),
                    ("type", format!("{} · {}", kind_name(header), header.mapper)),
                    (
                        "rom / ram",
                        format!("{} / {}", bytes(header.rom_bytes), bytes(header.ram_bytes)),
                    ),
                    (
                        "battery",
                        if header.battery {
                            "yes".into()
                        } else {
                            "no".to_string()
                        },
                    ),
                    (
                        "checksum",
                        if header.header_checksum_ok {
                            "✓ ok".into()
                        } else {
                            "✗ wrong".to_string()
                        },
                    ),
                    (
                        "palette",
                        app.config.palette_for(&entry.id).label().to_string(),
                    ),
                    (
                        "played",
                        format!(
                            "{} · {} starts",
                            played(entry.stats.seconds),
                            entry.stats.launches
                        ),
                    ),
                    ("file", tilde(&entry.path)),
                    ("state", state),
                ],
            );
            pics.thumbs.clear();
        }
    }

    let status = match &app.library.typing {
        Some(Typing::Folder(text)) => {
            format!("add rom folder: {text}▏   enter adds it · esc cancels")
        }
        _ => format!(
            "LIBRARY · {} roms · {folders} · sort {}",
            app.library.scan.entries.len(),
            app.sort_label()
        ),
    };
    bottom(buf, app, theme, None, &status, "", LIBRARY_KEYS);
    ("■ no cartridge".into(), "LIBRARY".into())
}

/// Every key, over whatever is on screen.
fn help(buf: &mut Buffer, app: &App, theme: &Theme) {
    let area = Rect::new(0, 1, buf.area.width, buf.area.height.saturating_sub(2));
    let inner = chrome::dialog(
        buf,
        chrome::centered(area, 66, 21),
        theme,
        "CONTROLS · УПРАВЛЕНИЕ",
    );
    let all = |button| {
        app.keys
            .keys(button)
            .into_iter()
            .map(|key| key.name())
            .collect::<Vec<_>>()
            .join(" / ")
    };
    let d_pad = format!("{} · {}", all(Button::Up), all(Button::Left));
    let left: [(&str, String); 9] = [
        ("A", all(Button::A)),
        ("B", all(Button::B)),
        ("START", all(Button::Start)),
        ("SELECT", all(Button::Select)),
        ("D-PAD", "arrows / wasd".into()),
        ("", chrome::fit(&d_pad, 20)),
        ("", String::new()),
        ("p / esc", "pause menu".into()),
        ("q", "back to the library".into()),
    ];
    let right: [(&str, &str); 9] = [
        ("F5 / F8", "save / load state"),
        ("1-4", "slot (in the menu)"),
        ("tab", "turbo ×4: tap or hold"),
        ("m", "sound on / off"),
        ("F6", "palette"),
        ("F2", "view: shell, panels, screen"),
        ("+ -", "shell size"),
        ("F3", "theme: phosphor / amber"),
        ("?", "this list"),
    ];
    for (index, ((name, keys), (key, action))) in left.iter().zip(right).enumerate() {
        let y = i32::from(inner.y) + 1 + index as i32 * 2;
        let x = i32::from(inner.x) + 1;
        put(buf, inner, x, y, name, bold(theme.accent));
        put(buf, inner, x + 9, y, keys, st(theme.text));
        put(buf, inner, x + 31, y, key, bold(theme.accent));
        put(buf, inner, x + 40, y, action, st(theme.line_title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{app_with_rom, press, settle};
    use crate::chrome::buffer_text;
    use crossterm::event::KeyCode;

    fn draw(app: &mut App, cols: u16, rows: u16) -> (Buffer, Pictures) {
        app.size = (cols, rows);
        let mut buf = Buffer::empty(Rect::new(0, 0, cols, rows));
        let pics = render(&mut buf, app, "2026-11-14 19:40:12");
        (buf, pics)
    }

    #[test]
    fn the_library_lists_games_and_shows_the_cartridge_when_there_is_room() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        let (buf, _) = draw(&mut app, 140, 39);
        let text = buffer_text(&buf);
        for part in [
            "ВОСТОК-БОЙ",
            "■ no cartridge",
            "LIBRARY · БИБЛИОТЕКА",
            "▶ Kometka Run",
            "NAME",
            "sort: recent",
            "1 roms",
        ] {
            assert!(text.contains(part), "{part}\n{text}");
        }
        assert!(
            text.contains("GAME CARTRIDGE")
                && text.contains("no save state")
                && text.contains("insert")
        );
        let (small, _) = draw(&mut app, 60, 20);
        let text = buffer_text(&small);
        assert!(text.contains("▶ Kometka Run") && !text.contains("GAME CARTRIDGE"));
        // Nothing panics at any size.
        for (cols, rows) in [(8, 3), (20, 5), (39, 12), (139, 38), (300, 90)] {
            draw(&mut app, cols, rows);
        }
    }

    #[test]
    fn each_terminal_size_draws_its_shell_around_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        settle(&mut app);
        type Case = (u16, u16, &'static [&'static str], (u16, u16));
        let cases: [Case; 6] = [
            (
                140,
                39,
                &[
                    "DOT MATRIX · ВОСТОК",
                    "BATT",
                    "SELECT",
                    "START",
                    "PHONES",
                    "KEYS · КНОПКИ",
                    "BUTTONS",
                    "▶ RUNNING",
                ],
                (45, 21),
            ),
            (70, 60, &["◂OFF·ON▸", "DOT MATRIX", "PHONES"], (45, 21)),
            (100, 28, &["SELECT", "START"], (54, 25)),
            // Small terminals: the picture takes the room, the controls go under it.
            (80, 24, &["▶ RUNNING"], (31, 14)),
            (68, 38, &["VOSTOK BOY", "SELECT", "START"], (58, 26)),
            (100, 45, &["DOT MATRIX", "PHONES"], (45, 21)),
        ];
        for (cols, rows, words, hole) in cases {
            let (buf, pics) = draw(&mut app, cols, rows);
            let text = buffer_text(&buf);
            for word in words {
                assert!(text.contains(word), "{cols}x{rows}: {word}\n{text}");
            }
            assert!(text.contains("ВОСТОК-БОЙ") && text.contains("Kometka Run"));
            assert!(
                pics.game.is_none(),
                "no terminal picture without pixel support"
            );
            app.pixels = true;
            let (pixels, pics) = draw(&mut app, cols, rows);
            app.pixels = false;
            let picture = pics.game.expect("a picture rectangle");
            // In cells the picture is ▀ all over; with a terminal picture the glass stays dark.
            assert_eq!(buf[(picture.x + 1, picture.y + 1)].symbol(), "▀");
            assert_eq!(
                pixels[(picture.x + 1, picture.y + 1)].bg,
                art::color(app.theme, b'd')
            );
            assert!(
                picture.width.abs_diff(hole.0) <= 2 && picture.height.abs_diff(hole.1) <= 2,
                "{cols}x{rows}: {picture:?}"
            );
            assert!(picture.right() <= cols && picture.bottom() < rows);
        }
        assert!(
            !buffer_text(&draw(&mut app, 100, 45).0).contains("KEYS · КНОПКИ"),
            "no side lists when narrow"
        );
    }

    #[test]
    fn panels_screen_only_and_too_small() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        press(&mut app, KeyCode::Enter);
        settle(&mut app);
        app.view = View::Panels;
        let text = buffer_text(&draw(&mut app, 200, 56).0);
        for part in [
            "CARTRIDGE · КАРТРИДЖ",
            "SAVE STATES",
            "STATUS · СОСТОЯНИЕ",
            "RENDER · ВЫВОД",
            "INPUT · КНОПКИ",
            "F2 panels",
            "MBC1",
        ] {
            assert!(text.contains(part), "{part}\n{text}");
        }
        // Wide but short: the wide body with two panels a side, no key lists.
        let text = buffer_text(&draw(&mut app, 180, 45).0);
        for part in [
            "CARTRIDGE · КАРТРИДЖ",
            "SAVE STATES",
            "STATUS",
            "INPUT · КНОПКИ",
        ] {
            assert!(
                text.contains(part),
                "{part}
{text}"
            );
        }
        assert!(!text.contains("RENDER · ВЫВОД") && !text.contains("KEYS · КНОПКИ"));
        // No room beside the body: the plain shell.
        assert!(!buffer_text(&draw(&mut app, 105, 75).0).contains("CARTRIDGE"));
        app.view = View::Screen;
        app.pixels = true;
        let (_, pics) = draw(&mut app, 160, 75);
        assert_eq!(pics.game, Some(Rect::new(0, 1, 160, 72)));
        app.view = View::Shell;
        let text = buffer_text(&draw(&mut app, 40, 12).0);
        assert!(
            text.contains("TERMINAL TOO SMALL")
                && text.contains("now  40×12")
                && text.contains("half ½")
        );
    }

    #[test]
    fn the_pause_menu_shows_the_slots_and_takes_the_picture_away() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rom(dir.path());
        app.pixels = true;
        press(&mut app, KeyCode::Enter);
        settle(&mut app);
        press(&mut app, KeyCode::F(5));
        settle(&mut app);
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Down);
        let (buf, pics) = draw(&mut app, 140, 39);
        let text = buffer_text(&buf);
        for part in [
            "PAUSED · ПАУЗА",
            "RESUME",
            "▶ SAVE STATE ▸ 1",
            "QUIT TO LIBRARY",
            "SLOT 1",
            "SLOT 4",
            "· empty ·",
            "esc resume",
            "today",
        ] {
            assert!(text.contains(part), "{part}\n{text}");
        }
        assert!(pics.game.is_none());
        assert_eq!(pics.thumbs.len(), 1, "one saved slot, one preview");
        // The body behind the menu is dimmed.
        assert_ne!(buf[(40, 2)].bg, art::color(app.theme, b'b'));
        let small = buffer_text(&draw(&mut app, 80, 24).0);
        assert!(small.contains("▶ SAVE STATE ▸ 1") && small.contains("[1] today"));
        press(&mut app, KeyCode::Char('?'));
        assert!(buffer_text(&draw(&mut app, 140, 39).0).contains("CONTROLS · УПРАВЛЕНИЕ"));
    }
}
