//! Phase 6a: check what was asked against what the code shows.
//!
//! Each intent fact from the prompt (Phase 5) gets a verdict:
//! - **verified**: the code shows it was done / kept
//! - **violated**: the code contradicts it
//! - **unknown**: can't tell from the code
//!
//! Only **precise** kinds (measured on held-out prompts: see
//! docs/DECISIONS.md, "Phase 5 results") may be *violated*, and only those
//! become findings. Other kinds are information: verified or unknown,
//! never violated.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde_json::{Value, json};

use crate::code::{DEP_FIELDS, FileChange, deps};
use crate::events::Outcome;
use crate::intent::{IntentFact, Polarity, RepoIndex, words};
use crate::paths::{is_lockfile, is_source_file, is_test_file};
use crate::report::Finding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Violated,
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Verified => "verified",
            Verdict::Violated => "violated",
            Verdict::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FactResult {
    pub fact: IntentFact,
    pub verdict: Verdict,
    /// Precise kinds may be violated; the rest are information only.
    pub precise: bool,
    pub evidence: String,
    /// Files involved in a violation.
    pub files: Vec<String>,
}

impl FactResult {
    pub fn to_json(&self) -> Value {
        json!({
            "fact": self.fact.to_json(),
            "verdict": self.verdict.as_str(),
            "precise": self.precise,
            "evidence": self.evidence,
            "files": self.files,
        })
    }
}

/// What the session did, at session level (first human prompt → end).
pub struct SessionEvidence<'a> {
    /// Turns whose prompt only asks a question or for a check (see
    /// `asks::read_only`): a target they mention isn't expected to change.
    pub read_only_turns: BTreeSet<usize>,
    /// The human prompts, by turn.
    pub prompts: Vec<String>,
    /// Dependencies added between the start of each turn and the end of the
    /// session (index = turn). A "no new dependencies" rule stated in turn k
    /// applies from then on (big-test flagship A1: set in turn 5, `zod`
    /// added in turn 16, in a project whose package.json was created in
    /// turn 1).
    pub added_deps_by_turn: Vec<Vec<String>>,
    pub changes: &'a [FileChange],
    /// Project index before the session (for grounding) and after.
    pub index_before: &'a RepoIndex,
    pub index_after: &'a RepoIndex,
    /// Every code file at the end (path → contents).
    pub code_after: &'a BTreeMap<String, String>,
    /// Result of the last test run in the session, if any ran.
    pub last_test_run: Option<Option<Outcome>>,
}

/// Kinds that may report a violation (precise on held-out prompts).
pub fn is_precise(f: &IntentFact) -> bool {
    match f.kind {
        "constraint.deps_unchanged" => f.polarity == Polarity::Must,
        // "per ip" is often a library default with no literal in the code
        // (held-out R105: express-rate-limit keys on the IP by default).
        "param.literal" => f.rule != "per",
        "constraint.tests_pass" | "scope.only" | "target.unchanged" | "symbol.rename" | "param.charset" => true,
        "route.exists" | "target.modify" => f.rule == "explicit-route",
        _ => false,
    }
}

fn result(fact: &IntentFact, verdict: Verdict, evidence: impl Into<String>, files: Vec<String>) -> FactResult {
    let precise = is_precise(fact);
    // Imprecise kinds can never be violated.
    let verdict = if !precise && verdict == Verdict::Violated { Verdict::Unknown } else { verdict };
    FactResult { fact: fact.clone(), verdict, precise, evidence: evidence.into(), files }
}

fn changed_paths<'a>(ev: &'a SessionEvidence) -> Vec<&'a str> {
    ev.changes.iter().map(|c| c.path.as_str()).filter(|p| !is_lockfile(p)).collect()
}

/// Text of changed code files after the session.
fn changed_code<'a>(ev: &'a SessionEvidence) -> Vec<(&'a str, &'a str)> {
    ev.changes
        .iter()
        .filter(|c| is_source_file(&c.path) || is_test_file(&c.path))
        .filter_map(|c| c.after.as_deref().map(|a| (c.path.as_str(), a)))
        .collect()
}

fn added_deps(ev: &SessionEvidence) -> Vec<String> {
    added_dependencies(ev.changes)
}

/// Dependencies added to package.json files that existed before.
pub fn added_dependencies(changes: &[FileChange]) -> Vec<String> {
    let parse = |s: &Option<String>| s.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok());
    changes
        .iter()
        .filter(|c| c.path.ends_with("package.json") && !c.path.contains("node_modules/"))
        .filter_map(|c| Some((parse(&c.before)?, parse(&c.after)?)))
        .flat_map(|(b, a)| deps(&a, DEP_FIELDS).difference(&deps(&b, DEP_FIELDS)).cloned().collect::<Vec<_>>())
        .collect()
}

