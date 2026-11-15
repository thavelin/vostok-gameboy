//! Which shell fits the terminal. Pure: sizes in, a plan out.
//!
//! The Figma file draws the player at fixed terminal sizes (200x56, 140x39,
//! 70x60, 100x28, 80x24, 40x12). The planner picks the largest drawing that
//! fits and centres it; a drawing wider than the terminal may be cropped at
//! the sides as long as the body itself still fits.

use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 200x56: the tall body with information panels left and right.
    Large,
    /// 140x39: the wide body, key lists at the sides when there is room.
    Wide,
    /// 70x60: the tall body alone.
    Tall,
    /// 100x28: screen in the middle, controls either side, no wordmark.
    Compact,
    /// 80x24: the picture at half size.
    Small,
    /// No body: the picture as large as the terminal allows.
    Screen,
    /// Nothing fits: say so (the game is paused).
    TooSmall,
}

/// What F2 cycles through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Shell,
    Panels,
    Screen,
}

impl View {
    pub fn next(self) -> View {
        match self {
            View::Shell => View::Panels,
            View::Panels => View::Screen,
            View::Screen => View::Shell,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            View::Shell => "shell",
            View::Panels => "shell+panels",
            View::Screen => "screen",
        }
    }
    pub fn parse(text: &str) -> Option<View> {
        [View::Shell, View::Panels, View::Screen]
            .into_iter()
            .find(|view| view.name() == text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub kind: Kind,
    /// Where the corner of the drawing lands (may be negative: cropped).
    pub origin: (i32, i32),
    /// Everything between the header and the bottom lines.
    pub content: Rect,
    /// Wide only: the key lists beside the body.
    pub sides: bool,
    /// A status line above the footer.
    pub status: bool,
    /// How many smaller shells fit (for the - key).
    pub smaller: u8,
}

/// The smallest terminal with room for the picture alone.
pub const SCREEN_MIN: (u16, u16) = (20, 10);

/// (kind, designed size, smallest terminal it is used at, sides).
type Shell = (Kind, (u16, u16), (u16, u16), bool);
const SHELLS: [Shell; 5] = [
    (Kind::Wide, (140, 39), (140, 39), true),
    (Kind::Tall, (70, 60), (70, 60), false),
    (Kind::Wide, (140, 39), (86, 39), false),
    (Kind::Compact, (100, 28), (100, 28), false),
    (Kind::Small, (80, 24), (46, 24), false),
];

/// `shrink` skips that many of the shells that fit, largest first.
pub fn plan(cols: u16, rows: u16, view: View, shrink: u8) -> Plan {
    let fits = |need: (u16, u16)| cols >= need.0 && rows >= need.1;
    let place = |kind, size: (u16, u16), sides, smaller| {
        // Compact has no status line: its body runs down to the footer.
        let status = kind != Kind::Compact;
        let bottom = if status { 2 } else { 1 };
        let content = Rect::new(0, 1, cols, rows.saturating_sub(1 + bottom));
        let origin = (
            (i32::from(cols) - i32::from(size.0)) / 2,
            i32::from(rows.saturating_sub(size.1)) / 2,
        );
        Plan {
            kind,
            origin,
            content,
            sides,
            status,
            smaller,
        }
    };
    if view == View::Screen {
        let kind = if fits(SCREEN_MIN) {
            Kind::Screen
        } else {
            Kind::TooSmall
        };
        return place(kind, (cols, rows), false, 0);
    }
    if view == View::Panels && fits((200, 56)) {
        return place(Kind::Large, (200, 56), false, 0);
    }
    // The wide body without its lists is only for terminals too narrow for them.
    let fitting: Vec<_> = SHELLS
        .iter()
        .filter(|shell| fits(shell.2) && (shell.3 || shell.0 != Kind::Wide || !fits((140, 39))))
        .collect();
    match fitting.get(usize::from(shrink).min(fitting.len().saturating_sub(1))) {
        Some((kind, size, _, sides)) => place(*kind, *size, *sides, fitting.len() as u8 - 1),
        None => place(Kind::TooSmall, (40, 12), false, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_designed_sizes_get_their_own_shell() {
        let kind = |cols, rows, view| plan(cols, rows, view, 0).kind;
        assert_eq!(kind(200, 56, View::Panels), Kind::Large);
        assert_eq!(kind(200, 56, View::Shell), Kind::Wide);
        assert_eq!(kind(140, 39, View::Shell), Kind::Wide);
        assert!(plan(140, 39, View::Shell, 0).sides);
        assert_eq!(kind(70, 60, View::Shell), Kind::Tall);
        assert_eq!(kind(100, 28, View::Shell), Kind::Compact);
        assert_eq!(kind(80, 24, View::Shell), Kind::Small);
        assert_eq!(kind(40, 12, View::Shell), Kind::TooSmall);
        assert_eq!(kind(40, 12, View::Screen), Kind::Screen);
        assert_eq!(kind(12, 4, View::Screen), Kind::TooSmall);
        // Panels fall back to the plain shell where they do not fit.
        assert_eq!(kind(150, 45, View::Panels), Kind::Wide);
    }

    #[test]
    fn the_drawing_is_centred_and_cropped_not_squeezed() {
        let exact = plan(140, 39, View::Shell, 0);
        assert_eq!(
            (exact.origin, exact.content),
            ((0, 0), Rect::new(0, 1, 140, 36))
        );
        let roomy = plan(160, 45, View::Shell, 0);
        assert_eq!(roomy.origin, (10, 3));
        // Narrower than the drawing: the body stays, the side lists go.
        let narrow = plan(100, 40, View::Shell, 0);
        assert_eq!(
            (narrow.kind, narrow.sides, narrow.origin),
            (Kind::Wide, false, (-20, 0))
        );
        let compact = plan(100, 28, View::Shell, 0);
        assert!(!compact.status);
        assert_eq!(compact.content, Rect::new(0, 1, 100, 26));
    }

    #[test]
    fn minus_steps_down_through_the_shells_that_fit() {
        let kinds: Vec<Kind> = (0..6)
            .map(|shrink| plan(200, 60, View::Shell, shrink).kind)
            .collect();
        assert_eq!(
            kinds,
            [
                Kind::Wide,
                Kind::Tall,
                Kind::Compact,
                Kind::Small,
                Kind::Small,
                Kind::Small
            ]
        );
        assert_eq!(plan(200, 60, View::Shell, 0).smaller, 3);
        assert_eq!(plan(80, 24, View::Shell, 3).kind, Kind::Small);
    }
}
