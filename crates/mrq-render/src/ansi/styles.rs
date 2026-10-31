//! The style of each element: from the [`Theme`], then the theme's `overrides.ansi`
//! file.

use serde::Deserialize;

use super::TextStyle;
use crate::html::css::ALERT_KINDS;
use crate::theme::{Color, Theme};

/// One element's entry in an `overrides.ansi` file: the properties it sets.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ElementOverride {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: Option<bool>,
    dim: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strikethrough: Option<bool>,
    reverse: Option<bool>,
}

impl ElementOverride {
    fn apply(self, style: &mut TextStyle) {
        style.fg = self.fg.or(style.fg);
        style.bg = self.bg.or(style.bg);
        let flags = [
            (self.bold, &mut style.bold),
            (self.dim, &mut style.dim),
            (self.italic, &mut style.italic),
            (self.underline, &mut style.underline),
            (self.strikethrough, &mut style.strikethrough),
            (self.reverse, &mut style.reverse),
        ];
        for (set, flag) in flags {
            if let Some(set) = set {
                *flag = set;
            }
        }
    }
}

macro_rules! elements {
    ($( $(#[$doc:meta])* $name:ident, )*) => {
        /// The style of every element the ANSI backend draws.
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub(crate) struct Styles {
            $( $(#[$doc])* pub(crate) $name: TextStyle, )*
        }

        /// The keys of an `overrides.ansi` file, one per element.
        #[derive(Debug, Clone, Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Overrides {
            $( $name: Option<ElementOverride>, )*
        }

        impl Styles {
            fn apply(&mut self, overrides: &Overrides) {
                $(
                    if let Some(o) = overrides.$name {
                        o.apply(&mut self.$name);
                    }
                )*
            }
        }
    };
}

elements! {
    h1, h2, h3, h4, h5, h6,
    /// The `#` marks before a heading.
    heading_marker,
    /// The rule under a heading with `underline`.
    heading_rule,
    emphasis, strong, strikethrough,
    /// `==mark==`.
    highlight,
    /// Inline code.
    code,
    /// Code blocks without a highlighted language, and the padding of highlighted ones.
    code_block,
    link,
    /// A link's URL shown after its text without hyperlinks.
    link_url,
    /// Blockquote text.
    quote,
    /// The bar before blockquote lines.
    quote_bar,
    /// Thematic breaks and the rule before the footnotes.
    rule,
    table_border, table_header,
    /// Bullets and numbers.
    list_marker,
    /// Task check boxes.
    task,
    tag,
    footnote_ref,
    /// The footnotes at the end.
    footnote,
    /// Raw HTML.
    html,
    math,
    /// Alt text of images shown as text.
    image,
    definition_term,
    alert_note, alert_tip, alert_important, alert_warning, alert_caution,
}

/// A foreground color.
fn fg(color: Color) -> TextStyle {
    TextStyle {
        fg: Some(color),
        ..TextStyle::default()
    }
}

impl Styles {
    /// The theme's styles. Body text and headings keep the terminal's colors (unless a
    /// heading style sets one), since the theme's foreground was chosen for its own
    /// background, which a terminal does not paint.
    pub(crate) fn from_theme(theme: &Theme) -> Self {
        let c = &theme.colors;
        let heading = |level: u8| {
            let style = theme.headings.level(level);
            TextStyle {
                fg: style.color,
                bold: style.weight.is_none_or(|w| w.number() >= 600),
                italic: style.italic.unwrap_or(false),
                ..TextStyle::default()
            }
        };
        let code = TextStyle {
            fg: Some(c.code_foreground),
            bg: Some(c.code_background),
            ..TextStyle::default()
        };
        Self {
            h1: heading(1),
            h2: heading(2),
            h3: heading(3),
            h4: heading(4),
            h5: heading(5),
            h6: heading(6),
            heading_marker: fg(c.accent),
            heading_rule: fg(c.border),
            emphasis: TextStyle {
                italic: true,
                ..TextStyle::default()
            },
            strong: TextStyle {
                bold: true,
                ..TextStyle::default()
            },
            strikethrough: TextStyle {
                strikethrough: true,
                ..TextStyle::default()
            },
            highlight: TextStyle {
                reverse: true,
                ..TextStyle::default()
            },
            code,
            code_block: code,
            link: TextStyle {
                underline: true,
                ..fg(c.accent)
            },
            link_url: fg(c.muted),
            quote: fg(c.muted),
            quote_bar: fg(c.border),
            rule: fg(c.border),
            table_border: fg(c.border),
            table_header: TextStyle {
                bold: true,
                ..TextStyle::default()
            },
            list_marker: fg(c.accent),
            task: fg(c.accent),
            tag: fg(c.accent),
            footnote_ref: fg(c.accent),
            footnote: fg(c.muted),
            html: fg(c.muted),
            math: TextStyle {
                italic: true,
                ..fg(c.code_foreground)
            },
            image: TextStyle {
                italic: true,
                ..fg(c.muted)
            },
            definition_term: TextStyle {
                bold: true,
                ..TextStyle::default()
            },
            alert_note: fg(c.alerts.note),
            alert_tip: fg(c.alerts.tip),
            alert_important: fg(c.alerts.important),
            alert_warning: fg(c.alerts.warning),
            alert_caution: fg(c.alerts.caution),
        }
    }

    /// Applies an `overrides.ansi` file: YAML mapping element names to `fg`, `bg`,
    /// `bold`, `dim`, `italic`, `underline`, `strikethrough` and `reverse`.
    pub(crate) fn apply_overrides(&mut self, yaml: &str) -> Result<(), serde_saphyr::Error> {
        let overrides: Option<Overrides> = serde_saphyr::from_str(yaml)?;
        if let Some(overrides) = overrides {
            self.apply(&overrides);
        }
        Ok(())
    }

    /// The style of heading level `level` (1–6).
    pub(crate) const fn heading(&self, level: u8) -> TextStyle {
        match level {
            0 | 1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            5 => self.h5,
            _ => self.h6,
        }
    }

    /// The style of an alert, callout or admonition kind; unknown kinds are notes.
    pub(crate) fn alert(&self, kind: &str) -> TextStyle {
        let kind = kind.to_lowercase();
        let alert = ALERT_KINDS
            .iter()
            .find(|(_, kinds)| kinds.contains(&kind.as_str()))
            .map_or("note", |(alert, _)| alert);
        match alert {
            "tip" => self.alert_tip,
            "important" => self.alert_important,
            "warning" => self.alert_warning,
            "caution" => self.alert_caution,
            _ => self.alert_note,
        }
    }
}
