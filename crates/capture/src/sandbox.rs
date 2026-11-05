//! `tattle sandbox init`: prepare a fresh folder for recording one test session.
//!
//! Test apps are built outside this repo so the coding agent can't read the
//! trap descriptions in docs/ (see docs/DECISIONS.md). The folder gets
//! `.claude/settings.local.json` with hooks that call `tattle hook`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Hook events recorded. Tool events match every tool.
///
/// `PostToolUseFailure` matters: a command that exits non-zero (e.g. a
/// failing test run) fires it *instead of* `PostToolUse`.
pub const HOOK_EVENTS: &[(&str, bool)] = &[
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("PreToolUse", true),
    ("PostToolUse", true),
    ("PostToolUseFailure", true),
    ("PermissionDenied", true),
    ("SubagentStart", false),
    ("SubagentStop", false),
    ("PreCompact", false),
    ("Stop", false),
    ("StopFailure", false),
    ("SessionEnd", false),
];

/// Folders not copied from a base app.
const SKIP_FROM_BASE: &[&str] = &[".checker", "node_modules", ".claude"];

/// Quote a path for a POSIX shell command line.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The project's own Claude Code settings already run `tattle hook` (a research
/// sandbox from `tattle sandbox init`, or this repo's development hooks).
pub fn has_recorder_hooks(project: &Path) -> bool {
    [".claude/settings.json", ".claude/settings.local.json"].iter().any(|f| {
        fs::read_to_string(project.join(f))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .is_some_and(|v| hook_commands(&v["hooks"]).iter().any(|c| runs_cv_hook(c)))
    })
}

fn hook_commands(hooks: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for entries in hooks.as_object().into_iter().flat_map(|m| m.values()) {
        for entry in entries.as_array().into_iter().flatten() {
            for h in entry["hooks"].as_array().into_iter().flatten() {
                if let Some(c) = h["command"].as_str() {
                    out.push(c.to_string());
                }
            }
        }
    }
    out
}

/// `… tattle hook` (or the old `… tattle hook`), with the binary path quoted
/// or not.
fn runs_cv_hook(command: &str) -> bool {
    let words: Vec<String> = command.split_whitespace().map(|w| w.replace(['\'', '"'], "")).collect();
    let is_binary = |w: &str| {
        let name = w.rsplit(['/', '\\']).next().unwrap_or(w);
        let name = name.strip_suffix(".exe").unwrap_or(name);
        name == "tattle" || name == "cv"
    };
    words.windows(2).any(|w| w[1] == "hook" && is_binary(&w[0]))
}

/// The Claude Code settings that wire up the recorder.
pub fn hook_settings(cv_exe: &Path, manifest: bool) -> Value {
    let env = if manifest { "TATTLE_MANIFEST=1 " } else { "" };
    // Claude Code runs hook commands through a shell (Git Bash on Windows),
    // which takes `C:/…` more reliably than `C:\…`.
    let exe = cv_exe.to_string_lossy();
    let exe = if cfg!(windows) { exe.replace('\\', "/") } else { exe.into_owned() };
    let command = format!("{env}{} hook", shell_quote(&exe));
    let mut hooks = serde_json::Map::new();
    for (event, is_tool_event) in HOOK_EVENTS {
        let mut entry = json!({
            "hooks": [{ "type": "command", "command": command, "timeout": 30 }]
        });
        if *is_tool_event {
            entry["matcher"] = json!("*");
        }
        hooks.insert((*event).to_string(), json!([entry]));
    }
    json!({ "hooks": hooks })
}

pub fn init(dir: &Path, base: Option<&Path>, force: bool, cv_exe: &Path, manifest: bool) -> Result<PathBuf> {
    if dir.exists() {
        let has_entries = fs::read_dir(dir)?.next().is_some();
        if has_entries && !force {
            bail!(
                "{} is not empty. Each run needs a fresh folder (use --force to reuse it).",
                dir.display()
            );
        }
    }
    fs::create_dir_all(dir)?;
    let dir = crate::fsx::canonical(dir)?;

    if let Some(base) = base {
        let base = crate::fsx::canonical(base)
            .with_context(|| format!("base folder not found: {}", base.display()))?;
        if dir.starts_with(&base) {
            bail!("the sandbox folder can't be inside its base folder");
        }
        copy_dir(&base, &dir)?;
    }

    let claude = dir.join(".claude");
    fs::create_dir_all(&claude)?;
    let settings = serde_json::to_string_pretty(&hook_settings(cv_exe, manifest))? + "\n";
    fs::write(claude.join("settings.local.json"), settings)?;
    Ok(dir)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIP_FROM_BASE.iter().any(|s| name == *s) {
            continue;
        }
        let kind = entry.file_type()?;
        let target = to.join(&name);
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
        // Symlinks are skipped on purpose.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn finds_projects_that_record_themselves() {
        assert!(runs_cv_hook("'/Users/me/Code/cv-sandbox/_bin/abc/cv' hook"));
        assert!(runs_cv_hook("CV_MANIFEST=1 '/opt/cv/bin/cv' hook"), "sandboxes made before the rename");
        assert!(runs_cv_hook("TATTLE_MANIFEST=1 '/opt/tattle/bin/tattle' hook"));
        assert!(runs_cv_hook("TATTLE_SOURCE=project \"$CLAUDE_PROJECT_DIR/plugin/scripts/tattle\" hook"));
        assert!(runs_cv_hook("C:/Users/ann/tattle.exe hook"));
        assert!(!runs_cv_hook("cat notes-on-the-tattle-hook.md"));
        assert!(runs_cv_hook("CV_SOURCE=project \"$CLAUDE_PROJECT_DIR\"/plugin/scripts/cv hook"));
        assert!(!runs_cv_hook("npx prettier --write ."));
        assert!(!runs_cv_hook("./scripts/cvhook.sh"));
        let tmp = tempfile::tempdir().unwrap();
        assert!(!has_recorder_hooks(tmp.path()));
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let settings = hook_settings(Path::new("/opt/cv bin/cv"), false);
        fs::write(tmp.path().join(".claude/settings.local.json"), settings.to_string()).unwrap();
        assert!(has_recorder_hooks(tmp.path()));
    }

    #[test]
    fn settings_cover_every_event() {
        let s = hook_settings(Path::new("/opt/cv/bin/cv"), false);
        let m = hook_settings(Path::new("/opt/cv/bin/cv"), true);
        assert_eq!(m["hooks"]["Stop"][0]["hooks"][0]["command"], "TATTLE_MANIFEST=1 '/opt/cv/bin/cv' hook");
        for (event, is_tool) in HOOK_EVENTS {
            let entry = &s["hooks"][event][0];
            assert_eq!(entry["hooks"][0]["command"], "'/opt/cv/bin/cv' hook");
            assert_eq!(entry.get("matcher").is_some(), *is_tool, "{event}");
        }
    }
}
