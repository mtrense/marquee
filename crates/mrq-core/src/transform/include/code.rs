//! Code includes: an include with `lang` or a symbol (`function`, `struct`, `class`,
//! `symbol`) is a fenced code block of the file's text, not Markdown. See [`Run::code`].

use std::path::Path;

use super::{INCLUDE_CODE, Included, Run, Spec, dedent};
use crate::ast::DirectiveAttribute;
use crate::diag::{Diagnostic, Severity};
use crate::document::Document;
use crate::symbol::{Symbol, SymbolKind, split_path};

/// The attributes selecting a symbol, and the kind each selects (any for `symbol`).
pub(super) const SYMBOL_KINDS: &[(&str, Option<SymbolKind>)] = &[
    ("function", Some(SymbolKind::Function)),
    ("struct", Some(SymbolKind::Struct)),
    ("class", Some(SymbolKind::Class)),
    ("symbol", None),
];

/// How many symbols a "not found" diagnostic lists at most.
const MAX_LISTED: usize = 10;

/// How a code include picks and fences its code.
#[derive(Debug)]
pub(super) struct CodeSpec<'d> {
    /// The fence's info string; with a symbol, the language's name if `None`.
    pub lang: Option<&'d DirectiveAttribute>,
    /// The lines to keep (all if `None`), and the attribute.
    pub lines: Option<(LineRange, &'d DirectiveAttribute)>,
    /// The symbol to keep (the whole file if `None`).
    pub symbol: Option<SymbolSpec<'d>>,
}

/// The symbol a code include keeps.
#[derive(Debug)]
pub(super) struct SymbolSpec<'d> {
    /// The attribute naming it (`function="Point::new"`).
    pub attr: &'d DirectiveAttribute,
    /// The kind of symbol it selects; any if `None`.
    pub kind: Option<SymbolKind>,
    /// Whether to keep the doc comments, attributes and decorators before it.
    pub with_docs: bool,
}

impl<'d> CodeSpec<'d> {
    /// The code include of the code attributes `attrs` (`lang`, `lines`, the symbol
    /// attributes and `with_docs`) of an include in `host`, the first of each name; `None`
    /// without a `lang` or symbol, or after reporting an error in `out`.
    pub fn parse(
        host: &Document,
        attrs: &[&'d DirectiveAttribute],
        out: &mut Vec<Diagnostic>,
    ) -> Option<Self> {
        let error = |attr: &DirectiveAttribute, message: String| {
            Diagnostic::new(
                INCLUDE_CODE,
                Severity::Error,
                host.name(),
                attr.span,
                message,
            )
        };
        let lang = attrs.iter().find(|attr| attr.name == "lang").copied();
        let mut symbol: Option<SymbolSpec<'d>> = None;
        let mut lines = None;
        let mut with_docs = None;
        let mut failed = false;
        for &attr in attrs {
            let value = attr.value.as_str();
            let kind = SYMBOL_KINDS.iter().find(|(name, _)| *name == attr.name);
            match attr.name.as_str() {
                "lang" if value.trim().is_empty() || value.contains(['\n', '\r']) => {
                    out.push(error(
                        attr,
                        "`lang` must name a language, like `lang=rust`".into(),
                    ));
                    failed = true;
                }
                "lines" => match LineRange::parse(value) {
                    Ok(range) => lines = Some((range, attr)),
                    Err(message) => out.push(error(attr, message)),
                },
                "with_docs" => match value {
                    "true" => with_docs = Some((true, attr)),
                    "false" => with_docs = Some((false, attr)),
                    _ => out.push(error(
                        attr,
                        format!("`with_docs` must be `true` or `false`, not `{value}`"),
                    )),
                },
                name if kind.is_some() => {
                    if let Some(first) = &symbol {
                        out.push(error(
                            attr,
                            format!(
                                "`{}` and `{name}` both select a symbol; keep one",
                                first.attr.name
                            ),
                        ));
                    } else if value.trim().is_empty() {
                        out.push(error(
                            attr,
                            format!("`{name}` must name a symbol, like `{name}=main`"),
                        ));
                        failed = true;
                    } else {
                        symbol = Some(SymbolSpec {
                            attr,
                            kind: kind.and_then(|(_, kind)| *kind),
                            with_docs: false,
                        });
                    }
                }
                _ => {}
            }
        }
        if lang.is_none()
            && symbol.is_none()
            && let Some((_, attr)) = lines
        {
            out.push(error(
                attr,
                "`lines` only applies to code includes, which need a `lang`".into(),
            ));
        }
        if let Some((with_docs, attr)) = with_docs {
            match &mut symbol {
                Some(symbol) => symbol.with_docs = with_docs,
                None => out.push(error(
                    attr,
                    "`with_docs` only applies with `function`, `struct`, `class` or `symbol`"
                        .into(),
                )),
            }
        }
        if failed || (lang.is_none() && symbol.is_none()) {
            return None;
        }
        Some(Self {
            lang,
            lines,
            symbol,
        })
    }
}

/// A `lines` range: from line `start` through line `end` (the last line if `None`), both
/// counted from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LineRange {
    start: usize,
    end: Option<usize>,
}

