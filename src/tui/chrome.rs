//! Vostok TUI chrome shared by vostok-files and vostok-browser: header, panels,
//! priority footer, dialog frame. Pure render functions over a ratatui Buffer.
//! Keep this file the same in both repos (see MAP.md).

use crate::tokens::Theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

pub fn width(text: &str) -> usize {
    Line::raw(text).width()
}

/// Cut `text` to at most `max` display cells, ending in … when cut.
pub fn fit(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = width(ch.encode_utf8(&mut [0; 4]));
        if used + w > max - 1 {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Keep the end of `text` (paths): `…tes/docs`.
pub fn fit_left(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for ch in text.chars().rev() {
        let w = width(ch.encode_utf8(&mut [0; 4]));
        if used + w > max - 1 {
            break;
        }
        used += w;
        tail.push(ch);
    }
    let mut out = String::from("…");
    out.extend(tail.iter().rev());
    out
}

/// Pad or cut to exactly `w` cells.
pub fn pad(text: &str, w: usize) -> String {
    let cut = fit(text, w);
    let used = width(&cut);
    cut + &" ".repeat(w.saturating_sub(used))
}

pub fn st(fg: Color) -> Style {
    Style::new().fg(fg)
}

pub fn bold(fg: Color) -> Style {
    Style::new().fg(fg).add_modifier(Modifier::BOLD)
}

/// Vostok Panel: line border, title in the border (focus colour when focused).
pub fn panel<'a>(theme: &Theme, title: &str, focused: bool) -> Block<'a> {
    let line = if focused {
        theme.line_focus
    } else {
        theme.line_panel
    };
    let title_style = if focused {
        bold(theme.line_focus)
    } else {
        st(theme.line_title)
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(st(line))
        .style(Style::new().bg(theme.bg));
    if !title.is_empty() {
        block = block.title(Span::styled(format!(" {title} "), title_style));
    }
    block
}

/// Draw a panel and return its inner area (one cell of side padding).
pub fn draw_panel(buf: &mut Buffer, rect: Rect, theme: &Theme, title: &str, focused: bool) -> Rect {
    if rect.width < 2 || rect.height < 2 {
        return Rect::new(rect.x, rect.y, 0, 0);
    }
    let room = usize::from(rect.width).saturating_sub(4);
    let block = panel(theme, &fit(title, room), focused);
    let inner = block.inner(rect);
    block.render(rect, buf);
    if inner.width > 2 {
        Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height)
    } else {
        inner
    }
}

/// A line padded to `w` cells with `right` right-aligned (dropped when it does not fit).
pub fn spread<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, w: usize) -> Line<'a> {
    let lw: usize = left.iter().map(|s| s.width()).sum();
    let rw: usize = right.iter().map(|s| s.width()).sum();
    let mut spans = left;
    if lw + rw < w {
        spans.push(Span::raw(" ".repeat(w - lw - rw)));
        spans.extend(right);
    }
    Line::from(spans)
}

/// ★ ВОСТОК-<APP> // VOSTOK <APP> :: part :: part :: clock
/// `parts` are dropped from the end until the line fits; the clock goes first.
pub fn header(
    buf: &mut Buffer,
    area: Rect,
    theme: &Theme,
    ru: &str,
    en: &str,
    parts: &[String],
    clock: &str,
) {
    if area.is_empty() {
        return;
    }
    let total = usize::from(area.width);
    let sep = || Span::styled(" :: ", st(theme.dim));
    let mut spans = vec![
        Span::styled(" ★ ", st(theme.star)),
        Span::styled(ru.to_string(), bold(theme.bright)),
    ];
    if total >= 90 {
        spans.push(Span::styled(format!(" // {en}"), st(theme.label)));
    }
    let mut used: usize = spans.iter().map(|s| s.width()).sum();
    let clock = if total >= 120 {
        clock.to_string()
    } else {
        clock.rsplit(' ').next().unwrap_or_default().to_string()
    };
    let clock_w = if clock.is_empty() {
        0
    } else {
        4 + width(&clock)
    };
    for (index, part) in parts.iter().enumerate() {
        // The first part (a path or address) may be cut; later ones are all-or-nothing.
        let room = total.saturating_sub(used + 4 + clock_w + 1);
        let text = if index == 0 {
            fit_left(part, room.max(8))
        } else {
            part.clone()
        };
        if width(&text) > room {
            break;
        }
        spans.push(sep());
        spans.push(Span::styled(text.clone(), st(theme.text)));
        used += 4 + width(&text);
    }
    if clock_w > 0 && used + clock_w <= total {
        spans.push(sep());
        spans.push(Span::styled(clock, st(theme.text)));
    }
    Paragraph::new(Line::from(spans))
        .style(Style::new().bg(theme.panel))
        .render(area, buf);
}

/// One footer key chip: key, action, drop priority (higher drops first).
pub type Key = (&'static str, &'static str, u8);

