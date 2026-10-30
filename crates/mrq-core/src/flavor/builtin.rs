//! The built-in flavors.

use super::{Flavor, GithubSlugger, Slugger};
use crate::parse::ParseOptions;

/// Strict `CommonMark`: no extensions.
///
/// `CommonMark` defines no heading anchors; this flavor uses GitHub slugs, the ids most
/// `CommonMark` renderers and editors produce.
#[derive(Debug, Clone, Copy, Default)]
pub struct Commonmark;

impl Flavor for Commonmark {
    fn name(&self) -> &'static str {
        "commonmark"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions::commonmark()
    }

    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(GithubSlugger::new())
    }
}

/// GitHub Flavored Markdown ([`ParseOptions::gfm`]) with GitHub slugs; the default.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gfm;

impl Flavor for Gfm {
    fn name(&self) -> &'static str {
        "gfm"
    }

    fn parse_options(&self) -> ParseOptions {
        ParseOptions::gfm()
    }

    fn slugger(&self) -> Box<dyn Slugger> {
        Box::new(GithubSlugger::new())
    }
}
