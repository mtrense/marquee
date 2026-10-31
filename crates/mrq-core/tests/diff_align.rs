//! Fixture tests for `mrq_core::diff::diff`: each `tests/fixtures/diff/<name>.old.md`
//! diffed against `<name>.new.md`, with the options [`FIXTURES`] gives, renders (see
//! [`render`]) as `<name>.diff.txt`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use mrq_core::Document;
use mrq_core::diff::{self, BlockOp, DiffOptions, DocDiff, SectionStatus, WordOp};

/// Fixture name and whether to diff modified blocks word by word.
const FIXTURES: &[(&str, bool)] = &[
    ("move", false),
    ("rename", false),
    ("insert", false),
    ("delete", false),
    ("edit", true),
    ("move-edit", false),
    ("rename-edit", false),
    ("reformat", false),
];

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diff")
}

fn diff_str(old: &str, new: &str, word: bool) -> DocDiff {
    diff::diff(
        &Document::parse(old),
        &Document::parse(new),
        &DiffOptions { word },
    )
}

fn fixture(name: &str) -> (String, String) {
    let dir = fixture_dir();
    let read = |side: &str| fs::read_to_string(dir.join(format!("{name}.{side}.md"))).unwrap();
    (read("old"), read("new"))
}

/// A path as `A / B`, the root as `(root)`.
fn path(path: &[String]) -> String {
    if path.is_empty() {
        "(root)".to_owned()
    } else {
        path.join(" / ")
    }
}

/// A canonical block on one line.
fn one_line(text: &str) -> String {
    text.replace('\n', "\\n")
}

/// One line per section: `=` unchanged, `+` added, `-` removed, `~` changed (with its
/// flags and, when moved or renamed, `<- old path`). Changed sections list their block
/// ops: `= block`, `+ block`, `- block`, `~ old => new` or, with word ops,
/// `~ same{+inserted+}[-deleted-]`.
fn render(diff: &DocDiff) -> String {
    let mut out = String::new();
    for section in &diff.sections {
        let new = section.new.as_ref().map(|s| path(&s.path));
        let old = section.old.as_ref().map(|s| path(&s.path));
        match section.status {
            SectionStatus::Added => writeln!(out, "+ {}", new.unwrap()),
            SectionStatus::Removed => writeln!(out, "- {}", old.unwrap()),
            SectionStatus::Matched if section.is_unchanged() => {
                writeln!(out, "= {}", new.unwrap())
            }
            SectionStatus::Matched => {
                let flags: Vec<&str> = [
                    (section.moved, "moved"),
                    (section.renamed, "renamed"),
                    (section.modified, "modified"),
                ]
                .into_iter()
                .filter_map(|(on, flag)| on.then_some(flag))
                .collect();
                write!(out, "~ {} [{}]", new.unwrap(), flags.join(", ")).unwrap();
                if section.moved || section.renamed {
                    write!(out, " <- {}", old.unwrap()).unwrap();
                }
                writeln!(out)
            }
        }
        .unwrap();
        if section.is_unchanged() {
            continue;
        }
        for op in &section.blocks {
            let line = match op {
                BlockOp::Equal { new, .. } => format!("= {}", one_line(&new.canonical)),
                BlockOp::Insert { new } => format!("+ {}", one_line(&new.canonical)),
                BlockOp::Delete { old } => format!("- {}", one_line(&old.canonical)),
                BlockOp::Modify {
                    words: Some(words), ..
                } => {
                    let mut line = "~ ".to_owned();
                    for word in words {
                        match word {
                            WordOp::Equal(text) => line.push_str(text),
                            WordOp::Insert(text) => write!(line, "{{+{text}+}}").unwrap(),
                            WordOp::Delete(text) => write!(line, "[-{text}-]").unwrap(),
                        }
                    }
                    one_line(&line)
                }
                BlockOp::Modify { old, new, .. } => format!(
                    "~ {} => {}",
                    one_line(&old.canonical),
                    one_line(&new.canonical)
                ),
            };
            writeln!(out, "  {line}").unwrap();
        }
    }
    out
}

#[test]
fn fixtures_diff_to_their_expected_output() {
    for &(name, word) in FIXTURES {
        let (old, new) = fixture(name);
        let expected = fs::read_to_string(fixture_dir().join(format!("{name}.diff.txt"))).unwrap();
        assert_eq!(render(&diff_str(&old, &new, word)), expected, "{name}");
    }
}

#[test]
fn every_fixture_is_listed() {
    for entry in fs::read_dir(fixture_dir()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap();
        let name = file.split('.').next().unwrap();
        assert!(
            FIXTURES.iter().any(|&(listed, _)| listed == name),
            "{file} is not in FIXTURES"
        );
    }
}

#[test]
fn only_formatting_changes_give_an_empty_diff() {
    for &(name, word) in FIXTURES {
        let (old, new) = fixture(name);
        let diff = diff_str(&old, &new, word);
        assert_eq!(diff.is_empty(), name == "reformat", "{name}");
        assert!(diff_str(&old, &old, word).is_empty(), "{name}");
    }
}

#[test]
fn diffs_are_deterministic() {
    for &(name, word) in FIXTURES {
        let (old, new) = fixture(name);
        assert_eq!(
            diff_str(&old, &new, word),
            diff_str(&old, &new, word),
            "{name}"
        );
    }
}

