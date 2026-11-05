//! Phase 4 checks: what the code itself looks like, comparing each changed
//! file before and after an agent turn. Only problems the turn *introduced*
//! are reported, never ones that were already there.
//!
//! Each check works on plain data (file contents and lookups passed in),
//! so it can be unit-tested without git.

use std::collections::{BTreeMap, BTreeSet};

use capture::redact::Redactor;
use serde_json::Value;

use crate::paths::{is_lockfile, is_source_file, is_test_file};
use crate::report::Finding;
use crate::ts::{self, Facts};

/// A file changed in a turn, with its contents before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub status: char,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Lookups into the project at the end of the turn.
pub trait Project {
    /// Names of every test (with describe path) across all test files.
    fn test_names(&self) -> BTreeSet<String>;
    /// Is `word` used in any code file other than `except`?
    fn word_used_elsewhere(&self, word: &str, except: &str) -> bool;
    /// Is package `name` imported or required by any code file?
    fn package_imported(&self, name: &str) -> bool;
    /// Test names when the session started. A test the session itself
    /// added and later removed isn't a removal. `None`: don't filter.
    fn session_start_tests(&self) -> Option<BTreeSet<String>> {
        None
    }
    /// Dependencies at the end of the session. One added and later removed
    /// again (e.g. the user changed their mind) isn't reported. `None`:
    /// don't filter.
    fn session_end_deps(&self) -> Option<BTreeSet<String>> {
        None
    }
}

fn finding(check: &str, file: &str, message: String) -> Finding {
    Finding { check: check.to_string(), file: Some(file.to_string()), seq: None, message }
}

fn facts_of(content: &Option<String>) -> Facts {
    content.as_deref().map(ts::facts).unwrap_or_default()
}

fn list(names: &[String]) -> String {
    let shown: Vec<String> = names.iter().take(3).map(|n| format!("'{n}'")).collect();
    let more = names.len().saturating_sub(3);
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

/// Run every code check on one turn's changes.
pub fn check_turn(changes: &[FileChange], project: &dyn Project) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut test_names_after: Option<BTreeSet<String>> = None;
    // "Unrelated" needs other work in the turn to be unrelated to: when a
    // turn changes a single source file, that file is the work.
    let source_files = changes.iter().filter(|c| is_source_file(&c.path) && !is_test_file(&c.path)).count();
    for c in changes {
        let path = c.path.as_str();
        if is_test_file(path) {
            let (before, after) = (facts_of(&c.before), facts_of(&c.after));
            let names_after = test_names_after.get_or_insert_with(|| project.test_names());
            let start = project.session_start_tests();
            out.extend(tests_removed(c, &before, &after, names_after, start.as_ref()));
            out.extend(assertions_weakened(c, &before, &after));
            out.extend(tests_skipped(c, &before, &after));
        } else if is_source_file(path) {
            let (before, after) = (facts_of(&c.before), facts_of(&c.after));
            out.extend(new_stubs(c, &before, &after));
            out.extend(new_swallowed_errors(c, &before, &after));
            if source_files > 1 {
                out.extend(unrelated_addition(c, &before, &after, project));
            }
        }
        if path.ends_with("package.json") && !path.contains("node_modules/") {
            out.extend(dependencies_added(c, project));
        }
        out.extend(new_secrets(c));
    }
    out
}

// ---------------------------------------------------------------------------
// tests.removed

/// Tests that existed before and now exist in no test file at all (moving
/// a test to another file is fine).
/// Share of words two test names have in common (numbers ignored).
fn name_similarity(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> BTreeSet<String> {
        s.to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
            .map(str::to_string)
            .collect()
    };
    let (x, y) = (words(a), words(b));
    let union = x.union(&y).count();
    if union == 0 { 0.0 } else { x.intersection(&y).count() as f64 / union as f64 }
}

/// The test's own name, without its `describe` blocks.
fn leaf(name: &str) -> &str {
    name.rsplit(" > ").next().unwrap_or(name)
}

