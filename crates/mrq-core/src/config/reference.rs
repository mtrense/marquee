//! The reference configuration written by `mrq config init`.

/// A commented `.mrq.yaml` listing every setting at its built-in default.
///
/// Settings are commented out with `# ` at the start of the line, so the file as written
/// changes nothing; explanations start with `##`. Uncommenting every setting line gives
/// exactly [`Config::default`](super::Config::default) (checked by a test), so the file
/// cannot drift from the model.
pub const REFERENCE_CONFIG: &str = include_str!("reference.yaml");

#[cfg(test)]
mod tests {
    use super::REFERENCE_CONFIG;
    use crate::config::Config;

    /// The reference with every setting line (`# ` at column 0) uncommented.
    fn uncommented() -> String {
        REFERENCE_CONFIG
            .split_inclusive('\n')
            .map(|line| line.strip_prefix("# ").unwrap_or(line))
            .collect()
    }

    #[test]
    fn as_written_it_sets_nothing() {
        let value: Option<serde_json::Value> = serde_saphyr::from_str(REFERENCE_CONFIG).unwrap();
        assert!(value.is_none(), "reference sets a value: {value:?}");
    }

    #[test]
    fn uncommented_it_is_valid_and_equals_the_defaults() {
        let text = uncommented();
        let config: Config = serde_saphyr::from_str(&text).unwrap();
        assert_eq!(config, Config::default());
    }

    /// Every key of the model (also those whose default is empty or null) is listed,
    /// and nothing else: the uncommented reference is the serialized defaults.
    #[test]
    fn uncommented_it_lists_every_key() {
        let listed: serde_json::Value = serde_saphyr::from_str(&uncommented()).unwrap();
        let defaults = serde_json::to_value(Config::default()).unwrap();
        assert_eq!(listed, defaults);
    }

    #[test]
    fn explanations_stay_comments_when_uncommented() {
        for line in uncommented().lines() {
            let trimmed = line.trim_start();
            assert!(
                !trimmed.starts_with("# ") || line.starts_with('#'),
                "indented explanation {line:?} should start with `##` at column 0"
            );
        }
    }
}
