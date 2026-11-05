//! Phase 5: turn a prompt into intent facts, without any AI.
//!
//! Pipeline (all deterministic):
//! 1. Split each turn into sentences and clauses.
//! 2. Apply the versioned constraint lexicon (`LEXICON_VERSION`): fixed
//!    phrase patterns, each producing one kind of fact.
//! 3. Ground targets ("the header", "the shorten endpoint") against an
//!    index of the real project: files, top-level names and routes.
//! 4. Resolve "it" / "that case" to the previous target.
//! 5. Merge turns: a later turn replaces an earlier fact with the same key.
//! 6. Account for every word: used by a fact, filler, or left over as
//!    "couldn't check" (the residual). Nothing is dropped silently.
//!
//! The fact kinds are a closed list (`KINDS`). Anything the lexicon doesn't
//! understand stays in the residual rather than being guessed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::paths::{is_lockfile, is_source_file, is_test_file};

/// Bump when lexicon rules change, so results can be compared.
pub const LEXICON_VERSION: &str = "v3";

/// Every kind of intent fact. Closed: no other kinds are ever produced.
pub const KINDS: &[&str] = &[
    "stack",
    "feature",
    "storage",
    "symbols",
    "route.exists",
    "param.literal",
    "param.charset",
    "deps.framework",
    "require.tests_added",
    "constraint.tests_pass",
    "constraint.deps_unchanged",
    "target.modify",
    "target.unchanged",
    "scope.only",
    "symbol.rename",
];

macro_rules! regex {
    ($p:expr) => {{
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new($p).expect("lexicon regex"))
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Polarity {
    Must,
    MustNot,
    Prefer,
}

impl Polarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Polarity::Must => "must",
            Polarity::MustNot => "must_not",
            Polarity::Prefer => "prefer",
        }
    }
}

/// How well a target matched something real in the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Exactly one project item, with the same words.
    Exact,
    /// Exactly one project item contains the words.
    SingleMatch,
    /// Several project items match.
    MultiMatch,
    /// Nothing in the project matches (e.g. new code, or a new project).
    Ungrounded,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Exact => "exact",
            Confidence::SingleMatch => "single-match",
            Confidence::MultiMatch => "multi-match",
            Confidence::Ungrounded => "ungrounded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The words from the prompt, e.g. "streak logic".
    pub text: String,
    /// Matching project items: `file:src/streak.js`, `symbol:currentStreak`,
    /// `route:POST /api/shorten`.
    pub grounded: Vec<String>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub turn: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntentFact {
    pub kind: &'static str,
    pub polarity: Polarity,
    pub value: Option<Value>,
    pub unit: Option<String>,
    pub targets: Vec<Target>,
    /// Where in the prompt it came from.
    pub source: Span,
    /// The lexicon rule that produced it.
    pub rule: &'static str,
}

impl IntentFact {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "polarity": self.polarity.as_str(),
            "value": self.value,
            "unit": self.unit,
            "targets": self.targets.iter().map(|t| json!({
                "text": t.text, "grounded": t.grounded, "confidence": t.confidence.as_str(),
            })).collect::<Vec<_>>(),
            "source": { "turn": self.source.turn, "start": self.source.start, "end": self.source.end, "text": self.source.text },
            "rule": self.rule,
            "lexicon": LEXICON_VERSION,
        })
    }

    /// Facts with the same key describe the same thing; a later turn's
    /// fact replaces an earlier one.
    fn key(&self) -> String {
        let target = self.targets.first().map(|t| t.text.as_str()).unwrap_or("");
        match self.kind {
            "storage" | "constraint.tests_pass" | "constraint.deps_unchanged" | "deps.framework" | "scope.only" => {
                self.kind.to_string()
            }
            "param.literal" => format!("{}:{}", self.kind, self.unit.as_deref().unwrap_or("")),
            "feature" | "stack" | "param.charset" | "symbols" => format!("{}:{}", self.kind, self.value.clone().unwrap_or(Value::Null)),
            "symbol.rename" => format!("{}:{}", self.kind, self.value.as_ref().and_then(|v| v.get("old")).cloned().unwrap_or(Value::Null)),
            _ => format!("{}:{}", self.kind, target),
        }
    }
}

/// How the words of the prompt were used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    pub total_chars: usize,
    pub consumed_chars: usize,
    pub residual_chars: usize,
    pub filler_chars: usize,
    pub content_words: usize,
    /// Content words neither used by a fact nor in the residual. Must be 0.
    pub unaccounted_words: usize,
}

