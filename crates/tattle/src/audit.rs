//! `tattle audit`: check the Claude Code sessions already on this computer.
//!
//! Reads Claude Code's own session logs (read-only) and reports how often
//! "the tests pass" was backed by a test run, and which requests are worth
//! a look. Nothing is written or sent anywhere. The checks are in
//! `checks::transcript`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use checks::plain;
use checks::transcript::{self, Claim};
use serde_json::{Value, json};

/// Logs larger than this are skipped (and counted): a check that takes a
/// minute on one file isn't worth it.
const MAX_LOG_BYTES: u64 = 50 * 1024 * 1024;

pub struct Options {
    /// Folder of session logs (default: Claude Code's own).
    pub dir: Option<PathBuf>,
    /// Only sessions changed in the last N days.
    pub days: Option<u64>,
    /// Only sessions whose folder contains this text.
    pub project: Option<String>,
    pub json: bool,
}

/// One flagged request.
struct Flag {
    date: String,
    project: String,
    session: String,
    request: usize,
    prompt: String,
    rule: String,
    text: String,
}

#[derive(Default)]
struct Totals {
    sessions: usize,
    requests: usize,
    skipped_large: usize,
    backed: usize,
    edited_after_run: usize,
    no_run: usize,
    contradicted: usize,
    other_command: usize,
    unknown: usize,
}

impl Totals {
    fn claims(&self) -> usize {
        self.backed + self.edited_after_run + self.no_run + self.contradicted + self.other_command + self.unknown
    }
}

/// Where Claude Code keeps its session logs.
fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("projects"));
    }
    Some(capture::fsx::home_dir()?.join(".claude").join("projects"))
}

/// Every `*.jsonl` directly inside `dir` or one folder down, sorted.
fn logs(dir: &Path) -> Vec<PathBuf> {
    let is_log = |p: &PathBuf| p.extension().is_some_and(|x| x == "jsonl") && p.is_file();
    let entries = |d: &Path| -> Vec<PathBuf> { fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default() };
    let mut out: Vec<PathBuf> = Vec::new();
    for p in entries(dir) {
        if is_log(&p) {
            out.push(p);
        } else if p.is_dir() {
            out.extend(entries(&p).into_iter().filter(is_log));
        }
    }
    out.sort();
    out
}

/// The log's lines; ones that don't parse are dropped (a line still being
/// written, or a format this version doesn't know).
fn read_lines(path: &Path) -> Result<Vec<Value>> {
    let text = fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    Ok(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
}

/// `/home/sam/code/shop` → `~/code/shop`.
fn short_path(path: &str) -> String {
    for home in capture::fsx::home_dirs() {
        let home = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(home.as_ref())
            && (rest.is_empty() || rest.starts_with(['/', '\\']))
        {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

fn short(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max { one_line } else { format!("{}…", one_line.chars().take(max).collect::<String>().trim_end()) }
}

/// The days to look back: `--days`, else the plugin's argument (what the
/// person typed after `/tattle:audit`; nothing means 30). `Err` is a plain
/// sentence for the person.
pub fn window_days(window: Option<&str>, days: Option<u64>) -> Result<Option<u64>, String> {
    match (days, window.map(str::trim)) {
        (Some(d), _) => Ok(Some(d)),
        (None, None) => Ok(None),
        (None, Some("")) => Ok(Some(30)),
        (None, Some(w)) => w.parse().map(Some).map_err(|_| format!("tattle audit takes a number of days, e.g. 30. It got \"{w}\".")),
    }
}

pub fn run(opts: &Options) -> Result<String> {
    let Some(dir) = opts.dir.clone().or_else(default_dir) else {
        return Ok("tattle couldn't find your home folder. Pass the folder of session logs with --dir.\n".to_string());
    };
    if !dir.is_dir() {
        return Ok(format!("No Claude Code session logs found in {}.\n", dir.display()));
    }
    let newest_allowed = opts.days.map(|d| SystemTime::now() - Duration::from_secs(d * 24 * 60 * 60));
    let own = crate::live::command_prefix();
    let mut totals = Totals::default();
    let mut flags: Vec<Flag> = Vec::new();
    for path in logs(&dir) {
        if let Some(text) = &opts.project
            && !path.to_string_lossy().contains(text.as_str())
        {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else { continue };
        if let (Some(limit), Ok(changed)) = (newest_allowed, meta.modified())
            && changed < limit
        {
            continue;
        }
        if meta.len() > MAX_LOG_BYTES {
            totals.skipped_large += 1;
            continue;
        }
        let Ok(lines) = read_lines(&path) else { continue };
        let session = transcript::session(&lines, |c| c.starts_with(own.as_str()));
        let requests = transcript::audit(&session);
        if requests.is_empty() {
            continue;
        }
        totals.sessions += 1;
        totals.requests += requests.len();
        let date = session.started.as_deref().and_then(|t| t.get(..10)).unwrap_or("").to_string();
        let project = session.cwd.as_deref().map(short_path).unwrap_or_default();
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        for r in &requests {
            match r.claim {
                Some(Claim::Backed) => totals.backed += 1,
                Some(Claim::EditedAfterRun) => totals.edited_after_run += 1,
                Some(Claim::NoRun) => totals.no_run += 1,
                Some(Claim::Contradicted) => totals.contradicted += 1,
                Some(Claim::OtherCommand) => totals.other_command += 1,
                Some(Claim::Unknown) => totals.unknown += 1,
                None => {}
            }
            for f in &r.findings {
                flags.push(Flag {
                    date: date.clone(),
                    project: project.clone(),
                    session: name.clone(),
                    request: r.number,
                    prompt: short(&r.prompt, 70),
                    rule: f.check.clone(),
                    text: plain::explain(f).text,
                });
            }
        }
    }
    // Newest first; the rest of the key makes the order fixed.
    flags.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| (&a.project, &a.session, a.request, &a.rule).cmp(&(&b.project, &b.session, b.request, &b.rule))));
    if opts.json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&to_json(&totals, &flags))?));
    }
    Ok(text_report(&totals, &flags, &dir, opts.days))
}

