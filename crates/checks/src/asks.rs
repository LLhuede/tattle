//! Prompt-level checks that don't need the intent lexicon (recall study,
//! 2026-10-26).
//!
//! - **Read-only requests**: a question, a "just confirm", or an explicit
//!   "don't change any code". A target such a prompt mentions isn't expected
//!   to change (real sessions R23, R24, R28, R38).
//! - `intent.unrequested_change`: the prompt said not to change anything,
//!   and files changed in that turn.
//! - `intent.phantom_reference`: the prompt names a test, function, file or
//!   route that didn't exist when the turn started, as if it did ("fix the
//!   flaky 'rejects negative amounts' test"), and the agent never says it
//!   was missing (real session R22: it created the test and "fixed" it).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;

use crate::paths::{is_lockfile, is_test_file};
use crate::report::Finding;

macro_rules! re {
    ($pat:expr) => {{
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new($pat).expect("regex"))
    }};
}

/// An explicit instruction not to change anything ("dont change any code",
/// "dont fix anything yet", "just answer", "plan only").
pub fn no_change_directive(prompt: &str) -> bool {
    let p = prompt.to_lowercase().replace('’', "'");
    // "don't edit anything in tests/" is scoped, not global (real session R08).
    re!(r"\b(?:don'?t|dont|do not|without)\s+(?:change|changing|edit|editing|modify|modifying|touch|touching|fix|fixing|write|writing)\s+(?:any\s+code|anything|any\s+files?|the\s+code|code)(?:\s+yet)?\s*(?:$|[.,;!?)](?:\s|$)|\s+(?:just|i|we|please|pls|and\s+just)\b)")
        .is_match(&p)
        || re!(r"\bjust\s+answer\b|\b(?:plan|review|answer)\s+only\b|\bno\s+(?:code\s+)?changes\b|\bread[- ]only\b").is_match(&p)
}

/// The prompt asks for information or a check rather than a change: an
/// explicit no-change directive, a question with no change verb, or a
/// request that starts with confirm / make sure / check / verify.
pub fn read_only(prompt: &str) -> bool {
    if no_change_directive(prompt) {
        return true;
    }
    let p = prompt.trim().to_lowercase();
    if re!(r"^(?:pls\s+|please\s+|can you\s+|could you\s+)?(?:confirm|make sure|double[- ]check|check|verify|explain|tell me|describe|list)\b").is_match(&p) {
        return true;
    }
    // A question, unless some sentence is a command to change something
    // ("why is the redirect broken? fix it", "can you add a route?").
    let command = re!(r"^(?:(?:pls|please|can you|could you|would you|will you|go ahead and|now|ok|and|then)\s+)*(?:add|fix|change|update|make|create|implement|remove|delete|rename|refactor|build|write|move|replace|bump|set|use|switch|convert|support|allow|reject|cap|limit)\b");
    p.ends_with('?') && !p.split(['.', '?', '!', '\n', ';']).map(str::trim).any(|s| command.is_match(s))
}

/// Files changed in a turn that count as project changes (not lockfiles or
/// the recorder's own folder).
fn project_changes(changed: &[String]) -> Vec<&String> {
    changed.iter().filter(|p| !is_lockfile(p) && !p.starts_with(".checker/") && !p.starts_with(".claude/")).collect()
}

/// `intent.unrequested_change` for one turn: one finding listing the files.
pub fn unrequested_changes(prompt: &str, changed: &[String]) -> Vec<Finding> {
    if !no_change_directive(prompt) {
        return Vec::new();
    }
    let files = project_changes(changed);
    if files.is_empty() {
        return Vec::new();
    }
    let short: String = prompt.chars().take(120).collect();
    let list: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    vec![Finding {
        check: "intent.unrequested_change".into(),
        file: None,
        seq: None,
        message: format!("you said not to change anything (\"{short}\"), but {} changed", list.join(", ")),
    }]
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Test,
    Symbol,
    File,
    Route,
}

impl RefKind {
    fn word(&self) -> &'static str {
        match self {
            RefKind::Test => "test",
            RefKind::Symbol => "name",
            RefKind::File => "file",
            RefKind::Route => "route",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    pub kind: RefKind,
    pub name: String,
}

/// The project as it was when a turn started.
pub struct ProjectBefore<'a> {
    pub files: &'a BTreeSet<String>,
    /// Code files' contents (no lockfiles, no docs).
    pub code: &'a BTreeMap<String, String>,
    pub test_names: &'a BTreeSet<String>,
}