impl LineRange {
    /// The range written `10-20`, `10-`, `-20` or `10`; the error message if `value` is
    /// none of these.
    pub fn parse(value: &str) -> Result<Self, String> {
        let syntax =
            || format!("`lines` must be a range like `10-20`, `10-` or `-20`, not `{value}`");
        let number = |text: &str| -> Result<Option<usize>, String> {
            if text.is_empty() {
                Ok(None)
            } else if text.bytes().all(|b| b.is_ascii_digit()) {
                text.parse().map(Some).map_err(|_| syntax())
            } else {
                Err(syntax())
            }
        };
        let (start, end) = if let Some((start, end)) = value.trim().split_once('-') {
            (number(start)?, number(end)?)
        } else {
            let line = number(value.trim())?;
            (line, line)
        };
        if start.is_none() && end.is_none() {
            return Err(syntax());
        }
        if start == Some(0) || end == Some(0) {
            return Err(format!("`lines` count from line 1, not `{value}`"));
        }
        if let (Some(start), Some(end)) = (start, end)
            && end < start
        {
            return Err(format!("`lines` range `{value}` ends before it starts"));
        }
        Ok(Self {
            start: start.unwrap_or(1),
            end,
        })
    }

    /// The lines of `text` in the range, its end cut to the last line; the number of lines
    /// if the range starts after it.
    fn select(self, text: &str) -> Result<String, usize> {
        let count = text.split_inclusive('\n').count();
        if self.start > count {
            return Err(count);
        }
        let end = self.end.map_or(count, |end| end.min(count));
        Ok(text
            .split_inclusive('\n')
            .skip(self.start - 1)
            .take(end + 1 - self.start)
            .collect())
    }
}

