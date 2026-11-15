//! Chooses how the picture is drawn (terminal pixels or half-blocks) from the
//! environment. No terminal queries are sent: probes are unreliable through
//! Windows console layers. Detection is a pure function of injected values
//! (the same rules as vostok-camera and vostok-files).

#[derive(Clone, Debug, Default)]
pub struct Env {
    pub term: String,
    pub term_program: String,
    pub herdr: bool,
    pub kitty: bool,
    pub ghostty: bool,
    pub wezterm: bool,
    pub konsole: bool,
    pub tmux: bool,
    /// `VOSTOK_GAMEBOY_RENDER`: `pixels`/`kitty` or `text`/`blocks`.
    pub requested: String,
}

impl Env {
    pub fn from_process() -> Self {
        let text = |name: &str| std::env::var(name).unwrap_or_default();
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        Self {
            term: text("TERM"),
            term_program: text("TERM_PROGRAM"),
            herdr: set("HERDR_ENV"),
            kitty: set("KITTY_WINDOW_ID"),
            ghostty: set("GHOSTTY_RESOURCES_DIR"),
            wezterm: set("WEZTERM_EXECUTABLE"),
            konsole: set("KONSOLE_VERSION"),
            tmux: set("TMUX"),
            requested: text("VOSTOK_GAMEBOY_RENDER"),
        }
    }

    pub fn pixel_capable(&self) -> bool {
        let term = self.term.to_ascii_lowercase();
        let program = self.term_program.to_ascii_lowercase();
        self.herdr
            || self.kitty
            || self.ghostty
            || self.wezterm
            || self.konsole
            || term.contains("kitty")
            || term.contains("ghostty")
            || matches!(program.as_str(), "ghostty" | "wezterm" | "kitty")
    }
}

/// `explicit` is the command-line override: `Some(true)` pixels, `Some(false)` text.
pub fn pixels(explicit: Option<bool>, env: &Env) -> bool {
    if let Some(choice) = explicit {
        return choice;
    }
    match env.requested.to_ascii_lowercase().as_str() {
        "kitty" | "pixels" | "graphics" => true,
        "blocks" | "halfblocks" | "text" => false,
        _ => env.pixel_capable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_only_where_the_terminal_can_draw_them() {
        let plain = Env::default();
        assert!(!pixels(None, &plain));
        assert!(pixels(Some(true), &plain));
        let herdr = Env {
            herdr: true,
            ..Env::default()
        };
        assert!(pixels(None, &herdr));
        assert!(!pixels(Some(false), &herdr));
        let asked = Env {
            requested: "text".into(),
            wezterm: true,
            ..Env::default()
        };
        assert!(!pixels(None, &asked));
        let kitty = Env {
            term: "xterm-kitty".into(),
            ..Env::default()
        };
        assert!(pixels(None, &kitty));
    }
}