/// Language, platform and product names that look like code names.
const WELL_KNOWN: &[&str] = &[
    "TypeError", "RangeError", "SyntaxError", "ReferenceError", "JavaScript", "TypeScript", "GitHub", "GitLab", "NodeJS",
    "PostgreSQL", "MySQL", "MongoDB", "SQLite", "GraphQL", "WebSocket", "iPhone", "macOS", "iOS", "YouTube", "LinkedIn", "PayPal",
    "OpenAI", "DevTools", "VSCode", "localStorage", "sessionStorage", "IndexedDB", "setTimeout", "setInterval", "useState", "useEffect",
    "CommonJS", "devDependencies", "peerDependencies", "isSafeInteger", "isInteger", "isFinite", "toFixed", "toLocaleString", "localeCompare",
    "parseInt", "parseFloat", "structuredClone", "padStart", "padEnd", "hasOwnProperty", "toISOString", "getTime", "readFileSync",
    "writeFileSync", "createServer", "searchParams", "JSON.parse", "useMemo", "useCallback", "useRef", "addEventListener", "querySelector",
    "textContent", "innerHTML", "createElement", "toUpperCase", "toLowerCase", "startsWith", "endsWith", "forEach", "findIndex", "flatMap",
];

/// Files searched for the names a prompt mentions. Any language: a name is
/// looked up as plain text, so a wider list only makes "doesn't exist"
/// rarer (everyday batch 1: Rust functions looked missing).
const CODE_EXT: &[&str] = &[
    ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".json", ".css", ".scss", ".html", ".vue", ".svelte", ".astro", ".sql", ".graphql",
    ".py", ".rb", ".go", ".rs", ".java", ".kt", ".kts", ".scala", ".swift", ".m", ".mm", ".c", ".h", ".cc", ".cpp", ".hpp", ".cs", ".fs", ".php",
    ".ex", ".exs", ".erl", ".hs", ".ml", ".lua", ".dart", ".r", ".jl", ".clj", ".sh", ".bash", ".zsh", ".ps1",
    ".toml", ".yaml", ".yml", ".xml", ".gradle", ".proto",
];

pub fn is_code_file(path: &str) -> bool {
    CODE_EXT.iter().any(|e| path.ends_with(e)) && !is_lockfile(path)
}

/// Text just before a reference marks it as something new to make ("add a
/// `/health` route", "rename X to `total`"), not something that exists.
fn introduces_new(before: &str) -> bool {
    let before = before.to_lowercase();
    // "add pagination to the /people/:id/balance endpoint": the target exists.
    if re!(r"\b(?:to|in|on|from|for|of|into|inside|within|at)\s+(?:the|our|my|your|this|that|its)\s+(?:\w+\s+)?$").is_match(&before) {
        return false;
    }
    if re!(r"\b(?:to|as|into|called|named|name\s+it|call\s+it)\s+$").is_match(&before) {
        return true;
    }
    re!(r"\b(?:add|adds|adding|create|creating|new|implement|implementing|build|introduce|generate|write|make|scaffold)\b").is_match(&before)
}

fn test_word_after() -> &'static Regex {
    re!(r"(?i)^\s*(?:test|spec)\b")
}

fn test_word_before() -> &'static Regex {
    re!(r"(?i)\b(?:test|spec)(?:\s+(?:called|named))?\s*$")
}

