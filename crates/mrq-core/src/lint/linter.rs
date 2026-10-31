//! Rule selection from config, and running the selected rules.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::Registry;
use super::rule::{AnyOptions, DynRule, RuleMeta};
use super::suppress::{self, Action, LineStates};
use crate::Document;
use crate::config::RuleSettings;
use crate::config::{LintConfig, LintExtends, RuleSetting, RuleSettingError};
use crate::diag::Diagnostic;
use crate::source::PositionEncoding;

/// The rules of a registry with their parsed options, and which of them a
/// [`LintConfig`] enables.
pub struct Linter {
    registry: Registry,
    config: LintConfig,
    /// Every rule of the registry, sorted by id; disabled ones have default options, so
    /// an inline `enable` comment can switch them on.
    rules: Vec<(Arc<dyn DynRule>, AnyOptions)>,
    /// Indices into `rules` of the rules the config enables.
    enabled: BTreeSet<usize>,
    /// The names given to [`restrict_to`](Self::restrict_to), if any.
    only: Option<Vec<String>>,
}

impl std::fmt::Debug for Linter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.rules().map(|meta| meta.id))
            .finish()
    }
}

impl Linter {
    /// Selects rules from `registry` the way markdownlint does: `extends: default` starts
    /// with every rule that is on by default (`none` with no rule), then tag settings
    /// switch every rule with the tag, then rule settings (by id or alias) switch single
    /// rules. An options object enables its rule with those options; the others get
    /// their default options.
    ///
    /// Fails on a key the registry does not know or on options that do not parse.
    pub fn new(registry: &Registry, config: &LintConfig) -> Result<Self, RuleSettingError> {
        config.rules.validate(registry)?;
        let on = RuleSetting::Enabled(true);
        let mut selected: Vec<(Arc<dyn DynRule>, Option<&RuleSetting>)> = registry
            .iter()
            .map(|rule| {
                let enabled = config.extends == LintExtends::Default && rule.meta().default_enabled;
                (Arc::clone(rule), enabled.then_some(&on))
            })
            .collect();
        for (key, setting) in config.rules.iter() {
            for (rule, slot) in &mut selected {
                if rule.meta().has_tag(key) && !rule.meta().is_named(key) {
                    *slot = Some(setting);
                }
            }
        }
        for (key, setting) in config.rules.iter() {
            for (rule, slot) in &mut selected {
                if rule.meta().is_named(key) {
                    *slot = Some(setting);
                }
            }
        }

        let mut rules = Vec::new();
        let mut enabled = BTreeSet::new();
        for (index, (rule, setting)) in selected.into_iter().enumerate() {
            let options = match setting {
                Some(RuleSetting::Options(options)) => Some(options.clone()),
                Some(RuleSetting::Enabled(_)) | None => None,
            };
            if setting.is_some_and(RuleSetting::is_enabled) {
                enabled.insert(index);
            }
            let options = rule
                .parse_options(options)
                .map_err(|err| RuleSettingError {
                    key: rule.meta().id.to_owned(),
                    message: format!("invalid options for lint rule `{}`: {err}", rule.meta().id),
                })?;
            rules.push((rule, options));
        }
        Ok(Self {
            registry: registry.clone(),
            config: config.clone(),
            rules,
            enabled,
            only: None,
        })
    }

