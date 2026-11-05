//! Phase 6b: verify the agent's manifest against the code.
//!
//! The agent (asked through the recorder's hooks) lists each requirement
//! with the files, names, routes and tests behind it. Every listed item is
//! checked statically; the agent's words never count as evidence on their
//! own. A requirement whose items all check out is "backed by the code".

use std::collections::BTreeSet;

use regex::Regex;
use serde_json::{Value, json};

use crate::code::FileChange;
use crate::events::Outcome;
use crate::intent::{RepoIndex, words};
use crate::paths::is_test_file;
use crate::report::Finding;

/// One requirement from the manifest, after checking.
#[derive(Debug, Clone)]
pub struct Requirement {
    pub request: String,
    pub status: String,
    /// Listed items that exist in the code (`file:x`, `symbol:y`, …).
    pub supported: Vec<String>,
    /// Listed items the code doesn't bear out.
    pub unsupported: Vec<String>,
    /// Listed items that aren't names at all ("port constant", "30 passed"),
    /// so neither count (recall study, Haiku 4.5 manifests).
    pub uncheckable: Vec<String>,
}

impl Requirement {
    /// Claimed done or partial, listed some evidence, and all of it checks out.
    pub fn backed(&self) -> bool {
        matches!(self.status.as_str(), "done" | "partial") && !self.supported.is_empty() && self.unsupported.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({ "request": self.request, "status": self.status, "supported": self.supported, "unsupported": self.unsupported, "uncheckable": self.uncheckable, "backed": self.backed() })
    }
}

pub struct Evidence<'a> {
    pub changes: &'a [FileChange],
    pub index_after: &'a RepoIndex,
    pub code_after: &'a std::collections::BTreeMap<String, String>,
    pub added_deps: &'a [String],
    pub last_test_run: Option<Option<Outcome>>,
    /// The whole session changed no project files (a question, review or
    /// plan): a missing manifest doesn't matter, and listed files are ones
    /// the agent read (recall study R37, R40–R48).
    pub read_only_session: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Verification {
    pub present: bool,
    pub requirements: Vec<Requirement>,
    pub findings: Vec<Finding>,
}

/// Does a name the agent listed exist? Identifiers match as whole words;
/// `Class.method` matches its method name; anything else (CSS selectors
/// like `.theme-toggle`) matches as text. Found in a real session (H27, H35).
fn name_exists(name: &str, files: &std::collections::BTreeMap<String, String>) -> bool {
    let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    let word = |w: &str| {
        let re = Regex::new(&format!(r"(?:^|[^\w$]){}(?:$|[^\w$])", regex::escape(w))).expect("escaped");
        files.values().any(|src| re.is_match(src))
    };
    if ident(name) {
        word(name)
    } else if name.split('.').all(ident) {
        word(name.rsplit('.').next().unwrap_or(name))
    } else {
        files.values().any(|src| src.contains(name))
    }
}

fn keep_request_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?:confirm|check|verify|make sure|ensure|keep|leave)\b|\b(?:no|dont|don't|do not|without|never)\s+(?:new\s+|extra\s+|more\s+|any\s+)*(?:packages?|deps|dependencies|libraries|changes?|touch|change|edit)\b|\b(?:stays?|remain|unchanged|the same|still pass|tests? pass)\b").expect("regex"))
}