impl Coverage {
    /// consumed + residual + filler = every character, and no content word
    /// is unaccounted for.
    pub fn holds(&self) -> bool {
        self.consumed_chars + self.residual_chars + self.filler_chars == self.total_chars && self.unaccounted_words == 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Extraction {
    /// Final facts after merging turns.
    pub facts: Vec<IntentFact>,
    /// Facts replaced by a later turn.
    pub replaced: Vec<IntentFact>,
    /// Text the lexicon couldn't turn into facts ("couldn't check").
    pub residual: Vec<Span>,
    pub coverage: Coverage,
}

impl Extraction {
    pub fn to_json(&self) -> Value {
        let span = |s: &Span| json!({ "turn": s.turn, "start": s.start, "end": s.end, "text": s.text });
        json!({
            "lexicon": LEXICON_VERSION,
            "facts": self.facts.iter().map(IntentFact::to_json).collect::<Vec<_>>(),
            "replaced": self.replaced.iter().map(IntentFact::to_json).collect::<Vec<_>>(),
            "couldnt_check": self.residual.iter().map(span).collect::<Vec<_>>(),
            "coverage": {
                "total_chars": self.coverage.total_chars,
                "consumed_chars": self.coverage.consumed_chars,
                "residual_chars": self.coverage.residual_chars,
                "filler_chars": self.coverage.filler_chars,
                "content_words": self.coverage.content_words,
                "unaccounted_words": self.coverage.unaccounted_words,
                "holds": self.coverage.holds(),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Words

/// Lowercase words of a name or phrase: camelCase, snake_case, kebab-case
/// and paths are split; plurals are reduced (`expenses` → `expense`).
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            cur.push(c.to_ascii_lowercase());
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|w| singular(&w)).collect()
}

/// Simple English singular for matching.
pub fn singular(w: &str) -> String {
    match w {
        "people" => "person".into(),
        "children" => "child".into(),
        "styling" | "styles" => "style".into(),
        _ if w.len() > 4 && w.ends_with("ies") => format!("{}y", &w[..w.len() - 3]),
        _ if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") => w[..w.len() - 1].to_string(),
        _ => w.to_string(),
    }
}

/// Words that say what kind of thing a target is, not which one.
const GENERIC: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "my", "our", "its", "of", "for", "to", "in", "on", "at",
    "and", "or", "how", "work", "existing", "all", "whole", "component", "logic", "endpoint", "route", "function",
    "file", "page", "screen", "calculation", "math", "maths", "code", "field", "module", "part", "bit", "one",
];

/// Phrases that mean "the API as a whole".
const API_WORDS: &[&str] = &["api", "endpoint", "route"];

// ---------------------------------------------------------------------------
// Repo index and grounding

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    words: BTreeSet<String>,
    is_route: bool,
    /// File that declares it (symbols and routes).
    file: Option<String>,
}

/// Files, top-level names and routes of a project, for grounding targets.
#[derive(Debug, Clone, Default)]
pub struct RepoIndex {
    entries: Vec<Entry>,
}

/// Top-level names of each file's contents, so a file that hasn't changed
/// isn't parsed again: shared by the before and after indexes of one check,
/// and kept on disk between the turns of a live session. Parsing is the slow
/// part of indexing a large project (20,000 files: about 1.5 s per index).
/// The names come from the same parser either way, so results are identical.
#[derive(Default)]
pub struct NameCache {
    names: BTreeMap<String, Vec<String>>,
    used: BTreeSet<String>,
    changed: bool,
}

/// Bump when `ts::facts` changes what it reports as top-level names.
const NAME_CACHE_FORMAT: &str = concat!("cv-names/1/", env!("CARGO_PKG_VERSION"));

impl NameCache {
    /// Load from `path`; empty when missing, unreadable or from another version.
    pub fn load(path: &Path) -> Self {
        let parsed = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(v) = parsed.filter(|v| v["format"] == NAME_CACHE_FORMAT) else { return Self::default() };
        let names = v["names"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();
        Self { names, ..Self::default() }
    }

    /// Write the entries used in this run (so the file never outgrows the
    /// project), when anything changed. Best effort.
    pub fn save(&self, path: &Path) {
        if !self.changed && self.used.len() == self.names.len() {
            return;
        }
        let names: serde_json::Map<String, Value> =
            self.names.iter().filter(|(k, _)| self.used.contains(*k)).map(|(k, v)| (k.clone(), json!(v))).collect();
        let text = json!({ "format": NAME_CACHE_FORMAT, "names": names }).to_string();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    fn top_level(&mut self, content: &str) -> Vec<String> {
        let key = capture::text::sha256_hex(content.as_bytes());
        self.used.insert(key.clone());
        if let Some(names) = self.names.get(&key) {
            return names.clone();
        }
        let names = crate::ts::facts(content).top_level;
        self.names.insert(key, names.clone());
        self.changed = true;
        names
    }
}

impl RepoIndex {
    /// Build from (path, contents) pairs. Lockfiles are skipped; names and
    /// routes come from JS/TS source and test files.
    pub fn build(files: &[(String, String)]) -> Self {
        Self::build_cached(files, &mut NameCache::default())
    }

    /// [`RepoIndex::build`], reusing names already parsed.
    pub fn build_cached(files: &[(String, String)], cache: &mut NameCache) -> Self {
        let route_re = regex!(r#"\b(?:app|router|server|api|fastify|hono|routes|r)\.(get|post|put|patch|delete|all)\(\s*["'`]([^"'`]+)["'`]"#);
        // Plain Node http: `req.url === '/api/x' && req.method === 'PUT'` (real session T2).
        let http_re = regex!(r#"\burl\s*===?\s*["'`](/[^"'`]*)["'`](?:\s*&&\s*\w+\.method\s*===?\s*["'`](\w+)["'`])?"#);
        let mut entries = Vec::new();
        for (path, content) in files {
            if is_lockfile(path) {
                continue;
            }
            let file_name = path.rsplit('/').next().unwrap_or(path);
            let stem = file_name.split('.').next().unwrap_or(file_name);
            entries.push(Entry { label: format!("file:{path}"), words: words(stem).into_iter().collect(), is_route: false, file: Some(path.clone()) });
            if !(is_source_file(path) || is_test_file(path)) {
                continue;
            }
            if is_source_file(path) {
                for name in cache.top_level(content) {
                    entries.push(Entry { label: format!("symbol:{name}"), words: words(&name).into_iter().collect(), is_route: false, file: Some(path.clone()) });
                }
            }
            let express = route_re.captures_iter(content).map(|c| (c[1].to_ascii_uppercase(), c[2].to_string()));
            let http = http_re
                .captures_iter(content)
                .map(|c| (c.get(2).map_or("ALL".to_string(), |m| m.as_str().to_ascii_uppercase()), c[1].to_string()));
            for (method, route) in express.chain(http).collect::<Vec<_>>() {
                let ws: BTreeSet<String> = words(&route).into_iter().filter(|w| w != "api").collect();
                entries.push(Entry { label: format!("route:{method} {route}"), words: ws, is_route: true, file: Some(path.clone()) });
            }
        }
        entries.sort_by(|a, b| a.label.cmp(&b.label));
        entries.dedup_by(|a, b| a.label == b.label);
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every route label (`route:POST /api/shorten`).
    pub fn route_labels(&self) -> BTreeSet<String> {
        self.entries.iter().filter(|e| e.is_route).map(|e| e.label.clone()).collect()
    }

    /// Files declaring a grounded item (`symbol:x`, `route:GET /y`, `file:z`).
    pub fn files_of(&self, label: &str) -> Vec<String> {
        self.entries.iter().filter(|e| e.label == label).filter_map(|e| e.file.clone()).collect()
    }

    /// Match a phrase from the prompt to project items. All the phrase's
    /// specific words must appear in an item. A phrase about "the api" or
    /// "the endpoints" with nothing more specific means every route.
    pub fn ground(&self, phrase: &str) -> Target {
        // A route ("GET /expenses", "/api/shorten") names a route directly.
        if let Some(c) = regex!(r"^(?:(get|post|put|patch|delete)\s+)?(/\S*)$").captures(phrase.trim()) {
            let method = c.get(1).map(|m| m.as_str().to_ascii_uppercase());
            let path = c[2].trim_end_matches('/').split('?').next().unwrap_or("").to_ascii_lowercase();
            let hits: Vec<String> = self
                .entries
                .iter()
                .filter(|e| e.is_route)
                .filter(|e| {
                    let label = e.label.trim_start_matches("route:");
                    let (m, p) = label.split_once(' ').unwrap_or(("", label));
                    p.trim_end_matches('/').to_ascii_lowercase() == path && (m == "ALL" || method.as_deref().is_none_or(|want| want == m))
                })
                .map(|e| e.label.clone())
                .collect();
            let confidence = match hits.len() {
                0 => Confidence::Ungrounded,
                1 => Confidence::Exact,
                _ => Confidence::MultiMatch,
            };
            return Target { text: phrase.to_string(), grounded: hits, confidence };
        }
        // A path ("src/streak.js", "app.ts") names a file directly.
        let p = phrase.trim().trim_start_matches("./");
        if p.contains('/') || regex!(r"^[\w-]+\.[a-z]{1,5}$").is_match(p) {
            let files: Vec<&Entry> = self
                .entries
                .iter()
                .filter(|e| {
                    e.label.strip_prefix("file:").is_some_and(|f| {
                        let f = f.to_ascii_lowercase();
                        let p = p.to_ascii_lowercase();
                        f == p || f.ends_with(&format!("/{p}"))
                    })
                })
                .collect();
            if !files.is_empty() {
                let confidence = if files.len() == 1 { Confidence::Exact } else { Confidence::MultiMatch };
                return Target { text: phrase.to_string(), grounded: files.iter().map(|e| e.label.clone()).collect(), confidence };
            }
        }
        let all = words(phrase);
        let specific: BTreeSet<String> = all.iter().filter(|w| !GENERIC.contains(&w.as_str()) && !API_WORDS.contains(&w.as_str())).cloned().collect();
        let mentions_api = all.iter().any(|w| API_WORDS.contains(&w.as_str()));
        let mut hits: Vec<&Entry> = if specific.is_empty() {
            if mentions_api { self.entries.iter().filter(|e| e.is_route).collect() } else { Vec::new() }
        } else {
            self.entries.iter().filter(|e| specific.is_subset(&e.words)).collect()
        };
        // When the phrase says "endpoint"/"route", prefer routes.
        if mentions_api && hits.iter().any(|e| e.is_route) {
            hits.retain(|e| e.is_route);
        }
        let confidence = match hits.as_slice() {
            [] => Confidence::Ungrounded,
            [one] if one.words == specific => Confidence::Exact,
            [_] => Confidence::SingleMatch,
            _ => Confidence::MultiMatch,
        };
        let mut grounded: Vec<String> = hits.iter().map(|e| e.label.clone()).collect();
        grounded.truncate(8);
        Target { text: phrase.to_string(), grounded, confidence }
    }
}

// ---------------------------------------------------------------------------
// Lexicon

/// Function words: never facts, never "couldn't check".
const FILLER: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "so", "to", "of", "in", "on", "at", "for", "with", "by", "from", "as", "it",
    "its", "it's", "this", "that", "these", "those", "them", "they", "their", "i", "i'm", "me", "my", "we", "our",
    "us", "you", "your", "can", "could", "would", "should", "will", "just", "also", "too", "please", "pls", "want",
    "need", "like", "some", "any", "all", "each", "every", "is", "are", "be", "was", "were", "do", "does", "did",
    "have", "has", "had", "there", "here", "then", "than", "if", "when", "which", "who", "what", "how", "very",
    "really", "make", "makes", "get", "let's", "lets", "ok", "okay", "hey", "hi", "thanks", "thank", "use", "using",
    "into", "up", "out", "only",
];

/// Technologies recognised as `stack` facts: (pattern, value).
const TECHS: &[(&str, &str)] = &[
    (r"react", "react"),
    (r"vite", "vite"),
    (r"vue", "vue"),
    (r"svelte", "svelte"),
    (r"next\.?js|nextjs", "next"),
    (r"express", "express"),
    (r"fastify", "fastify"),
    (r"koa", "koa"),
    (r"nest\.?js", "nest"),
    (r"sqlite", "sqlite"),
    (r"postgres(?:ql)?", "postgres"),
    (r"mysql", "mysql"),
    (r"mongo(?:db)?", "mongodb"),
    (r"prisma", "prisma"),
    (r"typescript", "typescript"),
    (r"javascript", "javascript"),
    (r"node(?:\.?js)?", "node"),
    (r"tailwind(?:css)?", "tailwind"),
    (r"vitest", "vitest"),
    (r"jest", "jest"),
    (r"python", "python"),
    (r"fastapi", "fastapi"),
    (r"flask", "flask"),
    (r"django", "django"),
    (r"redis", "redis"),
    (r"redux", "redux"),
    (r"zustand", "zustand"),
    (r"axios", "axios"),
    (r"lodash", "lodash"),
    (r"jquery", "jquery"),
    (r"bootstrap", "bootstrap"),
    (r"graphql", "graphql"),
];

/// Features recognised as `feature` facts: (pattern, value).
const FEATURES: &[(&str, &str)] = &[
    (r"log\s?ins?|logging in|sign\s?ins?|sign\s?ups?|authentication|auth|user accounts?|api[\s-]?keys?", "auth"),
    (r"servers?|back\s?ends?", "server"),
    (r"e-?mails?", "email"),
    (r"table of contents", "toc"),
    (r"dark mode", "dark-mode"),
    (r"rate[\s-]?limit(?:ing|er|s)?", "rate-limiting"),
    (r"pagination|paginated?", "pagination"),
    (r"search(?:ing)?(?:\s+(?:box|bar|field))?", "search"),
    (r"drag\s*(?:and|&|n|'n')\s*drop|drag(?:ging)?", "drag-and-drop"),
    (r"export\w*\s+(?:\w+\s+){0,4}?(?:as|to|in)\s+(?:a\s+)?csv|csv\s+exports?", "csv-export"),
    (r"export\w*\s+(?:\w+\s+){0,4}?(?:as|to|in)\s+(?:an?\s+)?json(?:\s+file)?|(?:download|save)s?\s+(?:\w+\s+){0,4}?as\s+(?:an?\s+)?json(?:\s+file)?", "json-export"),
    (r"import\s+(?:button|from\s+(?:an?\s+)?json|(?:an?\s+)?json)", "json-import"),
    (r"notif(?:y|ies|ications?)", "notifications"),
    (r"dry[\s-]?runs?", "dry-run"),
    (r"uploads?|uploading", "file-upload"),
];

const NEGATION: &str = r"\b(?:no|not|without|don['’]?t|do not|never|forget(?: about)?|skip|remove|avoid|scratch|ditch|drop (?:the|that)|get rid of|instead of|rather than)\b";

/// Words that make a feature or stack choice optional.
const HEDGE: &str = r"\bmaybe\b|\bif (?:it['’]?s|its|it is) (?:easy|simple|quick|possible)\b|\bif possible\b|\bideally\b|\boptional(?:ly)?\b|\bnice to have\b|\bbonus\b|\bif you (?:can|have time)\b";

fn neg_re() -> &'static Regex {
    regex!(NEGATION)
}

/// "keep / leave / preserve" before a route.
fn keep_before_re() -> &'static Regex {
    regex!(r"\b(?:keep|leave|preserve)\b")
}

/// "… has to stay the same", "… as is" after a route.
fn keep_after_re() -> &'static Regex {
    regex!(r"^\s*(?:\S+\s+){0,4}?(?:the same|as is|as it is|alone|unchanged|intact)\b")
}

fn hedge_re() -> &'static Regex {
    regex!(HEDGE)
}

/// Text right after a number that makes it a rate ("10 per minute").
fn rate_after_re() -> &'static Regex {
    regex!(r"^\s*(?:per|a|an|/|every)\b")
}

/// Words that turn "if" / "that" etc. into the end of a target phrase.
const TARGET_STOPS: &[&str] = &[
    " and ", " so ", " but ", " that ", " which ", " with ", " if ", " because ", " when ", " where ", " to make ",
    // Trailing asides (real session H43: "only touch App.jsx for this").
    " for this", " for that", " for now", " for me", " pls", " please", " though", " tho", " too", " as well", " thanks", " lol", " rn",
];

fn cut_target(s: &str) -> String {
    let mut t = format!(" {} ", s.trim());
    for stop in TARGET_STOPS {
        // Only stops *after* the first word end the phrase ("that case"
        // must survive so it can be resolved).
        if let Some(i) = t[1..].find(stop).map(|i| i + 1) {
            t.truncate(i);
        }
    }
    let mut t = t.trim().trim_end_matches(['.', '/']).replace('`', "").trim().to_string();
    for lead in ["the ", "a ", "an ", "my ", "our ", "your "] {
        if let Some(rest) = t.strip_prefix(lead) {
            t = rest.to_string();
        }
    }
    t.trim().to_string()
}

fn is_anaphor(t: &str) -> bool {
    matches!(t, "it" | "that" | "this" | "them" | "those" | "these" | "that case" | "this case" | "that bug" | "this bug" | "that one" | "this one")
}

struct Ctx<'a> {
    turn: usize,
    text: &'a str,
    lower: String,
    index: &'a RepoIndex,
    facts: Vec<IntentFact>,
    consumed: Vec<(usize, usize)>,
    last_target: Option<String>,
}

struct NewFact<'r> {
    kind: &'static str,
    polarity: Polarity,
    value: Option<Value>,
    unit: Option<String>,
    targets: Vec<String>,
    span: (usize, usize),
    consume: &'r [(usize, usize)],
    rule: &'static str,
}

