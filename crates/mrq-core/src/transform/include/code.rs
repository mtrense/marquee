//! Code includes: an include with `lang` is a fenced code block of the file's text,
//! not Markdown. See [`Run::code`].

use super::{INCLUDE_CODE, Included, Run, Spec, dedent};
use crate::ast::DirectiveAttribute;
use crate::diag::{Diagnostic, Severity};
use crate::document::Document;

/// How an include with `lang` picks and fences its code.
// TODO(M7.6): symbol selection (`function`, `struct`, `class`, `symbol`, `with_docs`)
// goes here too, picking the lines `Run::code` starts from.
#[derive(Debug)]
pub(super) struct CodeSpec<'d> {
    /// The fence's info string.
    pub lang: &'d DirectiveAttribute,
    /// The lines to keep (all if `None`), and the attribute.
    pub lines: Option<(LineRange, &'d DirectiveAttribute)>,
}

impl<'d> CodeSpec<'d> {
    /// The code include of the code attributes `attrs` (`lang`, `lines`) of an include in
    /// `host`, the first of each name; `None` without a `lang`, or after reporting an
    /// error in `out`.
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
        let lang = attrs.iter().find(|attr| attr.name == "lang");
        let mut spec = lang.map(|&lang| Self { lang, lines: None });
        for &attr in attrs {
            let value = attr.value.as_str();
            match attr.name.as_str() {
                "lang" if value.trim().is_empty() || value.contains(['\n', '\r']) => {
                    out.push(error(
                        attr,
                        "`lang` must name a language, like `lang=rust`".into(),
                    ));
                    spec = None;
                }
                "lines" if lang.is_none() => out.push(error(
                    attr,
                    "`lines` only applies to code includes, which need a `lang`".into(),
                )),
                "lines" => match LineRange::parse(value) {
                    Ok(range) => {
                        if let Some(spec) = &mut spec {
                            spec.lines = Some((range, attr));
                        }
                    }
                    Err(message) => out.push(error(attr, message)),
                },
                _ => {}
            }
        }
        spec
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
    /// 2. `lines` keeps the lines in its range.
    /// 3. `dedent` (on by default) strips the indentation all non-blank lines share.
    /// 4. Leading and trailing blank lines are dropped.
    /// 5. The rest is fenced with `lang` as the info string, in backticks (tildes if the
    ///    info string has a backtick), one more than the longest run of them in the code
    ///    and at least three.
    pub(super) fn code(
        &mut self,
        host: &Document,
        spec: &Spec<'_>,
        code: &CodeSpec<'_>,
        text: &str,
    ) -> Option<Included> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut text = text.replace("\r\n", "\n");
        if let Some((range, attr)) = code.lines {
            text = match range.select(&text) {
                Ok(text) => text,
                Err(count) => {
                    let message = format!(
                        "`lines` starts at line {}, but `{}` has {count} line{}",
                        range.start,
                        spec.path.value,
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
            text: fence(code.lang.value.trim(), &trim_blank_lines(&text)),
            frontmatter: None,
        })
    }
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
