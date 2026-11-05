//! Secret detection and redaction.
//!
//! Used in two places:
//! - On capture: every string from a hook event or transcript is redacted
//!   before it is written to disk.
//! - On export: snapshot contents are scanned, and export refuses if a
//!   secret is found (unless explicitly allowed for synthetic test keys).
//!
//! Over-redacting a recording is cheap; leaking a secret is not. So the
//! generic rules lean towards redacting.

use regex::Regex;

/// A detected secret: which rule matched and the byte range of the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub start: usize,
    pub end: usize,
}

struct Rule {
    id: &'static str,
    re: Regex,
    /// Capture group holding the secret value (0 = whole match).
    group: usize,
    /// Minimum Shannon entropy (bits per char) for the value to count.
    /// Kept low (2.5) because human passwords are low-entropy.
    min_entropy: Option<f64>,
    /// Skip values that look like placeholders (`your-key-here`, `${VAR}`).
    skip_placeholders: bool,
}

fn rule(
    id: &'static str,
    pattern: &str,
    group: usize,
    min_entropy: Option<f64>,
    skip_placeholders: bool,
) -> Rule {
    Rule {
        id,
        re: Regex::new(pattern).expect("secret rule pattern must compile"),
        group,
        min_entropy,
        skip_placeholders,
    }
}

pub struct Redactor {
    rules: Vec<Rule>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    pub fn new() -> Self {
        let rules = vec![
            rule(
                "private-key",
                r"-----BEGIN[A-Z ]*PRIVATE KEY-----[\s\S]*?-----END[A-Z ]*PRIVATE KEY-----",
                0,
                None,
                false,
            ),
            // A key block cut off before its END line (e.g. trimmed output).
            rule(
                "private-key",
                r"-----BEGIN[A-Z ]*PRIVATE KEY-----[A-Za-z0-9+/=\s]{20,}",
                0,
                None,
                false,
            ),
            rule("aws-access-key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0, None, false),
            rule(
                "github-token",
                r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})",
                0,
                None,
                false,
            ),
            rule("anthropic-key", r"\bsk-ant-[A-Za-z0-9_\-]{20,}", 0, None, false),
            rule(
                "openai-key",
                r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_\-]{20,}",
                0,
                None,
                false,
            ),
            rule("stripe-key", r"\b[rs]k_(?:live|test)_[A-Za-z0-9]{16,}", 0, None, false),
            rule("slack-token", r"\bxox[abposr]-[A-Za-z0-9\-]{10,}", 0, None, false),
            rule("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}", 0, None, false),
            rule(
                "jwt",
                r"\beyJ[A-Za-z0-9_\-]{10,}\.eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
                0,
                None,
                false,
            ),
            // postgres://user:PASSWORD@host
            rule(
                "url-password",
                r#"\b[a-zA-Z][a-zA-Z0-9+.\-]*://[^\s:/@"']+:([^\s@/"']+)@"#,
                1,
                None,
                true,
            ),
            // api_key = "VALUE", "password": "VALUE", clientSecret: 'VALUE'
            rule(
                "assigned-secret",
                r#"(?i)[a-z0-9_\-]*(?:secret|token|passw(?:or)?d|pwd|api[_\-]?key|access[_\-]?key|private[_\-]?key|auth[_\-]?key)[a-z0-9_\-]*["']?\s*[:=]\s*["']([^"'\s]{8,})["']"#,
                1,
                Some(2.5),
                true,
            ),
            // .env style: API_KEY=VALUE (unquoted)
            rule(
                "env-secret",
                r"(?m)^[ \t]*(?:export[ \t]+)?[A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|API_KEY|APIKEY|ACCESS_KEY|PRIVATE_KEY|AUTH_KEY)[A-Z0-9_]*[ \t]*=[ \t]*([^\s\x22'#]{8,})",
                1,
                Some(2.5),
                true,
            ),
        ];
        Self { rules }
    }

    /// Find secrets in `s`. Results are sorted and never overlap.
    pub fn find(&self, s: &str) -> Vec<Finding> {
        let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
        for (index, rule) in self.rules.iter().enumerate() {
            for caps in rule.re.captures_iter(s) {
                let Some(m) = caps.get(rule.group) else {
                    continue;
                };
                let value = m.as_str();
                if rule.skip_placeholders && looks_like_placeholder(value) {
                    continue;
                }
                if let Some(min) = rule.min_entropy
                    && shannon_entropy(value) < min
                {
                    continue;
                }
                candidates.push((m.start(), m.end(), index));
            }
        }
        // Earliest first; for the same start, longest first; then rule order.
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then((b.1 - b.0).cmp(&(a.1 - a.0)))
                .then(a.2.cmp(&b.2))
        });
        let mut findings = Vec::new();
        let mut covered_to = 0;
        for (start, end, index) in candidates {
            if start < covered_to {
                continue;
            }
            findings.push(Finding {
                rule: self.rules[index].id,
                start,
                end,
            });
            covered_to = end;
        }
        findings
    }

    /// Replace each secret with `[REDACTED:<rule>]`. Returns the new string
    /// and how many secrets were replaced.
    pub fn redact(&self, s: &str) -> (String, usize) {
        let findings = self.find(s);
        if findings.is_empty() {
            return (s.to_string(), 0);
        }
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for f in &findings {
            out.push_str(&s[last..f.start]);
            out.push_str("[REDACTED:");
            out.push_str(f.rule);
            out.push(']');
            last = f.end;
        }
        out.push_str(&s[last..]);
        (out, findings.len())
    }
}

fn looks_like_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "process.env",
        "import.meta.env",
        "os.environ",
        "${",
        "{{",
        "<",
        ">",
        "xxxx",
        "****",
        "your",
        "example",
        "changeme",
        "placeholder",
        "redacted",
        "dummy",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => chars.all(|c| c == first),
        None => true,
    }
}

