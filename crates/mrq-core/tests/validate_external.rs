//! Acceptance tests for external link checks through the [`ExternalChecker`] seam, with a
//! fake checker: no network access.

use std::cell::RefCell;
use std::path::PathBuf;

use mrq_core::config::Config;
use mrq_core::diag::{Diagnostic, Severity};
use mrq_core::document::{Document, DocumentOptions, Workspace};
use mrq_core::source::SourceFile;
use mrq_core::validate::{
    Checker, ExternalChecker, ExternalExclude, ExternalOptions, ExternalStatus, codes,
};

/// Reports URLs containing `broken` as broken, everything else as fine, and records
/// every batch it is asked to check.
#[derive(Debug, Default)]
struct Fake {
    batches: RefCell<Vec<Vec<String>>>,
}

impl Fake {
    fn checked(&self) -> Vec<String> {
        self.batches.borrow().iter().flatten().cloned().collect()
    }
}

impl ExternalChecker for Fake {
    fn check(&self, urls: &[&str]) -> Vec<ExternalStatus> {
        self.batches
            .borrow_mut()
            .push(urls.iter().map(|url| (*url).to_owned()).collect());
        urls.iter()
            .map(|url| {
                if url.contains("broken") {
                    ExternalStatus::Broken("404 Not Found".into())
                } else {
                    ExternalStatus::Ok
                }
            })
            .collect()
    }
}

fn doc_at(path: &str, text: &str) -> Document {
    let source = SourceFile::new(Some(PathBuf::from(path)), text);
    DocumentOptions::new().parse_source(source).unwrap()
}

/// (code, severity, message, source of the span) of every diagnostic.
fn summary(
    doc: &Document,
    diagnostics: Vec<Diagnostic>,
) -> Vec<(String, Severity, String, String)> {
    diagnostics
        .into_iter()
        .map(|diag| {
            let source = doc.text()[diag.range.start..diag.range.end].to_owned();
            (diag.code, diag.severity, diag.message, source)
        })
        .collect()
}

#[test]
fn broken_external_links_are_reported_at_each_link() {
    let ws = Workspace::new("site");
    let doc = doc_at(
        "site/a.md",
        "[a](https://x.org/broken#top) [ok](https://x.org/) ![img](http://x.org/broken.png)\n\n\
         <https://x.org/broken#top>\n",
    );
    let fake = Fake::default();
    let exclude = ExternalExclude::default();
    let diagnostics = Checker::new(&ws)
        .with_external_links(&fake, &exclude)
        .check(&doc);
    let error = |message: &str, source: &str| {
        (
            codes::BROKEN_EXTERNAL_LINK.to_owned(),
            Severity::Error,
            message.to_owned(),
            source.to_owned(),
        )
    };
    assert_eq!(
        summary(&doc, diagnostics),
        [
            error(
                "external link `https://x.org/broken#top` is broken: 404 Not Found",
                "[a](https://x.org/broken#top)"
            ),
            error(
                "external link `http://x.org/broken.png` is broken: 404 Not Found",
                "![img](http://x.org/broken.png)"
            ),
            error(
                "external link `https://x.org/broken#top` is broken: 404 Not Found",
                "<https://x.org/broken#top>"
            ),
        ]
    );
    // Duplicates are checked once, in document order.
    assert_eq!(
        *fake.batches.borrow(),
        [vec![
            "https://x.org/broken#top".to_owned(),
            "https://x.org/".into(),
            "http://x.org/broken.png".into(),
        ]]
    );
}

#[test]
fn only_http_urls_are_checked_and_excludes_apply() {
    let ws = Workspace::new("site");
    let doc = doc_at(
        "site/a.md",
        "[f](ftp://x.org/broken) [t](tel:123) [m](mailto:broken@x.org) [p](//x.org/broken)\n\
         [l](https://localhost/broken) [u](HTTPS://X.ORG/broken) [file](b.md)\n",
    );
    let fake = Fake::default();
    let exclude = ExternalExclude::new(["^https://localhost"]).unwrap();
    let diagnostics = Checker::new(&ws)
        .with_external_links(&fake, &exclude)
        .check(&doc);
    assert_eq!(fake.checked(), ["HTTPS://X.ORG/broken"]);
    let codes: Vec<_> = diagnostics.iter().map(|diag| diag.code.as_str()).collect();
    assert_eq!(codes, [codes::BROKEN_EXTERNAL_LINK, codes::MISSING_FILE]);
}

#[test]
fn a_workspace_is_checked_in_one_batch_without_duplicates() {
    let mut ws = Workspace::new("site");
    ws.insert(doc_at(
        "site/a.md",
        "[x](https://x.org/broken) [y](https://y.org/)\n",
    ));
    ws.insert(doc_at(
        "site/b.md",
        "[x](https://x.org/broken) [z](https://z.org/)\n",
    ));
    let fake = Fake::default();
    let exclude = ExternalExclude::default();
    let mut checker = Checker::new(&ws).with_external_links(&fake, &exclude);
    let diagnostics = checker.check_workspace();
    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code == codes::BROKEN_EXTERNAL_LINK)
    );
    assert_eq!(
        *fake.batches.borrow(),
        [vec![
            "https://x.org/broken".to_owned(),
            "https://y.org/".into(),
            "https://z.org/".into(),
        ]]
    );
    // Results are remembered: checking again asks nothing.
    let doc = doc_at("site/c.md", "[x](https://x.org/broken)\n");
    assert_eq!(checker.check(&doc).len(), 1);
    assert_eq!(fake.batches.borrow().len(), 1);
}

#[test]
fn skipped_urls_are_not_reported() {
    #[derive(Debug)]
    struct Skipping;
    impl ExternalChecker for Skipping {
        fn check(&self, urls: &[&str]) -> Vec<ExternalStatus> {
            urls.iter().map(|_| ExternalStatus::Skipped).collect()
        }
    }
    let ws = Workspace::new("site");
    let doc = doc_at("site/a.md", "[x](https://x.org/broken)\n");
    let exclude = ExternalExclude::default();
    let diagnostics = Checker::new(&ws)
        .with_external_links(&Skipping, &exclude)
        .check(&doc);
    assert_eq!(diagnostics, Vec::<Diagnostic>::new());
}

#[test]
fn external_links_are_not_checked_by_default() {
    let ws = Workspace::new("site");
    let doc = doc_at("site/a.md", "[x](https://x.org/broken)\n");
    assert_eq!(Checker::new(&ws).check(&doc), Vec::<Diagnostic>::new());

    let options = ExternalOptions::from_config(&Config::default()).unwrap();
    assert!(!options.enabled);
    assert!(options.with_flag(true).enabled);
}

#[test]
fn external_options_come_from_the_config() {
    let mut config = Config::default();
    config.validate.external = true;
    config.validate.external_exclude = vec!["^https://localhost".into(), "example\\.org".into()];
    let options = ExternalOptions::from_config(&config).unwrap();
    assert!(options.enabled);
    // The flag only ever turns checking on.
    assert!(options.clone().with_flag(false).enabled);
    assert!(options.exclude.is_excluded("https://localhost:8080/x"));
    assert!(options.exclude.is_excluded("https://www.example.org/"));
    assert!(!options.exclude.is_excluded("https://x.org/"));

    config.validate.external_exclude = vec!["ok".into(), "(unclosed".into()];
    let error = ExternalOptions::from_config(&config).unwrap_err();
    assert_eq!(error.pattern, "(unclosed");
    assert!(error.to_string().contains("(unclosed"), "{error}");
}
