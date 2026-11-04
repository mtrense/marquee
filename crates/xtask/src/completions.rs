//! Shell completion scripts for every shell `mrq completions` supports.

use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use mrq_cli::cli::CompletionShell;

/// The conventional file name for `shell`'s script: what the shell autoloads from its
/// completion directory, and what Homebrew's `*_completion.install` expects.
fn file_name(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => "mrq.bash",
        CompletionShell::Carapace => "mrq.yaml",
        CompletionShell::Elvish => "mrq.elv",
        CompletionShell::Fish => "mrq.fish",
        CompletionShell::Nushell => "mrq.nu",
        CompletionShell::Powershell => "_mrq.ps1",
        CompletionShell::Zsh => "_mrq",
    }
}

/// Writes one completion script per shell into `out` (created if missing) and returns
/// the written paths, sorted.
pub fn generate(out: &Path) -> io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out)?;
    let mut files = Vec::new();
    for &shell in CompletionShell::value_variants() {
        let mut buf = Vec::new();
        shell.generate(&mut mrq_cli::cli::command(), "mrq", &mut buf);
        let path = out.join(file_name(shell));
        std::fs::write(&path, buf)?;
        files.push(path);
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn writes_one_non_empty_script_per_shell() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("nested/completions");
        let files = generate(&out).unwrap();

        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            [
                "_mrq", "_mrq.ps1", "mrq.bash", "mrq.elv", "mrq.fish", "mrq.nu", "mrq.yaml"
            ]
        );
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 7);
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            assert!(!text.trim().is_empty(), "{} is empty", file.display());
        }
        let zsh = std::fs::read_to_string(out.join("_mrq")).unwrap();
        assert!(zsh.starts_with("#compdef mrq"), "{zsh}");
        let nu = std::fs::read_to_string(out.join("mrq.nu")).unwrap();
        assert!(nu.contains("export extern mrq"), "{nu}");
        let spec = std::fs::read_to_string(out.join("mrq.yaml")).unwrap();
        assert!(spec.contains("name: mrq"), "{spec}");
    }
}
