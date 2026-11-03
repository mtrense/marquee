//! Paths shown to people and tools use `/` separators on every platform, so output,
//! diagnostics and snapshots read the same on Windows as elsewhere.

use std::path::Path;

/// A path rendered with `/` separators, for messages and output.
pub trait Slashed {
    /// The path as text with `/` separators (lossy for non-UTF-8 paths).
    ///
    /// On Windows the verbatim prefix `fs::canonicalize` adds is dropped as well:
    /// `\\?\C:\docs\a.md` reads `C:/docs/a.md`, `\\?\UNC\host\share\a.md` reads
    /// `//host/share/a.md`.
    fn slashed(&self) -> String;
}

impl Slashed for Path {
    fn slashed(&self) -> String {
        let text = self.to_string_lossy();
        if std::path::MAIN_SEPARATOR == '\\' {
            windows_slashed(&text)
        } else {
            text.into_owned()
        }
    }
}

/// `text` without a verbatim prefix and with `\` replaced by `/`.
fn windows_slashed(text: &str) -> String {
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(text).to_owned()
    };
    plain.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_style_paths_are_unchanged() {
        assert_eq!(Path::new("docs/a b.md").slashed(), "docs/a b.md");
    }

    #[test]
    fn windows_paths_lose_the_verbatim_prefix_and_backslashes() {
        assert_eq!(windows_slashed(r"C:\docs\a.md"), "C:/docs/a.md");
        assert_eq!(windows_slashed(r"\\?\C:\docs\a.md"), "C:/docs/a.md");
        assert_eq!(
            windows_slashed(r"\\?\UNC\host\share\a.md"),
            "//host/share/a.md"
        );
        assert_eq!(windows_slashed(r"\\host\share\a.md"), "//host/share/a.md");
        assert_eq!(windows_slashed(r"docs\a.md"), "docs/a.md");
    }

    #[cfg(windows)]
    #[test]
    fn windows_separators_become_slashes() {
        assert_eq!(Path::new(r"\\?\C:\docs\a.md").slashed(), "C:/docs/a.md");
    }
}
