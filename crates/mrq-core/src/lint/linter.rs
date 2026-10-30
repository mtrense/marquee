//! Rule selection from config, and running the selected rules.

use std::sync::Arc;

use super::Registry;
use super::rule::{AnyOptions, DynRule, RuleMeta};
use crate::Document;
use crate::config::{LintConfig, LintExtends, RuleSetting, RuleSettingError};
use crate::diag::Diagnostic;

/// The enabled rules of a [`LintConfig`], each with its parsed options.
pub struct Linter {
    rules: Vec<(Arc<dyn DynRule>, AnyOptions)>,
}

impl std::fmt::Debug for Linter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.rules.iter().map(|(rule, _)| rule.meta().id))
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
        for (rule, setting) in selected {
            let options = match setting {
                Some(RuleSetting::Options(options)) => Some(options.clone()),
                Some(RuleSetting::Enabled(true)) => None,
                Some(RuleSetting::Enabled(false)) | None => continue,
            };
            let options = rule
                .parse_options(options)
                .map_err(|err| RuleSettingError {
                    key: rule.meta().id.to_owned(),
                    message: format!("invalid options for lint rule `{}`: {err}", rule.meta().id),
                })?;
            rules.push((rule, options));
        }
        Ok(Self { rules })
    }

    /// The enabled rules, sorted by id.
    pub fn rules(&self) -> impl Iterator<Item = RuleMeta> + '_ {
        self.rules.iter().map(|(rule, _)| rule.meta())
    }

    /// Every finding of every enabled rule, sorted by position, then by rule id.
    pub fn check(&self, doc: &Document) -> Vec<Diagnostic> {
        let mut diagnostics: Vec<Diagnostic> = self
            .rules
            .iter()
            .flat_map(|(rule, options)| rule.check(doc, &**options))
            .collect();
        diagnostics.sort_by(|a, b| {
            (a.range.start, a.range.end, &a.code).cmp(&(b.range.start, b.range.end, &b.code))
        });
        diagnostics
    }
}