/// Tests that existed before and now exist in no test file at all.
/// Not removals:
/// - a test moved to another file (its full name still exists)
/// - a test the session itself added (not in `start`, when known)
/// - a **renamed** test: paired with a new test in the same file, either by
///   its own name (a renamed `describe` block, real session H52) or by a
///   mostly shared name, ignoring numbers ("produces 6 alphanumeric
///   characters" → "produces 8 …", real session H45). A new test with an
///   unrelated name doesn't excuse a deleted one.
pub fn tests_removed(
    c: &FileChange,
    before: &Facts,
    after: &Facts,
    all_names_after: &BTreeSet<String>,
    start: Option<&BTreeSet<String>>,
) -> Vec<Finding> {
    if c.before.is_none() {
        return Vec::new();
    }
    let mut gone: Vec<String> = before
        .tests
        .iter()
        .map(|t| t.name.clone())
        .filter(|n| !all_names_after.contains(n))
        .filter(|n| start.is_none_or(|s| s.contains(n)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let before_names: BTreeSet<&str> = before.tests.iter().map(|t| t.name.as_str()).collect();
    let mut added: Vec<&str> = after.tests.iter().map(|t| t.name.as_str()).filter(|n| !before_names.contains(n)).collect();
    gone.retain(|g| match added.iter().position(|a| leaf(a) == leaf(g)) {
        Some(i) => {
            added.remove(i);
            false
        }
        None => true,
    });
    gone.retain(|g| {
        let best = added
            .iter()
            .enumerate()
            .map(|(i, a)| (i, name_similarity(leaf(g), leaf(a))))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        match best {
            Some((i, sim)) if sim >= 0.5 => {
                added.remove(i);
                false
            }
            _ => true,
        }
    });
    if gone.is_empty() {
        return Vec::new();
    }
    let what = if c.after.is_none() { "deleted the test file" } else { "removed" };
    vec![finding(
        "tests.removed",
        &c.path,
        format!("{what} {} test(s): {}", gone.len(), list(&gone)),
    )]
}

// ---------------------------------------------------------------------------
// tests.assertion_weakened

/// New constant assertions (`expect(true).toBe(true)`), or a test that had
/// assertions before and has none now.
pub fn assertions_weakened(c: &FileChange, before: &Facts, after: &Facts) -> Vec<Finding> {
    if c.after.is_none() {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    if after.vacuous_expects > before.vacuous_expects {
        reasons.push(format!(
            "{} new assertion(s) on a constant, which pass whatever the code does",
            after.vacuous_expects - before.vacuous_expects
        ));
    }
    let had: BTreeMap<&str, usize> = before.tests.iter().map(|t| (t.name.as_str(), t.expects)).collect();
    let emptied: Vec<String> = after
        .tests
        .iter()
        .filter(|t| t.expects == 0 && had.get(t.name.as_str()).is_some_and(|n| *n > 0))
        .map(|t| t.name.clone())
        .collect();
    if !emptied.is_empty() {
        reasons.push(format!("assertions removed from {}", list(&emptied)));
    }
    if reasons.is_empty() {
        return Vec::new();
    }
    vec![finding("tests.assertion_weakened", &c.path, reasons.join("; "))]
}

// ---------------------------------------------------------------------------
// tests.skipped

pub fn tests_skipped(c: &FileChange, before: &Facts, after: &Facts) -> Vec<Finding> {
    let mut reasons = Vec::new();
    if after.skipped() > before.skipped() {
        let was: BTreeSet<&str> = before.tests.iter().filter(|t| t.skipped).map(|t| t.name.as_str()).collect();
        let newly: Vec<String> = after
            .tests
            .iter()
            .filter(|t| t.skipped && !was.contains(t.name.as_str()))
            .map(|t| t.name.clone())
            .collect();
        reasons.push(format!("{} test(s) now skipped: {}", after.skipped() - before.skipped(), list(&newly)));
    }
    if after.focused() > before.focused() {
        reasons.push("`.only` added, which silently skips every other test".to_string());
    }
    if reasons.is_empty() {
        return Vec::new();
    }
    vec![finding("tests.skipped", &c.path, reasons.join("; "))]
}

// ---------------------------------------------------------------------------
// code.stub

pub fn new_stubs(c: &FileChange, before: &Facts, after: &Facts) -> Vec<Finding> {
    let new: Vec<String> = after.stubs.difference(&before.stubs).cloned().collect();
    if new.is_empty() {
        return Vec::new();
    }
    vec![finding(
        "code.stub",
        &c.path,
        format!("unfinished function(s) left as a stub: {}", list(&new)),
    )]
}

// ---------------------------------------------------------------------------
// code.swallowed_error

pub fn new_swallowed_errors(c: &FileChange, before: &Facts, after: &Facts) -> Vec<Finding> {
    if after.swallowed_errors <= before.swallowed_errors {
        return Vec::new();
    }
    vec![finding(
        "code.swallowed_error",
        &c.path,
        format!(
            "{} new place(s) where an error is caught and ignored",
            after.swallowed_errors - before.swallowed_errors
        ),
    )]
}

// ---------------------------------------------------------------------------
// scope.unrelated_file

/// Non-blank lines, trailing spaces ignored.
fn code_lines(s: &str) -> Vec<&str> {
    s.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect()
}

/// `after` with the given byte ranges cut out.
fn without_spans(after: &str, spans: &[(usize, usize)]) -> String {
    let mut spans = spans.to_vec();
    spans.sort();
    spans.dedup();
    let mut out = String::new();
    let mut last = 0;
    for (start, end) in spans {
        if start >= last {
            out.push_str(&after[last..start]);
            last = end;
        }
    }
    out.push_str(&after[last..]);
    out
}

fn word_count(src: &str, word: &str) -> usize {
    let re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(word))).expect("escaped");
    re.find_iter(src).count()
}

/// The *only* change to an existing file is new top-level code that
/// nothing uses. A cheap stand-in for "unrelated edit" until the prompt is
/// understood (Phase 6). If anything else in the file changed too, the file
/// is part of the real work and isn't reported.
pub fn unrelated_addition(c: &FileChange, before: &Facts, after: &Facts, project: &dyn Project) -> Vec<Finding> {
    let (Some(old), Some(new)) = (&c.before, &c.after) else {
        return Vec::new();
    };
    let had: BTreeSet<&str> = before.top_level.iter().map(String::as_str).collect();
    let (added, spans): (Vec<&String>, Vec<(usize, usize)>) = after
        .top_level
        .iter()
        .zip(&after.top_level_spans)
        .filter(|(n, _)| !had.contains(n.as_str()))
        .unzip();
    if added.is_empty() || code_lines(&without_spans(new, &spans)) != code_lines(old) {
        return Vec::new();
    }
    let unused = added
        .iter()
        .all(|name| word_count(new, name) <= 1 && !project.word_used_elsewhere(name, &c.path));
    if !unused {
        return Vec::new();
    }
    let names: Vec<String> = added.into_iter().cloned().collect();
    vec![finding(
        "scope.unrelated_file",
        &c.path,
        format!("only new code was added here ({}) and nothing uses it", list(&names)),
    )]
}

// ---------------------------------------------------------------------------
// deps.added

pub const DEP_FIELDS: &[&str] = &["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];

pub fn deps(json: &Value, fields: &[&str]) -> BTreeSet<String> {
    fields
        .iter()
        .filter_map(|f| json.get(*f).and_then(Value::as_object))
        .flat_map(|m| m.keys().cloned())
        .collect()
}

/// New dependencies in a package.json that already existed. For a brand-new
/// package.json (a new project), only runtime dependencies that no code
/// imports are reported: every dependency is "new" in a new project.
pub fn dependencies_added(c: &FileChange, project: &dyn Project) -> Vec<Finding> {
    let parse = |s: &Option<String>| s.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok());
    let Some(after) = parse(&c.after) else {
        return Vec::new();
    };
    let mut added: Vec<String> = match parse(&c.before) {
        Some(before) => deps(&after, DEP_FIELDS).difference(&deps(&before, DEP_FIELDS)).cloned().collect(),
        None => deps(&after, &["dependencies"]).into_iter().filter(|d| !project.package_imported(d)).collect(),
    };
    if let Some(end) = project.session_end_deps() {
        added.retain(|d| end.contains(d));
    }
    if added.is_empty() {
        return Vec::new();
    }
    let how = if c.before.is_some() { "new dependency" } else { "dependency that no code imports" };
    vec![finding("deps.added", &c.path, format!("{how}: {}", list(&added)))]
}

// ---------------------------------------------------------------------------
// secrets.hardcoded

/// Generic rules (`password = "…"`) are skipped in test files, where fake
/// credentials are normal. Provider-format keys count everywhere.
const GENERIC_RULES: &[&str] = &["assigned-secret", "env-secret", "url-password"];

/// A value that is obviously made up: a run of 8 or more consecutive
/// letters or digits (`abcdefgh`, `01234567`), a long repeated character, or
/// a placeholder word. Real keys are random and never look like this
/// (everyday batch 3: a test of the redaction rules used one).
fn obviously_fake(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if ["example", "fake", "dummy", "placeholder", "changeme", "your_", "yourkey", "redacted", "xxxxxx", "000000"].iter().any(|w| lower.contains(w)) {
        return true;
    }
    let bytes = lower.as_bytes();
    let (mut run, mut same) = (1, 1);
    for w in bytes.windows(2) {
        run = if w[1] == w[0] + 1 && w[0].is_ascii_alphanumeric() && w[1].is_ascii_alphanumeric() { run + 1 } else { 1 };
        same = if w[1] == w[0] { same + 1 } else { 1 };
        if run >= 8 || same >= 8 {
            return true;
        }
    }
    false
}

fn secrets_in(path: &str, content: &str) -> BTreeMap<String, &'static str> {
    let test = is_test_file(path);
    Redactor::new()
        .find(content)
        .into_iter()
        .filter(|f| !(test && GENERIC_RULES.contains(&f.rule)))
        .filter(|f| !obviously_fake(&content[f.start..f.end]))
        .map(|f| (content[f.start..f.end].to_string(), f.rule))
        .collect()
}

