//! `human`: miette graphical reports (feature `human`).

use std::fmt;
use std::io::{self, Write};

use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteError, MietteSpanContents,
    SourceCode, SourceSpan, SpanContents,
};

use super::location::{display_path, find_source};
use super::{Diagnostic, HumanStyle, Severity};
use crate::source::{ByteRange, SourceFile};

/// Source text with a display name, borrowed from a [`SourceFile`].
struct Named<'a> {
    name: String,
    text: &'a str,
}

impl SourceCode for Named<'_> {
    fn read_span<'a>(
        &'a self,
        span: &SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> Result<Box<dyn SpanContents<'a> + 'a>, MietteError> {
        let inner = self
            .text
            .read_span(span, context_lines_before, context_lines_after)?;
        Ok(Box::new(MietteSpanContents::new_named(
            self.name.clone(),
            inner.data(),
            *inner.span(),
            inner.line(),
            inner.column(),
            inner.line_count(),
        )))
    }
}

/// Adapter presenting a [`Diagnostic`] through miette's `Diagnostic` protocol.
struct Report<'a> {
    diag: &'a Diagnostic,
    source: Option<Named<'a>>,
}

impl Report<'_> {
    fn new<'a>(diag: &'a Diagnostic, sources: &'a [SourceFile]) -> Report<'a> {
        let source = find_source(sources, &diag.file).map(|file| Named {
            name: display_path(&diag.file),
            text: file.text(),
        });
        Report { diag, source }
    }
}

fn span(range: ByteRange) -> SourceSpan {
    (range.start, range.len()).into()
}

impl fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.source.is_none() {
            // Without a snippet the file name would not appear anywhere.
            write!(f, "{}: ", display_path(&self.diag.file))?;
        }
        f.write_str(&self.diag.message)
    }
}

impl fmt::Debug for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.diag, f)
    }
}

impl std::error::Error for Report<'_> {}

impl miette::Diagnostic for Report<'_> {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(&self.diag.code))
    }

    fn severity(&self) -> Option<miette::Severity> {
        Some(match self.diag.severity {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
            Severity::Info | Severity::Hint => miette::Severity::Advice,
        })
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        let fix = self
            .diag
            .fix
            .as_ref()
            .map(|fix| format!("fix: {}", fix.message));
        let help = match (self.diag.help.as_deref(), fix) {
            (None, None) => return None,
            (Some(help), None) => help.to_owned(),
            (None, Some(fix)) => fix,
            (Some(help), Some(fix)) => format!("{help}\n{fix}"),
        };
        Some(Box::new(help))
    }

    fn url<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.diag
            .url
            .as_ref()
            .map(|url| Box::new(url) as Box<dyn fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn SourceCode> {
        self.source.as_ref().map(|source| source as &dyn SourceCode)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        self.source.as_ref()?;
        let primary = LabeledSpan::new_primary_with_span(None, span(self.diag.range));
        let secondary = self.diag.labels.iter().map(|label| {
            LabeledSpan::new_with_span(Some(label.message.clone()), span(label.range))
        });
        Some(Box::new(std::iter::once(primary).chain(secondary)))
    }
}

fn theme(style: &HumanStyle) -> GraphicalTheme {
    match (style.color, style.unicode) {
        (true, true) => GraphicalTheme::unicode(),
        (true, false) => GraphicalTheme::ascii(),
        (false, true) => GraphicalTheme::unicode_nocolor(),
        (false, false) => GraphicalTheme::none(),
    }
}

/// Writes miette graphical reports with source snippets, separated by blank lines.
/// Colors and box drawing follow `style` only; nothing is read from the environment.
pub fn emit_human(
    diagnostics: &[Diagnostic],
    sources: &[SourceFile],
    style: &HumanStyle,
    out: &mut impl Write,
) -> io::Result<()> {
    let handler = GraphicalReportHandler::new_themed(theme(style))
        .with_width(style.width)
        .with_links(false);
    for (i, diag) in diagnostics.iter().enumerate() {
        let mut rendered = String::new();
        handler
            .render_report(&mut rendered, &Report::new(diag, sources))
            .map_err(io::Error::other)?;
        if i > 0 {
            writeln!(out)?;
        }
        out.write_all(rendered.as_bytes())?;
    }
    Ok(())
}
