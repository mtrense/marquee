//! A [carapace-spec](https://carapace.sh) generator for the clap tree.
//!
//! Hand-written rather than `carapace_spec_clap`: that crate cannot filter files by
//! extension (Markdown inputs complete `*.md`/`*.markdown` here) and would add a second
//! YAML serializer (`yaml_serde`) next to serde-saphyr.

use std::collections::BTreeMap;
use std::io::Write;

use clap::{Arg, ArgAction, ValueHint};
use serde::Serialize;

/// Completion of positional paths: the Markdown inputs. carapace still offers
/// directories, so inputs can name one.
const MARKDOWN_FILES: &str = "$files([.md, .markdown])";

/// One command of the spec.
#[derive(Serialize)]
struct Spec {
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    flags: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    persistentflags: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Completion::is_empty")]
    completion: Completion,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    commands: Vec<Spec>,
}

/// Value completions of a command's flags and positional arguments.
#[derive(Default, Serialize)]
struct Completion {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    flag: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    positional: Vec<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    positionalany: Vec<String>,
}

impl Completion {
    fn is_empty(&self) -> bool {
        self.flag.is_empty() && self.positional.is_empty() && self.positionalany.is_empty()
    }
}

/// Writes the carapace spec of `cmd`, invoked as `bin_name`, to `buf`.
pub fn generate(cmd: &mut clap::Command, bin_name: &str, buf: &mut dyn Write) {
    cmd.set_bin_name(bin_name);
    cmd.build();
    let mut spec = spec_for(cmd, true);
    bin_name.clone_into(&mut spec.name);
    let yaml = serde_saphyr::to_string(&spec).expect("the spec serializes");
    write!(
        buf,
        "# yaml-language-server: $schema=https://carapace.sh/schemas/command.json\n{yaml}"
    )
    .expect("failed to write the completion spec");
}

fn spec_for(cmd: &clap::Command, root: bool) -> Spec {
    let mut flags = BTreeMap::new();
    let mut persistentflags = BTreeMap::new();
    let mut completion = Completion::default();
    for arg in cmd.get_arguments().filter(|a| !a.is_positional()) {
        // Global flags are declared once, as persistent flags of the root.
        let global = arg.is_global_set();
        if global && !root {
            continue;
        }
        let Some(key) = arg
            .get_long()
            .map_or_else(|| arg.get_short().map(String::from), |l| Some(l.to_owned()))
        else {
            continue;
        };
        let target = if global {
            &mut persistentflags
        } else {
            &mut flags
        };
        target.insert(signature(arg), help(arg));
        let values = values_for(arg, false);
        if !values.is_empty() {
            completion.flag.insert(key, values);
        }
    }
    let mut positionals: Vec<&Arg> = cmd.get_positionals().collect();
    positionals.sort_by_key(|a| a.get_index());
    for arg in positionals {
        let values = values_for(arg, true);
        if arg
            .get_num_args()
            .is_some_and(|n| n.max_values() == usize::MAX)
        {
            completion.positionalany = values;
        } else {
            completion.positional.push(values);
        }
    }
    // Trailing positionals without completions say nothing, unless they keep
    // `positionalany` from applying too early.
    while completion.positionalany.is_empty()
        && completion.positional.last().is_some_and(Vec::is_empty)
    {
        completion.positional.pop();
    }
    Spec {
        name: cmd.get_name().to_owned(),
        description: cmd.get_about().map(ToString::to_string).unwrap_or_default(),
        flags,
        persistentflags,
        completion,
        commands: cmd
            .get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| spec_for(c, false))
            .collect(),
    }
}

/// The flag key: `-s, --long` plus carapace's modifiers (`=` takes a value, `*` is
/// repeatable, `!` is required, `&` is hidden).
fn signature(arg: &Arg) -> String {
    let mut key = match (arg.get_short(), arg.get_long()) {
        (Some(s), Some(l)) => format!("-{s}, --{l}"),
        (Some(s), None) => format!("-{s}"),
        (None, Some(l)) => format!("--{l}"),
        (None, None) => unreachable!("flags have a short or long name"),
    };
    if arg.get_action().takes_values() {
        if arg.is_required_set() {
            key.push('!');
        }
        key.push('=');
    }
    if matches!(arg.get_action(), ArgAction::Append | ArgAction::Count) {
        key.push('*');
    }
    if arg.is_hide_set() {
        key.push('&');
    }
    key
}

fn help(arg: &Arg) -> String {
    arg.get_help().map(ToString::to_string).unwrap_or_default()
}

/// Static completions of `arg`'s values: path macros for path hints (Markdown files
/// for positionals), else the possible values with their help as descriptions.
fn values_for(arg: &Arg, positional: bool) -> Vec<String> {
    if !arg.get_action().takes_values() {
        return Vec::new();
    }
    match arg.get_value_hint() {
        ValueHint::AnyPath | ValueHint::FilePath if positional => {
            return vec![MARKDOWN_FILES.to_owned()];
        }
        ValueHint::AnyPath | ValueHint::FilePath => return vec!["$files".to_owned()],
        ValueHint::DirPath => return vec!["$directories".to_owned()],
        _ => {}
    }
    arg.get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| match v.get_help() {
            Some(help) => format!("{}\t{help}", v.get_name()),
            None => v.get_name().to_owned(),
        })
        .collect()
}