impl Ctx<'_> {
    fn consume(&mut self, start: usize, end: usize) {
        if end > start {
            self.consumed.push((start, end));
        }
    }

    fn add(&mut self, f: NewFact) {
        for &(s, e) in f.consume {
            self.consume(s, e);
        }
        let mut targets = Vec::new();
        for t in f.targets {
            let t = cut_target(&t);
            if t.is_empty() {
                continue;
            }
            let text = if is_anaphor(&t) {
                match &self.last_target {
                    Some(prev) => prev.clone(),
                    None => continue,
                }
            } else {
                self.last_target = Some(t.clone());
                t
            };
            targets.push(self.index.ground(&text));
        }
        let fact = IntentFact {
            kind: f.kind,
            polarity: f.polarity,
            value: f.value,
            unit: f.unit,
            targets,
            source: Span { turn: self.turn, start: f.span.0, end: f.span.1, text: self.text[f.span.0..f.span.1].to_string() },
            rule: f.rule,
        };
        let same = |g: &IntentFact| {
            g.kind == fact.kind && g.polarity == fact.polarity && g.value == fact.value && g.unit == fact.unit
                && g.targets.iter().map(|t| &t.text).eq(fact.targets.iter().map(|t| &t.text))
        };
        if !self.facts.iter().any(same) {
            self.facts.push(fact);
        }
    }
}

/// (start, end) ranges of `text` split at any of `delims`, delimiters
/// excluded, empty pieces dropped.
fn pieces(text: &str, base: usize, delims: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if delims.contains(&c) {
            if !text[start..i].trim().is_empty() {
                out.push((base + start, base + i));
            }
            start = i + c.len_utf8();
        }
    }
    if !text[start..].trim().is_empty() {
        out.push((base + start, base + text.len()));
    }
    out
}

/// Sentence ranges: split at newlines, and at `.`, `!`, `?` only when
/// followed by a space or the end, so `src/streak.js` and `1.5` stay whole.
fn sentences(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let ends = c == '\n'
            || (matches!(c, '.' | '!' | '?') && bytes.get(i + 1).is_none_or(|b| b.is_ascii_whitespace()));
        if ends {
            if !text[start..i].trim().is_empty() {
                out.push((start, i));
            }
            start = i + c.len_utf8();
        }
    }
    if !text[start..].trim().is_empty() {
        out.push((start, text.len()));
    }
    out
}

/// Run every lexicon rule on one turn.
fn apply_rules(ctx: &mut Ctx) {
    let lower = ctx.lower.clone();
    for (s0, s1) in sentences(&lower) {
        let sentence = &lower[s0..s1];
        let clauses = pieces(sentence, s0, &[',', ';', '(', ')']);
        sentence_rules(ctx, sentence, s0);
        for &(c0, c1) in &clauses {
            clause_rules(ctx, &lower[c0..c1], c0, sentence);
        }
    }
}

