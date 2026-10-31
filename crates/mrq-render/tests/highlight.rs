//! Syntax highlighting: language lookup by fence info, code theme resolution and the
//! styled spans every backend consumes.

use std::fmt::Write as _;
use std::fs;

use mrq_render::highlight::{HighlightError, Highlighted, Highlighter, Style, language_token};
use mrq_render::theme::{BUILTIN_THEMES, CodeStyle, Color, Theme};

fn highlighter(theme: &str) -> Highlighter {
    Highlighter::new(&CodeStyle {
        theme: theme.to_owned(),
    })
    .unwrap()
}

/// One line per source line: each span as its quoted text followed by its style, e.g.
/// `"fn"{#a71d5d b}`; a span without any style has none.
fn render(highlighted: &Highlighted) -> String {
    let mut out = format!(
        "language: {}\nforeground: {}\nbackground: {}\n",
        highlighted.language.unwrap_or("-"),
        opt(highlighted.foreground),
        opt(highlighted.background),
    );
    for line in &highlighted.lines {
        let spans: Vec<String> = line
            .iter()
            .map(|span| format!("{:?}{}", span.text, style(&span.style)))
            .collect();
        writeln!(out, "{}", spans.join(" ")).unwrap();
    }
    out
}

fn opt(color: Option<Color>) -> String {
    color.map_or_else(|| "-".to_owned(), |c| c.to_string())
}

fn style(style: &Style) -> String {
    let mut parts = Vec::new();
    if let Some(fg) = style.foreground {
        parts.push(fg.to_string());
    }
    if let Some(bg) = style.background {
        parts.push(format!("bg={bg}"));
    }
    for (on, flag) in [
        (style.bold, "b"),
        (style.italic, "i"),
        (style.underline, "u"),
    ] {
        if on {
            parts.push(flag.to_owned());
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", parts.join(" "))
    }
}

#[test]
fn every_builtin_theme_code_theme_resolves() {
    for name in BUILTIN_THEMES {
        let theme = Theme::builtin(name).unwrap();
        assert!(
            Highlighter::new(&theme.code).is_ok(),
            "{name}: code theme `{}` does not resolve",
            theme.code.theme
        );
    }
}

#[test]
fn bundled_theme_names_match_case_insensitively() {
    let exact = highlighter("Solarized (light)");
    let folded = highlighter("solarized (LIGHT)");
    assert_eq!(exact.background(), folded.background());
}

#[test]
fn unknown_theme_is_an_error_listing_bundled_themes() {
    let err = Highlighter::new(&CodeStyle {
        theme: "Nope".to_owned(),
    })
    .err()
    .unwrap();
    assert!(matches!(err, HighlightError::UnknownTheme { .. }));
    let message = err.to_string();
    assert!(
        message.starts_with("unknown code theme `Nope`"),
        "{message}"
    );
    assert!(message.contains("InspiredGitHub"), "{message}");
    assert!(message.contains("Solarized (light)"), "{message}");
}

#[test]
fn tmtheme_file_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mini.tmTheme");
    fs::write(&path, MINI_TMTHEME).unwrap();
    let highlighter = highlighter(path.to_str().unwrap());
    assert_eq!(highlighter.foreground(), Some("#010203".parse().unwrap()));
    assert_eq!(highlighter.background(), Some("#fafafa".parse().unwrap()));
    let out = highlighter.highlight("// hi\nx\n", "rust");
    insta::assert_snapshot!(render(&out));
}

#[test]
fn missing_tmtheme_file_is_an_error() {
    let err = Highlighter::new(&CodeStyle {
        theme: "/does/not/exist.tmTheme".to_owned(),
    })
    .err()
    .unwrap();
    assert!(matches!(err, HighlightError::ThemeFile { .. }), "{err}");
    assert!(
        err.to_string()
            .starts_with("cannot load code theme /does/not/exist.tmTheme")
    );
}

