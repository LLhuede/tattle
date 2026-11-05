//! Phase 5 scoring: extract intent facts from every corpus prompt and
//! compare them with the hand-written expected facts.
//!
//! Measured:
//! - **recall**: expected facts the extractor found (implicit facts, which
//!   the prompt never states, are reported separately)
//! - **invented**: extracted facts that match no expected fact (target: 0)
//! - **couldn't check**: the corpus's "couldn't check" phrases must end up
//!   in the residual, not be turned into facts
//! - **coverage invariant**: every word of every prompt is accounted for

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use checks::intent::{self, Extraction, IntentFact, RepoIndex};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct PromptResult {
    /// (kind, matched?) for every non-implicit expected fact.
    pub expected_kinds: Vec<(String, bool)>,
    /// Kinds of invented facts.
    pub invented_kinds: Vec<String>,
    pub id: String,
    pub base: Option<String>,
    pub base_missing: bool,
    pub expected: usize,
    pub implicit: usize,
    pub matched: Vec<String>,
    pub missed: Vec<String>,
    pub invented: Vec<String>,
    pub couldnt_check_total: usize,
    pub couldnt_check_missed: Vec<String>,
    pub coverage_holds: bool,
    pub extraction: Extraction,
}

#[derive(Debug, Clone, Default)]
pub struct IntentScore {
    pub results: Vec<PromptResult>,
}

/// Words compared when matching a target hint (fewer generic words than
/// grounding: "code" matters here).
const HINT_STOP: &[&str] = &["the", "a", "an", "of", "to", "for", "in", "on", "and", "how", "work", "existing", "all"];

/// Words that name the same thing.
const SYNONYMS: &[&[&str]] = &[&["api", "route", "endpoint"], &["style", "css"], &["test", "spec"]];

fn hint_words(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for w in intent::words(s).into_iter().filter(|w| !HINT_STOP.contains(&w.as_str())) {
        for group in SYNONYMS {
            if group.contains(&w.as_str()) {
                out.extend(group.iter().map(|g| g.to_string()));
            }
        }
        out.insert(w);
    }
    out
}

/// A target hint matches if they share a specific word (after synonyms),
/// allowing word forms ("sorting" ~ "sort", "weekdays" ~ "weekday").
fn target_matches(hint: &str, text: &str) -> bool {
    let (h, t) = (hint_words(hint), hint_words(text));
    h.iter().any(|a| t.iter().any(|b| a == b || (a.len() >= 4 && b.len() >= 4 && (a.starts_with(b.as_str()) || b.starts_with(a.as_str())))))
}

fn lower(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.to_ascii_lowercase()),
        other => other.clone(),
    }
}

/// Does `got` satisfy expected fact `exp` (for single-fact kinds)?
fn fact_matches(exp: &Value, got: &IntentFact) -> bool {
    if exp["kind"].as_str() != Some(got.kind) || exp["polarity"].as_str() != Some(got.polarity.as_str()) {
        return false;
    }
    let params = &exp["params"];
    let target_ok = |hint: &Value| match hint.as_str() {
        None => true,
        Some(h) => got.targets.iter().any(|t| target_matches(h, &t.text)),
    };
    match got.kind {
        "feature" | "storage" | "param.charset" => got.value.as_ref().map(lower) == Some(lower(&params["value"])),
        "param.literal" => {
            got.value.as_ref().map(lower) == Some(lower(&params["value"]))
                && (params.get("unit").is_none() || got.unit.as_deref() == params["unit"].as_str())
        }
        "require.tests_added" | "route.exists" | "target.modify" | "target.unchanged" => target_ok(&params["target_hint"]),
        "scope.only" => params["targets"]
            .as_array()
            .is_none_or(|ts| ts.iter().all(|h| got.targets.iter().any(|t| target_matches(h.as_str().unwrap_or(""), &t.text)))),
        // Prompt text is lowercased, so compare names without case.
        "symbol.rename" => {
            let v = got.value.clone().unwrap_or(Value::Null);
            let eq = |a: &Value, b: &Value| a.as_str().map(str::to_ascii_lowercase) == b.as_str().map(str::to_ascii_lowercase);
            eq(&v["old"], &params["old"]) && eq(&v["new"], &params["new"])
        }
        _ => true,
    }
}

/// Kinds whose expected facts list several values, each found as its own
/// extracted fact (`stack`) or inside one (`symbols`).
fn multi_value(kind: &str) -> bool {
    matches!(kind, "stack" | "symbols")
}

/// Content words of a phrase (for "couldn't check" phrases).
fn content_words(s: &str) -> BTreeSet<String> {
    const FILL: &[&str] = &["a", "an", "the", "it", "and", "make", "to", "of"];
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty() && !FILL.contains(w))
        .map(str::to_string)
        .collect()
}

