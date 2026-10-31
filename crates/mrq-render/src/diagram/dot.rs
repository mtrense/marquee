//! [`DotRenderer`]: Graphviz DOT through layout-rs, in pure Rust.

use std::panic::{self, AssertUnwindSafe};

use layout::backends::svg::SVGWriter;
use layout::gv::{DotParser, GraphBuilder};

use super::{DiagramError, DiagramFormat, DiagramOutput, DiagramRenderer};

/// The layout-rs version, for the fingerprint; keep it in step with Cargo.toml.
const LAYOUT_RS: &str = "0.1.3";

/// A [`DiagramRenderer`] for `dot` and `graphviz` to SVG with layout-rs. It knows the
/// common node shapes, records, styles and `rankdir=LR`, but not subgraph clusters or
/// HTML labels, so the README puts it after the real `dot`. A source it cannot parse
/// [fails](DiagramError::Failed); so does a panic inside layout-rs, which is caught (the
/// panic message still goes to stderr through the panic hook).
#[derive(Debug, Clone)]
pub struct DotRenderer {
    languages: Vec<String>,
}

impl DotRenderer {
    pub fn new() -> Self {
        Self {
            languages: vec!["dot".to_owned(), "graphviz".to_owned()],
        }
    }
}

impl Default for DotRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagramRenderer for DotRenderer {
    fn id(&self) -> &'static str {
        "builtin:dot"
    }

    fn languages(&self) -> &[String] {
        &self.languages
    }

    fn formats(&self, _lang: &str) -> &[DiagramFormat] {
        &[DiagramFormat::Svg]
    }

    fn fingerprint(&self) -> String {
        format!(
            "{} layout-rs {LAYOUT_RS} mrq {}",
            self.id(),
            env!("CARGO_PKG_VERSION")
        )
    }

    fn render(
        &self,
        _lang: &str,
        src: &str,
        format: DiagramFormat,
    ) -> Result<DiagramOutput, DiagramError> {
        if format != DiagramFormat::Svg {
            return Err(DiagramError::Unavailable(format!("renders no {format}")));
        }
        let svg = panic::catch_unwind(AssertUnwindSafe(|| layout_svg(src)))
            .map_err(|_| DiagramError::Failed("layout-rs panicked".to_owned()))?
            .map_err(DiagramError::Failed)?;
        DiagramOutput::from_bytes(DiagramFormat::Svg, sorted_styles(&svg).into_bytes())
            .map_err(DiagramError::Failed)
    }
}

/// `src` parsed, laid out and drawn as SVG, or the parse error.
fn layout_svg(src: &str) -> Result<String, String> {
    let graph = DotParser::new(src).process()?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    if visual.num_nodes() == 0 {
        return Err("the graph has no nodes".to_owned());
    }
    let mut writer = SVGWriter::new();
    visual.do_it(false, false, false, &mut writer);
    Ok(writer.finalize())
}

/// `svg` with the lines of its first `<style>` sorted: layout-rs writes one class per
/// font size from a hash map, so their order changes from run to run, and so would the
/// output and its cache entries.
fn sorted_styles(svg: &str) -> String {
    let (Some(start), Some(end)) = (svg.find("<style>\n"), svg.find("</style>")) else {
        return svg.to_owned();
    };
    let body_start = start + "<style>\n".len();
    if end < body_start {
        return svg.to_owned();
    }
    let mut lines: Vec<&str> = svg[body_start..end].lines().collect();
    lines.sort_unstable();
    let mut sorted = String::with_capacity(svg.len());
    sorted.push_str(&svg[..body_start]);
    for line in lines {
        sorted.push_str(line);
        sorted.push('\n');
    }
    sorted.push_str(&svg[end..]);
    sorted
}

#[cfg(test)]
mod tests {
    use super::sorted_styles;

    #[test]
    fn style_lines_are_sorted() {
        assert_eq!(
            sorted_styles("<svg><style>\n.b {}\n.a {}\n</style><g/></svg>"),
            "<svg><style>\n.a {}\n.b {}\n</style><g/></svg>"
        );
        assert_eq!(sorted_styles("<svg/>"), "<svg/>");
    }
}