/// Shannon entropy in bits per character.
pub fn shannon_entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let mut total = 0usize;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|&n| {
            let p = n as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redacted(s: &str) -> String {
        Redactor::new().redact(s).0
    }

    // Test inputs shaped like real keys, so the rules can be tested. All fake:
    // FAKE in the value where the format allows, and split in the source so
    // secret scanners (GitHub's included) don't take them for real keys.
    const AWS: &str = concat!("AKIA", "IOSFODNN7EXAMPLE"); // AWS's documented example key
    const GITHUB: &str = concat!("ghp_", "FAKEtestTOKENnotREAL0000000000000000");
    const ANTHROPIC: &str = concat!("sk-ant-", "api03-FAKE-test-key-not-real");
    const OPENAI: &str = concat!("sk-", "proj-FAKE-test-key-not-real");
    const STRIPE: &str = concat!("sk_", "live_FAKEtestKEYnotREAL");
    const SLACK: &str = concat!("xoxb-", "FAKE-test-token");
    const GOOGLE: &str = concat!("AIza", "Sy-FAKE-test-key-not-real-000000000");
    // Header {"alg":"HS256"}, payload {"sub":"FAKE"}, no real signature.
    const JWT: &str = concat!("eyJ", "hbGciOiJIUzI1NiJ9.", "eyJ", "zdWIiOiJGQUtFIn0.", "FAKE-signature-not-real");
    const PEM_BEGIN: &str = concat!("-----BEGIN RSA ", "PRIVATE KEY-----");
    const PEM_END: &str = concat!("-----END RSA ", "PRIVATE KEY-----");

    fn rules_found(s: &str) -> Vec<&'static str> {
        Redactor::new().find(s).into_iter().map(|f| f.rule).collect()
    }

    #[test]
    fn known_token_formats_are_redacted() {
        let aws = format!("key {AWS} here");
        let cases = [
            (aws.as_str(), "aws-access-key"),
            (GITHUB, "github-token"),
            (ANTHROPIC, "anthropic-key"),
            (OPENAI, "openai-key"),
            (STRIPE, "stripe-key"),
            (SLACK, "slack-token"),
            (GOOGLE, "google-api-key"),
            (JWT, "jwt"),
        ];
        for (input, rule) in cases {
            assert_eq!(rules_found(input), vec![rule], "input: {input}");
            assert!(redacted(input).contains(&format!("[REDACTED:{rule}]")));
        }
    }

    #[test]
    fn anthropic_key_wins_over_openai_pattern() {
        assert_eq!(
            rules_found(ANTHROPIC),
            vec!["anthropic-key"]
        );
    }

    #[test]
    fn private_key_block_is_redacted_whole() {
        let s = format!("before\n{PEM_BEGIN}\nFAKE+TEST+KEY+NOT+REAL+0000000000\n{PEM_END}\nafter");
        let out = redacted(&s);
        assert_eq!(out, "before\n[REDACTED:private-key]\nafter");
    }

    #[test]
    fn truncated_private_key_is_redacted() {
        let s = format!("{}\nFAKE+TEST+KEY+NOT+REAL+00000000000000", concat!("-----BEGIN ", "PRIVATE KEY-----"));
        assert_eq!(redacted(&s), "[REDACTED:private-key]");
    }

    #[test]
    fn url_password_redacts_only_the_password() {
        let s = "DATABASE_URL=postgres://app:s3cr3tP4ss@localhost:5432/db";
        assert_eq!(
            redacted(s),
            "DATABASE_URL=postgres://app:[REDACTED:url-password]@localhost:5432/db"
        );
    }

    #[test]
    fn assigned_and_env_secrets_are_redacted() {
        assert_eq!(
            redacted(r#"const apiKey = "q8Xz2LmP9vR4tK7w";"#),
            r#"const apiKey = "[REDACTED:assigned-secret]";"#
        );
        assert_eq!(
            redacted(r#"{"password": "hunter2hunter2!"}"#),
            r#"{"password": "[REDACTED:assigned-secret]"}"#
        );
        assert_eq!(
            redacted("JWT_SECRET=a8f5f167f44f4964e6c998dee827110c\nPORT=3000"),
            "JWT_SECRET=[REDACTED:env-secret]\nPORT=3000"
        );
    }

    #[test]
    fn ordinary_code_is_left_alone() {
        let safe = [
            "commit 3f2a9c8e1b7d6f5a4c3b2a1908f7e6d5c4b3a291",
            "id: 123e4567-e89b-12d3-a456-426614174000",
            "const token = getToken();",
            r#"password: """#,
            r#"const apiKey = process.env.API_KEY;"#,
            r#"API_KEY="your-api-key-here""#,
            "API_KEY=${API_KEY}",
            r#"const secret = "aaaaaaaaaaaa";"#,
            "export function tokenize(input: string) {}",
        ];
        for s in safe {
            assert_eq!(rules_found(s), Vec::<&str>::new(), "false positive: {s}");
            assert_eq!(redacted(s), s);
        }
    }

    #[test]
    fn redaction_is_deterministic_and_counts() {
        let r = Redactor::new();
        let s = format!("a {AWS} b {GITHUB} c");
        let (out1, n1) = r.redact(&s);
        let (out2, n2) = r.redact(&s);
        assert_eq!(out1, out2);
        assert_eq!(n1, 2);
        assert_eq!(n2, 2);
    }

    #[test]
    fn entropy_values() {
        assert_eq!(shannon_entropy(""), 0.0);
        assert_eq!(shannon_entropy("aaaa"), 0.0);
        assert!((shannon_entropy("abcd") - 2.0).abs() < 1e-9);
    }
}