impl Run<'_> {
    /// The content of an include of `text` as code, by `code`, a part of `spec`; `None`
    /// after reporting why there is none. It is made in this order:
    ///
    /// 1. Line endings become `\n`, and a byte order mark is dropped.
    /// 2. A symbol attribute keeps the symbol's definition ([`Run::symbol`]).
    /// 3. `lines` keeps the lines in its range.
    /// 4. `dedent` (on by default) strips the indentation all non-blank lines share.
    /// 5. Leading and trailing blank lines are dropped.
    /// 6. The rest is fenced with `lang` (else the symbol's language) as the info string,
    ///    in backticks (tildes if the info string has a backtick), one more than the
    ///    longest run of them in the code and at least three.
    pub(super) fn code(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        code: &CodeSpec<'_>,
        text: &str,
    ) -> Option<Included> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut text = text.replace("\r\n", "\n");
        let mut info = code.lang.map(|lang| lang.value.trim().to_owned());
        // What `lines` counts in, for diagnostics.
        let mut within = format!("`{}`", spec.path.value);
        if let Some(symbol) = &code.symbol {
            let (language, kept) = self.symbol(host, spec, code, symbol, &text)?;
            info.get_or_insert(language);
            within = format!("`{}` in `{}`", symbol.attr.value, spec.path.value);
            text = kept;
        }
        if let Some((range, attr)) = code.lines {
            text = match range.select(&text) {
                Ok(text) => text,
                Err(count) => {
                    let message = format!(
                        "`lines` starts at line {}, but {within} has {count} line{}",
                        range.start,
                        if count == 1 { "" } else { "s" },
                    );
                    self.diagnostics.push(Diagnostic::new(
                        INCLUDE_CODE,
                        Severity::Error,
                        host.name(),
                        attr.span,
                        message,
                    ));
                    return None;
                }
            };
        }
        if spec.dedent() {
            text = dedent(&text);
        }
        Some(Included {
            text: fence(
                info.as_deref().unwrap_or_default(),
                &trim_blank_lines(&text),
            ),
            frontmatter: None,
        })
    }

    /// The language of `text`, the code included by `spec`, and the lines of the symbol
    /// `symbol` selects in it: from the start of its definition (of its docs, with
    /// `with_docs`), or of the line it starts on if only indentation comes before it,
    /// through its end. `None` after reporting why there are none.
    ///
    /// A name selects the symbols of the kind whose qualified names end with its parts
    /// (`new` selects `Point::new`; `::` and `.` both separate parts), the one whose name
    /// is the whole name if there are several. None, or several, is an error listing the
    /// candidates.
    fn symbol(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        code: &CodeSpec<'_>,
        symbol: &SymbolSpec<'_>,
        text: &str,
    ) -> Option<(String, String)> {
        let attr = symbol.attr;
        let path = &spec.path.value;
        let report = |message: String| {
            Diagnostic::new(
                INCLUDE_CODE,
                Severity::Error,
                host.name(),
                attr.span,
                message,
            )
        };
        let Some(locator) = &self.includer.symbols else {
            self.diagnostics.push(report(format!(
                "`{}` needs mrq built with the `tree-sitter` feature; the include is skipped",
                attr.name
            )));
            return None;
        };
        let lang = code.lang.map(|lang| lang.value.as_str());
        let Some(language) = locator.language(Path::new(path), lang) else {
            self.diagnostics.push(report(format!(
                "cannot find `{}` in `{path}`: no grammar for its language (known: {})",
                attr.name,
                locator.languages().join(", "),
            )));
            return None;
        };
        let symbols = locator.symbols(&language, text);
        let found = match select(&symbols, symbol, text, path) {
            Ok(found) => found,
            Err(diagnostic) => {
                self.diagnostics.push(diagnostic.apply(report));
                return None;
            }
        };
        let start = if symbol.with_docs {
            found.docs_start
        } else {
            found.range.start
        };
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let start = if text[line_start..start].trim().is_empty() {
            line_start
        } else {
            start
        };
        let mut kept = text[start..found.range.end].to_owned();
        kept.push('\n');
        Some((language, kept))
    }
}

/// Why no symbol was selected: the message and help of the diagnostic.
struct NotSelected {
    message: String,
    help: Option<String>,
}

impl NotSelected {
    /// The diagnostic `report` makes of the message, with the help.
    fn apply(self, report: impl FnOnce(String) -> Diagnostic) -> Diagnostic {
        let diagnostic = report(self.message);
        match self.help {
            Some(help) => diagnostic.with_help(help),
            None => diagnostic,
        }
    }
}

/// The symbol of `symbols`, found in `text` at `path`, that `spec` selects; see
/// [`Run::symbol`].
fn select<'s>(
    symbols: &'s [Symbol],
    spec: &SymbolSpec<'_>,
    text: &str,
    path: &str,
) -> Result<&'s Symbol, NotSelected> {
    let wanted = split_path(&spec.attr.value);
    let named = |symbol: &&Symbol| symbol.path().ends_with(&wanted);
    let of_kind = |symbol: &&Symbol| spec.kind.is_none_or(|kind| symbol.kind == kind);
    let what = spec.attr.name.as_str();
    let name = spec.attr.value.trim();
    let mut found: Vec<&Symbol> = symbols.iter().filter(of_kind).filter(named).collect();
    if found.len() > 1 {
        let exact: Vec<&Symbol> = found
            .iter()
            .copied()
            .filter(|symbol| symbol.path() == wanted)
            .collect();
        if exact.len() == 1 {
            found = exact;
        }
    }
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(not_found(symbols, spec, text, path)),
        several => Err(NotSelected {
            message: format!(
                "{what} `{name}` is ambiguous in `{path}`: {}",
                list(several, text)
            ),
            help: Some(format!(
                "qualify the name, like `{what}=\"{}\"`",
                several[0].name
            )),
        }),
    }
}