fn negated(prefix: &str) -> bool {
    neg_re().is_match(prefix)
}

/// Rules that may cross commas.
fn sentence_rules(ctx: &mut Ctx, s: &str, base: usize) {
    // symbols: "it needs people, expenses (…)"
    if let Some(c) = regex!(r"\bneeds?\s+(?:to have\s+)?([a-z]+(?:\s*,\s*[a-z]+)+)").captures(s) {
        let list = c.get(1).expect("group");
        let names: Vec<Value> = list.as_str().split(',').map(|n| json!(singular(n.trim()))).collect();
        let m = c.get(0).expect("match");
        ctx.add(NewFact {
            kind: "symbols", polarity: Polarity::Must, value: Some(Value::Array(names)), unit: None, targets: vec![],
            span: (base + m.start(), base + m.end()), consume: &[(base + m.start(), base + m.end())], rule: "needs-list",
        });
    }
    // symbols: "theres companies and jobs", "recipes have ingredients and steps"
    // Entity lists keep nouns only: "there is fine and i" isn't a list of
    // entities (seen in a real session).
    const NOT_ENTITIES: &[&str] = &["fine", "good", "ok", "okay", "nice", "great", "bad", "cool", "wrong", "right", "done", "broken"];
    let list_of = |s: &str| -> Vec<Value> {
        s.split([',', ' '])
            .map(str::trim)
            .filter(|w| w.len() > 2 && *w != "and" && !FILLER.contains(w) && !NOT_ENTITIES.contains(w))
            .map(|w| json!(singular(w)))
            .collect()
    };
    if let Some(c) = regex!(r"\bthere(?:['’]?s|\s+is|\s+are|\s+will be)\s+([a-z]+(?:\s*,\s*[a-z]+)*\s+and\s+[a-z]+)\b").captures(s)
        && list_of(&c[1]).len() >= 2
    {
        let m = c.get(0).expect("match");
        ctx.add(NewFact {
            kind: "symbols", polarity: Polarity::Must, value: Some(Value::Array(list_of(&c[1]))), unit: None, targets: vec![],
            span: (base + m.start(), base + m.end()), consume: &[(base + m.start(), base + m.end())], rule: "theres-list",
        });
    }
    if let Some(c) = regex!(r"\b([a-z]+s)\s+(?:have|has|contain|contains|include|includes)\s+([a-z]+(?:\s*,\s*[a-z]+)*\s+and\s+[a-z]+)\b").captures(s)
        && !c[2].starts_with("to ")
        && !matches!(&c[1], "this" | "us" | "its")
        && list_of(&c[2]).len() >= 2
    {
        let m = c.get(0).expect("match");
        let mut names = vec![json!(singular(&c[1]))];
        names.extend(list_of(&c[2]));
        ctx.add(NewFact {
            kind: "symbols", polarity: Polarity::Must, value: Some(Value::Array(names)), unit: None, targets: vec![],
            span: (base + m.start(), base + m.end()), consume: &[(base + m.start(), base + m.end())], rule: "have-list",
        });
    }
    // dependency constraint, with hedges ("if you can avoid it") as prefer
    if let Some(m) = regex!(r"\b(?:don['’]?t|do not|no|not|can['’]?t|cannot|without|never|avoid)\b[^.;]*?\b(?:new\s+|extra\s+|additional\s+|more\s+)?(?:npm\s+)?(?:packages?|dependenc(?:y|ies)|deps|libraries|library)\b").find(s) {
        let hedge = regex!(r"\bif (?:you can(?: avoid it)?|possible)\b|\bideally\b|\bpreferably\b|\bwhere possible\b|\btry (?:not )?to\b").find(s);
        let mut consume = vec![(base + m.start(), base + m.end())];
        if let Some(h) = hedge {
            consume.push((base + h.start(), base + h.end()));
        }
        ctx.add(NewFact {
            kind: "constraint.deps_unchanged",
            polarity: if hedge.is_some() { Polarity::Prefer } else { Polarity::Must },
            value: None, unit: None, targets: vec![], span: (base + m.start(), base + m.end()), consume: &consume, rule: "no-new-deps",
        });
    }
    // tests must pass
    let tests_pass = regex!(r"\b(?:keep|make sure|ensure)\b[^.;]*?\btests?\b[^.;]*?\b(?:pass(?:ing|es)?|green)\b|\bmake\s+(?:them|the tests|tests|it|everything)\s+pass\b|\b(?:don['’]?t|do not|without)\s+break(?:ing)?\s+(?:the\s+|any\s+)?(?:existing\s+)?tests?\b");
    if let Some(m) = tests_pass.find(s)
        && s.contains("test")
    {
        ctx.add(NewFact {
            kind: "constraint.tests_pass", polarity: Polarity::Must, value: None, unit: None, targets: vec![],
            span: (base + m.start(), base + m.end()), consume: &[(base + m.start(), base + m.end())], rule: "tests-pass",
        });
    }
}

/// Compiled word-boundary patterns for a (pattern, value) table.
fn compile(table: &'static [(&'static str, &'static str)]) -> Vec<(Regex, &'static str)> {
    table.iter().map(|(p, v)| (Regex::new(&format!(r"\b(?:{p})\b")).expect("lexicon regex"), *v)).collect()
}

fn tech_res() -> &'static Vec<(Regex, &'static str)> {
    static RES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RES.get_or_init(|| compile(TECHS))
}

fn feature_res() -> &'static Vec<(Regex, &'static str)> {
    static RES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RES.get_or_init(|| compile(FEATURES))
}