/// Is `path` covered by a scope target?
fn in_scope(path: &str, target: &crate::intent::Target) -> bool {
    // Prompt text is lowercased, so compare paths without case.
    let t = target.text.trim().trim_start_matches("./").to_ascii_lowercase();
    let lower = path.to_ascii_lowercase();
    if lower == t || lower.ends_with(&format!("/{t}")) {
        return true;
    }
    if target.grounded.iter().any(|g| g.strip_prefix("file:") == Some(path)) {
        return true;
    }
    let tw: BTreeSet<String> = words(&t).into_iter().collect();
    let style = ["style", "css", "theme", "theming"];
    if tw.iter().any(|w| style.contains(&w.as_str())) && regex_is(r"\.(?:css|scss|sass|less)$", path) {
        return true;
    }
    if tw.iter().any(|w| w == "test") && is_test_file(path) {
        return true;
    }
    // A file whose name carries the target's words ("header component" →
    // Header.jsx), including files the agent created.
    let generic = ["component", "file", "page", "logic", "module", "code", "the"];
    let specific: BTreeSet<String> = tw.into_iter().filter(|w| !generic.contains(&w.as_str())).collect();
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    let fw: BTreeSet<String> = words(stem).into_iter().collect();
    !specific.is_empty() && specific.is_subset(&fw)
}

/// Can we tell what a scope target covers?
fn scope_target_known(target: &crate::intent::Target, changed: &[&str]) -> bool {
    !target.grounded.is_empty()
        || target.text.contains('/')
        || target.text.contains('.')
        || words(&target.text).iter().any(|w| ["style", "css", "theme", "test"].contains(&w.as_str()))
        || changed.iter().any(|p| in_scope(p, target))
}

fn regex_is(pat: &str, s: &str) -> bool {
    Regex::new(pat).map(|r| r.is_match(s)).unwrap_or(false)
}

fn declaration<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let f = crate::ts::facts(src);
    f.top_level.iter().zip(&f.top_level_spans).find(|(n, _)| n.as_str() == name).map(|(_, (s, e))| &src[*s..*e])
}

fn identifier_re(name: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(name))).expect("escaped")
}

/// What the other facts in the prompt say, which changes how one fact is
/// judged.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// The prompt also asks for tests, so test files are implicitly in
    /// scope for "only change X" (real session H28).
    pub tests_requested: bool,
    /// Files the prompt says not to touch: a rename shouldn't reach into
    /// them (real session H44: "don't touch settle.ts").
    pub protected: BTreeSet<String>,
}