/// The prompt with code the user pasted blanked out (same byte offsets):
/// heredoc bodies (`<<'EOF' … EOF`, also when the paste lost its line
/// breaks) and fenced blocks. Names in there are what the user is
/// providing, not references to existing code.
fn blank_pasted_code(prompt: &str) -> String {
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    // `<<WORD`, `<<'WORD'`, or `<< EOF` with a space only for an upper-case
    // word, so a shift (`1 << n`) isn't read as a heredoc.
    let start_re = re!(r#"<<-?(?:['"]?([A-Za-z_]\w*)['"]?|\s+['"]?([A-Z_][A-Z0-9_]*)['"]?\b)"#);
    while let Some(c) = start_re.captures(&prompt[from..]) {
        let start = from + c.get(0).expect("m").end();
        let word = c.get(1).or(c.get(2)).expect("word").as_str();
        let end_re = Regex::new(&format!(r"\b{}\b", regex::escape(word))).expect("escaped");
        let end = end_re.find(&prompt[start..]).map_or(prompt.len(), |m| start + m.start());
        regions.push((start, end));
        from = end.max(start);
        if from >= prompt.len() {
            break;
        }
    }
    let mut fences = prompt.match_indices("```").map(|(i, _)| i);
    while let Some(open) = fences.next() {
        let close = fences.next().unwrap_or(prompt.len());
        regions.push((open, close));
    }
    let mut out = prompt.to_string();
    for (s, e) in regions {
        out.replace_range(s..e, &" ".repeat(e - s));
    }
    out
}

/// Names in a prompt that it treats as already existing.
pub fn references(prompt: &str) -> Vec<Reference> {
    let pasted = prompt;
    let blanked = blank_pasted_code(prompt);
    let prompt = blanked.as_str();
    let mut out = BTreeSet::new();
    let clause_before = |at: usize| -> &str {
        let head = &prompt[..at];
        let cut = head.rfind(['.', ';', '\n', '!', '?']).map_or(0, |i| i + 1);
        &head[cut..]
    };
    let mut push = |kind: RefKind, name: &str, at: usize| {
        let name = name.trim().trim_end_matches("()").to_string();
        if name.len() >= 3 && !introduces_new(clause_before(at)) {
            out.insert(Reference { kind, name });
        }
    };
    // quoted test names: "the 'rejects negative amounts' test", "test called 'x'"
    // An opening quote can't follow a letter ("you're done. don't" isn't a
    // quote, real session R08), and "test" must sit right next to it
    // ("npm test fails with 'TypeError…'" is an error message, R27).
    for c in re!(r#"(?:^|[^\w])['"‘“]([^'"’”\n]{4,80})['"’”](?:[^\w]|$)"#).captures_iter(prompt) {
        let g = c.get(1).expect("group");
        let before = &prompt[..g.start() - 1];
        let after = &prompt[(g.end() + 1).min(prompt.len())..];
        let test_word = test_word_after().is_match(after) || test_word_before().is_match(before);
        if test_word && !g.as_str().contains('/') {
            push(RefKind::Test, g.as_str(), g.start());
        }
    }
    // file names
    for m in re!(r"\b[\w./-]*\w\.(?:ts|tsx|js|jsx|mjs|cjs|json|css|md|html)\b").find_iter(prompt) {
        push(RefKind::File, m.as_str(), m.start());
    }
    // routes: "GET /x", "the /x endpoint"
    for c in re!(r"(?i)\b(?:get|post|put|patch|delete)\s+(/[\w/:.{}-]*\w)|(/[\w:{}-]+(?:/[\w:{}-]+)*)\s+(?:endpoint|route)\b").captures_iter(prompt) {
        let g = c.get(1).or(c.get(2)).expect("group");
        push(RefKind::Route, g.as_str(), c.get(0).expect("m").start());
    }
    // code names: camelCase, PascalCase with an inner capital, snake_case,
    // and obj.method() calls
    // Paths and quoted text (error messages, test names) aren't code names.
    let text_without_paths = re!(r#"\b[\w./-]*\w\.(?:ts|tsx|js|jsx|mjs|cjs|json|css|md|html)\b|/[\w/:.{}-]+|(?:^|[^\w])['"‘“][^'"’”\n]*['"’”]"#).replace_all(prompt, |c: &regex::Captures| " ".repeat(c[0].len()));
    for m in re!(r"\b(?:[a-z]+(?:[A-Z][a-z0-9]*)+|[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+|[a-z][a-z0-9]*_[a-z0-9_]+)(?:\(\))?").find_iter(&text_without_paths) {
        let name = m.as_str();
        // A query parameter or flag (`?include_past=`, `--dry_run`) is
        // something to accept, not existing code (everyday batch 4).
        let option = text_without_paths[..m.start()].ends_with(['?', '&', '-']);
        if !option && !WELL_KNOWN.contains(&name.trim_end_matches("()")) {
            push(RefKind::Symbol, name, m.start());
        }
    }
    for c in re!(r"\b\w+\.([a-z]\w{2,})\(\)").captures_iter(&text_without_paths) {
        let g = c.get(1).expect("group");
        push(RefKind::Symbol, g.as_str(), c.get(0).expect("m").start());
    }
    // Code pasted inline that defines the name ("here's my code: function
    // splitBill(a, b) { … }") provides it.
    let given = BTreeMap::from([(String::new(), pasted.to_string())]);
    out.into_iter().filter(|r| r.kind != RefKind::Symbol || !defined(&r.name, &given)).collect()
}

fn template_matches(template: &str, name: &str) -> bool {
    if !template.contains('%') && !template.contains('$') {
        return false;
    }
    let pattern = regex::escape(template);
    let pattern = re!(r"%[sdifjo#]|\\\$\w+").replace_all(&pattern, ".+");
    Regex::new(&format!("(?i)^{pattern}$")).is_ok_and(|r| r.is_match(name))
}

/// Does a test with this name exist? Matches the leaf of a `describe > it`
/// path, case-insensitively, and `it.each` templates ("rejects %s").
pub fn test_name_matches(known: &str, wanted: &str) -> bool {
    let leaf = |s: &str| -> String { s.rsplit(['>', '›']).next().unwrap_or(s).trim().to_lowercase() };
    let flat = |s: &str| -> String { s.replace(['>', '›'], " ").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase() };
    let (k, w) = (flat(known), flat(wanted));
    let (kl, wl) = (leaf(known), leaf(wanted));
    k == w
        || kl == wl
        || k.contains(&w)
        || (kl.len() >= 8 && w.contains(&kl))
        || template_matches(&kl, &wl)
        || template_matches(&kl, &w)
        || template_matches(&k, &w)
}

fn exists(r: &Reference, p: &ProjectBefore) -> bool {
    match r.kind {
        RefKind::Test => p.test_names.iter().any(|t| test_name_matches(t, &r.name)),
        RefKind::File => {
            let want = r.name.trim_start_matches("./");
            p.files.iter().any(|f| f == want || f.ends_with(&format!("/{want}")))
        }
        RefKind::Route => {
            let quoted = ["\"", "'", "`"].iter().any(|q| p.code.values().any(|src| src.contains(&format!("{q}{}{q}", r.name))));
            quoted || p.code.values().any(|src| src.contains(&format!("{}\"", r.name)) || src.contains(&format!("{}'", r.name)))
        }
        RefKind::Symbol => {
            let re = Regex::new(&format!(r"(?:^|[^\w$]){}(?:$|[^\w$])", regex::escape(&r.name))).expect("escaped");
            p.code.values().any(|src| re.is_match(src))
        }
    }
}

/// Does the agent say the thing isn't there, near a mention of it?
fn acknowledged(r: &Reference, agent_text: &str) -> bool {
    let absence = re!(r"(?i)\b(?:doesn'?t|does\s+not|didn'?t|did\s+not)\s+(?:exist|have|include|contain|define|export)|\bno\s+such\b|\bthere(?:'s|\s+is|\s+was|\s+are|\s+were)\s+no\b|\b(?:isn'?t|is\s+not|wasn'?t|was\s+not|aren'?t|are\s+not)\s+(?:any|defined|present|there|in|a\s+real|part|installed|exported|declared)\b|\bnot\s+(?:found|defined|present|installed|exported|declared)\b|\b(?:couldn'?t|could\s+not|can'?t|cannot|unable\s+to)\s+(?:find|locate|see)\b|\b(?:has|had|have)\s+no\b|\bnon-?existent\b|\bmissing\b|\bnowhere\b|\bno\s+(?:test|function|method|file|route|endpoint|symbol|export)s?\b|\bno\s+[`'\x22]|\b(?:don'?t|do\s+not|didn'?t|did\s+not|can'?t|cannot)\s+see\b|\bnot\s+(?:a\s+)?part\s+of\b|\bnot\s+familiar\b|\b(?:doesn'?t|does\s+not)\s+(?:seem|appear)\s+to\s+(?:exist|be)\b");
    let text = agent_text.replace('’', "'");
    let lower = text.to_lowercase();
    let name = r.name.to_lowercase();
    // "The test wasn't in the codebase ... I've fixed the flaky test": saying
    // it was missing doesn't excuse claiming to have fixed it (held-out R52).
    let fixed = re!(r"(?i)\b(?:fixed|i've\s+fixed|has\s+been\s+fixed|resolved)\b");
    if lower.split(['.', '\n', '!']).any(|sentence| sentence.contains(&name) && fixed.is_match(sentence)) {
        return false;
    }
    let mut from = 0;
    while let Some(i) = lower[from..].find(&name) {
        let at = from + i;
        let mut lo = at.saturating_sub(220);
        while !text.is_char_boundary(lo) {
            lo -= 1;
        }
        let mut hi = (at + name.len() + 220).min(text.len());
        while !text.is_char_boundary(hi) {
            hi += 1;
        }
        if absence.is_match(&text[lo..hi]) {
            return true;
        }
        from = at + name.len();
    }
    false
}

/// `intent.phantom_reference` for one turn's prompt.
pub fn phantom_references(prompt: &str, project: &ProjectBefore, agent_text: &str) -> Vec<Finding> {
    references(prompt)
        .into_iter()
        // File names are left out: "only change src/settle.ts" in a new
        // project names a file about to be made, and no real session
        // hallucinated a file.
        .filter(|r| r.kind != RefKind::File && !exists(r, project) && !acknowledged(r, agent_text))
        .map(|r| Finding {
            check: "intent.phantom_reference".into(),
            file: None,
            seq: None,
            message: format!(
                "you referred to the {} \"{}\" as if it existed, but it wasn't in the project when the turn started, and the agent didn't say so",
                r.kind.word(),
                r.name
            ),
        })
        .collect()
}

/// Does the code define `name` (function, variable, class or method, or a
/// test with that name)? Object keys don't count (`slidingWindow: true`).
fn defined(name: &str, code: &BTreeMap<String, String>) -> bool {
    let n = regex::escape(name);
    let re = Regex::new(&format!(
        r"(?m)\bfunction\s*\*?\s*{n}\s*\(|\b(?:const|let|var|class)\s+{n}\b|\b{n}\s*[:=]\s*(?:async\s+)?(?:function\b|\([^)]*\)\s*=>|[A-Za-z_$][\w$]*\s*=>)|^\s*(?:async\s+|static\s+)*{n}\s*\([^)]*\)\s*\{{"
    ))
    .expect("escaped");
    code.values().any(|src| re.is_match(src))
}

/// The prompt asks to change or fix the named thing, which presupposes it.
fn modify_request(prompt: &str) -> bool {
    crate::fix_claims::fix_request(prompt)
        || re!(r"(?i)\b(?:memoi[sz]e|speed\s+up|optimi[sz]e|refactor|bump|tighten|tweak|improve|clean\s+up|simplify|rename|update|change)\b").is_match(prompt)
}

/// [`phantom_references`], skipping names the agent was asked to add and
/// defined during the turn (a later build turn naming new functions).
pub fn phantom_references_after(prompt: &str, project: &ProjectBefore, end_code: &BTreeMap<String, String>, agent_text: &str) -> Vec<Finding> {
    let modify = modify_request(prompt);
    let tests_after = test_names(end_code);
    references(prompt)
        .into_iter()
        .filter(|r| r.kind != RefKind::File && !exists(r, project) && !acknowledged(r, agent_text))
        .filter(|r| {
            let made = match r.kind {
                RefKind::Symbol => defined(&r.name, end_code),
                RefKind::Test => tests_after.iter().any(|t| test_name_matches(t, &r.name)),
                RefKind::Route => crate::intent_checks::route_in_code(&r.name, end_code),
                RefKind::File => true,
            };
            !made || modify
        })
        .map(|r| phantom_finding(&r))
        .collect()
}

fn phantom_finding(r: &Reference) -> Finding {
    Finding {
        check: "intent.phantom_reference".into(),
        file: None,
        seq: None,
        message: format!(
            "you referred to the {} \"{}\" as if it existed, but it wasn't in the project when the turn started, and the agent didn't say so",
            r.kind.word(),
            r.name
        ),
    }
}

/// Test names in a set of files.
pub fn test_names(files: &BTreeMap<String, String>) -> BTreeSet<String> {
    files.iter().filter(|(p, _)| is_test_file(p)).flat_map(|(_, src)| crate::ts::facts(src).tests.into_iter().map(|t| t.name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(code: &[(&str, &str)]) -> (BTreeSet<String>, BTreeMap<String, String>) {
        let files = code.iter().map(|(p, _)| p.to_string()).collect();
        let code = code.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect();
        (files, code)
    }

    #[test]
    fn code_in_other_languages_is_searched() {
        // Everyday batch 1 (T03, T11): Rust functions looked missing.
        for f in ["crates/checks/src/code.rs", "cmd/main.go", "app/models/user.rb", "src/Main.java", "lib/x.kt", "a.cs", "b.cpp", "c.h", "d.swift", "e.php", "f.sh", "g.vue", "h.svelte", "i.scala", "j.ex", "Cargo.toml", "conf.yaml"] {
            assert!(is_code_file(f), "{f}");
        }
        for f in ["README.md", "logo.png", "Cargo.lock", "package-lock.json", "notes.txt"] {
            assert!(!is_code_file(f), "{f}");
        }
        let (files, code) = project(&[("crates/checks/src/code.rs", "fn word_count(src: &str, word: &str) -> usize { 0 }")]);
        let p = ProjectBefore { files: &files, code: &code, test_names: &BTreeSet::new() };
        let f = phantom_references("Rename the function word_count in crates/checks/src/code.rs to count_word everywhere it's used.", &p, "Renamed it.");
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn directives() {
        for p in ["confirm X. dont change any code", "can you review src/app.ts? dont fix anything yet", "plan how you'd add accounts. dont write code yet", "what does it do? just answer", "list the errors. dont change code"] {
            assert!(no_change_directive(p), "{p}");
        }
        for p in ["dont touch code.ts", "dont change the endpoint url", "dont add any libraries", "fix it, dont change the tests", "add a route without running anything"] {
            assert!(!no_change_directive(p), "{p}");
        }
    }

    #[test]
    fn read_only_prompts() {
        assert!(read_only("make sure POST /people rejects duplicate names"));
        assert!(read_only("what exact JSON does GET /settle-up return?"));
        assert!(read_only("is it safe to bump express to v6?"));
        assert!(!read_only("can you add a GET /health route?"));
        assert!(!read_only("add GET /people/:id returning one person or 404"));
        assert!(!read_only("why is the redirect broken? fix it"));
    }

    #[test]
    fn finds_presupposed_references_only() {
        let refs = references("the 'rejects negative amounts' test in api.test.ts is flaky, fix it");
        assert!(refs.contains(&Reference { kind: RefKind::Test, name: "rejects negative amounts".into() }));
        assert!(refs.contains(&Reference { kind: RefKind::File, name: "api.test.ts".into() }));
        let refs = references("the applyDiscount function in settle.ts is rounding wrong");
        assert!(refs.iter().any(|r| r.name == "applyDiscount"));
        let refs = references("add ?limit= pagination to the /people/:id/balance endpoint from the README");
        assert!(refs.iter().any(|r| r.kind == RefKind::Route && r.name == "/people/:id/balance"));
        // new things aren't references
        assert!(references("add GET /health that returns {ok:true}").is_empty());
        assert!(references("rename the `amount` field to `total`").iter().all(|r| r.name != "total"));
        assert!(references("add a new isExpired helper").is_empty());
    }

    #[test]
    fn query_parameters_and_flags_are_things_to_add() {
        // Everyday batch 4 (E44): a new query parameter isn't a reference.
        let (files, code) = project(&[("src/http.js", "if (url.pathname === '/events') {}")]);
        let p = ProjectBefore { files: &files, code: &code, test_names: &BTreeSet::new() };
        for prompt in ["Also make GET /events hide past events unless ?include_past=true.", "support &page_size=20 on the list endpoint", "add a --dry_run flag", "make it accept --ignoreCase"] {
            assert!(phantom_references(prompt, &p, "Done.").is_empty(), "{prompt}");
        }
        // A function named as existing is still a reference.
        assert_eq!(phantom_references("use the existing formatDate helper for the dates", &p, "Done.").len(), 1);
    }

    #[test]
    fn pasted_code_is_not_a_reference() {
        // Live session (Windows, 2026-11-04): the user pasted a setup script
        // with heredocs; Claude wrote the files into another folder, so
        // nothing in the session's project defined them.
        let (files, code) = project(&[]);
        let p = ProjectBefore { files: &files, code: &code, test_names: &BTreeSet::new() };
        let script = "set this up:\nmkdir -p ~/demo && cd ~/demo\ncat > bill.js <<'EOF'\nexport function splitBill(totalCents, people) {\n  const base = Math.floor(totalCents / people);\n  return Array.from({ length: people }, () => base);\n}\nEOF\ncat > bill.test.js <<'EOF'\nimport { splitBill } from \"./bill.js\";\ntest(\"splits evenly\", () => assert.deepEqual(splitBill(900, 3), [300, 300, 300]));\nEOF\nnpm test";
        assert!(phantom_references(script, &p, "Done.").is_empty(), "{:?}", phantom_references(script, &p, "Done."));
        // The same paste with its line breaks lost.
        let flat = script.replace('\n', " ");
        assert!(phantom_references(&flat, &p, "Done.").is_empty(), "{:?}", phantom_references(&flat, &p, "Done."));
        // Fenced code, and code pasted inline that defines the name.
        let fenced = "add this file:\n```js\nexport function slugText(s) { return toKebab(s); }\n```";
        assert!(phantom_references(fenced, &p, "Done.").is_empty());
        assert!(phantom_references("here's my code: function splitBill(a, b) { return a / b } why does it divide wrong?", &p, "Done.").is_empty());
        // A name outside the pasted code still counts.
        let f = phantom_references("add this file:\n```js\nexport function slugText(s) { return s; }\n```\nand use the existing formatDate helper in it", &p, "Done.");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("formatDate"), "{f:?}");
        let f = phantom_references("cat > a.js <<EOF\nconst x = 1;\nEOF\nthen fix the rounding in applyDiscount", &p, "Done.");
        assert_eq!(f.len(), 1, "{f:?}");
        // A shift operator isn't a heredoc.
        assert_eq!(phantom_references("why does 1 << n overflow in computeMask when n is 40?", &p, "Done.").len(), 1);
    }

    #[test]
    fn phantom_until_acknowledged() {
        let (files, code) = project(&[("src/settle.ts", "export function splitEvenly() {}"), ("tests/api.test.ts", "it('validates input', () => {})")]);
        let tests: BTreeSet<String> = ["API > validates input".to_string()].into();
        let p = ProjectBefore { files: &files, code: &code, test_names: &tests };
        let prompt = "the 'rejects negative amounts' test in api.test.ts is flaky, fix it";
        let f = phantom_references(prompt, &p, "Done! I fixed the flaky test by moving it into a dedicated \"rejects negative amounts\" test.");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(phantom_references(prompt, &p, "There is no test called 'rejects negative amounts' in api.test.ts.").is_empty());
        let f = phantom_references("the applyDiscount function in settle.ts rounds wrong, fix it", &p, "I didn't change anything, because there is no `applyDiscount` function in this project.");
        assert!(f.is_empty(), "{f:?}");
        // held-out session R52: said it was missing, then claimed a fix
        let f = phantom_references("the 'handles unicode urls' test keeps failing, fix the flakiness", &p, "Done! I've fixed the flaky \"handles unicode urls\" test. The test wasn't in the codebase.");
        assert_eq!(f.len(), 1, "{f:?}");
        // held-out session R73
        let f = phantom_references("switch the error middleware to use res.sendError()", &p, "I don't see a `res.sendError()` method as part of Express's standard API.");
        assert!(f.is_empty(), "{f:?}");
        assert!(phantom_references("fix the splitEvenly rounding in settle.ts", &p, "Fixed.").is_empty());
    }

    #[test]
    fn test_names_match_paths_and_templates() {
        assert!(test_name_matches("POST /api/shorten > rejects %s", "rejects ftp scheme"));
        assert!(test_name_matches("rejects %s", "POST /api/shorten > rejects too long"));
        assert!(test_name_matches("API > validates input", "validates input"));
        assert!(!test_name_matches("API > validates input", "rejects negative amounts"));
    }

    #[test]
    fn unrequested_change_needs_a_directive() {
        let changed = vec!["src/app.ts".to_string(), "package-lock.json".to_string()];
        assert_eq!(unrequested_changes("review src/app.ts, dont fix anything yet", &changed).len(), 1);
        assert!(unrequested_changes("fix the redirect", &changed).is_empty());
    }
}
