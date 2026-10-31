//! Render backends for `mrq` (html, ansi, pdf, man, md, json), highlighting, diagrams and themes.

pub mod highlight;
pub mod html;
pub mod theme;

pub use theme::Theme;