/// Why `spec` selects none of `symbols`, found in `text` at `path`: the symbols of other
/// kinds with the name, else those of the kind with a similar name, else those of the
/// kind.
fn not_found(symbols: &[Symbol], spec: &SymbolSpec<'_>, text: &str, path: &str) -> NotSelected {
    let wanted = split_path(&spec.attr.value);
    let what = spec.attr.name.as_str();
    let name = spec.attr.value.trim();
    let missing = format!("no {what} `{name}` in `{path}`");
    let other_kinds: Vec<&Symbol> = symbols
        .iter()
        .filter(|symbol| symbol.path().ends_with(&wanted))
        .collect();
    if !other_kinds.is_empty() {
        return NotSelected {
            message: format!("{missing}; did you mean {}?", list(&other_kinds, text)),
            help: Some("`symbol=` selects a symbol of any kind".into()),
        };
    }
    let of_kind: Vec<&Symbol> = symbols
        .iter()
        .filter(|symbol| spec.kind.is_none_or(|kind| symbol.kind == kind))
        .collect();
    let last = wanted.last().copied().unwrap_or_default().to_lowercase();
    let similar: Vec<&Symbol> = of_kind
        .iter()
        .copied()
        .filter(|symbol| {
            let own = symbol
                .path()
                .last()
                .copied()
                .unwrap_or_default()
                .to_lowercase();
            distance(&own, &last) <= (last.chars().count() / 3).max(1)
        })
        .collect();
    if !similar.is_empty() {
        return NotSelected {
            message: format!("{missing}; did you mean {}?", list(&similar, text)),
            help: None,
        };
    }
    let message = if of_kind.is_empty() {
        let plural = if what.ends_with('s') { "es" } else { "s" };
        format!("{missing}; it has no {what}{plural}")
    } else {
        format!("{missing}; it has {}", list(&of_kind, text))
    };
    NotSelected {
        message,
        help: None,
    }
}

/// The list ``function `Point::new` (line 14), struct `Point` (line 7)`` of `symbols` in `text`, the
/// first [`MAX_LISTED`] of them.
fn list(symbols: &[&Symbol], text: &str) -> String {
    let mut out: Vec<String> = symbols
        .iter()
        .take(MAX_LISTED)
        .map(|symbol| {
            let line = text[..symbol.range.start].matches('\n').count() + 1;
            format!("{} `{}` (line {line})", symbol.kind, symbol.name)
        })
        .collect();
    if symbols.len() > MAX_LISTED {
        out.push(format!("and {} more", symbols.len() - MAX_LISTED));
    }
    out.join(", ")
}

/// The edit distance between `a` and `b` in characters: insertions, deletions,
/// substitutions and swaps of neighbours (`nrom` is 1 from `norm`).
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // Rows i - 2, i - 1 and i of the distances between the prefixes of `a` and `b`.
    let mut before: Vec<usize> = Vec::new();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (previous[j - 1] + cost)
                .min(previous[j] + 1)
                .min(row[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut previous, row);
    }
    previous[b.len()]
}

/// `text` without leading and trailing blank lines, ending with a newline unless it is
/// empty.
fn trim_blank_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let blank = |line: &&str| line.trim().is_empty();
    let Some(first) = lines.iter().position(|line| !blank(line)) else {
        return String::new();
    };
    let last = lines.iter().rposition(|line| !blank(line)).unwrap_or(first);
    let mut out = lines[first..=last].join("\n");
    out.push('\n');
    out
}

/// `code` (empty, or lines ending with a newline) as a fenced code block with the info
/// string `info`.
fn fence(info: &str, code: &str) -> String {
    let fence_char = if info.contains('`') { '~' } else { '`' };
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        run = if c == fence_char { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = fence_char.to_string().repeat((longest + 1).max(3));
    format!("{fence}{info}\n{code}{fence}\n")
}