/// Read a project's files (skipping dependencies and big files), for the
/// repo index.
pub fn read_project(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !matches!(name.as_str(), "node_modules" | ".git" | ".checker" | "dist" | "build") {
                    stack.push(p);
                }
            } else if e.metadata().map(|m| m.len()).unwrap_or(0) < 1_000_000
                && let (Ok(text), Ok(rel)) = (fs::read_to_string(&p), p.strip_prefix(dir))
            {
                out.push((rel.to_string_lossy().replace('\\', "/"), text));
            }
        }
    }
    out.sort();
    out
}

/// Which base folder a prompt starts from: seeded bases use the prompt's
/// own id (e.g. `T1`), others the base they come from.
fn base_dir_name(prompt: &Value) -> Option<String> {
    let base = prompt.get("base")?;
    let from = base.get("from")?.as_str()?;
    if base.get("seed").is_some() {
        prompt["id"].as_str().map(str::to_string)
    } else {
        Some(from.to_string())
    }
}

pub fn score_prompt(prompt: &Value, bases: &Path) -> PromptResult {
    let id = prompt["id"].as_str().unwrap_or("?").to_string();
    let base = base_dir_name(prompt);
    let (index, base_missing) = match &base {
        None => (RepoIndex::default(), false),
        Some(name) => {
            let dir = bases.join(name);
            if dir.is_dir() { (RepoIndex::build(&read_project(&dir)), false) } else { (RepoIndex::default(), true) }
        }
    };
    let turns: Vec<&str> = prompt["turns"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let extraction = intent::extract(&turns, &index);

    let expected: Vec<&Value> = prompt["expected_facts"].as_array().map(|f| f.iter().collect()).unwrap_or_default();
    let implicit = expected.iter().filter(|e| e["implicit"] == true).count();
    let mut used = vec![false; extraction.facts.len()];
    let mut matched = Vec::new();
    let mut missed = Vec::new();
    let mut expected_kinds = Vec::new();
    for exp in expected.iter().filter(|e| e["implicit"] != true) {
        let id = exp["id"].as_str().unwrap_or("?").to_string();
        let kind = exp["kind"].as_str().unwrap_or("");
        let ok = if multi_value(kind) {
            // Every expected value must be among the extracted facts of this kind.
            let values: Vec<Value> = exp["params"]["value"].as_array().cloned().unwrap_or_default();
            let polarity = exp["polarity"].as_str();
            let mut hits = Vec::new();
            let all = values.iter().all(|v| {
                extraction.facts.iter().enumerate().any(|(i, f)| {
                    let found = f.kind == kind
                        && Some(f.polarity.as_str()) == polarity
                        && match &f.value {
                            Some(Value::Array(a)) => a.iter().any(|x| lower(x) == lower(v)),
                            Some(x) => lower(x) == lower(v),
                            None => false,
                        };
                    if found {
                        hits.push(i);
                    }
                    found
                })
            });
            // Correct values count as correct even when another expected
            // value is missing (the expected fact is still missed).
            for i in hits {
                used[i] = true;
            }
            all
        } else {
            match extraction.facts.iter().enumerate().position(|(i, f)| !used[i] && fact_matches(exp, f)) {
                Some(i) => {
                    used[i] = true;
                    true
                }
                // An already-used fact can satisfy a second expectation too.
                None => extraction.facts.iter().any(|f| fact_matches(exp, f)),
            }
        };
        expected_kinds.push((kind.to_string(), ok));
        if ok { matched.push(id) } else { missed.push(id) }
    }
    let invented_facts: Vec<&IntentFact> = extraction.facts.iter().zip(&used).filter(|(_, u)| !**u).map(|(f, _)| f).collect();
    let invented: Vec<String> = invented_facts.iter().map(|f| describe(f)).collect();
    let invented_kinds: Vec<String> = invented_facts.iter().map(|f| f.kind.to_string()).collect();

    let residual_words: BTreeSet<String> = extraction.residual.iter().flat_map(|r| content_words(&r.text)).collect();
    let phrases: Vec<String> = prompt["couldnt_check"].as_array().map(|p| p.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let couldnt_check_missed = phrases.iter().filter(|p| !content_words(p).is_subset(&residual_words)).cloned().collect();

    PromptResult {
        expected_kinds,
        invented_kinds,
        id,
        base,
        base_missing,
        expected: expected.len() - implicit,
        implicit,
        matched,
        missed,
        invented,
        couldnt_check_total: phrases.len(),
        couldnt_check_missed,
        coverage_holds: extraction.coverage.holds(),
        extraction,
    }
}

fn describe(f: &IntentFact) -> String {
    let v = f.value.as_ref().map(|v| format!(" {v}")).unwrap_or_default();
    let t = if f.targets.is_empty() { String::new() } else { format!(" [{}]", f.targets.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(", ")) };
    format!("{}:{}{v}{t} ← \"{}\"", f.kind, f.polarity.as_str(), f.source.text)
}

pub fn score_intent(prompts: &Path, bases: &Path) -> Result<IntentScore> {
    let mut files: Vec<_> = fs::read_dir(prompts)
        .with_context(|| format!("no corpus prompts in {}", prompts.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut results = Vec::new();
    for f in files {
        let prompt: Value = serde_json::from_str(&fs::read_to_string(&f)?).with_context(|| format!("{}", f.display()))?;
        results.push(score_prompt(&prompt, bases));
    }
    Ok(IntentScore { results })
}

impl IntentScore {
    pub fn expected(&self) -> usize {
        self.results.iter().map(|r| r.expected).sum()
    }
    pub fn matched(&self) -> usize {
        self.results.iter().map(|r| r.matched.len()).sum()
    }
    pub fn invented(&self) -> usize {
        self.results.iter().map(|r| r.invented.len()).sum()
    }
    pub fn recall(&self) -> Option<f64> {
        (self.expected() > 0).then(|| self.matched() as f64 / self.expected() as f64)
    }
    pub fn coverage_holds(&self) -> (usize, usize) {
        (self.results.iter().filter(|r| r.coverage_holds).count(), self.results.len())
    }
    pub fn couldnt_check(&self) -> (usize, usize) {
        let total: usize = self.results.iter().map(|r| r.couldnt_check_total).sum();
        let missed: usize = self.results.iter().map(|r| r.couldnt_check_missed.len()).sum();
        (total - missed, total)
    }

    /// Per kind: (expected, found, invented).
    pub fn by_kind(&self) -> std::collections::BTreeMap<String, (usize, usize, usize)> {
        let mut m: std::collections::BTreeMap<String, (usize, usize, usize)> = std::collections::BTreeMap::new();
        for r in &self.results {
            for (k, ok) in &r.expected_kinds {
                let e = m.entry(k.clone()).or_default();
                e.0 += 1;
                e.1 += usize::from(*ok);
            }
            for k in &r.invented_kinds {
                m.entry(k.clone()).or_default().2 += 1;
            }
        }
        m
    }

    pub fn to_text(&self) -> String {
        let mut s = format!("Intent extraction (Phase 5, lexicon {})\n", intent::LEXICON_VERSION);
        s.push_str(&format!("{:<5} {:>8} {:>8} {:>8}  notes\n", "prompt", "expected", "found", "invented"));
        for r in &self.results {
            let mut notes = Vec::new();
            if !r.missed.is_empty() {
                notes.push(format!("missed: {}", r.missed.join(", ")));
            }
            if r.implicit > 0 {
                notes.push(format!("{} implicit (not in the text)", r.implicit));
            }
            if r.base_missing {
                notes.push(format!("base {} missing", r.base.as_deref().unwrap_or("?")));
            }
            if !r.couldnt_check_missed.is_empty() {
                notes.push(format!("not left unchecked: {}", r.couldnt_check_missed.join(", ")));
            }
            if !r.coverage_holds {
                notes.push("COVERAGE BROKEN".into());
            }
            s.push_str(&format!("{:<5} {:>8} {:>8} {:>8}  {}\n", r.id, r.expected, r.matched.len(), r.invented.len(), notes.join("; ")));
            for inv in &r.invented {
                s.push_str(&format!("      invented: {inv}\n"));
            }
        }
        s.push_str("By kind (expected / found / invented):");
        for (k, (e, f, i)) in self.by_kind() {
            s.push_str(&format!(" {k} {e}/{f}/{i};"));
        }
        s.push('\n');
        let fmt = |v: Option<f64>| v.map_or("n/a".to_string(), |x| format!("{x:.2}"));
        let (cov_ok, cov_total) = self.coverage_holds();
        let (cc_ok, cc_total) = self.couldnt_check();
        s.push_str(&format!(
            "Recall {} ({}/{}), invented {}, couldn't-check phrases left unchecked {cc_ok}/{cc_total}, coverage invariant {cov_ok}/{cov_total}\n",
            fmt(self.recall()), self.matched(), self.expected(), self.invented()
        ));
        s
    }

    pub fn to_json(&self) -> Value {
        let (cov_ok, cov_total) = self.coverage_holds();
        let (cc_ok, cc_total) = self.couldnt_check();
        json!({
            "lexicon": intent::LEXICON_VERSION,
            "expected": self.expected(),
            "matched": self.matched(),
            "recall": self.recall(),
            "invented": self.invented(),
            "by_kind": self.by_kind().into_iter().map(|(k, (e, f, i))| (k, json!({"expected": e, "found": f, "invented": i}))).collect::<serde_json::Map<_, _>>(),
            "couldnt_check": { "left_unchecked": cc_ok, "total": cc_total },
            "coverage_invariant": { "holds": cov_ok, "total": cov_total },
            "prompts": self.results.iter().map(|r| json!({
                "id": r.id, "base": r.base, "expected": r.expected, "implicit": r.implicit,
                "matched": r.matched, "missed": r.missed, "invented": r.invented,
                "couldnt_check_missed": r.couldnt_check_missed, "coverage_holds": r.coverage_holds,
                "extraction": r.extraction.to_json(),
            })).collect::<Vec<_>>(),
        })
    }
}
