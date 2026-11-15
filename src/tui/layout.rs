//! Which shell fits the terminal. Pure: sizes in, a plan out.
//!
//! The Figma file draws the player at fixed terminal sizes (200x56, 140x39,
//! 70x60, 100x28, 40x12); below those the focus layout is drawn to measure. The planner picks the largest drawing that
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
    /// Small terminals: the picture as large as fits in a slim bezel, the
    /// controls in a strip under it. Drawn to measure, not from a drawing.
    Focus,
    /// No body: the picture as large as the terminal allows.
    Screen,
    /// Nothing fits: say so (the game is paused).
    TooSmall,
}

/// What F2 cycles through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// The body alone (key lists beside the wide one).
    Shell,
    /// The body with information panels wherever there is room for them.
    #[default]
    Panels,
    Screen,
}

impl View {
    pub fn next(self) -> View {
        match self {
            View::Panels => View::Shell,
            View::Shell => View::Screen,
            View::Screen => View::Panels,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            View::Shell => "plain",
            View::Panels => "panels",
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
    /// Where the information panels go, left and right of the body.
    pub panels: Option<(Rect, Rect)>,
}

/// The widest a panel column gets, and the narrowest worth drawing.
const PANEL_WIDTH: (i32, i32) = (62, 34);
/// The fewest rows a panel column needs.
const PANEL_ROWS: i32 = 12;

/// The smallest terminal with room for the picture alone.
pub const SCREEN_MIN: (u16, u16) = (20, 10);

/// (kind, designed size, smallest terminal it is used at, sides, the first
/// and last column of the body inside the drawing).
type Shell = (Kind, (u16, u16), (u16, u16), bool, (u16, u16));
const SHELLS: [Shell; 5] = [
    (Kind::Wide, (140, 39), (140, 39), true, (30, 110)),
    (Kind::Tall, (70, 60), (70, 60), false, (3, 67)),
    (Kind::Wide, (140, 39), (86, 39), false, (30, 110)),
    (Kind::Compact, (100, 28), (100, 28), false, (2, 98)),
    (Kind::Focus, (0, 0), FOCUS_MIN, false, (0, 0)),
];
/// The smallest terminal the focus layout is used at.
pub const FOCUS_MIN: (u16, u16) = (30, 19);
/// The 200x56 drawing: the tall body with room for full panels.
const LARGE: Shell = (Kind::Large, (200, 56), (200, 56), false, (66, 134));

/// The body columns of each drawing, for checking against the art.
#[cfg(test)]
pub fn bodies() -> Vec<(Kind, (u16, u16))> {
    SHELLS
        .iter()
        .chain([&LARGE])
        .filter(|shell| shell.0 != Kind::Focus)
        .map(|shell| (shell.0, shell.4))
        .collect()
}

/// `shrink` skips that many of the shells that fit, largest first.
pub fn plan(cols: u16, rows: u16, view: View, shrink: u8) -> Plan {
    let fits = |need: (u16, u16)| cols >= need.0 && rows >= need.1;
    let place = |kind, size: (u16, u16), sides, smaller, body: Option<(u16, u16)>| {
        // Compact has no status line: its body runs down to the footer.
        let status = kind != Kind::Compact;
        let bottom = if status { 2 } else { 1 };
        let content = Rect::new(0, 1, cols, rows.saturating_sub(1 + bottom));
        let origin = (
            (i32::from(cols) - i32::from(size.0)) / 2,
            i32::from(rows.saturating_sub(size.1)) / 2,
        );
        // Panels fill the columns beside the body, as wide as the design
        // has them and no taller than the drawing.
        let panels = body.filter(|_| view == View::Panels).and_then(|body| {
            let left_edge = origin.0 + i32::from(body.0);
            let right_edge = origin.0 + i32::from(body.1) + 1;
            let left = (left_edge - 4).min(PANEL_WIDTH.0);
            let right = (i32::from(cols) - right_edge - 3).min(PANEL_WIDTH.0);
            let top = (i32::from(content.y) + 1).max(origin.1 + 2);
            let bottom = i32::from(content.bottom()).min(origin.1 + i32::from(size.1) - 2);
            let rows = bottom - top;
            (left.min(right) >= PANEL_WIDTH.1 && rows >= PANEL_ROWS).then(|| {
                let rect =
                    |x: i32, width: i32| Rect::new(x as u16, top as u16, width as u16, rows as u16);
                (
                    rect(left_edge - 3 - left, left),
                    rect(right_edge + 2, right),
                )
            })
        });
        Plan {
            kind,
            origin,
            content,
            sides: sides && panels.is_none(),
            status,
            smaller,
            panels,
        }
    };
    if view == View::Screen {
        let kind = if fits(SCREEN_MIN) {
            Kind::Screen
        } else {
            Kind::TooSmall
        };
        return place(kind, (cols, rows), false, 0, None);
    }
    if view == View::Panels && shrink == 0 && fits(LARGE.2) {
        return place(LARGE.0, LARGE.1, false, 0, Some(LARGE.4));
    }
    // The wide body without its lists is only for terminals too narrow for them.
    let fitting: Vec<_> = SHELLS
        .iter()
        .filter(|shell| fits(shell.2) && (shell.3 || shell.0 != Kind::Wide || !fits((140, 39))))
        .collect();
    match fitting.get(usize::from(shrink).min(fitting.len().saturating_sub(1))) {
        Some((kind, size, _, sides, body)) => {
            // The focus layout fills the terminal: no room is left for panels.
            let body = (*kind != Kind::Focus).then_some(*body);
            place(*kind, *size, *sides, fitting.len() as u8 - 1, body)
        }
        None => place(Kind::TooSmall, (40, 12), false, 0, None),
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
        assert_eq!(kind(80, 24, View::Shell), Kind::Focus);
        assert_eq!(kind(68, 38, View::Panels), Kind::Focus);
        assert_eq!(kind(30, 19, View::Shell), Kind::Focus);
        assert_eq!(kind(30, 18, View::Shell), Kind::TooSmall);
        assert_eq!(kind(40, 12, View::Shell), Kind::TooSmall);
        assert_eq!(kind(40, 12, View::Screen), Kind::Screen);
        assert_eq!(kind(12, 4, View::Screen), Kind::TooSmall);
        // Panels fall back to the key lists where they do not fit.
        let tight = plan(150, 45, View::Panels, 0);
        assert_eq!(
            (tight.kind, tight.sides, tight.panels),
            (Kind::Wide, true, None)
        );
    }

    #[test]
    fn panels_take_the_room_beside_the_body() {
        // The designed 200x56: 62 columns each side, rows 2 to 53.
        let designed = plan(200, 56, View::Panels, 0);
        assert_eq!(
            designed.panels,
            Some((Rect::new(1, 2, 62, 52), Rect::new(137, 2, 62, 52)))
        );
        // A wide but short terminal: the wide body, narrower panels, no key lists.
        let short = plan(180, 45, View::Panels, 0);
        assert_eq!((short.kind, short.sides), (Kind::Wide, false));
        assert_eq!(
            short.panels,
            Some((Rect::new(1, 5, 46, 35), Rect::new(133, 5, 46, 35)))
        );
        // Bigger than the design: the panels stay beside the body.
        let roomy = plan(260, 72, View::Panels, 0);
        assert_eq!(roomy.kind, Kind::Large);
        assert_eq!(
            roomy.panels,
            Some((Rect::new(31, 10, 62, 52), Rect::new(167, 10, 62, 52)))
        );
        // The plain view never has them; a narrow pane has no room.
        assert_eq!(plan(260, 72, View::Shell, 0).panels, None);
        assert_eq!(plan(105, 75, View::Panels, 0).panels, None);
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
                Kind::Focus,
                Kind::Focus,
                Kind::Focus
            ]
        );
        assert_eq!(plan(200, 60, View::Shell, 0).smaller, 3);
        assert_eq!(plan(80, 24, View::Shell, 3).kind, Kind::Focus);
    }
}