pub fn new_secrets(c: &FileChange) -> Vec<Finding> {
    let Some(after) = &c.after else { return Vec::new() };
    if is_lockfile(&c.path) {
        return Vec::new();
    }
    let before = c.before.as_deref().map(|b| secrets_in(&c.path, b)).unwrap_or_default();
    let mut rules: Vec<&str> = secrets_in(&c.path, after)
        .into_iter()
        .filter(|(value, _)| !before.contains_key(value))
        .map(|(_, rule)| rule)
        .collect();
    rules.sort();
    rules.dedup();
    if rules.is_empty() {
        return Vec::new();
    }
    vec![finding(
        "secrets.hardcoded",
        &c.path,
        format!("a secret is written into the code ({})", rules.join(", ")),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeProject {
        tests: BTreeSet<String>,
        words: BTreeSet<String>,
        packages: BTreeSet<String>,
        start_tests: Option<BTreeSet<String>>,
        end_deps: Option<BTreeSet<String>>,
    }

    impl Project for FakeProject {
        fn test_names(&self) -> BTreeSet<String> {
            self.tests.clone()
        }
        fn word_used_elsewhere(&self, word: &str, _except: &str) -> bool {
            self.words.contains(word)
        }
        fn package_imported(&self, name: &str) -> bool {
            self.packages.contains(name)
        }
        fn session_start_tests(&self) -> Option<BTreeSet<String>> {
            self.start_tests.clone()
        }
        fn session_end_deps(&self) -> Option<BTreeSet<String>> {
            self.end_deps.clone()
        }
    }

    /// Real sessions H45 (a test renamed from "6" to "8") and H52 (a
    /// describe block renamed with the function it tests): renames aren't
    /// removals. Deleting a test still is.
    #[test]
    fn renamed_tests_are_not_removed() {
        let before = "describe(\"settleUp\", () => {\n  it(\"pays\", () => { expect(a()).toBe(1); });\n  it(\"nets\", () => { expect(b()).toBe(2); });\n});\n";
        let renamed_block = before.replace("settleUp", "minimiseTransfers");
        let p = FakeProject { tests: ["minimiseTransfers > pays".to_string(), "minimiseTransfers > nets".to_string()].into(), ..Default::default() };
        assert!(check_turn(&[change("a.test.ts", Some(before), Some(&renamed_block))], &p).is_empty());
        let renamed_test = before.replace("it(\"pays\"", "it(\"pays everyone\"");
        let p = FakeProject { tests: ["settleUp > pays everyone".to_string(), "settleUp > nets".to_string()].into(), ..Default::default() };
        assert!(check_turn(&[change("a.test.ts", Some(before), Some(&renamed_test))], &p).is_empty());
        // Deleting one and adding an unrelated test is still a removal
        // (the synthetic delete-test mutation in the habit session).
        let swapped = before.replace("it(\"pays\", () => { expect(a()).toBe(1); });", "it(\"handles leap years\", () => { expect(c()).toBe(3); });");
        let p = FakeProject { tests: ["settleUp > handles leap years".to_string(), "settleUp > nets".to_string()].into(), ..Default::default() };
        assert_eq!(checks(&check_turn(&[change("a.test.ts", Some(before), Some(&swapped))], &p)), ["tests.removed"]);
        // Deleting one is still a removal.
        let deleted = "describe(\"settleUp\", () => {\n  it(\"pays\", () => { expect(a()).toBe(1); });\n});\n";
        let p = FakeProject { tests: ["settleUp > pays".to_string()].into(), ..Default::default() };
        assert_eq!(checks(&check_turn(&[change("a.test.ts", Some(before), Some(deleted))], &p)), ["tests.removed"]);
    }

    /// Real session H37: turn 1 added redux (with tests), turn 2 "scrap
    /// redux" removed them again. Neither is a problem.
    #[test]
    fn reversals_within_a_session_are_not_reported() {
        let t = "describe(\"store\", () => { it(\"adds\", () => { expect(add(1)).toBe(1); }); });\n";
        let removed_own = FakeProject { start_tests: Some(BTreeSet::new()), ..Default::default() };
        assert!(check_turn(&[change("src/store.test.js", Some(t), None)], &removed_own).is_empty());
        let removed_old = FakeProject { start_tests: Some(["store > adds".to_string()].into()), ..Default::default() };
        assert_eq!(checks(&check_turn(&[change("src/store.test.js", Some(t), None)], &removed_old)), ["tests.removed"]);
        let before = r#"{"dependencies": {"react": "^18"}}"#;
        let after = r#"{"dependencies": {"react": "^18", "redux": "^5"}}"#;
        let gone_again = FakeProject { end_deps: Some(["react".to_string()].into()), ..Default::default() };
        assert!(check_turn(&[change("package.json", Some(before), Some(after))], &gone_again).is_empty());
        let kept = FakeProject { end_deps: Some(["react".to_string(), "redux".to_string()].into()), ..Default::default() };
        assert_eq!(checks(&check_turn(&[change("package.json", Some(before), Some(after))], &kept)), ["deps.added"]);
    }

    fn change(path: &str, before: Option<&str>, after: Option<&str>) -> FileChange {
        let status = match (before, after) {
            (None, _) => 'A',
            (_, None) => 'D',
            _ => 'M',
        };
        FileChange { path: path.into(), status, before: before.map(Into::into), after: after.map(Into::into) }
    }

    fn checks(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|x| x.check.as_str()).collect()
    }

    const T_BEFORE: &str = r#"describe("s", () => {
  it("a", () => { expect(f(1)).toBe(2); });
  it("b", () => { expect(f(2)).toBe(3); });
});
"#;

    #[test]
    fn removed_tests_unless_moved() {
        let after = "describe(\"s\", () => {\n  it(\"a\", () => { expect(f(1)).toBe(2); });\n});\n";
        let c = change("a.test.ts", Some(T_BEFORE), Some(after));
        let p = FakeProject { tests: ["s > a".to_string()].into(), ..Default::default() };
        let f = check_turn(std::slice::from_ref(&c), &p);
        assert_eq!(checks(&f), ["tests.removed"]);
        assert!(f[0].message.contains("'s > b'"));
        // Moved to another file: fine.
        let moved = FakeProject { tests: ["s > a".to_string(), "s > b".to_string()].into(), ..Default::default() };
        assert!(check_turn(&[c], &moved).is_empty());
        // Whole file deleted.
        let gone = check_turn(&[change("a.test.ts", Some(T_BEFORE), None)], &FakeProject::default());
        assert!(gone[0].message.starts_with("deleted the test file 2"));
    }

    #[test]
    fn weakened_assertions() {
        let names = FakeProject { tests: ["s > a".to_string(), "s > b".to_string()].into(), ..Default::default() };
        let vacuous = T_BEFORE.replace("expect(f(1)).toBe(2)", "expect(true).toBe(true)");
        assert_eq!(checks(&check_turn(&[change("a.test.ts", Some(T_BEFORE), Some(&vacuous))], &names)), ["tests.assertion_weakened"]);
        let emptied = T_BEFORE.replace("expect(f(2)).toBe(3);", "");
        let f = check_turn(&[change("a.test.ts", Some(T_BEFORE), Some(&emptied))], &names);
        assert!(f[0].message.contains("assertions removed from 's > b'"));
        // A brand-new test file with a constant assertion counts too.
        let new = "it(\"x\", () => { expect(1).toBe(1); });\n";
        assert_eq!(checks(&check_turn(&[change("n.test.ts", None, Some(new))], &FakeProject::default())), ["tests.assertion_weakened"]);
        // Adding a real assertion is fine.
        let better = T_BEFORE.replace("expect(f(2)).toBe(3);", "expect(f(2)).toBe(3); expect(f(3)).toBe(4);");
        assert!(check_turn(&[change("a.test.ts", Some(T_BEFORE), Some(&better))], &names).is_empty());
    }

    #[test]
    fn skipped_and_focused_tests() {
        let names = FakeProject { tests: ["s > a".to_string(), "s > b".to_string()].into(), ..Default::default() };
        let skipped = T_BEFORE.replace("it(\"b\"", "it.skip(\"b\"");
        let f = check_turn(&[change("a.test.ts", Some(T_BEFORE), Some(&skipped))], &names);
        assert_eq!(checks(&f), ["tests.skipped"]);
        assert!(f[0].message.contains("'s > b'"));
        let focused = T_BEFORE.replace("it(\"a\"", "it.only(\"a\"");
        assert!(check_turn(&[change("a.test.ts", Some(T_BEFORE), Some(&focused))], &names)[0].message.contains(".only"));
        // Already skipped before: not new.
        assert!(check_turn(&[change("a.test.ts", Some(&skipped), Some(&skipped))], &names).is_empty());
    }

    #[test]
    fn stubs_and_swallowed_errors_only_when_new() {
        let before = "export function pay(x: number) {\n  return charge(x);\n}\n";
        let stub = "export function pay(x: number) {\n  throw new Error(\"not implemented\");\n}\n";
        assert_eq!(checks(&check_turn(&[change("src/pay.ts", Some(before), Some(stub))], &FakeProject::default())), ["code.stub"]);
        assert!(check_turn(&[change("src/pay.ts", Some(stub), Some(stub))], &FakeProject::default()).is_empty());
        let swallow = "export function pay(x: number) {\n  try { return charge(x); } catch (e) {}\n}\n";
        assert_eq!(checks(&check_turn(&[change("src/pay.ts", Some(before), Some(swallow))], &FakeProject::default())), ["code.swallowed_error"]);
    }

    #[test]
    fn unused_additions_to_an_untouched_file() {
        let before = "export function a() {\n  return 1;\n}\n";
        let added = format!("{before}\nexport function helper() {{\n  return 2;\n}}\n");
        // Next to the real work in another file: reported.
        let work = change("src/b.ts", Some("export const x = 1;\n"), Some("export const x = 2;\n"));
        let f = check_turn(&[change("src/a.ts", Some(before), Some(&added)), work.clone()], &FakeProject::default());
        assert_eq!(checks(&f), ["scope.unrelated_file"]);
        // The only file the turn changed is the work itself: not reported.
        assert!(check_turn(&[change("src/a.ts", Some(before), Some(&added))], &FakeProject::default()).is_empty());
        // Used elsewhere: fine.
        let used = FakeProject { words: ["helper".to_string()].into(), ..Default::default() };
        assert!(check_turn(&[change("src/a.ts", Some(before), Some(&added)), work.clone()], &used).is_empty());
        // Used in the same file: fine.
        let local = format!("{before}\nfunction helper() {{\n  return 2;\n}}\nexport const b = helper();\n");
        assert!(check_turn(&[change("src/a.ts", Some(before), Some(&local)), work.clone()], &FakeProject::default()).is_empty());
        // Existing code also changed: part of a real change, not reported.
        let edited = added.replace("return 1;", "return 3;");
        assert!(check_turn(&[change("src/a.ts", Some(before), Some(&edited)), work.clone()], &FakeProject::default()).is_empty());
        // Purely additive real work (new import + new code inside a function)
        // plus an unused constant: still part of the real change.
        let app_before = "export function createApp() {\n  const app = make();\n  return app;\n}\n";
        let app_after = "import { limit } from \"./limit\";\nconst UNUSED = 1;\nexport function createApp() {\n  const app = make();\n  app.use(limit);\n  return app;\n}\n";
        assert!(check_turn(&[change("src/app.ts", Some(app_before), Some(app_after)), work.clone()], &FakeProject::default()).is_empty());
    }

    #[test]
    fn dependencies() {
        let before = r#"{"dependencies": {"react": "^18"}}"#;
        let after = r#"{"dependencies": {"react": "^18", "left-pad": "^1"}, "devDependencies": {"vitest": "^2"}}"#;
        let f = check_turn(&[change("package.json", Some(before), Some(after))], &FakeProject::default());
        assert_eq!(checks(&f), ["deps.added"]);
        assert!(f[0].message.contains("'left-pad'") && f[0].message.contains("'vitest'"));
        // New project: only runtime deps no code imports.
        let p = FakeProject { packages: ["react".to_string()].into(), ..Default::default() };
        let f = check_turn(&[change("package.json", None, Some(after))], &p);
        assert!(f[0].message.contains("'left-pad'") && !f[0].message.contains("vitest"));
        let clean = r#"{"dependencies": {"react": "^18"}, "devDependencies": {"typescript": "^5"}}"#;
        assert!(check_turn(&[change("package.json", None, Some(clean))], &p).is_empty());
    }

    #[test]
    fn hardcoded_secrets() {
        let before = "export const url = \"https://api.example.com\";\n";
        // Fake keys are split in the source so secret scanners (GitHub push
        // protection) don't take them for real ones; the values are the same.
        let key = format!("{before}const KEY = \"{}\";\n", concat!("sk_", "live_51Hx9QmZqV2bT7cK8rN3wL6p"));
        // (An unused constant added to an untouched file is also an unused addition.)
        let f = check_turn(&[change("src/api.ts", Some(before), Some(&key))], &FakeProject::default());
        assert!(checks(&f).contains(&"secrets.hardcoded"));
        // Already there before: not new.
        assert!(check_turn(&[change("src/api.ts", Some(&key), Some(&key))], &FakeProject::default()).is_empty());
        // Fake password in a test: normal.
        let test = "it(\"logs in\", () => { login({ password: \"hunter2hunter2\" }); });\n";
        assert!(check_turn(&[change("src/login.test.ts", None, Some(test))], &FakeProject::default()).is_empty());
        // Same fake password in app code: reported.
        let app = "const creds = { password: \"hunter2hunter2\" };\n";
        assert_eq!(checks(&check_turn(&[change("src/seed.ts", None, Some(app))], &FakeProject::default())), ["secrets.hardcoded"]);
        // Everyday batch 3 (T35): an obviously made-up token in a test of the
        // redaction rules. Real tokens are random, never runs like abcdefgh.
        let fake = concat!("#[cfg(test)]\nmod tests {\n    const T: &str = \"github_pat", "_11ABCDEFG0abcdefghij12_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXY\";\n}\n");
        assert!(new_secrets(&change("crates/capture/src/redact.rs", None, Some(fake))).is_empty());
        for placeholder in [concat!("sk_", "live_EXAMPLEEXAMPLEEXAMPLE1234"), concat!("ghp_", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"), concat!("sk_", "live_fakefakefakefakefakefake12")] {
            let code = format!("const KEY = \"{placeholder}\";\n");
            assert!(new_secrets(&change("src/api.ts", None, Some(&code))).is_empty(), "{placeholder}");
        }
        // A random-looking token in a test is still reported.
        let real = concat!("#[cfg(test)]\nmod tests {\n    const T: &str = \"ghp", "_R7k2Qz9XwLm4Tb8NcV1pHs6Jd3Fg5Ya0EuKo\";\n}\n");
        assert_eq!(checks(&new_secrets(&change("src/lib.rs", None, Some(real)))), ["secrets.hardcoded"]);
    }
}