/// Rules that work within one clause.
fn clause_rules(ctx: &mut Ctx, c: &str, base: usize, sentence: &str) {
    let at = |m: regex::Match| (base + m.start(), base + m.end());

    // stack
    for (re, value) in tech_res() {
        for m in re.find_iter(c) {
            let neg = negated(&c[..m.start()]);
            ctx.add(NewFact {
                kind: "stack", polarity: if neg { Polarity::MustNot } else { Polarity::Must }, value: Some(json!(value)),
                unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "tech",
            });
        }
    }

    // feature
    for (re, value) in feature_res() {
        for m in re.find_iter(c) {
            let prefix = &c[..m.start()];
            let neg = neg_re().find_iter(prefix).last();
            let hedge = hedge_re().find(c);
            let mut consume = vec![at(m)];
            if let Some(n) = neg {
                consume.push((base + n.start(), base + n.end()));
            }
            if let Some(h) = hedge {
                consume.push((base + h.start(), base + h.end()));
            }
            let polarity = match (neg.is_some(), hedge.is_some()) {
                (true, _) => Polarity::MustNot,
                (false, true) => Polarity::Prefer,
                _ => Polarity::Must,
            };
            ctx.add(NewFact {
                kind: "feature", polarity, value: Some(json!(value)),
                unit: None, targets: vec![], span: at(m), consume: &consume, rule: "feature",
            });
        }
    }

    // storage
    if let Some(m) = regex!(r"\b(?:sav|stor|keep|kept|persist)\w*\b[^,;]*?\b(?:in|to|using)\s+(?:the\s+)?(?:browser|local\s?storage|indexed\s?db)\b|\blocal\s?storage\b").find(c) {
        let polarity = if negated(&c[..m.start()]) { Polarity::MustNot } else { Polarity::Must };
        ctx.add(NewFact { kind: "storage", polarity, value: Some(json!("browser")), unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "storage-browser" });
    }
    if let Some(m) = regex!(r"\b(?:sav|stor|writ|persist)\w*\b[^,;]*?\b(?:to|in)\s+(?:a\s+)?(?:json\s+|text\s+)?files?\b").find(c) {
        ctx.add(NewFact { kind: "storage", polarity: Polarity::Must, value: Some(json!("file")), unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "storage-file" });
    }

    // no frameworks
    if let Some(m) = regex!(r"\b(?:no|without|don['’]?t use|not use|avoid)\s+(?:any\s+)?frameworks?\b|\bvanilla\b").find(c) {
        ctx.add(NewFact { kind: "deps.framework", polarity: Polarity::MustNot, value: None, unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "no-framework" });
    }

    // tests added
    if let Some(cap) = regex!(r"\b(?:add|write|include|create|with)\s+(?:[a-z-]+\s+){0,3}?tests?\b(?:\s+(?:for|of|on|covering)\s+([a-z0-9_`.][a-z0-9\s'/._`-]*))?|^\s*(?:unit\s+|some\s+)?tests?\s+for\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        let target = cap.get(1).or(cap.get(2)).map(|t| t.as_str().to_string()).into_iter().collect();
        ctx.add(NewFact { kind: "require.tests_added", polarity: Polarity::Must, value: None, unit: None, targets: target, span: at(m), consume: &[at(m)], rule: "tests-added" });
    }

    // only change X and Y
    if let Some(cap) = regex!(r"\bonly\s+(?:change|touch|edit|modify|update)\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        let targets = cap[1].split(" and ").flat_map(|t| t.split(" or ")).map(str::to_string).collect();
        ctx.add(NewFact { kind: "scope.only", polarity: Polarity::Must, value: None, unit: None, targets, span: at(m), consume: &[at(m)], rule: "only-change" });
    }

    // don't touch X
    if let Some(cap) = regex!(r"\b(?:don['’]?t|do not|never|without)\s+(?:touch|chang|modif|edit|alter)\w*\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        ctx.add(NewFact { kind: "target.unchanged", polarity: Polarity::MustNot, value: None, unit: None, targets: vec![cap[1].to_string()], span: at(m), consume: &[at(m)], rule: "dont-touch" });
    }

    // leave X alone / keep X the same / X has to stay the same
    if let Some(cap) = regex!(r"\b(?:leave|keep)\s+([a-z0-9_`.][a-z0-9\s'/._`-]*?)\s+(?:alone|as is|as it is|the same|unchanged|untouched|intact)\b").captures(c)
        .or_else(|| regex!(r"(?:^|\b(?:the|but|and)\s+)([a-z0-9_`.][a-z0-9'/._`-]*(?:\s+[a-z0-9'/._`-]+){0,2}?)\s+(?:has to|must|should|needs to|needs? to)\s+stay\s+(?:the same|as is|unchanged|intact)\b").captures(c))
    {
        let m = cap.get(0).expect("match");
        let target = cut_target(&cap[1]);
        if !target.is_empty() && !target.contains("test") {
            ctx.add(NewFact { kind: "target.unchanged", polarity: Polarity::MustNot, value: None, unit: None, targets: vec![target], span: at(m), consume: &[at(m)], rule: "leave-alone" });
        }
    }

    // add … in/to the X
    if let Some(cap) = regex!(r"\b(?:add|put|place|insert)\b[^,;]*?\b(?:in|to|into|on)\s+the\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        let target = cut_target(&cap[1]);
        let end = base + cap.get(1).expect("group").start() + target.len();
        let span = (base + m.start(), end.min(base + m.end()));
        ctx.add(NewFact { kind: "target.modify", polarity: Polarity::Must, value: None, unit: None, targets: vec![target], span, consume: &[span], rule: "add-to" });
    }
    // X is wrong / broken
    if let Some(cap) = regex!(r"^\s*(?:the\s+)?([a-z0-9_`.][a-z0-9\s'/._`-]*?)\s+(?:is|are|seems?)\s+(?:wrong|broken|not working|buggy|failing|incorrect)\b").captures(c)
        && !cap[1].contains("test")
    {
        let m = cap.get(0).expect("match");
        ctx.add(NewFact { kind: "target.modify", polarity: Polarity::Must, value: None, unit: None, targets: vec![cap[1].to_string()], span: at(m), consume: &[at(m)], rule: "is-wrong" });
    }
    // fix X
    if let Some(cap) = regex!(r"\bfix\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        let target = cut_target(&cap[1]);
        let span = (base + m.start(), base + cap.get(1).expect("group").start() + target.len().min(cap[1].len()));
        ctx.add(NewFact { kind: "target.modify", polarity: Polarity::Must, value: None, unit: None, targets: vec![target], span, consume: &[span], rule: "fix" });
    }

    // new endpoint
    if let Some(cap) = regex!(r"\b(?:an?|new)\s+([a-z0-9_`.][a-z0-9\s'/._`-]*?)\s+(?:endpoint|route)\b").captures(c)
        && !cap[1].contains('/')
    {
        let m = cap.get(0).expect("match");
        ctx.add(NewFact { kind: "route.exists", polarity: Polarity::Must, value: None, unit: None, targets: vec![cap[1].to_string()], span: at(m), consume: &[at(m)], rule: "new-endpoint" });
    }
    if let Some(cap) = regex!(r"\b(?:an?|new)\s+(?:endpoint|route)\s+(?:to|for|that)\s+([a-z0-9_`.][a-z0-9\s'/._`-]*)").captures(c) {
        let m = cap.get(0).expect("match");
        ctx.add(NewFact { kind: "route.exists", polarity: Polarity::Must, value: None, unit: None, targets: vec![cap[1].to_string()], span: at(m), consume: &[at(m)], rule: "endpoint-to" });
    }
    if let Some(m) = regex!(r"\bredirect(?:s|ing)?\s+(?:them|users?|people|visitors?|the user|to)\b").find(c) {
        ctx.add(NewFact { kind: "route.exists", polarity: Polarity::Must, value: None, unit: None, targets: vec!["redirect".into()], span: at(m), consume: &[at(m)], rule: "redirect" });
    }

    // explicit routes: "GET /jobs", "/api/shorten". An existing route is a
    // change to it; a route the project doesn't have yet is a new one.
    for cap in regex!(r"(?:^|[\s(`])(?:(get|post|put|patch|delete)\s+)?(/[a-z0-9_/:.{}\-]*[a-z0-9_}])").captures_iter(c) {
        let path_m = cap.get(2).expect("path");
        let start = cap.get(1).map_or(path_m.start(), |m| m.start());
        let span = (base + start, base + path_m.end());
        let text = c[start..path_m.end()].to_string();
        let grounded = ctx.index.ground(&text);
        let exists = !grounded.grounded.is_empty();
        if !exists && cap.get(1).is_none() {
            continue; // a bare path that isn't a route here (could be a file path)
        }
        // "dont change the endpoint url tho /settle-up has to stay the same"
        // means leave it alone (real session H52).
        let before_route = &c[..start];
        let after_route = &c[path_m.end()..];
        let keep = negated(before_route) || keep_before_re().is_match(before_route) || keep_after_re().is_match(after_route);
        let (kind, polarity) = match (keep, exists) {
            (true, _) => ("target.unchanged", Polarity::MustNot),
            (false, true) => ("target.modify", Polarity::Must),
            (false, false) => ("route.exists", Polarity::Must),
        };
        ctx.add(NewFact { kind, polarity, value: None, unit: None, targets: vec![text], span, consume: &[span], rule: "explicit-route" });
    }

    // numbers: lengths count only with a constraint word ("cap at 50
    // characters", "should be 6 chars", "between 3 and 20 chars"), so a
    // number describing a bug ("a name thats like 500 chars") isn't one.
    let length_cue = regex!(r"\b(?:cap|caps|capped|limit|limited|max|maximum|at most|up to|no more than|no longer than|min|minimum|at least|between|should|must|has to|have to|needs? to|stay|stays|remain|exactly|be|are|is)\b");
    let subject = regex!(r"([a-z][a-z\s]*?)\s+(?:should|must|need to|have to|has to|will)?\s*(?:be|are|is|stay|stays)\s+\d").captures(c).map(|s| s[1].to_string());
    if let Some(cap) = regex!(r"\bbetween\s+(\d+)\s+(?:and|to|-)\s+(\d+)\s*(?:characters?|chars?|digits?|letters?)\b").captures(c) {
        let m = cap.get(0).expect("match");
        for g in [1, 2] {
            let n: u64 = cap[g].parse().unwrap_or(0);
            ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(n)), unit: Some("characters".into()), targets: vec![], span: at(m), consume: &[at(m)], rule: "length-range" });
        }
    }
    for cap in regex!(r"\b(\d+)[\s-]*(?:characters?|chars?|digits?|letters?)\b(\s+long)?").captures_iter(c) {
        let m = cap.get(0).expect("match");
        if !(length_cue.is_match(&c[..m.start()]) || cap.get(2).is_some()) {
            continue;
        }
        let n: u64 = cap[1].parse().unwrap_or(0);
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(n)), unit: Some("characters".into()), targets: subject.clone().into_iter().collect(), span: at(m), consume: &[at(m)], rule: "length" });
    }
    // durations: "25 min work then 5 min break"
    for cap in regex!(r"\b(\d+)\s*(min(?:ute)?s?|secs?|seconds?|hours?|hrs?)\b").captures_iter(c) {
        let m = cap.get(0).expect("match");
        if rate_after_re().is_match(&c[m.end()..]) {
            continue; // a rate, handled below
        }
        let unit = match &cap[2] {
            u if u.starts_with("min") => "minute",
            u if u.starts_with("sec") => "second",
            _ => "hour",
        };
        let n: u64 = cap[1].parse().unwrap_or(0);
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(n)), unit: Some(unit.into()), targets: vec![], span: at(m), consume: &[at(m)], rule: "duration" });
    }
    if let Some(cap) = regex!(r"\b(\d+)\s+(?:requests?|reqs?|calls?|hits?|times?)?\s*(?:per|a|an|/|every)\s+(second|minute|hour|day)\b").captures(c) {
        let m = cap.get(0).expect("match");
        let n: u64 = cap[1].parse().unwrap_or(0);
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(n)), unit: Some(cap[2].to_string()), targets: vec![], span: at(m), consume: &[at(m)], rule: "rate" });
    }
    if let Some(cap) = regex!(r"\bat\s+(\d{1,2})(?::\d\d)?\s*(am|pm|o['’]?clock)?\b").captures(c)
        && (cap.get(2).is_some() || regex!(r"\b(?:morning|evening|night|noon|midnight|daily|every day|each day|remind\w*|schedul\w*|cron|tonight|tomorrow)\b").is_match(sentence))
    {
        let m = cap.get(0).expect("match");
        let n: u64 = cap[1].parse().unwrap_or(0);
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(n)), unit: Some("time".into()), targets: vec![], span: at(m), consume: &[at(m)], rule: "time" });
    }
    if let Some(cap) = regex!(r"\bper\s+(ip(?:\s+address)?|user|client|account)\b").captures(c) {
        let m = cap.get(0).expect("match");
        let v = if cap[1].starts_with("ip") { "ip" } else { &cap[1] };
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(v)), unit: Some("per".into()), targets: vec![], span: at(m), consume: &[at(m)], rule: "per" });
    }
    if let Some(cap) = regex!(r"\bstart(?:ing|s)?\s+with\s+(?:an?\s+)?(underscore|dot|period|dash|hyphen|hash)").captures(c) {
        let m = cap.get(0).expect("match");
        let v = match &cap[1] {
            "underscore" => "_",
            "dot" | "period" => ".",
            "dash" | "hyphen" => "-",
            _ => "#",
        };
        ctx.add(NewFact { kind: "param.literal", polarity: Polarity::Must, value: Some(json!(v)), unit: Some("prefix".into()), targets: vec![], span: at(m), consume: &[at(m)], rule: "starts-with" });
    }
    if let Some(m) = regex!(r"\b(?:letters and (?:numbers|digits)|numbers and letters|alphanumeric)(?:\s+only)?\b").find(c) {
        ctx.add(NewFact { kind: "param.charset", polarity: Polarity::Must, value: Some(json!("alphanumeric")), unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "charset" });
    }

    // rename X to Y
    if let Some(cap) = regex!(r#"\brename\s+(?:the\s+)?(?:[a-z]+\s+(?:['"`]))?['"`]?([a-z_]\w*)['"`]?\s+(?:(?:field|column|property|prop|variable|key)\s+)?(?:on\s+\w+\s+)?to\s+['"`]?([a-z_]\w*)['"`]?"#).captures(c) {
        let m = cap.get(0).expect("match");
        ctx.add(NewFact { kind: "symbol.rename", polarity: Polarity::Must, value: Some(json!({"old": &cap[1], "new": &cap[2]})), unit: None, targets: vec![], span: at(m), consume: &[at(m)], rule: "rename" });
    }
}