#[test]
fn fence_info_aliases_find_languages() {
    let highlighter = highlighter("InspiredGitHub");
    let cases = [
        ("rust", "Rust"),
        ("rs", "Rust"),
        ("Rust", "Rust"),
        ("rust,ignore", "Rust"),
        ("rust title=\"main.rs\"", "Rust"),
        ("{.rust}", "Rust"),
        ("python", "Python"),
        ("py", "Python"),
        ("python3", "Python"),
        ("js", "JavaScript"),
        ("javascript", "JavaScript"),
        ("jsx", "JavaScript"),
        ("mjs", "JavaScript"),
        ("ts", "TypeScript"),
        ("typescript", "TypeScript"),
        ("tsx", "TypeScriptReact"),
        ("sh", "Bourne Again Shell (bash)"),
        ("bash", "Bourne Again Shell (bash)"),
        ("shell", "Bourne Again Shell (bash)"),
        ("zsh", "Bourne Again Shell (bash)"),
        ("console", "Bourne Again Shell (bash)"),
        ("yaml", "YAML"),
        ("yml", "YAML"),
        ("json", "JSON"),
        ("toml", "TOML"),
        ("c", "C"),
        ("c++", "C++"),
        ("cpp", "C++"),
        ("cs", "C#"),
        ("csharp", "C#"),
        ("go", "Go"),
        ("golang", "Go"),
        ("html", "HTML"),
        ("css", "CSS"),
        ("md", "Markdown"),
        ("markdown", "Markdown"),
        ("dockerfile", "Dockerfile"),
        ("docker", "Dockerfile"),
        ("diff", "Diff"),
        ("patch", "Diff"),
        ("sql", "SQL"),
        ("rb", "Ruby"),
        ("ruby", "Ruby"),
        ("kt", "Kotlin"),
        ("kotlin", "Kotlin"),
        ("java", "Java"),
        ("make", "Makefile"),
        ("makefile", "Makefile"),
        ("hs", "Haskell"),
        ("tex", "LaTeX"),
        ("latex", "LaTeX"),
        ("xml", "XML"),
        ("nix", "Nix"),
    ];
    let mut wrong = Vec::new();
    for (info, expected) in cases {
        let found = highlighter.language(info);
        if found != Some(expected) {
            wrong.push(format!("{info:?}: expected {expected}, got {found:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn plain_text_infos_find_no_language() {
    let highlighter = highlighter("InspiredGitHub");
    for info in [
        "",
        "   ",
        "text",
        "txt",
        "plain",
        "plaintext",
        "nosuchlang",
        "{}",
    ] {
        assert_eq!(highlighter.language(info), None, "{info:?}");
    }
}

#[test]
fn language_token_is_the_first_word_lowercased() {
    assert_eq!(language_token("Rust title=\"x\""), Some("rust".to_owned()));
    assert_eq!(language_token("rust,ignore"), Some("rust".to_owned()));
    assert_eq!(
        language_token("{.Python .numberLines}"),
        Some("python".to_owned())
    );
    assert_eq!(language_token("{python}"), Some("python".to_owned()));
    assert_eq!(language_token("  "), None);
    assert_eq!(language_token("{}"), None);
}

#[test]
fn plain_text_keeps_lines_without_style() {
    let highlighter = highlighter("InspiredGitHub");
    let out = highlighter.highlight("a <b>\n\n  c\n", "text");
    assert_eq!(out.language, None);
    assert_eq!(out.foreground, None);
    assert_eq!(out.background, None);
    insta::assert_snapshot!(render(&out));
}

#[test]
fn line_endings_are_not_part_of_spans() {
    let highlighter = highlighter("InspiredGitHub");
    let lf = highlighter.highlight("let x = 1;\nlet y = 2;", "rust");
    let crlf = highlighter.highlight("let x = 1;\r\nlet y = 2;\r\n", "rust");
    assert_eq!(lf.lines, crlf.lines);
    assert_eq!(lf.lines.len(), 2);
    let text: String = lf.lines[0].iter().map(|s| s.text.as_str()).collect();
    assert_eq!(text, "let x = 1;");
}

#[test]
fn spans_reproduce_the_source() {
    let highlighter = highlighter("base16-ocean.dark");
    let source = "def f(x):\n    return x * 2  # double\n\n\nprint(f(21))\n";
    let out = highlighter.highlight(source, "py");
    let text: Vec<String> = out
        .lines
        .iter()
        .map(|line| line.iter().map(|s| s.text.as_str()).collect())
        .collect();
    assert_eq!(text.join("\n") + "\n", source);
    // Adjacent spans never share a style.
    for line in &out.lines {
        for pair in line.windows(2) {
            assert_ne!(pair[0].style, pair[1].style, "{line:?}");
        }
    }
}

#[test]
fn style_css_declarations() {
    let style = Style {
        foreground: Some("#123456".parse().unwrap()),
        background: Some("#abcdef80".parse().unwrap()),
        bold: true,
        italic: true,
        underline: true,
    };
    assert_eq!(
        style.css(),
        "color:#123456;background-color:#abcdef80;font-weight:bold;font-style:italic;text-decoration:underline"
    );
    assert_eq!(Style::default().css(), "");
}

#[test]
fn snapshot_rust() {
    let out = highlighter("InspiredGitHub").highlight(RUST, "rust");
    insta::assert_snapshot!(render(&out));
}

#[test]
fn snapshot_python() {
    let out = highlighter("InspiredGitHub").highlight(PYTHON, "python");
    insta::assert_snapshot!(render(&out));
}

#[test]
fn snapshot_typescript() {
    let out = highlighter("GitHub").highlight(TYPESCRIPT, "ts");
    insta::assert_snapshot!(render(&out));
}

#[test]
fn snapshot_shell() {
    let out = highlighter("base16-ocean.dark").highlight(SHELL, "sh");
    insta::assert_snapshot!(render(&out));
}

#[test]
fn snapshot_yaml() {
    let out = highlighter("base16-ocean.dark").highlight(YAML, "yml");
    insta::assert_snapshot!(render(&out));
}

const RUST: &str = r#"/// Adds one.
#[inline]
pub fn add_one(x: i32) -> i32 {
    let s = "text";
    x + 1 // done
}
"#;

const PYTHON: &str = r#"import os

class Greeter:
    """Says hello."""

    def greet(self, name: str) -> str:
        return f"Hello, {name}!"  # friendly
"#;

const TYPESCRIPT: &str = r"interface Point {
  x: number;
}

export const origin: Point = { x: 0 };
";

const SHELL: &str = r#"#!/bin/sh
for f in *.md; do
  echo "file: $f" | grep -v draft
done
"#;

const YAML: &str = r#"# config
name: mrq
version: 1.2
tags: [cli, "markdown"]
enabled: true
"#;

const MINI_TMTHEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>name</key>
  <string>Mini</string>
  <key>settings</key>
  <array>
    <dict>
      <key>settings</key>
      <dict>
        <key>foreground</key>
        <string>#010203</string>
        <key>background</key>
        <string>#FAFAFA</string>
      </dict>
    </dict>
    <dict>
      <key>scope</key>
      <string>comment</string>
      <key>settings</key>
      <dict>
        <key>foreground</key>
        <string>#00FF00</string>
        <key>fontStyle</key>
        <string>italic</string>
      </dict>
    </dict>
  </array>
</dict>
</plist>
"#;
