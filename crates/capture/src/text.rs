//! Text clean-up applied to everything captured: redaction, path
//! placeholders and trimming of long text.
//!
//! Order matters: redact first (so a secret in the part that gets trimmed
//! can't survive in the kept head or tail), then placeholders, then trim.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::redact::Redactor;

/// Text longer than this many bytes is trimmed.
pub const TRIM_LIMIT: usize = 4096;
/// Bytes kept from the start and from the end of trimmed text.
pub const TRIM_KEEP: usize = 1024;

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Trim long text to its first and last `TRIM_KEEP` bytes, with a marker
/// recording the original length and SHA-256. Returns `None` if the text is
/// short enough to keep whole.
pub fn trim(s: &str) -> Option<String> {
    if s.len() <= TRIM_LIMIT {
        return None;
    }
    let mut head_end = TRIM_KEEP;
    while !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - TRIM_KEEP;
    while !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    Some(format!(
        "{}\n…[cv-trimmed bytes={} sha256={}]…\n{}",
        &s[..head_end],
        s.len(),
        sha256_hex(s.as_bytes()),
        &s[tail_start..]
    ))
}

/// A placeholder followed by a backslash path (Windows).
fn windows_tail() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(<PROJECT>|<HOME>)((?:\\[^\\\s"'<>|*?:;,]+)+\\?)"#).expect("regex"))
}

/// Replaces machine-specific paths and the username with placeholders.
pub struct Placeholders {
    /// (text to find, placeholder), longest first.
    pairs: Vec<(String, &'static str)>,
    user: Option<Regex>,
}

impl Placeholders {
    /// `project_paths` may include the same folder in different forms (e.g.
    /// `/var/...` and its canonical `/private/var/...`). Each is also
    /// canonicalised.
    pub fn new(project_paths: &[&Path], home: Option<&Path>, user: Option<&str>) -> Self {
        let mut pairs: Vec<(String, &'static str)> = Vec::new();
        let mut add = |p: &Path, label: &'static str| {
            let mut forms: Vec<PathBuf> = vec![p.to_path_buf()];
            if let Ok(c) = crate::fsx::canonical(p) {
                forms.push(c);
            }
            for form in forms {
                let text = form.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
                for text in crate::fsx::path_spellings(&text) {
                    // Never replace "" or "/": that would mangle every path.
                    if text.len() > 1 && !pairs.iter().any(|(t, _)| *t == text) {
                        pairs.push((text, label));
                    }
                }
            }
        };
        for p in project_paths {
            add(p, "<PROJECT>");
        }
        if let Some(h) = home {
            add(h, "<HOME>");
        }
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        let user = user
            .filter(|u| u.len() >= 3)
            .map(|u| Regex::new(&format!(r"\b{}\b", regex::escape(u))).expect("escaped regex"));
        Self { pairs, user }
    }

    pub fn none() -> Self {
        Self {
            pairs: Vec::new(),
            user: None,
        }
    }

    pub fn apply(&self, s: &str) -> String {
        let mut out = Cow::Borrowed(s);
        for (find, label) in &self.pairs {
            if out.contains(find.as_str()) {
                out = Cow::Owned(out.replace(find.as_str(), label));
            }
        }
        // Windows paths: `<PROJECT>\src\a.ts` becomes `<PROJECT>/src/a.ts`,
        // the form the checker reads.
        if out.contains(">\\") {
            let re = windows_tail();
            if re.is_match(&out) {
                out = Cow::Owned(re.replace_all(&out, |c: &regex::Captures| format!("{}{}", &c[1], c[2].replace('\\', "/"))).into_owned());
            }
        }
        if let Some(re) = &self.user
            && re.is_match(&out)
        {
            out = Cow::Owned(re.replace_all(&out, "<USER>").into_owned());
        }
        out.into_owned()
    }
}

/// Counts of what the clean-up changed, recorded on each event.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub redactions: usize,
    pub trimmed: usize,
}

/// Applies redaction, placeholders and trimming to strings and JSON.
pub struct Cleaner {
    redactor: Redactor,
    pub placeholders: Placeholders,
}

impl Cleaner {
    pub fn new(placeholders: Placeholders) -> Self {
        Self {
            redactor: Redactor::new(),
            placeholders,
        }
    }

    pub fn string(&self, s: &str, stats: &mut Stats) -> String {
        let (redacted, n) = self.redactor.redact(s);
        stats.redactions += n;
        let placed = self.placeholders.apply(&redacted);
        match trim(&placed) {
            Some(t) => {
                stats.trimmed += 1;
                t
            }
            None => placed,
        }
    }

    /// Clean every string value (and object key) in a JSON value.
    pub fn value(&self, v: Value, stats: &mut Stats) -> Value {
        match v {
            Value::String(s) => Value::String(self.string(&s, stats)),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|i| self.value(i, stats)).collect())
            }
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (self.placeholders.apply(&k), self.value(v, stats)))
                    .collect(),
            ),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn short_text_is_not_trimmed() {
        assert_eq!(trim("hello"), None);
        assert_eq!(trim(&"a".repeat(TRIM_LIMIT)), None);
    }

    #[test]
    fn long_text_keeps_head_tail_and_hash() {
        let s = format!("{}{}{}", "H".repeat(2000), "M".repeat(5000), "T".repeat(2000));
        let t = trim(&s).unwrap();
        assert!(t.starts_with(&"H".repeat(TRIM_KEEP)));
        assert!(t.ends_with(&"T".repeat(TRIM_KEEP)));
        assert!(t.contains(&format!("bytes={}", s.len())));
        assert!(t.contains(&sha256_hex(s.as_bytes())));
        assert!(!t.contains('M'));
        assert_eq!(trim(&s), Some(t), "trimming must be deterministic");
    }

    #[test]
    fn trimming_respects_utf8_boundaries() {
        let s = "é".repeat(5000); // 2 bytes each
        let t = trim(&s).unwrap();
        assert!(t.starts_with('é'));
        assert!(t.ends_with('é'));
    }

    #[test]
    fn placeholders_replace_project_home_and_user() {
        let p = Placeholders::new(
            &[Path::new("/Users/alice/Code/app")],
            Some(Path::new("/Users/alice")),
            Some("alice"),
        );
        assert_eq!(
            p.apply("/Users/alice/Code/app/src/main.ts and /Users/alice/.ssh and alice"),
            "<PROJECT>/src/main.ts and <HOME>/.ssh and <USER>"
        );
        // Whole words only.
        assert_eq!(p.apply("malice"), "malice");
    }

    #[test]
    fn root_and_short_values_are_never_placeholders() {
        let p = Placeholders::new(&[Path::new("/")], Some(Path::new("/")), Some("ab"));
        assert_eq!(p.apply("/etc/hosts ab"), "/etc/hosts ab");
    }

    #[test]
    fn cleaner_redacts_before_trimming() {
        // AWS's documented example key, split so secret scanners skip it.
        let secret = concat!("AKIA", "IOSFODNN7EXAMPLE");
        // Key at the start, then enough text to force trimming.
        let s = format!("{secret} {}", "x".repeat(10_000));
        let mut stats = Stats::default();
        let out = Cleaner::new(Placeholders::none()).string(&s, &mut stats);
        assert!(!out.contains(secret));
        assert_eq!(stats, Stats { redactions: 1, trimmed: 1 });
    }

    #[test]
    fn cleaner_walks_json() {
        let p = Placeholders::new(&[Path::new("/work/app")], None, None);
        let c = Cleaner::new(p);
        let mut stats = Stats::default();
        let v = c.value(
            json!({"file_path": "/work/app/a.ts", "n": 3, "list": [concat!("sk-ant-", "api03-FAKE-test-key-not-real")]}),
            &mut stats,
        );
        assert_eq!(
            v,
            json!({"file_path": "<PROJECT>/a.ts", "n": 3, "list": ["[REDACTED:anthropic-key]"]})
        );
        assert_eq!(stats.redactions, 1);
    }
}
