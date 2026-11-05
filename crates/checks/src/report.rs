//! Findings and reports, with a canonical (deterministic) JSON form.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// Stable id from [`crate::catalog::CHECKS`].
    pub check: String,
    /// Project-relative file the finding is about, if any.
    pub file: Option<String>,
    /// Event sequence number the finding is about, if any.
    pub seq: Option<u64>,
    pub message: String,
}

impl Finding {
    pub fn to_json(&self) -> Value {
        json!({ "check": self.check, "file": self.file, "seq": self.seq, "message": self.message })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// What was asked, and whether the code shows it was done (Phase 6).
    pub intent: Vec<Value>,
}

impl Report {
    /// Canonical JSON: findings sorted, keys sorted. Used for the
    /// determinism check, so it must not contain timings or paths.
    pub fn canonical_json(&self) -> String {
        let mut findings = self.findings.clone();
        findings.sort();
        let v = json!({ "findings": findings.iter().map(Finding::to_json).collect::<Vec<_>>(), "intent": self.intent });
        serde_json::to_string(&v).expect("serialisable")
    }
}
