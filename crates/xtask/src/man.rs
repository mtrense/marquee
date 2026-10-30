//! Man page generation from the `mrq` clap command tree.

use std::io;
use std::path::{Path, PathBuf};

/// Writes `mrq.1` plus one `mrq-<subcommand>.1` per subcommand into `out`
/// (created if missing) and returns the written paths, sorted.
pub fn generate(out: &Path) -> io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out)?;
    let cmd = mrq_cli::cli::command().disable_help_subcommand(true);
    clap_mangen::generate_to(cmd, out)?;
    let mut pages = std::fs::read_dir(out)?
        .map(|entry| entry.map(|e| e.path()))
        .filter(|p| {
            p.as_ref().map_or(true, |p| {
                p.extension().is_some_and(|ext| ext == "1")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("mrq"))
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    pages.sort();
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn writes_main_and_subcommand_pages() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("nested/man");
        let pages = generate(&out).unwrap();

        let names: Vec<_> = pages
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        for expected in ["mrq.1", "mrq-toc.1", "mrq-config.1", "mrq-config-show.1"] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing {expected} in {names:?}"
            );
        }
        assert!(
            !names.iter().any(|n| n.starts_with("mrq-help")),
            "{names:?}"
        );

        let main = std::fs::read_to_string(out.join("mrq.1")).unwrap();
        assert!(main.starts_with(".ie"), "not roff: {main}");
        assert!(main.contains(".TH mrq 1"), "{main}");
        assert!(main.contains("interpolate"), "{main}");
        let toc = std::fs::read_to_string(out.join("mrq-toc.1")).unwrap();
        assert!(toc.contains("\\-\\-depth"), "{toc}");
    }
}