/// A route inside a free-text symbol item.
fn route_in(item: &str) -> Option<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"(?:^|[\s('"`])(/[\w/:.{}-]*\w)"#).expect("regex"));
    re.captures(item).map(|c| c[1].to_string())
}

/// The route path appears as a string literal in the code.
fn route_literal(path: &str, code: &std::collections::BTreeMap<String, String>) -> bool {
    crate::intent_checks::route_in_code(path, code)
}

/// An identifier, `Class.method`, or a CSS selector: something that can be
/// looked up. Free text ("port constant") can't.
fn checkable_name(name: &str) -> bool {
    !name.trim().is_empty() && !name.trim().contains(' ')
}

/// A tests item that describes a run or a file rather than naming a test.
fn descriptive_test_item(t: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)^\s*(?:all\b|\d)|\(\d+\s+tests?\)|\b\d+\s+(?:tests?\s+)?pass|\bpassed\b|\s-\s|\.(?:test|spec)\.[jt]sx?\b|,\s.*,\s|^\w[\w\s]{0,30}:\s").expect("regex"));
    re.is_match(t)
}

/// A test file named in a free-text tests item, as a project path.
fn test_file_in(t: &str) -> Option<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[\w./-]*\.(?:test|spec)\.[jt]sx?").expect("regex"));
    re.find(t).map(|m| m.as_str().trim_start_matches("./").to_string())
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn finding(check: &str, file: Option<String>, message: String) -> Finding {
    Finding { check: check.into(), file, seq: None, message }
}

/// Check the latest manifest. `requested` says whether the recorder asked
/// for one (a missing manifest is only reported then).
pub fn verify(manifest: Option<&Value>, requested: bool, ev: &Evidence) -> Verification {
    let Some(m) = manifest else {
        let findings = if requested && !ev.read_only_session {
            vec![finding("manifest.missing", None, "a manifest was requested but the agent didn't write one".into())]
        } else {
            vec![]
        };
        return Verification { present: false, requirements: vec![], findings };
    };
    let changed: BTreeSet<&str> = ev.changes.iter().map(|c| c.path.as_str()).collect();
    let test_names: Vec<String> = ev
        .code_after
        .iter()
        .filter(|(p, _)| is_test_file(p))
        .flat_map(|(_, src)| crate::ts::facts(src).tests.into_iter().map(|t| t.name))
        .collect();
    let mut out = Verification { present: true, ..Verification::default() };
    for r in m["requirements"].as_array().into_iter().flatten() {
        let mut req = Requirement {
            request: r["request"].as_str().unwrap_or("").to_string(),
            status: r["status"].as_str().unwrap_or("").to_ascii_lowercase(),
            supported: vec![],
            unsupported: vec![],
            uncheckable: vec![],
        };
        // "no extra packages", "confirm the tests pass", "keep /x the same":
        // the files listed are ones that should stay as they are.
        let keeps = keep_request_re().is_match(&req.request.to_lowercase());
        let claimed = matches!(req.status.as_str(), "done" | "partial");
        for f in strs(&r["files"]) {
            let p = f.trim_start_matches("<PROJECT>/").trim_start_matches("./").to_string();
            if crate::paths::is_lockfile(&p) {
                continue; // rewritten by package managers; not evidence either way
            }
            let read = (ev.read_only_session || keeps) && ev.code_after.contains_key(&p);
            if changed.contains(p.as_str()) || read { req.supported.push(format!("file:{p}")) } else { req.unsupported.push(format!("file:{p}")) }
        }
        for s in strs(&r["symbols"]) {
            // `splitEvenly(totalCents)` names splitEvenly (held-out session R76).
            let name = match s.find('(') {
                Some(i) if i > 0 && route_in(&s).is_none() && checkable_name(&s[..i]) => &s[..i],
                _ => s.trim_end_matches("()"),
            };
            if let Some(route) = route_in(name) {
                // `app.get('/people/:id')`, `GET /people/:id endpoint`
                if route_literal(&route, ev.code_after) { req.supported.push(format!("route:{route}")) } else { req.unsupported.push(format!("route:{route}")) }
            } else if !checkable_name(name) {
                req.uncheckable.push(format!("symbol:{name}"));
            } else if name_exists(name, ev.code_after) {
                req.supported.push(format!("symbol:{name}"))
            } else {
                req.unsupported.push(format!("symbol:{name}"))
            }
        }
        for route in strs(&r["routes"]) {
            // Grounded in the route index, or the path appears literally in
            // the code (frameworks the index doesn't parse).
            let path = route.split_whitespace().last().unwrap_or("").to_string();
            let literal = path.starts_with('/') && route_literal(&path, ev.code_after);
            if ev.index_after.ground(&route.to_ascii_lowercase()).grounded.is_empty() && !literal {
                req.unsupported.push(format!("route:{route}"))
            } else {
                req.supported.push(format!("route:{route}"))
            }
        }
        for t in strs(&r["tests"]) {
            if descriptive_test_item(&t) {
                // "30 passed (code.test.ts: 13, …)", "api.test.ts - API tests":
                // count the test file if one is named and exists.
                // A named test file that exists counts, as before (real session H59).
                let found = test_file_in(&t).and_then(|f| ev.code_after.keys().find(|p| is_test_file(p) && (**p == f || p.ends_with(&format!("/{f}")))).cloned());
                match found {
                    Some(f) => req.supported.push(format!("test:{f}")),
                    None => req.uncheckable.push(format!("test:{t}")),
                }
                continue;
            }
            // A test *file* the agent points to (real session H59) counts if
            // it exists.
            let as_path = t.trim_start_matches("<PROJECT>/").trim_start_matches("./");
            let is_file = ev.code_after.keys().any(|p| is_test_file(p) && (p == as_path || p.ends_with(&format!("/{as_path}"))));
            // Describe paths ("POST /api/shorten > rejects too long") and
            // it.each names ("rejects %s") match too (recall study R07, R20).
            // A hand-rolled runner's test function (`async function
            // testHelpOption()`, big-test build B7) names a test too.
            let test_function = checkable_name(&t) && name_exists(&t, ev.code_after);
            if is_file || test_function || test_names.iter().any(|n| crate::asks::test_name_matches(n, &t)) {
                req.supported.push(format!("test:{t}"))
            } else {
                req.unsupported.push(format!("test:{t}"))
            }
        }
        if claimed {
            for item in &req.unsupported {
                let file = item.strip_prefix("file:").map(str::to_string);
                out.findings.push(finding(
                    "manifest.claim_unsupported",
                    file,
                    format!("the manifest says \"{}\" was {} using {item}, but the code doesn't show it", req.request, req.status),
                ));
            }
        }
        out.requirements.push(req);
    }
    let declared: BTreeSet<String> = m["dependencies_added"].as_array().into_iter().flatten().filter_map(|d| d["name"].as_str().or(d.as_str())).map(str::to_string).collect();
    let undeclared: Vec<&String> = ev.added_deps.iter().filter(|d| !declared.contains(*d)).collect();
    if !undeclared.is_empty() {
        out.findings.push(finding(
            "manifest.undeclared_dependency",
            Some("package.json".into()),
            format!("added but not declared in the manifest: {}", undeclared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")),
        ));
    }
    // "ran": false with "passed": true (real session R35: tests were
    // "verified through logic").
    if m["tests"]["passed"] == true && m["tests"]["ran"] == false {
        out.findings.push(finding("manifest.tests_unrun_passed", None, "the manifest says the tests passed but also that they weren't run".into()));
    }
    if m["tests"]["passed"] == true && ev.last_test_run == Some(Some(Outcome::Failed)) {
        out.findings.push(finding("manifest.false_test_claim", None, "the manifest says the tests passed, but the last recorded run failed".into()));
    }
    out
}