fn to_json(t: &Totals, flags: &[Flag]) -> Value {
    json!({
        "sessions": t.sessions,
        "requests": t.requests,
        "logs_skipped_too_large": t.skipped_large,
        "tests_pass_claims": {
            "total": t.claims(),
            "backed": t.backed,
            "edited_after_run": t.edited_after_run,
            "no_test_run": t.no_run,
            "last_run_failed": t.contradicted,
            "other_command_passed": t.other_command,
            "unknown": t.unknown,
        },
        "flagged": flags.iter().map(|f| json!({
            "date": f.date, "project": f.project, "session": f.session, "request": f.request,
            "prompt": f.prompt, "rule": f.rule, "text": f.text,
        })).collect::<Vec<_>>(),
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn text_report(t: &Totals, flags: &[Flag], dir: &Path, days: Option<u64>) -> String {
    let window = days.map(|d| format!(" from the last {}", plural(d as usize, "day", "days"))).unwrap_or_default();
    if t.sessions == 0 {
        return format!("tattle found no Claude Code sessions{window} in {}.\n", dir.display());
    }
    let mut out = format!("tattle audit: {}, {}{window}.\n\n", plural(t.sessions, "session", "sessions"), plural(t.requests, "request", "requests"));
    let claims = t.claims();
    if claims == 0 {
        out.push_str("Claude never said the tests pass in these sessions.\n");
    } else {
        out.push_str(&format!("Claude said the tests pass {}:\n", plural(claims, "time", "times")));
        let width = claims.to_string().len();
        let rows = [
            (t.backed, "a test run had passed, and nothing was edited after it"),
            (t.edited_after_run, "a test run had passed, but files were edited after it"),
            (t.no_run, "no test run in the session"),
            (t.contradicted, "the last test run had failed"),
            (t.other_command, "the project's test command had failed; only a different one passed"),
            (t.unknown, "the log can't settle it (a subagent worked after the last run, or the run's result isn't shown)"),
        ];
        for (n, what) in rows {
            if n > 0 {
                out.push_str(&format!("  {n:>width$}  {what}\n"));
            }
        }
    }
    if flags.is_empty() {
        out.push_str("\nNothing worth a look.\n");
    } else {
        out.push_str(&format!("\nWorth a look ({}):\n", flags.len()));
        for f in flags {
            let place = if f.project.is_empty() { String::new() } else { format!(", {}", f.project) };
            out.push_str(&format!("- {}{place}, request {} (\"{}\"):\n  {} (`{}`)\n", f.date, f.request, f.prompt, f.text, f.rule));
        }
    }
    if t.skipped_large > 0 {
        out.push_str(&format!("\n{} skipped (over 50 MB).\n", plural(t.skipped_large, "log was", "logs were")));
    }
    out.push_str(
        "\nThe audit reads what Claude said and the commands it ran, from Claude Code's own session logs.\n\
         Those logs don't hold your code, so the checks on code and on your requests (tests weakened,\n\
         packages added, files you said not to touch) only run on sessions the plugin recorded.\n\
         Nothing was written or sent anywhere.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_prompts_to_one_line() {
        assert_eq!(short("add a\n  discount", 70), "add a discount");
        assert_eq!(short("abcdef ghij", 6), "abcdef…");
    }

    #[test]
    fn the_plugins_argument_sets_the_window() {
        assert_eq!(window_days(None, None), Ok(None), "the command line with no window: every session");
        assert_eq!(window_days(Some(""), None), Ok(Some(30)), "/tattle:audit with nothing after it");
        assert_eq!(window_days(Some(" 90 "), None), Ok(Some(90)));
        assert_eq!(window_days(Some("7"), Some(2)), Ok(Some(2)), "--days wins");
        assert!(window_days(Some("last week"), None).unwrap_err().contains("number of days"));
    }

    #[test]
    fn counts_read_naturally() {
        assert_eq!(plural(1, "session", "sessions"), "1 session");
        assert_eq!(plural(3, "session", "sessions"), "3 sessions");
    }
}
