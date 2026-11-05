//! File classification shared by the checks and the harness.

use regex::Regex;
use std::sync::OnceLock;

fn test_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\.(test|spec)\.[cm]?[jt]sx?$)|(^|/)__tests__/").expect("regex"))
}

fn source_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.[cm]?[jt]sx?$").expect("regex"))
}

/// `x.test.ts`, `x.spec.tsx`, files under `__tests__/`, and so on.
pub fn is_test_file(path: &str) -> bool {
    test_re().is_match(path)
}

/// Non-test JS/TS source file.
pub fn is_source_file(path: &str) -> bool {
    source_re().is_match(path) && !is_test_file(path) && !path.ends_with(".d.ts")
}

/// Lockfiles are rewritten by package managers; changes to them are never
/// reported on their own.
pub fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package-lock.json" | "npm-shrinkwrap.json" | "yarn.lock" | "pnpm-lock.yaml" | "bun.lockb" | "bun.lock"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_files() {
        assert!(is_test_file("src/a.test.ts"));
        assert!(is_test_file("src/__tests__/a.ts"));
        assert!(is_test_file("b.spec.jsx"));
        assert!(!is_test_file("src/a.ts"));
        assert!(is_source_file("src/App.tsx"));
        assert!(!is_source_file("src/a.test.ts"));
        assert!(!is_source_file("src/types.d.ts"));
        assert!(is_lockfile("package-lock.json"));
        assert!(is_lockfile("web/pnpm-lock.yaml"));
        assert!(!is_lockfile("package.json"));
    }
}