    /// Runs only the rules with one of `names` as id, alias or tag (ignoring ASCII
    /// case), as `mrq lint --rule` does: they are on even if the config turns them off,
    /// with their configured options, and no other rule runs. Inline comments can still
    /// turn them off, but not turn other rules on; `configure-file` comments still set
    /// their options.
    ///
    /// Fails on a name that is neither a rule nor a tag of the registry.
    pub fn restrict_to<I, S>(mut self, names: I) -> Result<Self, RuleSettingError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let names: Vec<String> = names.into_iter().map(|n| n.as_ref().to_owned()).collect();
        if let Some(unknown) = names
            .iter()
            .find(|name| self.registry.get(name).is_none() && !self.registry.is_tag(name))
        {
            return Err(RuleSettingError {
                key: unknown.clone(),
                message: format!("unknown lint rule or tag `{unknown}`"),
            });
        }
        self.enabled = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, (rule, _))| {
                let meta = rule.meta();
                names
                    .iter()
                    .any(|name| meta.is_named(name) || meta.has_tag(name))
            })
            .map(|(index, _)| index)
            .collect();
        self.only = Some(names);
        Ok(self)
    }

    /// The rules the config enables, sorted by id.
    pub fn rules(&self) -> impl Iterator<Item = RuleMeta> + '_ {
        self.enabled.iter().map(|&index| self.rules[index].0.meta())
    }

    /// Every finding of every enabled rule, sorted by position, then by rule id.
    ///
    /// Inline configuration comments in `doc` switch rules per line, see
    /// [the module docs](super#inline-configuration).
    pub fn check(&self, doc: &Document) -> Vec<Diagnostic> {
        let directives = suppress::directives(doc);
        let configured = configure_file(&self.config, &self.registry, &directives);
        let linter = match &configured {
            Some(config) => {
                let linter = Self::new(&self.registry, config).expect("validated settings");
                &match &self.only {
                    Some(names) => linter.restrict_to(names).expect("validated names"),
                    None => linter,
                }
            }
            None => self,
        };
        linter.check_lines(doc, &directives)
    }

    fn check_lines(
        &self,
        doc: &Document,
        directives: &[suppress::Directive<'_>],
    ) -> Vec<Diagnostic> {
        let metas: Vec<RuleMeta> = self.rules.iter().map(|(rule, _)| rule.meta()).collect();
        let source = doc.source();
        let states = LineStates::resolve(&metas, &self.enabled, directives, source.line_count());
        let states = &states;
        let line_of = &|offset| {
            source
                .offset_to_position(offset, PositionEncoding::Utf8)
                .map_or(0, |position| position.line)
        };
        let mut diagnostics: Vec<Diagnostic> = self
            .rules
            .iter()
            .enumerate()
            .filter(|&(index, _)| {
                states.any(index) && (self.only.is_none() || self.enabled.contains(&index))
            })
            .flat_map(|(index, (rule, options))| {
                rule.check(doc, &**options)
                    .into_iter()
                    .filter(move |diag| states.is_on(index, line_of(diag.range.start)))
            })
            .collect();
        diagnostics.sort_by(|a, b| {
            (a.range.start, a.range.end, &a.code).cmp(&(b.range.start, b.range.end, &b.code))
        });
        diagnostics
    }
}

/// `config` with every `configure-file` comment's settings merged in, in order, like
/// markdownlint's shallow merge: a key replaces the setting under the same name
/// (ignoring ASCII case), and `default` sets `extends`. Comments that are not a JSON
/// object, and entries the registry rejects, are ignored. `None` when there are no
/// such comments.
fn configure_file(
    config: &LintConfig,
    registry: &Registry,
    directives: &[suppress::Directive<'_>],
) -> Option<LintConfig> {
    let mut merged = config.clone();
    let mut any = false;
    for directive in directives {
        if directive.action != Action::ConfigureFile {
            continue;
        }
        let Ok(serde_json::Value::Object(object)) =
            serde_json::from_str::<serde_json::Value>(directive.parameter)
        else {
            continue;
        };
        for (key, value) in object {
            if key == "default" {
                if let serde_json::Value::Bool(default) = value {
                    merged.extends = if default {
                        LintExtends::Default
                    } else {
                        LintExtends::None
                    };
                    any = true;
                }
                continue;
            }
            let Ok(setting) = serde_json::from_value::<RuleSetting>(value) else {
                continue;
            };
            let mut single = RuleSettings::default();
            single.insert(key.clone(), setting.clone());
            if single.validate(registry).is_err() {
                continue;
            }
            let mut rules = RuleSettings::default();
            for (existing, previous) in merged.rules.iter() {
                if !existing.eq_ignore_ascii_case(&key) {
                    rules.insert(existing, previous.clone());
                }
            }
            rules.insert(key, setting);
            merged.rules = rules;
            any = true;
        }
    }
    any.then_some(merged)
}
