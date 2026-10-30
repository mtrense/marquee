//! The rule registry.

use std::fmt;
use std::sync::{Arc, LazyLock};

use super::rule::{DynRule, Rule, RuleMeta};
use super::rules;

/// The rules mrq knows, keyed by id and sorted by it.
#[derive(Clone, Default)]
pub struct Registry {
    rules: Vec<Arc<dyn DynRule>>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.rules.iter().map(|rule| rule.meta().id))
            .finish()
    }
}

/// The built-in rules, shared by config deserialization and the schema.
static BUILTIN: LazyLock<Registry> = LazyLock::new(|| {
    let mut registry = Registry::new();
    rules::register_builtin(&mut registry);
    registry
});

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every built-in rule: markdownlint's `MD###` and mrq's `MRQ###`.
    pub fn builtin() -> Self {
        BUILTIN.clone()
    }

    /// The shared built-in registry.
    pub(crate) fn builtin_ref() -> &'static Self {
        &BUILTIN
    }

    /// Adds `rule`, replacing a rule with the same id.
    pub fn register<R: Rule>(&mut self, rule: R) {
        let id = R::META.id;
        let rule: Arc<dyn DynRule> = Arc::new(rule);
        match self.rules.binary_search_by(|probe| probe.meta().id.cmp(id)) {
            Ok(index) => self.rules[index] = rule,
            Err(index) => self.rules.insert(index, rule),
        }
    }

    /// The rule with id or alias `name`, ignoring ASCII case.
    pub fn get(&self, name: &str) -> Option<RuleMeta> {
        self.find(name).map(|rule| rule.meta())
    }

    /// Every rule's metadata, sorted by id.
    pub fn rules(&self) -> impl Iterator<Item = RuleMeta> + '_ {
        self.rules.iter().map(|rule| rule.meta())
    }

    /// Whether `name` is a tag of at least one rule, ignoring ASCII case.
    pub fn is_tag(&self, name: &str) -> bool {
        self.rules.iter().any(|rule| rule.meta().has_tag(name))
    }

    pub(crate) fn find(&self, name: &str) -> Option<&Arc<dyn DynRule>> {
        self.rules.iter().find(|rule| rule.meta().is_named(name))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Arc<dyn DynRule>> {
        self.rules.iter()
    }
}
