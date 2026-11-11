//! Crate metadata that feeds the Homebrew formula's `desc` and `homepage` (M13.2).

#[test]
fn description_follows_brew_audit_rules() {
    let desc = env!("CARGO_PKG_DESCRIPTION");
    assert!(desc.chars().count() <= 80, "{desc}");
    let lower = desc.to_lowercase();
    assert!(
        !lower.starts_with("mrq"),
        "must not start with the name: {desc}"
    );
    assert!(
        !lower.starts_with("a ") && !lower.starts_with("an "),
        "no leading article: {desc}"
    );
    assert!(!desc.ends_with('.'), "{desc}");
}

#[test]
fn homepage_and_repository_point_at_the_github_project() {
    assert_eq!(
        env!("CARGO_PKG_REPOSITORY"),
        "https://github.com/mtrense/marquee"
    );
    assert_eq!(
        env!("CARGO_PKG_HOMEPAGE"),
        "https://github.com/mtrense/marquee"
    );
}
