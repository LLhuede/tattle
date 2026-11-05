//! Static import scan: which project files do the tests import directly?
//!
//! Used as a cheap, deterministic approximation of "code the tests cover":
//! test files plus every project file a test file imports with a relative
//! path (`./x`, `../y`). Package imports (`react`) are ignored.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;

use crate::paths::is_test_file;

fn import_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?x)
              \bfrom\s*["'](\.{1,2}/[^"']+)["']                 # import x from "./a" / export * from "./a"
            | \bimport\s*["'](\.{1,2}/[^"']+)["']               # import "./a"
            | \bimport\s*\(\s*["'](\.{1,2}/[^"']+)["']\s*\)     # import("./a")
            | \brequire\s*\(\s*["'](\.{1,2}/[^"']+)["']\s*\)    # require("./a")
            | \bvi\.mock\s*\(\s*["'](\.{1,2}/[^"']+)["']        # vi.mock("./a")
            | \bjest\.mock\s*\(\s*["'](\.{1,2}/[^"']+)["']      # jest.mock("./a")
            "#,
        )
        .expect("regex")
    })
}

/// Relative import specifiers in a JS/TS file, in order of appearance.
pub fn relative_imports(src: &str) -> Vec<String> {
    import_re()
        .captures_iter(src)
        .filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str().to_string()))
        .collect()
}

/// Join `spec` onto the folder of `from_file`, resolving `.` and `..`.
/// `None` if it climbs out of the project.
fn join(from_file: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = from_file.split('/').collect();
    parts.pop(); // the file itself
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

const EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];

/// Resolve an import to a project file the way TS/bundlers do: exact path,
/// added extension, `.js` written for a `.ts` file, or a folder's `index`.
pub fn resolve(from_file: &str, spec: &str, files: &BTreeSet<String>) -> Option<String> {
    let base = join(from_file, spec)?;
    let mut candidates = vec![base.clone()];
    let stem = ["js", "jsx", "mjs", "cjs"]
        .iter()
        .find_map(|ext| base.strip_suffix(&format!(".{ext}")))
        .unwrap_or(&base)
        .to_string();
    for ext in EXTENSIONS {
        candidates.push(format!("{stem}.{ext}"));
    }
    for ext in EXTENSIONS {
        candidates.push(format!("{base}/index.{ext}"));
    }
    candidates.into_iter().find(|c| files.contains(c))
}

/// Test files plus the project files they import directly.
pub fn covered_files(files: &BTreeSet<String>, read: impl Fn(&str) -> Option<String>) -> BTreeSet<String> {
    let mut covered = BTreeSet::new();
    for test in files.iter().filter(|f| is_test_file(f)) {
        covered.insert(test.clone());
        let Some(src) = read(test) else { continue };
        for spec in relative_imports(&src) {
            if let Some(target) = resolve(test, &spec, files) {
                covered.insert(target);
            }
        }
    }
    covered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_relative_imports_only() {
        let src = r#"
import { describe, it } from "vitest";
import { currentStreak } from "./streak";
import type { Habit } from '../types';
import "./setup";
const lazy = await import("./lazy");
const old = require("./legacy");
vi.mock("./api");
export * from "./reexport";
"#;
        assert_eq!(
            relative_imports(src),
            ["./streak", "../types", "./setup", "./lazy", "./legacy", "./api", "./reexport"]
        );
    }

    #[test]
    fn resolves_like_typescript() {
        let files = set(&["src/streak.ts", "src/ui/index.tsx", "src/types.ts", "lib/util.js"]);
        assert_eq!(resolve("src/streak.test.ts", "./streak", &files).as_deref(), Some("src/streak.ts"));
        assert_eq!(resolve("src/streak.test.ts", "./streak.js", &files).as_deref(), Some("src/streak.ts"));
        assert_eq!(resolve("src/streak.test.ts", "./ui", &files).as_deref(), Some("src/ui/index.tsx"));
        assert_eq!(resolve("src/a/b.test.ts", "../types", &files).as_deref(), Some("src/types.ts"));
        assert_eq!(resolve("test/u.test.js", "../lib/util.js", &files).as_deref(), Some("lib/util.js"));
        assert_eq!(resolve("src/a.test.ts", "./missing", &files), None);
        assert_eq!(resolve("a.test.ts", "../../outside", &files), None);
    }

    #[test]
    fn covered_is_tests_plus_direct_imports() {
        let files = set(&["src/streak.ts", "src/streak.test.ts", "src/App.tsx", "src/Header.tsx"]);
        let covered = covered_files(&files, |p| {
            (p == "src/streak.test.ts").then(|| "import { currentStreak } from \"./streak\";".to_string())
        });
        assert_eq!(covered, set(&["src/streak.test.ts", "src/streak.ts"]));
    }
}