/// Does a manifest requirement address a prompt fact? They share a
/// specific word (the fact's value or target, or the words it came from).
pub fn covers(req: &Requirement, fact: &crate::intent::IntentFact) -> bool {
    const GENERIC: &[&str] = &["the", "a", "an", "add", "to", "for", "of", "and", "in", "on", "it", "make", "use", "with", "dont", "don't", "not", "no", "new", "any"];
    let mut fw: BTreeSet<String> = words(&fact.source.text).into_iter().collect();
    if let Some(v) = fact.value.as_ref().and_then(Value::as_str) {
        fw.extend(words(v));
    }
    for t in &fact.targets {
        fw.extend(words(&t.text));
    }
    fw.retain(|w| !GENERIC.contains(&w.as_str()) && w.len() > 2);
    let rw: BTreeSet<String> = words(&req.request).into_iter().collect();
    !fw.is_disjoint(&rw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str) -> FileChange {
        FileChange { path: path.into(), status: 'M', before: Some(String::new()), after: Some(String::new()) }
    }

    #[test]
    fn claims_are_checked_against_the_code() {
        let changes = vec![change("src/App.jsx"), change("src/search.test.js")];
        let mut code = std::collections::BTreeMap::new();
        code.insert("src/App.jsx".to_string(), "export function SearchBox() {}\napp.get(\"/api/stats/:code\", h);\n".to_string());
        code.insert("src/search.test.js".to_string(), "it(\"filters by name\", () => {});\n".to_string());
        let pairs: Vec<(String, String)> = code.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let index = RepoIndex::build(&pairs);
        let added = vec!["fuse.js".to_string()];
        let ev = Evidence { changes: &changes, index_after: &index, code_after: &code, added_deps: &added, last_test_run: Some(Some(Outcome::Failed)), read_only_session: false };
        let m = json!({
            "schema": "cv-manifest/1",
            "requirements": [
                {"request": "search box", "status": "done", "files": ["src/App.jsx"], "symbols": ["SearchBox"], "routes": ["GET /api/stats/:code"], "tests": ["filters by name"]},
                {"request": "export", "status": "done", "files": ["src/export.js"], "symbols": ["exportAll"], "tests": ["exports json"]},
                {"request": "dark mode", "status": "not_done", "files": ["src/theme.css"]}
            ],
            "dependencies_added": [],
            "tests": {"ran": true, "passed": true}
        });
        let v = verify(Some(&m), true, &ev);
        assert!(v.requirements[0].backed());
        // Dotted methods and CSS selectors (real sessions H35, H27).
        let mut files = std::collections::BTreeMap::new();
        files.insert("a.ts".to_string(), "class UrlStore {\n  recordClick(code) {}\n}\n".to_string());
        files.insert("a.css".to_string(), ":root[data-theme=\"dark\"] { color: #fff; }\n.theme-toggle { }\n".to_string());
        assert!(name_exists("UrlStore.recordClick", &files));
        assert!(name_exists(".theme-toggle", &files));
        assert!(name_exists(":root[data-theme=\"dark\"]", &files));
        assert!(!name_exists("UrlStore.missing", &files));
        assert!(!name_exists(".missing-class", &files));
        assert!(!v.requirements[1].backed());
        let ids: Vec<(&str, Option<&str>)> = v.findings.iter().map(|f| (f.check.as_str(), f.file.as_deref())).collect();
        assert_eq!(
            ids,
            [
                ("manifest.claim_unsupported", Some("src/export.js")),
                ("manifest.claim_unsupported", None),
                ("manifest.claim_unsupported", None),
                ("manifest.undeclared_dependency", Some("package.json")),
                ("manifest.false_test_claim", None),
            ],
            "not_done requirements aren't claims"
        );
        // Test files listed instead of test names (real session H59).
        let paths = json!({"requirements": [{"request": "tests still pass", "status": "done", "tests": ["src/search.test.js"]}]});
        assert!(verify(Some(&paths), false, &ev).requirements[0].backed());
        let missing = json!({"requirements": [{"request": "x", "status": "done", "tests": ["src/nope.test.js"]}]});
        assert!(!verify(Some(&missing), false, &ev).requirements[0].backed());
        let none = verify(None, true, &ev);
        assert_eq!(none.findings[0].check, "manifest.missing");
        assert!(verify(None, false, &ev).findings.is_empty());
    }
}