/// The keys that fit in `total` cells, and whether the mode label still fits.
pub fn footer_fit(keys: &[Key], mode: &str, total: usize) -> (Vec<Key>, bool) {
    let key_w = |k: &[Key]| -> usize {
        k.iter()
            .map(|(key, action, _)| width(key) + 1 + width(action) + 1)
            .sum::<usize>()
            + 1
    };
    let mut kept: Vec<Key> = keys.to_vec();
    let mode_w = width(mode) + 3;
    let mut show_mode = !mode.is_empty();
    while key_w(&kept) + if show_mode { mode_w } else { 0 } > total {
        if show_mode && kept.len() <= 6 {
            show_mode = false;
            continue;
        }
        if kept.len() <= 1 {
            break;
        }
        let drop = kept
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, _, p))| *p)
            .map(|(i, _)| i)
            .unwrap_or(0);
        kept.remove(drop);
    }
    (kept, show_mode)
}

/// Footer (Key Hint row). Drops high-number keys until the line fits.
pub fn footer(buf: &mut Buffer, area: Rect, theme: &Theme, keys: &[Key], mode: &str) {
    if area.is_empty() {
        return;
    }
    let total = usize::from(area.width);
    let (kept, show_mode) = footer_fit(keys, mode, total);
    let mut spans = vec![Span::raw(" ")];
    for (key, action, _) in &kept {
        spans.push(Span::styled(key.to_string(), bold(theme.focus)));
        spans.push(Span::styled(format!(" {action} "), st(theme.label)));
    }
    let used: usize = spans.iter().map(|s| s.width()).sum();
    let mode_w = width(mode) + 3;
    if show_mode && used + mode_w <= total {
        spans.push(Span::raw(" ".repeat(total - used - mode_w)));
        spans.push(Span::styled("▏", st(theme.dim)));
        spans.push(Span::styled(format!(" {mode} "), st(theme.label)));
    }
    Paragraph::new(Line::from(spans))
        .style(Style::new().bg(theme.panel))
        .render(area, buf);
}

/// A centred rectangle of at most `w` x `h` inside `area`.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// Clear a rectangle and draw a focused panel over it (dialogs, palettes).
pub fn dialog(buf: &mut Buffer, rect: Rect, theme: &Theme, title: &str) -> Rect {
    Clear.render(rect, buf);
    draw_panel(buf, rect, theme, title, true)
}

/// ▮▮▮▮▮▮▮▯▯▯ progress bar.
#[allow(dead_code)]
pub fn bar(value: f32, cells: usize) -> String {
    let on = ((value.clamp(0.0, 1.0) * cells as f32).round() as usize).min(cells);
    format!("{}{}", "▮".repeat(on), "▯".repeat(cells - on))
}

/// The whole buffer as text, one line per row (offline renders and tests).
pub fn buffer_text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        let mut row = String::new();
        let mut skip = 0usize;
        for x in area.x..area.x + area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = buf[(x, y)].symbol();
            row.push_str(symbol);
            skip = width(symbol).saturating_sub(1);
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::PHOSPHOR;

    #[test]
    fn fit_cuts_on_cells() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 6), "hello…");
        assert_eq!(fit_left("/home/demo/projects", 9), "…projects");
        assert_eq!(width(&pad("ab", 5)), 5);
        assert_eq!(width(&pad("abcdefgh", 5)), 5);
        assert_eq!(fit("x", 0), "");
    }

    #[test]
    fn footer_drops_low_priority_keys_first() {
        let keys: Vec<Key> = vec![
            ("q", "quit", 0),
            ("?", "help", 1),
            ("F3", "theme", 9),
            ("F2", "layout", 8),
        ];
        let (kept, _) = footer_fit(&keys, "miller", 22);
        assert!(kept.iter().any(|k| k.0 == "q"));
        assert!(!kept.iter().any(|k| k.0 == "F3"));
        let (all, mode) = footer_fit(&keys, "miller", 80);
        assert_eq!(all.len(), 4);
        assert!(mode);
    }

    #[test]
    fn header_keeps_the_brand_at_any_width() {
        for w in [40u16, 70, 100, 140, 200] {
            let area = Rect::new(0, 0, w, 1);
            let mut buf = Buffer::empty(area);
            header(
                &mut buf,
                area,
                &PHOSPHOR,
                "ВОСТОК-ФАЙЛЫ",
                "VOSTOK FILES",
                &["~/projects/atlas-notes".into(), "14 items".into()],
                "2026-11-12 09:41:07",
            );
            let text = buffer_text(&buf);
            assert!(text.contains("ВОСТОК-ФАЙЛЫ"), "{w}: {text}");
            if w >= 140 {
                assert!(text.contains("14 items"));
                assert!(text.contains("2026-11-12 09:41:07"));
            }
        }
    }
}