// ---------------------------------------------------------------------------
// Coverage

/// Residual pieces and coverage numbers for one turn.
fn account(turn: usize, text: &str, lower: &str, consumed: &[(usize, usize)]) -> (Vec<Span>, Coverage) {
    let word_re = regex!(r"[a-z0-9_'’]+(?:-[a-z0-9_]+)*");
    let in_consumed = |s: usize, e: usize| consumed.iter().any(|&(a, b)| s < b && e > a);
    let mut residual = Vec::new();
    let mut cov = Coverage { total_chars: text.len(), ..Coverage::default() };
    let clauses: Vec<(usize, usize)> = sentences(lower)
        .into_iter()
        .flat_map(|(s0, s1)| pieces(&lower[s0..s1], s0, &[',', ';', '(', ')']))
        .collect();
    for (c0, c1) in clauses {
        let clause = &lower[c0..c1];
        // (start, end, is_filler, is_consumed)
        let toks: Vec<(usize, usize, bool, bool)> = word_re
            .find_iter(clause)
            .map(|m| {
                let (s, e) = (c0 + m.start(), c0 + m.end());
                (s, e, FILLER.contains(&m.as_str()), in_consumed(s, e))
            })
            .collect();
        cov.content_words += toks.iter().filter(|t| !t.2).count();
        let mut run: Vec<(usize, usize, bool, bool)> = Vec::new();
        let mut flush = |run: &mut Vec<(usize, usize, bool, bool)>| {
            while run.first().is_some_and(|t| t.2) {
                run.remove(0);
            }
            while run.last().is_some_and(|t| t.2) {
                run.pop();
            }
            if let (Some(first), Some(last)) = (run.first(), run.last()) {
                residual.push(Span { turn, start: first.0, end: last.1, text: text[first.0..last.1].to_string() });
            }
            run.clear();
        };
        for t in toks {
            if t.3 {
                flush(&mut run);
            } else {
                run.push(t);
            }
        }
        flush(&mut run);
    }
    // Characters: consumed spans (merged), then residual spans, rest filler.
    let mut merged: Vec<(usize, usize)> = consumed.to_vec();
    merged.sort();
    let mut union: Vec<(usize, usize)> = Vec::new();
    for (s, e) in merged {
        match union.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => union.push((s, e)),
        }
    }
    cov.consumed_chars = union.iter().map(|(s, e)| e.min(&text.len()) - s.min(&text.len())).sum();
    cov.residual_chars = residual.iter().map(|r| r.end - r.start).sum();
    cov.filler_chars = cov.total_chars.saturating_sub(cov.consumed_chars + cov.residual_chars);
    // Every content word must be consumed or inside a residual piece.
    for m in word_re.find_iter(lower) {
        if FILLER.contains(&m.as_str()) {
            continue;
        }
        let (s, e) = (m.start(), m.end());
        let covered = in_consumed(s, e) || residual.iter().any(|r| s >= r.start && e <= r.end);
        if !covered {
            cov.unaccounted_words += 1;
        }
    }
    (residual, cov)
}

// ---------------------------------------------------------------------------
// Entry point