/// Evaluate one fact against the session's evidence.
pub fn evaluate_fact(f: &IntentFact, ev: &SessionEvidence, ctx: &PromptContext) -> FactResult {
    let tests_requested = ctx.tests_requested;
    let changed = changed_paths(ev);
    let code = changed_code(ev);
    match f.kind {
        "constraint.deps_unchanged" => {
            let added = match ev.added_deps_by_turn.get(f.source.turn) {
                Some(since) if f.source.turn > 0 => since.clone(),
                _ => added_deps(ev),
            };
            if added.is_empty() {
                result(f, Verdict::Verified, "no dependencies added", vec![])
            } else {
                result(f, Verdict::Violated, format!("added {}", added.join(", ")), vec!["package.json".into()])
            }
        }
        "constraint.tests_pass" => match ev.last_test_run {
            Some(Some(Outcome::Passed)) => result(f, Verdict::Verified, "the last test run passed", vec![]),
            Some(Some(Outcome::Failed)) => result(f, Verdict::Violated, "the last test run failed", vec![]),
            Some(None) => result(f, Verdict::Unknown, "the last test run's result isn't known", vec![]),
            None => result(f, Verdict::Unknown, "the tests weren't run", vec![]),
        },
        "scope.only" => {
            if !f.targets.iter().all(|t| scope_target_known(t, &changed)) {
                return result(f, Verdict::Unknown, "couldn't tell which files a target covers", vec![]);
            }
            let outside: Vec<String> = changed
                .iter()
                .filter(|p| !(tests_requested && is_test_file(p)))
                .filter(|p| !f.targets.iter().any(|t| in_scope(p, t)))
                .map(|p| p.to_string())
                .collect();
            if outside.is_empty() {
                result(f, Verdict::Verified, format!("{} changed file(s), all in scope", changed.len()), vec![])
            } else {
                result(f, Verdict::Violated, format!("changed outside the requested scope: {}", outside.join(", ")), outside)
            }
        }
        "target.unchanged" => {
            let mut violated = Vec::new();
            let mut kept = Vec::new();
            for t in &f.targets {
                for g in &t.grounded {
                    if let Some(path) = g.strip_prefix("file:") {
                        if changed.contains(&path) { violated.push(path.to_string()) } else { kept.push(g.clone()) }
                    } else if let Some(name) = g.strip_prefix("symbol:") {
                        for path in ev.index_before.files_of(g) {
                            let c = ev.changes.iter().find(|c| c.path == path);
                            let same = match c {
                                None => true,
                                Some(c) => {
                                    let before = c.before.as_deref().and_then(|b| declaration(b, name));
                                    let after = c.after.as_deref().and_then(|a| declaration(a, name));
                                    before == after
                                }
                            };
                            if same { kept.push(g.clone()) } else { violated.push(path) }
                        }
                    } else if g.starts_with("route:") {
                        // Routes: still there afterwards? (Handlers aren't compared.)
                        if ev.index_after.route_labels().contains(g) {
                            kept.push(g.clone())
                        } else {
                            violated.push(ev.index_before.files_of(g).first().cloned().unwrap_or_default())
                        }
                    }
                }
            }
            violated.sort();
            violated.dedup();
            if !violated.is_empty() {
                result(f, Verdict::Violated, format!("changed: {}", violated.join(", ")), violated)
            } else if !kept.is_empty() {
                result(f, Verdict::Verified, format!("unchanged: {}", kept.join(", ")), vec![])
            } else {
                result(f, Verdict::Unknown, "couldn't find what this refers to in the project", vec![])
            }
        }
        "symbol.rename" => {
            let v = f.value.clone().unwrap_or(Value::Null);
            let (old, new) = (v["old"].as_str().unwrap_or(""), v["new"].as_str().unwrap_or(""));
            if old.is_empty() || new.is_empty() {
                return result(f, Verdict::Unknown, "rename not understood", vec![]);
            }
            let new_re = identifier_re(new);
            // The old name still used *as a field*: `.amount`, `amount:`,
            // `"amount"`. Local variables (`const amount =`) are other
            // things with the same name (real session H44).
            let old_use = Regex::new(&format!(r#"(?i)(?:\.{o}\b|\b{o}\s*\??:|["'`]{o}["'`])"#, o = regex::escape(old))).expect("escaped");
            let has_new = ev.code_after.values().any(|src| new_re.is_match(src));
            // Only source files the session changed: tests may check the
            // old name is gone or rejected, and files the agent didn't
            // touch (or was told not to) aren't evidence of a half-done
            // rename.
            // "rename person to member everywhere": untouched files count
            // too, and so do compound names (`personIds`) once the agent has
            // renamed compounds itself (`PersonRow` -> `MemberRow`). Real
            // session R30 left 16 `personIds` and said "throughout".
            let everywhere = Regex::new(r"(?i)\b(?:everywhere|throughout|across\s+the\s+(?:code|codebase|project|app)|all\s+(?:files|occurrences|references))\b")
                .expect("regex")
                .is_match(ev.prompts.get(f.source.turn).map_or(&f.source.text, |p| p));
            let compound = |w: &str| {
                let cap = capitalise(w);
                Regex::new(&format!(r"\b(?:{w}[A-Z]\w*|\w*[a-z0-9]{cap}\w*|{cap}[A-Z]\w*)\b", w = regex::escape(w), cap = regex::escape(&cap))).expect("escaped")
            };
            let compounds_in_scope = everywhere && ev.code_after.values().any(|src| compound(new).is_match(src));
            let old_compound = compound(old);
            // "the amount field on expenses": only leftovers in code about
            // expenses count (real session H30: `amount` on settle-up
            // transfers is a different object).
            let object = Regex::new(r"(?i)\bfield\s+(?:on|of|in|for)\s+(?:the\s+)?([a-z]{3,})")
                .expect("regex")
                .captures(ev.prompts.get(f.source.turn).map_or(&f.source.text, |p| p))
                .map(|c| c[1].to_lowercase().trim_end_matches('s').to_string());
            // Names the prompt itself mentions ("amount_cents stays") are the
            // user's call, not leftovers.
            let prompt_text = ev.prompts.get(f.source.turn).map_or(f.source.text.as_str(), |p| p.as_str()).to_lowercase();
            let leftover_compound = |src: &str| old_compound.find_iter(src).any(|m| !prompt_text.contains(&m.as_str().to_lowercase()) && !prompt_text.contains(&snake(m.as_str())));
            let still_old: Vec<String> = ev
                .code_after
                .iter()
                .filter(|(p, _)| is_source_file(p) && (changed.contains(&p.as_str()) || everywhere) && !ctx.protected.contains(*p))
                .filter(|(_, src)| {
                    let hits: Vec<usize> = old_use.find_iter(src).map(|m| m.start()).chain(if compounds_in_scope { old_compound.find_iter(src).filter(|m| leftover_compound(m.as_str())).map(|m| m.start()).collect::<Vec<_>>() } else { vec![] }).collect();
                    hits.into_iter().any(|at| near_object(src, at, object.as_deref()))
                })
                .map(|(p, _)| p.clone())
                .collect();
            // "keep old files loading", "backward compatible", "keep X as an
            // alias": the old name stays on purpose, so leftovers can't be
            // judged (everyday batch 1, B05).
            // Only explicit compatibility wording: "keep the tests working"
            // isn't about the old name (code review).
            static COMPAT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
            let compat = COMPAT
                .get_or_init(|| {
                    Regex::new(r"(?i)\bbackwards?[\s-]compat|\bcompatib|\bold\b[^;\n]{0,40}\b(?:load|loads|loading|work|works|working|read|reading|support|supported|accept|accepted|valid)\b|\balias\b|\bdeprecat|\bmigrat|\blegacy\b")
                        .expect("regex")
                })
                .is_match(&prompt_text);
            if !has_new {
                result(f, Verdict::Violated, format!("'{new}' doesn't appear in the code"), vec![])
            } else if !still_old.is_empty() && compat {
                result(f, Verdict::Unknown, format!("'{old}' is still used in {}, and the request asked to keep the old name working", still_old.join(", ")), vec![])
            } else if !still_old.is_empty() {
                result(f, Verdict::Violated, format!("'{old}' is still used in {}", still_old.join(", ")), still_old)
            } else {
                result(f, Verdict::Verified, format!("'{old}' renamed to '{new}'"), vec![])
            }
        }
        "param.literal" | "param.charset" => {
            if code.is_empty() {
                return result(f, Verdict::Unknown, "no code was changed", vec![]);
            }
            let pattern = match (f.kind, f.value.as_ref()) {
                ("param.charset", _) => r"A-Za-z0-9|a-zA-Z0-9|0-9A-Za-z|0-9a-zA-Z|(?i)alphanumeric".to_string(),
                (_, Some(Value::Number(n))) => number_pattern(n.as_f64().unwrap_or(0.0), f.unit.as_deref()),
                (_, Some(Value::String(s))) if s.chars().all(|c| c.is_ascii_alphanumeric()) => format!(r"(?i)\b{}\b", regex::escape(s)),
                (_, Some(Value::String(s))) => format!(r#"["'`]{}"#, regex::escape(s)),
                _ => return result(f, Verdict::Unknown, "value not understood", vec![]),
            };
            let re = Regex::new(&pattern).expect("param regex");
            match code.iter().find(|(_, src)| re.is_match(src)) {
                Some((p, _)) => result(f, Verdict::Verified, format!("found in {p}"), vec![]),
                None => result(f, Verdict::Violated, "not found in the changed code", vec![]),
            }
        }
        "route.exists" => {
            let new_routes: Vec<String> = ev.index_after.route_labels().difference(&ev.index_before.route_labels()).cloned().collect();
            let target = f.targets.first();
            if f.rule == "explicit-route" {
                let grounded = target.map(|t| ev.index_after.ground(&t.text)).unwrap_or_else(|| ev.index_after.ground(""));
                let path = target.and_then(|t| t.text.split_whitespace().find(|w| w.starts_with('/'))).unwrap_or("");
                if grounded.grounded.is_empty() && !path.is_empty() && route_in_code(path, ev.code_after) {
                    result(f, Verdict::Verified, format!("{path} appears in the code"), vec![])
                } else if grounded.grounded.is_empty() {
                    result(f, Verdict::Violated, "the route isn't in the code", vec![])
                } else {
                    result(f, Verdict::Verified, grounded.grounded.join(", "), vec![])
                }
            } else {
                let want: BTreeSet<String> = target.map(|t| words(&t.text).into_iter().collect()).unwrap_or_default();
                let hit = new_routes.iter().find(|r| words(r).iter().any(|w| want.contains(w)));
                match hit {
                    Some(r) => result(f, Verdict::Verified, r.clone(), vec![]),
                    None => result(f, Verdict::Unknown, format!("{} new route(s); none clearly match", new_routes.len()), vec![]),
                }
            }
        }
        "target.modify" => {
            let mut files: Vec<String> = f
                .targets
                .iter()
                .flat_map(|t| t.grounded.iter())
                .flat_map(|g| ev.index_before.files_of(g))
                .collect();
            files.sort();
            files.dedup();
            if files.is_empty() {
                return result(f, Verdict::Unknown, "couldn't find what this refers to", vec![]);
            }
            let touched: Vec<&String> = files.iter().filter(|p| changed.contains(&p.as_str())).collect();
            if touched.is_empty() && ev.read_only_turns.contains(&f.source.turn) {
                result(f, Verdict::Unknown, "the request asked about this rather than for a change", vec![])
            } else if touched.is_empty() {
                result(f, Verdict::Violated, format!("{} wasn't changed", files.join(", ")), files.clone())
            } else {
                result(f, Verdict::Verified, format!("changed {}", touched.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")), vec![])
            }
        }
        "require.tests_added" => {
            let added: Vec<&str> = ev
                .changes
                .iter()
                .filter(|c| is_test_file(&c.path))
                .filter(|c| {
                    let before = c.before.as_deref().map(|b| crate::ts::facts(b).tests.len()).unwrap_or(0);
                    let after = c.after.as_deref().map(|a| crate::ts::facts(a).tests.len()).unwrap_or(0);
                    after > before
                })
                .map(|c| c.path.as_str())
                .collect();
            if added.is_empty() {
                result(f, Verdict::Unknown, "no new tests found", vec![])
            } else {
                result(f, Verdict::Verified, format!("new tests in {}", added.join(", ")), vec![])
            }
        }
        "stack" => {
            let v = f.value.as_ref().and_then(Value::as_str).unwrap_or("");
            let in_package = ev.code_after.iter().any(|(p, src)| p.ends_with("package.json") && src.contains(&format!("\"{v}")));
            let in_code = ev.code_after.values().any(|src| identifier_re(v).is_match(src));
            match (f.polarity, in_package || in_code) {
                (Polarity::MustNot, false) => result(f, Verdict::Verified, format!("{v} isn't used"), vec![]),
                (Polarity::MustNot, true) => result(f, Verdict::Unknown, format!("{v} appears in the project; check it isn't used"), vec![]),
                (_, true) => result(f, Verdict::Verified, format!("{v} is used"), vec![]),
                (_, false) => result(f, Verdict::Unknown, format!("couldn't find {v}"), vec![]),
            }
        }
        "storage" => {
            let browser = Regex::new(r"localStorage|sessionStorage|indexedDB").expect("regex");
            let uses = ev.code_after.values().any(|src| browser.is_match(src));
            match (f.polarity, f.value.as_ref().and_then(Value::as_str)) {
                (Polarity::MustNot, Some("browser")) if !uses => result(f, Verdict::Verified, "no browser storage", vec![]),
                (_, Some("browser")) if uses => result(f, Verdict::Verified, "uses browser storage", vec![]),
                _ => result(f, Verdict::Unknown, "couldn't confirm", vec![]),
            }
        }
        _ => {
            // feature, symbols, deps.framework, prefer-deps: information only.
            let names: Vec<String> = match &f.value {
                Some(Value::String(s)) => words(s),
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).flat_map(words).collect(),
                _ => vec![],
            };
            let found = !names.is_empty()
                && names.iter().all(|n| code.iter().any(|(_, src)| identifier_re(n).is_match(src)));
            if f.polarity == Polarity::Must && found {
                result(f, Verdict::Verified, "mentioned in the changed code (weak evidence)", vec![])
            } else {
                result(f, Verdict::Unknown, "couldn't confirm from the code", vec![])
            }
        }
    }
}

pub fn evaluate(facts: &[IntentFact], ev: &SessionEvidence) -> Vec<FactResult> {
    let mut ctx = PromptContext { tests_requested: facts.iter().any(|f| f.kind == "require.tests_added"), ..Default::default() };
    for f in facts.iter().filter(|f| f.kind == "target.unchanged") {
        for g in f.targets.iter().flat_map(|t| t.grounded.iter()) {
            match g.strip_prefix("file:") {
                Some(p) => {
                    ctx.protected.insert(p.to_string());
                }
                None if g.starts_with("symbol:") => ctx.protected.extend(ev.index_before.files_of(g)),
                None => {}
            }
        }
    }
    facts.iter().map(|f| evaluate_fact(f, ev, &ctx)).collect()
}

/// Is the code around `at` (3 lines either side) about `object`?
fn near_object(src: &str, at: usize, object: Option<&str>) -> bool {
    let Some(object) = object else { return true };
    let line = src[..at].matches('\n').count();
    let lines: Vec<&str> = src.lines().collect();
    let lo = line.saturating_sub(3);
    let hi = (line + 4).min(lines.len());
    lines[lo..hi].iter().any(|l| l.to_lowercase().contains(object))
}

/// `amountCents` -> `amount_cents`.
fn snake(w: &str) -> String {
    let mut out = String::new();
    for c in w.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn capitalise(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// A number, or the same duration in other units ("30 seconds" is also
/// 30000 ms; real big-test session B4), with optional `_` separators.
fn number_pattern(n: f64, unit: Option<&str>) -> String {
    let u = unit.unwrap_or("").to_ascii_lowercase();
    let factors: &[f64] = if u.starts_with("sec") {
        &[1.0, 1000.0]
    } else if u.starts_with("min") {
        &[1.0, 60.0, 60_000.0]
    } else if u.starts_with("hour") || u == "h" || u == "hr" {
        &[1.0, 60.0, 3600.0, 3_600_000.0]
    } else if u.starts_with("day") {
        &[1.0, 24.0, 86_400.0, 86_400_000.0]
    } else {
        &[1.0]
    };
    let alts: Vec<String> = factors
        .iter()
        .map(|k| n * k)
        .filter(|v| v.fract() == 0.0)
        .map(|v| {
            let digits = format!("{}", v as u64);
            let grouped: Vec<String> = digits.as_bytes().rchunks(3).rev().map(|c| String::from_utf8_lossy(c).into_owned()).collect();
            format!("{digits}|{}", grouped.join("_"))
        })
        .collect();
    let mut pattern = format!(r"\b(?:{})\b", alts.join("|"));
    if factors.len() > 1 {
        pattern.push_str(&format!(r"|\b{}\s*\*\s*(?:1000|60)\b", n as u64));
    }
    pattern
}

/// A route is in the code: the path as a string literal, or (with
/// parameters) its fixed prefix as a literal or in a regex, as hand-written
/// routers do (`pathname === '/notes'`, `/^\/notes\/[^/]+$/`; real big-test
/// session B1).
pub fn route_in_code(path: &str, code: &BTreeMap<String, String>) -> bool {
    let quoted = |p: &str| code.values().any(|src| ["\"", "'", "`"].iter().any(|q| src.contains(&format!("{q}{p}{q}")) || src.contains(&format!("{q}{p}/"))));
    if quoted(path) {
        return true;
    }
    let fixed: Vec<&str> = path.split('/').take_while(|seg| !seg.starts_with(':') && !seg.starts_with('{')).collect();
    let prefix = fixed.join("/");
    if prefix.len() < 2 || prefix == path {
        return false;
    }
    let escaped = prefix.replace('/', "\\/");
    quoted(&prefix) || code.values().any(|src| src.contains(&escaped))
}

/// Finding id for a violated precise fact.
fn finding_id(r: &FactResult) -> &'static str {
    match r.fact.kind {
        "constraint.deps_unchanged" => "intent.deps_unchanged",
        "constraint.tests_pass" => "intent.tests_pass",
        "scope.only" => "intent.scope_only",
        "target.unchanged" => "intent.target_unchanged",
        "symbol.rename" => "intent.rename_incomplete",
        "param.literal" | "param.charset" => "intent.param_missing",
        "route.exists" => "intent.route_missing",
        _ => "intent.target_not_changed",
    }
}

/// One finding per violated precise fact (per file when files are known).
pub fn findings(results: &[FactResult]) -> Vec<Finding> {
    findings_by_turn(results).into_iter().map(|(_, f)| f).collect()
}

/// [`findings`], each with the index of the prompt it came from.
pub fn findings_by_turn(results: &[FactResult]) -> Vec<(usize, Finding)> {
    let mut out = Vec::new();
    for r in results.iter().filter(|r| r.precise && r.verdict == Verdict::Violated) {
        let asked = &r.fact.source.text;
        let turn = r.fact.source.turn;
        let message = format!("you asked: \"{asked}\"; {}", r.evidence);
        if r.files.is_empty() {
            out.push((turn, Finding { check: finding_id(r).into(), file: None, seq: None, message }));
        } else {
            for f in &r.files {
                out.push((turn, Finding { check: finding_id(r).into(), file: Some(f.clone()), seq: None, message: message.clone() }));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_no_deps_rule_counts_from_its_turn() {
        let idx = RepoIndex::build(&[]);
        let prompts = ["build the api from the spec", "next add balances. no new dependencies from now on"];
        let facts = crate::intent::extract(&prompts, &idx).facts;
        let f = facts.iter().find(|f| f.kind == "constraint.deps_unchanged").expect("fact");
        assert_eq!(f.source.turn, 1);
        let code = BTreeMap::new();
        let ev = SessionEvidence {
            read_only_turns: BTreeSet::new(),
            prompts: prompts.iter().map(|p| p.to_string()).collect(),
            added_deps_by_turn: vec![vec!["express".into(), "zod".into()], vec!["zod".into()]],
            changes: &[],
            index_before: &idx,
            index_after: &idx,
            code_after: &code,
            last_test_run: None,
        };
        let r = evaluate_fact(f, &ev, &PromptContext::default());
        assert_eq!(r.verdict, Verdict::Violated, "{}", r.evidence);
        assert!(r.evidence.contains("zod") && !r.evidence.contains("express"), "{}", r.evidence);
    }
    use crate::intent::extract;

    fn change(path: &str, before: Option<&str>, after: Option<&str>) -> FileChange {
        let status = match (before, after) {
            (None, _) => 'A',
            (_, None) => 'D',
            _ => 'M',
        };
        FileChange { path: path.into(), status, before: before.map(Into::into), after: after.map(Into::into) }
    }

    struct World {
        before: Vec<(String, String)>,
        changes: Vec<FileChange>,
        last_run: Option<Option<Outcome>>,
    }

    impl World {
        fn verdicts(&self, prompt: &str) -> Vec<(String, &'static str)> {
            let index_before = RepoIndex::build(&self.before);
            let mut after: BTreeMap<String, String> = self.before.iter().cloned().collect();
            for c in &self.changes {
                match &c.after {
                    Some(a) => {
                        after.insert(c.path.clone(), a.clone());
                    }
                    None => {
                        after.remove(&c.path);
                    }
                }
            }
            let after_files: Vec<(String, String)> = after.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let index_after = RepoIndex::build(&after_files);
            let ev = SessionEvidence { read_only_turns: BTreeSet::new(), prompts: vec![prompt.to_string()], added_deps_by_turn: vec![], changes: &self.changes, index_before: &index_before, index_after: &index_after, code_after: &after, last_test_run: self.last_run };
            let facts = extract(&[prompt], &index_before).facts;
            evaluate(&facts, &ev).into_iter().map(|r| (r.fact.kind.to_string(), r.verdict.as_str())).collect()
        }
    }

    const PKG: &str = r#"{"dependencies": {"react": "^18"}}"#;
    const PKG_PLUS: &str = r#"{"dependencies": {"react": "^18", "left-pad": "^1"}}"#;
    const STREAK: &str = "export function currentStreak(d) {\n  return d.length;\n}\n";
    const STREAK2: &str = "export function currentStreak(d) {\n  return d.length + 1;\n}\n";
    const APP: &str = "import { currentStreak } from './streak';\nexport function App() { return null; }\n";
    const APP2: &str = "import { currentStreak } from './streak';\nexport function App() { return 1; }\n";

    fn base() -> Vec<(String, String)> {
        vec![("package.json".into(), PKG.into()), ("src/streak.js".into(), STREAK.into()), ("src/App.jsx".into(), APP.into())]
    }

    #[test]
    fn dependency_constraint() {
        let w = World { before: base(), changes: vec![change("package.json", Some(PKG), Some(PKG_PLUS))], last_run: None };
        assert_eq!(w.verdicts("dont add any new packages"), [("constraint.deps_unchanged".into(), "violated")]);
        // Soft ("if you can avoid it") is information, never violated.
        assert_eq!(w.verdicts("dont add any new packages if you can avoid it"), [("constraint.deps_unchanged".into(), "unknown")]);
        let ok = World { before: base(), changes: vec![change("src/App.jsx", Some(APP), Some(APP2))], last_run: None };
        assert_eq!(ok.verdicts("dont add any new packages"), [("constraint.deps_unchanged".into(), "verified")]);
    }

    #[test]
    fn tests_must_pass() {
        let mk = |r| World { before: base(), changes: vec![], last_run: r };
        assert_eq!(mk(Some(Some(Outcome::Passed))).verdicts("keep the tests passing")[0].1, "verified");
        assert_eq!(mk(Some(Some(Outcome::Failed))).verdicts("keep the tests passing")[0].1, "violated");
        assert_eq!(mk(None).verdicts("keep the tests passing")[0].1, "unknown");
    }

    #[test]
    fn scope_and_untouched() {
        let w = World {
            before: base(),
            changes: vec![
                change("src/App.jsx", Some(APP), Some(APP2)),
                change("src/App.css", None, Some("body{}")),
                change("src/Header.jsx", None, Some("export function Header(){}")),
            ],
            last_run: None,
        };
        // Styling → css; "header component" → the new Header.jsx; App.jsx is outside.
        assert_eq!(w.verdicts("only change the styling and the header component"), [("scope.only".into(), "violated")]);
        assert_eq!(w.verdicts("only change the styling and the header component and app.jsx"), [("scope.only".into(), "verified")]);
        // Asking for tests puts test files in scope (real session H28).
        let with_test = World {
            before: base(),
            changes: vec![change("src/App.jsx", Some(APP), Some(APP2)), change("src/streak.test.js", None, Some("it('x', () => {});"))],
            last_run: None,
        };
        assert_eq!(with_test.verdicts("only touch app.jsx. add a test that covers it")[0], ("scope.only".into(), "verified"));
        assert_eq!(with_test.verdicts("only touch app.jsx")[0], ("scope.only".into(), "violated"));
        assert_eq!(w.verdicts("dont touch the streak logic"), [("target.unchanged".into(), "verified")]);
        let touched = World { before: base(), changes: vec![change("src/streak.js", Some(STREAK), Some(STREAK2))], last_run: None };
        assert_eq!(touched.verdicts("dont touch the streak logic"), [("target.unchanged".into(), "violated")]);
        // Something that isn't in the project can't be judged.
        assert_eq!(touched.verdicts("dont touch the payment flow"), [("target.unchanged".into(), "unknown")]);
    }

    #[test]
    fn renames_routes_and_numbers() {
        let before = vec![("src/a.ts".to_string(), "export const x = { amount: 1 };\napp.get(\"/expenses\", h);\n".to_string())];
        let done = World { before: before.clone(), changes: vec![change("src/a.ts", Some(&before[0].1), Some("export const x = { total: 1 };\napp.get(\"/expenses\", h);\napp.get(\"/api/health\", h);\nconst MAX = 50;\n"))], last_run: None };
        assert_eq!(done.verdicts("rename the amount field to total"), [("symbol.rename".into(), "verified")]);
        assert_eq!(done.verdicts("add a GET /api/health endpoint"), [("route.exists".into(), "verified")]);
        assert_eq!(done.verdicts("cap names at 50 characters"), [("param.literal".into(), "verified")]);
        let half = World { before: before.clone(), changes: vec![change("src/a.ts", Some(&before[0].1), Some("export const x = { total: 1, amount: 1 };\napp.get(\"/expenses\", h);\n"))], last_run: None };
        // Real session H44: a local variable named `amount`, a test that the
        // old field is rejected, and a file the user protected are fine.
        let settle = "export function settle() {\n  const amount = 1;\n  return { amount: 1 };\n}\n".to_string();
        let h44_before = vec![("src/a.ts".to_string(), before[0].1.clone()), ("src/settle.ts".to_string(), settle.clone())];
        let h44 = World {
            before: h44_before,
            changes: vec![
                change("src/a.ts", Some(&before[0].1), Some("export const x = { total: 1 };\nfor (let amount = 0; amount < 3; amount++) {}\napp.get(\"/expenses\", h);\n")),
                change("tests/api.test.ts", None, Some("it('rejects the old field', () => { expect(body).not.toHaveProperty(\"amount\"); send({ amount: 1 }); });\n")),
            ],
            last_run: None,
        };
        assert_eq!(h44.verdicts("rename the amount field to total. dont touch settle.ts")[0], ("symbol.rename".into(), "verified"));
        assert_eq!(half.verdicts("rename the amount field to total"), [("symbol.rename".into(), "violated")]);
        // Everyday batch 1 (B05): the old name kept on purpose, as asked.
        for compat in [
            "rename the amount field to total, but keep old files loading correctly",
            "rename the amount field to total and stay backward compatible",
            "rename the amount field to total; old saved data must still load",
            "rename the amount field to total, keeping amount as a deprecated alias",
            // Everyday batch 1 (B05), word for word apart from the names.
            "Rename the amount field to total everywhere, but keep old expenses.json files loading correctly.",
        ] {
            assert_eq!(half.verdicts(compat)[0], ("symbol.rename".into(), "unknown"), "{compat}");
        }
        // Everyday wording that isn't about the old name (code review).
        assert_eq!(half.verdicts("rename the amount field to total and keep the tests working")[0], ("symbol.rename".into(), "violated"));
        assert_eq!(half.verdicts("add a GET /api/health endpoint"), [("route.exists".into(), "violated")]);
        assert_eq!(half.verdicts("cap names at 50 characters"), [("param.literal".into(), "violated")]);
        // Changing an existing route: was its file changed?
        assert_eq!(half.verdicts("add pagination to GET /expenses")[1], ("target.modify".into(), "verified"));
    }

    #[test]
    fn imprecise_kinds_are_never_violated() {
        let w = World { before: base(), changes: vec![change("src/App.jsx", Some(APP), Some(APP2))], last_run: None };
        for (_, v) in w.verdicts("add search and dark mode, recipes have ingredients and steps, add tests for search") {
            assert_ne!(v, "violated");
        }
    }
}
