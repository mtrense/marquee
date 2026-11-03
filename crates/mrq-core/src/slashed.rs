//! Paths shown to people and tools use `/` separators on every platform, so output,
//! diagnostics and snapshots read the same on Windows as elsewhere.

use std::path::Path;

/// A path rendered with `/` separators, for messages and output.
pub trait Slashed {
    /// The path as text with `/` separators (lossy for non-UTF-8 paths).
    fn slashed(&self) -> String;
}

impl Slashed for Path {
    fn slashed(&self) -> String {
        let text = self.to_string_lossy();
        if std::path::MAIN_SEPARATOR == '\\' {
            text.replace('\\', "/")
        } else {
            text.into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_style_paths_are_unchanged() {
        assert_eq!(Path::new("docs/a b.md").slashed(), "docs/a b.md");
    }

    #[cfg(windows)]
    #[test]
    fn windows_separators_become_slashes() {
        assert_eq!(Path::new(r"C:\docs\a.md").slashed(), "C:/docs/a.md");
    }
}
