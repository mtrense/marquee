//! The theme model: built-in themes, YAML loading, `extends` inheritance, path
//! resolution and the published JSON Schema.

use std::fs;
use std::path::{Path, PathBuf};

use mrq_render::theme::{
    self, BUILTIN_THEMES, Color, FontWeight, Length, LengthUnit, Theme, ThemeError,
};
use serde_json::Value;

/// Environment variable that makes [`schema_file_is_up_to_date`] rewrite the file.
const UPDATE_SCHEMA: &str = "MRQ_UPDATE_SCHEMA";

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/mrq-theme.schema.json")
}

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(&fs::read_to_string(schema_path()).unwrap()).unwrap();
    jsonschema::validator_for(&schema).expect("schema compiles")
}

fn yaml(text: &str) -> Theme {
    Theme::from_yaml(text, Path::new("/themes")).unwrap()
}

fn color(hex: &str) -> Color {
    hex.parse().unwrap()
}

/// A temp directory with theme files.
struct Dir {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Dir {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

#[test]
fn every_builtin_theme_loads() {
    assert_eq!(BUILTIN_THEMES, ["light", "dark", "github", "paper"]);
    for name in BUILTIN_THEMES {
        let theme = Theme::builtin(name).unwrap_or_else(|| panic!("{name} is missing"));
        assert_eq!(theme.extends, None, "{name}: a loaded theme has no extends");
    }
    assert!(Theme::builtin("solarized").is_none());
}

#[test]
fn light_is_the_default_theme() {
    assert_eq!(Theme::builtin("light").unwrap(), Theme::default());
}

#[test]
fn builtin_themes_differ_and_inherit_from_light() {
    let light = Theme::default();
    let dark = Theme::builtin("dark").unwrap();
    let github = Theme::builtin("github").unwrap();
    let paper = Theme::builtin("paper").unwrap();
    assert_ne!(dark.colors.background, light.colors.background);
    assert_ne!(dark.code.theme, light.code.theme);
    assert_ne!(github.fonts.body, light.fonts.body);
    assert_ne!(paper.fonts.body, light.fonts.body);
    // dark only recolors, so everything else comes from light.
    assert_eq!(dark.fonts, light.fonts);
    assert_eq!(dark.headings.h2.size, light.headings.h2.size);
}

#[test]
fn builtin_theme_sources_validate_against_the_schema() {
    let validator = validator();
    for (name, text) in theme::builtin_sources() {
        let value: Value = serde_saphyr::from_str(text).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
    }
}

#[test]
fn a_file_without_extends_builds_on_light() {
    let theme = yaml("colors:\n  accent: \"#ff0000\"\n");
    let light = Theme::default();
    assert_eq!(theme.colors.accent, color("#ff0000"));
    assert_eq!(theme.colors.foreground, light.colors.foreground);
    assert_eq!(theme.fonts, light.fonts);
}

#[test]
fn an_empty_file_is_light() {
    assert_eq!(yaml(""), Theme::default());
    assert_eq!(yaml("# nothing\n"), Theme::default());
}

#[test]
fn extends_a_builtin_and_overrides_single_keys() {
    let github = Theme::builtin("github").unwrap();
    let theme = yaml("extends: github\ncolors:\n  foreground: \"#010203\"\n");
    assert_eq!(theme.colors.foreground, color("#010203"));
    assert_eq!(theme.colors.background, github.colors.background);
    assert_eq!(theme.colors.alerts, github.colors.alerts);
    assert_eq!(theme.headings, github.headings);
    assert_eq!(theme.code, github.code);
}

#[test]
fn heading_styles_merge_property_by_property() {
    let github = Theme::builtin("github").unwrap();
    let theme = yaml("extends: github\nheadings:\n  h1: { italic: true }\n");
    assert_eq!(theme.headings.h1.italic, Some(true));
    assert_eq!(theme.headings.h1.size, github.headings.h1.size);
    assert_eq!(theme.headings.h1.weight, github.headings.h1.weight);
    assert_eq!(theme.headings.h2, github.headings.h2);
}

#[test]
fn null_unsets_an_inherited_optional_value() {
    let theme = yaml("headings:\n  h1: { underline: null }\nfonts:\n  heading: null\n");
    assert_eq!(theme.headings.h1.underline, None);
    assert_eq!(theme.fonts.heading, None);
}

#[test]
fn the_readme_example_loads() {
    let text = "\
extends: github
colors:
  foreground: \"#24292f\"
  accent: \"#0969da\"
  code_background: \"#f6f8fa\"
fonts:
  body: \"Inter\"
  mono: \"JetBrains Mono\"
headings:
  h1: { size: 2.0em, weight: bold, underline: true }
  h2: { size: 1.5em, weight: bold }
code:
  theme: \"Solarized (light)\"
overrides:
  html: ./extra.css
  pdf: ./template.typ
  ansi: ./ansi.yaml
";
    let value: Value = serde_saphyr::from_str(text).unwrap();
    assert!(validator().is_valid(&value));
    let theme = Theme::from_yaml(text, Path::new("/proj")).unwrap();
    assert_eq!(theme.fonts.body, "Inter");
    assert_eq!(theme.fonts.mono, "JetBrains Mono");
    assert_eq!(
        theme.headings.h1.size,
        Some(Length::new(2.0, LengthUnit::Em))
    );
    assert_eq!(theme.headings.h1.weight, Some(FontWeight::Bold));
    assert_eq!(theme.headings.h1.underline, Some(true));
    assert_eq!(theme.code.theme, "Solarized (light)");
    assert_eq!(
        theme.overrides.html.as_deref(),
        Some(Path::new("/proj/extra.css"))
    );
    assert_eq!(
        theme.overrides.pdf.as_deref(),
        Some(Path::new("/proj/template.typ"))
    );
    assert_eq!(
        theme.overrides.ansi.as_deref(),
        Some(Path::new("/proj/ansi.yaml"))
    );
}

#[test]
fn extends_files_relative_to_the_extending_file() {
    let dir = Dir::new();
    dir.write(
        "base/root.yaml",
        "extends: dark\nfonts:\n  body: Base\ncolors:\n  accent: \"#111111\"\n",
    );
    dir.write(
        "base/middle.yaml",
        "extends: ./root.yaml\ncolors:\n  accent: \"#222222\"\n  muted: \"#333333\"\n",
    );
    let leaf = dir.write(
        "leaf.yaml",
        "extends: base/middle.yaml\ncolors:\n  muted: \"#444444\"\n",
    );
    let theme = Theme::load(&leaf).unwrap();
    let dark = Theme::builtin("dark").unwrap();
    assert_eq!(theme.fonts.body, "Base");
    assert_eq!(theme.colors.accent, color("#222222"));
    assert_eq!(theme.colors.muted, color("#444444"));
    assert_eq!(theme.colors.background, dark.colors.background);
}

#[test]
fn paths_resolve_against_the_file_that_sets_them() {
    let dir = Dir::new();
    dir.write(
        "shared/base.yaml",
        "code:\n  theme: ./code/Mine.tmTheme\noverrides:\n  html: ./base.css\n  pdf: /abs/t.typ\n",
    );
    let leaf = dir.write(
        "site/theme.yaml",
        "extends: ../shared/base.yaml\noverrides:\n  ansi: ansi.yaml\n",
    );
    let theme = Theme::load(&leaf).unwrap();
    assert_eq!(
        theme.code.theme,
        dir.root
            .join("shared/./code/Mine.tmTheme")
            .display()
            .to_string()
    );
    assert_eq!(
        theme.overrides.html,
        Some(dir.root.join("shared/./base.css"))
    );
    assert_eq!(theme.overrides.pdf, Some(PathBuf::from("/abs/t.typ")));
    assert_eq!(theme.overrides.ansi, Some(dir.root.join("site/ansi.yaml")));
}

#[test]
fn a_code_theme_name_is_not_a_path() {
    let theme = yaml("code:\n  theme: base16-ocean.dark\n");
    assert_eq!(theme.code.theme, "base16-ocean.dark");
}

#[test]
fn resolve_takes_a_builtin_name_or_a_path() {
    let dir = Dir::new();
    dir.write("mine.yaml", "extends: paper\nfonts:\n  mono: Iosevka\n");
    assert_eq!(
        Theme::resolve("dark", &dir.root).unwrap(),
        Theme::builtin("dark").unwrap()
    );
    let mine = Theme::resolve("mine.yaml", &dir.root).unwrap();
    assert_eq!(mine.fonts.mono, "Iosevka");
    assert_eq!(mine.fonts.body, Theme::builtin("paper").unwrap().fonts.body);
}

#[test]
fn an_unknown_theme_name_is_an_error() {
    let err = Theme::resolve("solarized", Path::new("/")).unwrap_err();
    assert!(
        matches!(&err, ThemeError::Unknown { name } if name == "solarized"),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("light, dark, github, paper"),
        "{err}"
    );
    let err = Theme::from_yaml("extends: githb\n", Path::new("/")).unwrap_err();
    assert!(
        matches!(&err, ThemeError::Unknown { name } if name == "githb"),
        "{err:?}"
    );
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = Dir::new();
    let err = Theme::resolve("nope.yaml", &dir.root).unwrap_err();
    assert!(matches!(&err, ThemeError::Io { path, .. } if path == &dir.root.join("nope.yaml")));
    let leaf = dir.write("leaf.yaml", "extends: ./gone.yaml\n");
    let err = Theme::load(&leaf).unwrap_err();
    assert!(matches!(&err, ThemeError::Io { path, .. } if path == &dir.root.join("gone.yaml")));
}

#[test]
fn an_extends_cycle_is_an_error_with_the_chain() {
    let dir = Dir::new();
    let a = dir.write("a.yaml", "extends: b.yaml\n");
    dir.write("b.yaml", "extends: ./a.yaml\n");
    let err = Theme::load(&a).unwrap_err();
    let ThemeError::Cycle { chain } = &err else {
        panic!("expected a cycle, got {err:?}");
    };
    assert_eq!(chain, &[a.clone(), dir.root.join("b.yaml"), a.clone()]);
    let message = err.to_string();
    assert!(message.contains("a.yaml -> "), "{message}");

    let me = dir.write("me.yaml", "extends: me.yaml\n");
    assert!(matches!(Theme::load(&me), Err(ThemeError::Cycle { .. })));
}

#[test]
fn an_unknown_key_is_an_error_with_file_and_line() {
    let dir = Dir::new();
    let path = dir.write(
        "bad.yaml",
        "extends: dark\ncolors:\n  forground: \"#000000\"\n",
    );
    let err = Theme::load(&path).unwrap_err();
    let ThemeError::Invalid {
        path: at,
        line,
        message,
        ..
    } = &err
    else {
        panic!("expected an invalid theme, got {err:?}");
    };
    assert_eq!(at.as_deref(), Some(path.as_path()));
    assert_eq!(*line, 3);
    assert!(message.contains("forground"), "{message}");
    assert!(
        err.to_string()
            .starts_with(&format!("{}:3:", path.display())),
        "{err}"
    );
}

#[test]
fn an_invalid_value_is_an_error() {
    for text in [
        "colors:\n  accent: red\n",
        "colors:\n  accent: \"#12345\"\n",
        "headings:\n  h1: { size: big }\n",
        "headings:\n  h1: { weight: heavy }\n",
        "headings:\n  h7: {}\n",
        "extends: [a]\n",
        "- a list\n",
        "colors: [\n",
    ] {
        let err = Theme::from_yaml(text, Path::new("/")).unwrap_err();
        assert!(
            matches!(err, ThemeError::Invalid { path: None, .. }),
            "{text:?}: {err:?}"
        );
    }
}

#[test]
fn colors_parse_and_print_as_hex() {
    assert_eq!(color("#abc"), Color::rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(color("#A0B1C2"), Color::rgb(0xa0, 0xb1, 0xc2));
    assert_eq!(color("#a0b1c280"), Color::rgba(0xa0, 0xb1, 0xc2, 0x80));
    assert_eq!(color("#a0b1c2").to_string(), "#a0b1c2");
    assert_eq!(color("#a0b1c280").to_string(), "#a0b1c280");
    for bad in ["", "abc", "#ab", "#abcd", "#abcdefg", "#gggggg", "#ééé"] {
        assert!(bad.parse::<Color>().is_err(), "{bad:?}");
    }
}

#[test]
fn lengths_parse_and_print_with_their_unit() {
    assert_eq!("2.0em".parse(), Ok(Length::new(2.0, LengthUnit::Em)));
    assert_eq!("1.5rem".parse(), Ok(Length::new(1.5, LengthUnit::Rem)));
    assert_eq!("11pt".parse(), Ok(Length::new(11.0, LengthUnit::Pt)));
    assert_eq!("16px".parse(), Ok(Length::new(16.0, LengthUnit::Px)));
    assert_eq!(Length::new(2.0, LengthUnit::Em).to_string(), "2em");
    assert_eq!(Length::new(0.875, LengthUnit::Em).to_string(), "0.875em");
    for bad in ["", "2", "em", "2 em", "-1em", "2cm", "NaNem"] {
        assert!(bad.parse::<Length>().is_err(), "{bad:?}");
    }
}

#[test]
fn heading_levels_are_addressable() {
    let theme = Theme::default();
    assert_eq!(theme.headings.level(1), &theme.headings.h1);
    assert_eq!(theme.headings.level(6), &theme.headings.h6);
    assert_eq!(theme.headings.level(9), &theme.headings.h6);
    assert_eq!(FontWeight::Bold.number(), 700);
    assert_eq!(FontWeight::Normal.number(), 400);
}

#[test]
fn a_theme_round_trips_through_yaml() {
    for name in BUILTIN_THEMES {
        let theme = Theme::builtin(name).unwrap();
        let text = serde_saphyr::to_string(&theme).unwrap();
        assert_eq!(
            Theme::from_yaml(&text, Path::new("/")).unwrap(),
            theme,
            "{name}"
        );
    }
}

#[test]
fn schema_file_is_up_to_date() {
    let generated = theme::schema_json();
    let path = schema_path();
    if std::env::var_os(UPDATE_SCHEMA).is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &generated).unwrap();
    }
    let checked_in = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        checked_in == generated,
        "{} is stale; regenerate it with `just schema`",
        path.display()
    );
}

#[test]
fn schema_rejects_unknown_keys_and_bad_values() {
    let validator = validator();
    for text in [
        "colours: {}\n",
        "colors:\n  accent: red\n",
        "headings:\n  h1: { size: big }\n",
        "overrides:\n  man: ./x\n",
    ] {
        let value: Value = serde_saphyr::from_str(text).unwrap();
        assert!(!validator.is_valid(&value), "{text:?} should be invalid");
    }
}