#[test]
fn word_ops_only_with_the_word_option() {
    let (old, new) = fixture("edit");
    let has_words = |diff: &DocDiff| {
        diff.sections
            .iter()
            .flat_map(|s| &s.blocks)
            .any(|op| matches!(op, BlockOp::Modify { words: Some(_), .. }))
    };
    assert!(has_words(&diff_str(&old, &new, true)));
    assert!(!has_words(&diff_str(&old, &new, false)));
}

#[test]
fn word_ops_split_off_punctuation() {
    let diff = diff_str(
        "Run it.\n\nthe *old* way\n",
        "Run it now.\n\nthe *new* way\n",
        true,
    );
    assert_eq!(
        render(&diff),
        "~ (root) [modified]\n  ~ Run it{+ now+}.\n  ~ the *[-old-]{+new+}* way\n"
    );
}

#[test]
fn sections_with_unrelated_content_are_not_matched() {
    let diff = diff_str("# A\n\nfoo bar baz\n", "# B\n\nqux quux corge\n", false);
    assert_eq!(
        render(&diff),
        "= (root)\n- A\n  - # A\n  - foo bar baz\n+ B\n  + # B\n  + qux quux corge\n"
    );
    // Empty sections match by title alone.
    let diff = diff_str("# Alpha\n", "# Omega\n", false);
    assert_eq!(
        render(&diff),
        "= (root)\n- Alpha\n  - # Alpha\n+ Omega\n  + # Omega\n"
    );
}

#[test]
fn exact_paths_win_over_content() {
    let old = "# A\n\n## Notes\n\nabout a\n\n# B\n\n## Notes\n\nabout b\n";
    let new = "# A\n\n## Notes\n\nabout b\n\n# B\n\n## Notes\n\nabout a\n";
    // Exact paths win over content: both `Notes` stay and are edited.
    assert_eq!(
        render(&diff_str(old, new, false)),
        "= (root)\n= A\n~ A / Notes [modified]\n  = ## Notes\n  ~ about a => about b\n= B\n~ B / Notes [modified]\n  = ## Notes\n  ~ about b => about a\n"
    );
    // Duplicate paths pair up in order.
    let old = "# A\n\none\n\n# A\n\ntwo\n";
    let new = "# A\n\none\n\n# A\n\ntwo!\n";
    assert_eq!(
        render(&diff_str(old, new, false)),
        "= (root)\n= A\n~ A [modified]\n  = # A\n  ~ two => two!\n"
    );
}

#[test]
fn reordered_siblings_mark_the_moved_one() {
    let old = "# A\n\na\n\n# B\n\nb\n\n# C\n\nc\n";
    let new = "# A\n\na\n\n# C\n\nc\n\n# B\n\nb\n";
    let diff = diff_str(old, new, false);
    let moved: Vec<_> = diff
        .sections
        .iter()
        .filter(|s| s.moved)
        .map(|s| s.new.as_ref().unwrap().title.clone())
        .collect();
    assert_eq!(moved.len(), 1, "{}", render(&diff));
}

#[test]
fn heading_level_changes_are_modifications() {
    let diff = diff_str("# A\n\n## B\n\nb\n", "# A\n\n### B\n\nb\n", false);
    assert_eq!(
        render(&diff),
        "= (root)\n= A\n~ A / B [modified]\n  ~ ## B => ### B\n  = b\n"
    );
}

#[test]
fn unlike_blocks_are_deleted_and_inserted() {
    let diff = diff_str("# A\n\nsome text here\n", "# A\n\n```\ncode\n```\n", false);
    assert_eq!(
        render(&diff),
        "= (root)\n~ A [modified]\n  = # A\n  - some text here\n  + ```\\ncode\\n```\n"
    );
}

#[test]
fn preamble_changes_belong_to_the_root() {
    let diff = diff_str("intro\n\n# A\n", "intro!\n\n# A\n", false);
    assert_eq!(
        render(&diff),
        "~ (root) [modified]\n  ~ intro => intro!\n= A\n"
    );
}

#[test]
fn sides_carry_spans_and_lines() {
    let (old, new) = fixture("move");
    let diff = diff_str(&old, &new, false);
    let usage = diff.sections.iter().find(|s| s.moved).unwrap();
    let (o, n) = (usage.old.as_ref().unwrap(), usage.new.as_ref().unwrap());
    assert_eq!((o.line, n.line), (10, 14));
    assert!(old[o.span.start..o.span.end].starts_with("## Usage"));
    assert!(new[n.span.start..n.span.end].starts_with("## Usage"));
    let BlockOp::Equal { old: ob, new: nb } = &usage.blocks[1] else {
        panic!("{:?}", usage.blocks[1]);
    };
    assert_eq!((ob.lines.clone(), nb.lines.clone()), (12..13, 16..17));
}

#[test]
fn json_shape() {
    let diff = diff_str(
        "# Setup\n\nRun it.\n\n# Old\n\ngone\n",
        "# Install\n\nRun it now.\n\n# New\n\nfresh\n",
        true,
    );
    insta::assert_snapshot!(serde_json::to_string_pretty(&diff).unwrap());
}