/// Extract intent facts from a prompt's turns (oldest first).
pub fn extract(turns: &[&str], index: &RepoIndex) -> Extraction {
    let mut all: Vec<IntentFact> = Vec::new();
    let mut residual = Vec::new();
    let mut coverage = Coverage::default();
    let mut last_target: Option<String> = None;
    for (turn, text) in turns.iter().enumerate() {
        let mut ctx = Ctx {
            turn,
            text,
            lower: text.to_ascii_lowercase(),
            index,
            facts: Vec::new(),
            consumed: Vec::new(),
            last_target: last_target.clone(),
        };
        apply_rules(&mut ctx);
        let (res, cov) = account(turn, text, &ctx.lower, &ctx.consumed);
        residual.extend(res);
        coverage.total_chars += cov.total_chars;
        coverage.consumed_chars += cov.consumed_chars;
        coverage.residual_chars += cov.residual_chars;
        coverage.filler_chars += cov.filler_chars;
        coverage.content_words += cov.content_words;
        coverage.unaccounted_words += cov.unaccounted_words;
        last_target = ctx.last_target.clone();
        all.extend(ctx.facts);
    }
    // Merge turns: a later turn's fact replaces an earlier one with the same key.
    let mut facts: Vec<IntentFact> = Vec::new();
    let mut replaced = Vec::new();
    for f in all {
        if let Some(pos) = facts.iter().position(|g| g.key() == f.key() && g.source.turn < f.source.turn) {
            replaced.push(facts.remove(pos));
        }
        facts.push(f);
    }
    Extraction { facts, replaced, residual, coverage }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_cache_gives_the_same_index_and_survives_a_reload() {
        let files = vec![
            ("src/a.ts".to_string(), "export function shortenUrl() {}\napp.get('/api/x', h);\n".to_string()),
            ("src/b.ts".to_string(), "export const limit = 1;\n".to_string()),
        ];
        let plain = RepoIndex::build(&files);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("names.json");
        let mut cache = NameCache::load(&path);
        let first = RepoIndex::build_cached(&files, &mut cache);
        cache.save(&path);
        let mut reloaded = NameCache::load(&path);
        assert_eq!(reloaded.names.len(), 2);
        let second = RepoIndex::build_cached(&files, &mut reloaded);
        assert!(!reloaded.changed, "everything came from the cache");
        let labels = |i: &RepoIndex| i.entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&first), labels(&plain));
        assert_eq!(labels(&second), labels(&plain));
        // Another version's cache is ignored.
        std::fs::write(&path, r#"{"format":"cv-names/0","names":{"x":["y"]}}"#).unwrap();
        assert!(NameCache::load(&path).names.is_empty());
    }

    fn kinds(e: &Extraction) -> Vec<(String, &'static str, String)> {
        e.facts
            .iter()
            .map(|f| {
                let v = f.value.as_ref().map(|v| v.to_string()).unwrap_or_default();
                let t = f.targets.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join("|");
                (format!("{}:{}", f.kind, f.polarity.as_str()), f.rule, format!("{v}{t}"))
            })
            .collect()
    }

    fn has(e: &Extraction, kind: &str, polarity: Polarity, detail: &str) -> bool {
        e.facts.iter().any(|f| {
            let v = f.value.as_ref().map(|v| v.to_string()).unwrap_or_default();
            let t = f.targets.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join("|");
            f.kind == kind && f.polarity == polarity && (detail.is_empty() || v.contains(detail) || t.contains(detail))
        })
    }

    fn run(text: &str) -> Extraction {
        extract(&[text], &RepoIndex::default())
    }

    #[test]
    fn words_split_names_and_plurals() {
        assert_eq!(words("currentStreak"), ["current", "streak"]);
        assert_eq!(words("rate_limit-checks"), ["rate", "limit", "check"]);
        assert_eq!(words("/api/settle-up"), ["api", "settle", "up"]);
        assert_eq!(words("people expenses categories"), ["person", "expense", "category"]);
        assert_eq!(words("styling"), ["style"]);
    }

    #[test]
    fn negation_flips_features() {
        let e = run("keep it simple, no login. save everything in the browser so it works without a server");
        assert!(has(&e, "feature", Polarity::MustNot, "auth"));
        assert!(has(&e, "feature", Polarity::MustNot, "server"));
        assert!(has(&e, "storage", Polarity::Must, "browser"));
        let e = run("add login so only the person who created an expense can delete it");
        assert!(has(&e, "feature", Polarity::Must, "auth"), "{:?}", kinds(&e));
    }

    #[test]
    fn hedges_make_constraints_soft() {
        assert!(has(&run("dont add any new packages if you can avoid it"), "constraint.deps_unchanged", Polarity::Prefer, ""));
        assert!(has(&run("we cant use any new libraries, the client is strict"), "constraint.deps_unchanged", Polarity::Must, ""));
        assert!(has(&run("no frameworks"), "deps.framework", Polarity::MustNot, ""));
    }

    #[test]
    fn scope_and_untouched_targets() {
        let e = run("add a dark mode toggle in the header. only change the styling and the header component, dont touch the streak logic");
        assert!(has(&e, "feature", Polarity::Must, "dark-mode"));
        assert!(has(&e, "target.modify", Polarity::Must, "header"));
        let scope = e.facts.iter().find(|f| f.kind == "scope.only").unwrap();
        let t: Vec<&str> = scope.targets.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(t, ["styling", "header component"]);
        assert!(has(&e, "target.unchanged", Polarity::MustNot, "streak logic"));
    }

    #[test]
    fn numbers_and_units() {
        let e = run("add rate limiting to the shorten endpoint, 10 requests per minute per ip. keep the existing tests passing");
        assert!(e.facts.iter().any(|f| f.value == Some(json!(10)) && f.unit.as_deref() == Some("minute")));
        assert!(e.facts.iter().any(|f| f.value == Some(json!("ip"))));
        assert!(has(&e, "constraint.tests_pass", Polarity::Must, ""));
        assert!(has(&e, "target.modify", Polarity::Must, "shorten endpoint"));
        let e = run("short codes should be 6 characters, letters and numbers only");
        let len = e.facts.iter().find(|f| f.rule == "length").unwrap();
        assert_eq!((len.value.clone(), len.targets[0].text.as_str()), (Some(json!(6)), "short codes"));
        assert!(has(&e, "param.charset", Polarity::Must, "alphanumeric"));
        assert!(run("remind me every morning at 8").facts.iter().any(|f| f.value == Some(json!(8))));
        assert!(run("skip files starting with an underscore").facts.iter().any(|f| f.value == Some(json!("_"))));
    }

    #[test]
    fn it_and_that_case_point_back() {
        let e = run("settle up is wrong when someone pays for themselves. fix it and add a test for that case");
        let targets: Vec<(&str, &str)> = e.facts.iter().flat_map(|f| f.targets.iter().map(move |t| (f.kind, t.text.as_str()))).collect();
        assert!(targets.contains(&("target.modify", "settle up")));
        assert!(targets.contains(&("require.tests_added", "settle up")));
        assert_eq!(e.facts.iter().filter(|f| f.kind == "target.modify").count(), 1, "fix it = the same target");
    }

    #[test]
    fn later_turns_replace_earlier_ones() {
        let e = extract(
            &[
                "can you make it save habits to a json file on a little server so i can use it on my phone too",
                "actually no forget the server, keep it all in the browser like before",
            ],
            &RepoIndex::default(),
        );
        assert!(has(&e, "feature", Polarity::MustNot, "server"));
        assert!(has(&e, "storage", Polarity::Must, "browser"));
        assert!(!has(&e, "storage", Polarity::Must, "file"));
        assert_eq!(e.replaced.len(), 2, "server and file storage from turn 1 were replaced");
    }

    #[test]
    fn vague_prompts_give_no_facts_and_full_residual() {
        let e = run("make it look nicer and feel snappier");
        assert!(e.facts.is_empty());
        let res: Vec<&str> = e.residual.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(res, ["look nicer and feel snappier"]);
        assert!(e.coverage.holds());
    }

    #[test]
    fn coverage_accounts_for_every_word() {
        for text in [
            "make me a simple habit tracker web app. i want to add habits, tick them off each day and see a streak for each one. keep it simple, no login.",
            "build a small api for splitting bills between friends. it needs people, expenses (who paid, how much, who its split between) and a settle up endpoint",
            "rename the amount field on expenses to total everywhere, including the api and the database",
            "",
        ] {
            let e = run(text);
            assert!(e.coverage.holds(), "{text}: {:?}", e.coverage);
            assert_eq!(e.coverage.total_chars, text.len());
        }
    }

    #[test]
    fn only_known_kinds() {
        let e = run("use react, vite and sqlite. rename the amount field to total. add an endpoint to export csv. it needs people, expenses");
        assert!(!e.facts.is_empty());
        assert!(e.facts.iter().all(|f| KINDS.contains(&f.kind)));
    }

    #[test]
    fn grounding_uses_real_files_routes_and_names() {
        let files = vec![
            ("src/streak.js".to_string(), "export function currentStreak(days) { return 0; }\n".to_string()),
            ("src/App.jsx".to_string(), "export default function App() { return null; }\n".to_string()),
            ("src/app.ts".to_string(), "app.post(\"/api/shorten\", (req, res) => {});\napp.get(\"/:code\", (req, res) => {});\n".to_string()),
            ("package-lock.json".to_string(), "{}".to_string()),
        ];
        let index = RepoIndex::build(&files);
        let t = index.ground("streak logic");
        assert_eq!(t.confidence, Confidence::MultiMatch);
        assert!(t.grounded.contains(&"file:src/streak.js".to_string()));
        assert!(t.grounded.contains(&"symbol:currentStreak".to_string()));
        assert_eq!(index.ground("shorten endpoint").grounded, ["route:POST /api/shorten"]);
        assert_eq!(index.ground("shorten endpoint").confidence, Confidence::Exact);
        let api = index.ground("how the api works");
        assert_eq!(api.grounded.len(), 2, "the api = every route");
        assert_eq!(index.ground("header component").confidence, Confidence::Ungrounded);
        assert_eq!(index.ground("src/streak.js").grounded, ["file:src/streak.js"]);
        assert_eq!(index.ground("streak.js").confidence, Confidence::Exact);
        // Plain Node http routes (real session T2).
        let http = RepoIndex::build(&[("server/index.js".to_string(), "if (req.url === '/api/habits' && req.method === 'PUT') {}\nif (req.url === '/health') {}\n".to_string())]);
        assert_eq!(http.ground("put /api/habits").grounded, ["route:PUT /api/habits"]);
        assert_eq!(http.ground("get /health").grounded, ["route:ALL /health"]);
    }

    #[test]
    fn paths_and_code_survive_sentence_splitting() {
        let files = vec![("src/streak.js".to_string(), "export const x = 1;\n".to_string())];
        let index = RepoIndex::build(&files);
        let e = extract(&["only touch src/streak.js and the header, keep tests passing. dont change `config.ts`."], &index);
        let scope = e.facts.iter().find(|f| f.kind == "scope.only").unwrap();
        assert_eq!(scope.targets[0].text, "src/streak.js");
        assert_eq!(scope.targets[0].confidence, Confidence::Exact);
        assert_eq!(scope.targets[1].text, "header");
        let unchanged = e.facts.iter().find(|f| f.kind == "target.unchanged").unwrap();
        assert_eq!(unchanged.targets[0].text, "config.ts");
        assert!(e.coverage.holds());
        assert_eq!(sentences("use 1.5 seconds. ok"), [(0, 15), (16, 19)]);
    }

    // --- lexicon v2 (patterns from the development set) ---

    #[test]
    fn explicit_routes_are_new_or_existing() {
        let files = vec![("src/app.ts".to_string(), "app.get(\"/expenses\", h);\napp.post(\"/api/shorten\", h);\n".to_string())];
        let index = RepoIndex::build(&files);
        let e = extract(&["add pagination to GET /expenses. add a DELETE /people/:id endpoint. rate limit /api/shorten"], &index);
        let routes: Vec<(&str, &str, Confidence)> = e
            .facts
            .iter()
            .filter(|f| f.rule == "explicit-route")
            .map(|f| (f.kind, f.targets[0].text.as_str(), f.targets[0].confidence))
            .collect();
        assert_eq!(
            routes,
            [
                ("target.modify", "get /expenses", Confidence::Exact),
                ("route.exists", "delete /people/:id", Confidence::Ungrounded),
                ("target.modify", "/api/shorten", Confidence::Exact),
            ]
        );
        assert!(!e.facts.iter().any(|f| f.rule == "new-endpoint"), "explicit routes aren't counted twice");
        // A bare path that isn't a route here is left alone.
        assert!(extract(&["edit /tmp/notes"], &index).facts.is_empty());
    }

    #[test]
    fn hedges_and_reversals() {
        assert!(has(&run("recipes app. maybe dark mode if its easy"), "feature", Polarity::Prefer, "dark-mode"));
        let e = extract(&["notes saved in localstorage", "put the notes in sqlite behind a server instead of localstorage"], &RepoIndex::default());
        assert!(has(&e, "storage", Polarity::MustNot, "browser"), "{:?}", kinds(&e));
        let e = extract(&["add an api key so only we can create links", "actually scratch the api key thing"], &RepoIndex::default());
        assert!(has(&e, "feature", Polarity::MustNot, "auth"), "{:?}", kinds(&e));
        assert!(has(&run("try not to add any deps"), "constraint.deps_unchanged", Polarity::Prefer, ""));
    }

    #[test]
    fn numbers_need_a_constraint_word() {
        let e = run("POST /people lets you add a name thats like 500 chars of garbage. cap names at 50 characters");
        let values: Vec<Value> = e.facts.iter().filter(|f| f.kind == "param.literal").filter_map(|f| f.value.clone()).collect();
        assert_eq!(values, [json!(50)], "500 describes the bug; 'at 50' isn't a time");
        let e = run("aliases between 3 and 20 chars. codes stay 6 chars");
        let values: Vec<Value> = e.facts.iter().filter(|f| f.kind == "param.literal").filter_map(|f| f.value.clone()).collect();
        assert_eq!(values, [json!(3), json!(20), json!(6)]);
        let e = run("25 min work then 5 min break, max 10 requests per minute");
        let pairs: Vec<(Value, String)> = e.facts.iter().filter_map(|f| Some((f.value.clone()?, f.unit.clone()?))).collect();
        assert!(pairs.contains(&(json!(25), "minute".into())) && pairs.contains(&(json!(5), "minute".into())));
        assert_eq!(pairs.iter().filter(|(v, _)| *v == json!(10)).count(), 1, "a rate isn't also a duration");
    }

    #[test]
    fn looser_test_and_symbol_phrasing() {
        let e = run("write vitest tests for the jobs endpoints. keep all the existing tests green. tests for per-group settle up pls");
        let targets: Vec<&str> = e.facts.iter().filter(|f| f.kind == "require.tests_added").flat_map(|f| f.targets.iter().map(|t| t.text.as_str())).collect();
        assert_eq!(targets, ["jobs endpoints", "per-group settle up"]); // "pls" trimmed (v3)
        assert!(has(&e, "constraint.tests_pass", Polarity::Must, ""));
        assert!(has(&run("theres companies and jobs"), "symbols", Polarity::Must, "company"));
        assert!(has(&run("recipes have ingredients and steps"), "symbols", Polarity::Must, "ingredient"));
        assert!(!has(&run("companies have to log in and post jobs"), "symbols", Polarity::Must, ""));
        let e = run("rename the habit 'name' field to 'title' everywhere");
        assert_eq!(e.facts[0].value, Some(json!({"old": "name", "new": "title"})));
        assert!(run("dont slow down the redirect").facts.iter().all(|f| f.kind != "route.exists"));
    }

    // --- lexicon v3 (from real-session false alarms) ---

    #[test]
    fn v3_boundaries_negated_routes_and_leave_alone() {
        let files = vec![
            ("src/App.jsx".to_string(), "export default function App() {}\n".to_string()),
            ("src/streak.js".to_string(), "export function s() {}\n".to_string()),
            ("src/app.ts".to_string(), "app.get(\"/settle-up\", h);\n".to_string()),
        ];
        let index = RepoIndex::build(&files);
        // Real session H43.
        let e = extract(&["cap them at 40 chars. only touch App.jsx for this, leave streak.js alone"], &index);
        let scope = e.facts.iter().find(|f| f.kind == "scope.only").unwrap();
        assert_eq!(scope.targets[0].text, "app.jsx");
        assert_eq!(scope.targets[0].grounded, ["file:src/App.jsx"]);
        assert!(e.facts.iter().any(|f| f.kind == "target.unchanged" && f.targets[0].text == "streak.js"));
        // Real session H52.
        let e = extract(&["rename settleUp to minimiseTransfers but dont change the endpoint url tho /settle-up has to stay the same"], &index);
        let route = e.facts.iter().find(|f| f.rule == "explicit-route").unwrap();
        assert_eq!((route.kind, route.polarity), ("target.unchanged", Polarity::MustNot));
        assert!(!e.facts.iter().any(|f| f.kind == "target.modify"));
        // Changing a route still is a change.
        let e = extract(&["add pagination to GET /settle-up"], &index);
        assert!(e.facts.iter().any(|f| f.kind == "target.modify"));
        // Junk entity lists.
        assert!(!has(&run("there is fine and i dont mind"), "symbols", Polarity::Must, ""));
        assert!(has(&run("theres companies and jobs"), "symbols", Polarity::Must, "company"));
        for text in ["keep the api the same", "the login page has to stay the same"] {
            assert!(run(text).facts.iter().any(|f| f.kind == "target.unchanged"), "{text}");
        }
    }
}
