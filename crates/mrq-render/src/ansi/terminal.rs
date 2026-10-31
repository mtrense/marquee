//! What the terminal can show, guessed from `--color`, whether output is a terminal and
//! the environment, which [`Capabilities::detect`] takes as arguments so tests (and the
//! TUI) decide it themselves.

/// `--color auto|always|never`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorChoice {
    /// Colour on a terminal, unless `NO_COLOR` is set or `TERM` is `dumb`.
    #[default]
    Auto,
    /// Colour even on a pipe and with `NO_COLOR`.
    Always,
    /// No escape sequences at all.
    Never,
}

/// How colours are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorLevel {
    /// No SGR sequences: no colours and no bold, italic or underline either.
    None,
    /// The xterm 256-colour palette (`38;5;n`).
    Ansi256,
    /// 24-bit colour (`38;2;r;g;b`).
    TrueColor,
}

/// An inline image protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsProtocol {
    /// The kitty graphics protocol (kitty, Ghostty, Konsole, `WezTerm`).
    Kitty,
    /// iTerm2's inline images (`OSC 1337 File`; iTerm2, `WezTerm`, mintty).
    Iterm2,
    /// DEC sixel (foot, mlterm, xterm with `-ti vt340`, …); needs the `png` feature.
    Sixel,
}

/// What the output can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub color: ColorLevel,
    /// OSC 8 hyperlinks.
    pub hyperlinks: bool,
    pub graphics: Option<GraphicsProtocol>,
}

impl Capabilities {
    /// Plain text: no colour, hyperlinks or images.
    pub const fn plain() -> Self {
        Self {
            color: ColorLevel::None,
            hyperlinks: false,
            graphics: None,
        }
    }

    /// [`detect`](Self::detect) with the process environment.
    pub fn from_env(choice: ColorChoice, is_terminal: bool) -> Self {
        Self::detect(choice, is_terminal, |name| std::env::var(name).ok())
    }

    /// The capabilities for `choice` when output `is_terminal`, with `env` looking up
    /// environment variables:
    ///
    /// - **Colour:** none for [`ColorChoice::Never`]; for `Auto`, none on a pipe, with a
    ///   non-empty `NO_COLOR` (<https://no-color.org>) or `TERM=dumb`. The level is
    ///   true colour when `COLORTERM` is `truecolor` or `24bit`, else 256 colours.
    /// - **Hyperlinks:** on a terminal that is not `dumb`, unless `Never`; `NO_COLOR`
    ///   leaves them on.
    /// - **Graphics:** on a terminal only, unless `Never`, and not inside tmux or screen,
    ///   which drop the escapes. `MRQ_GRAPHICS` (`kitty`, `iterm2`, `sixel`, `none`)
    ///   decides when set; else kitty for `TERM=xterm-kitty`, `KITTY_WINDOW_ID` or
    ///   Ghostty, iTerm2 for iTerm2 (`TERM_PROGRAM` or `LC_TERMINAL`) and `WezTerm`, and
    ///   sixel for foot, mlterm and a `TERM` that mentions `sixel`.
    pub fn detect(
        choice: ColorChoice,
        is_terminal: bool,
        env: impl Fn(&str) -> Option<String>,
    ) -> Self {
        if choice == ColorChoice::Never {
            return Self::plain();
        }
        let var = |name: &str| env(name).filter(|value| !value.is_empty());
        let term = var("TERM").unwrap_or_default();
        let dumb = term == "dumb";
        let colored = match choice {
            ColorChoice::Always => true,
            _ => is_terminal && !dumb && var("NO_COLOR").is_none(),
        };
        let color = if !colored {
            ColorLevel::None
        } else if var("COLORTERM").is_some_and(|c| c == "truecolor" || c == "24bit") {
            ColorLevel::TrueColor
        } else {
            ColorLevel::Ansi256
        };
        let terminal = is_terminal && !dumb;
        let multiplexed = var("TMUX").is_some() || term.starts_with("screen");
        let graphics = if !terminal || multiplexed {
            None
        } else if let Some(forced) = var("MRQ_GRAPHICS") {
            match forced.to_ascii_lowercase().as_str() {
                "kitty" => Some(GraphicsProtocol::Kitty),
                "iterm2" | "iterm" => Some(GraphicsProtocol::Iterm2),
                "sixel" => Some(GraphicsProtocol::Sixel),
                _ => None,
            }
        } else {
            guess_graphics(&term, &var)
        };
        Self {
            color,
            hyperlinks: terminal,
            graphics,
        }
    }
}

fn guess_graphics(term: &str, var: &impl Fn(&str) -> Option<String>) -> Option<GraphicsProtocol> {
    let program = var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    if term == "xterm-kitty" || var("KITTY_WINDOW_ID").is_some() || program == "ghostty" {
        return Some(GraphicsProtocol::Kitty);
    }
    if program == "iterm.app"
        || program == "wezterm"
        || var("LC_TERMINAL").is_some_and(|t| t == "iTerm2")
    {
        return Some(GraphicsProtocol::Iterm2);
    }
    if term.starts_with("foot") || term.starts_with("mlterm") || term.contains("sixel") {
        return Some(GraphicsProtocol::Sixel);
    }
    None
}
