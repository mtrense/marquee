//! Render backends for `mrq` (html, ansi, pdf, man, md, json), highlighting, diagrams and themes.

pub mod ansi;
pub mod diagram;
pub mod highlight;
pub mod html;
pub mod json;
pub mod man;
pub mod md;
pub mod theme;

pub use theme::Theme;
