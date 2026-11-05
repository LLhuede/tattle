//! The person's plugin settings: rules muted in a project, and whether new
//! alerts are also handed back to Claude.
//!
//! - Per project: `.checker/config.json`, written by `tattle config` (the
//!   plugin's `/tattle:mute`, `/tattle:unmute` and `/tattle:tell-claude`).
//! - For every project: environment variables, set e.g. in Claude Code's
//!   settings `env`: `TATTLE_MUTE=deps.added,code.stub` (added to the project's
//!   list) and `TATTLE_TELL_CLAUDE=1` or `0` (overrides the project's setting).
//!
//! Settings only change what the plugin shows and sends. `tattle check` and
//! `tattle score` ignore them, so findings stay the same for a given session.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use capture::store::{CHECKER_DIR, ensure_checker, write_private};
use serde_json::{Value, json};

pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Rule ids (finding ids) not shown in this project.
    pub mute: BTreeSet<String>,
    /// New alerts are also sent to Claude at the end of a turn.
    pub tell_claude: bool,
}

fn path(project: &Path) -> PathBuf {
    project.join(CHECKER_DIR).join(CONFIG_FILE)
}

impl Config {
    /// The project's saved settings only (no environment).
    pub fn load_saved(project: &Path) -> Self {
        let Some(v) = fs::read_to_string(path(project)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            return Self::default();
        };
        Self {
            mute: v["mute"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            tell_claude: v["tell_claude"].as_bool().unwrap_or(false),
        }
    }

    /// The settings in effect: the project's, plus the environment.
    pub fn load(project: &Path) -> Self {
        Self::from_parts(Self::load_saved(project), capture::settings::var("MUTE").as_deref(), capture::settings::var("TELL_CLAUDE").as_deref())
    }

    fn from_parts(mut saved: Self, mute_env: Option<&str>, tell_env: Option<&str>) -> Self {
        saved.mute.extend(mute_env.unwrap_or("").split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        match tell_env.map(str::trim) {
            Some("1" | "true" | "on" | "yes") => saved.tell_claude = true,
            Some("0" | "false" | "off" | "no") => saved.tell_claude = false,
            _ => {}
        }
        saved
    }

    pub fn save(&self, project: &Path) -> Result<()> {
        ensure_checker(project)?;
        let text = serde_json::to_string_pretty(&json!({ "mute": self.mute, "tell_claude": self.tell_claude }))?;
        write_private(&path(project), format!("{text}\n").as_bytes(), false)
    }

    pub fn is_muted(&self, rule: &str) -> bool {
        self.mute.contains(rule)
    }
}

/// Accept a rule id as typed: `deps.added`, `` `deps.added` ``, `(deps.added)`.
fn clean_rule(rule: &str) -> String {
    rule.trim().trim_matches(|c| c == '`' || c == '(' || c == ')' || c == '"' || c == '\'').to_string()
}

/// `tattle config …`: change and show the project's settings. Returns the text
/// for the person.
pub fn command(project: &Path, action: &str, value: Option<&str>) -> Result<String> {
    let project = capture::fsx::canonical(project).with_context(|| format!("folder not found: {}", project.display()))?;
    let mut saved = Config::load_saved(&project);
    let p = crate::live::command_prefix();
    let msg = match action {
        "mute" | "unmute" => {
            let Some(rule) = value.map(clean_rule).filter(|r| !r.is_empty()) else {
                bail!("which rule? Each item in {p}report shows its rule name, e.g. deps.added");
            };
            if !checks::catalog::is_known(&rule) {
                bail!("tattle has no rule called '{rule}'. Each item in {p}report shows its rule name, e.g. deps.added");
            }
            if action == "mute" {
                saved.mute.insert(rule.clone());
                saved.save(&project)?;
                format!("Muted {rule} in this project. tattle won't show it here any more. {p}unmute {rule} brings it back.\n")
            } else if saved.mute.remove(&rule) {
                saved.save(&project)?;
                format!("{rule} is shown again in this project.\n")
            } else if Config::load(&project).is_muted(&rule) {
                format!("{rule} isn't muted in this project; it's muted for every project by the TATTLE_MUTE setting.\n")
            } else {
                format!("{rule} wasn't muted.\n")
            }
        }
        "tell-claude" => {
            let on = match value.map(|v| v.trim().to_lowercase()).as_deref() {
                Some("on" | "yes" | "true" | "1") => true,
                Some("off" | "no" | "false" | "0") => false,
                None | Some("") => {
                    let now = Config::load(&project).tell_claude;
                    return Ok(format!("Sending alerts to Claude is {} in this project. {p}tell-claude on or off to change it.\n", if now { "on" } else { "off" }));
                }
                Some(other) => bail!("say on or off, not '{other}'"),
            };
            saved.tell_claude = on;
            saved.save(&project)?;
            let mut msg = if on {
                format!("From the next turn, tattle also sends new alerts to Claude, which then corrects itself or says why tattle is wrong. You still see every alert. {p}tell-claude off to stop.\n")
            } else {
                "tattle no longer sends alerts to Claude in this project. You still see them.\n".to_string()
            };
            if Config::load(&project).tell_claude != on {
                msg.push_str("Note: the TATTLE_TELL_CLAUDE setting overrides this for every project.\n");
            }
            msg
        }
        "show" => {
            let c = Config::load(&project);
            let muted = if c.mute.is_empty() { "none".to_string() } else { c.mute.iter().cloned().collect::<Vec<_>>().join(", ") };
            format!("Muted rules: {muted}\nSending alerts to Claude: {}\n", if c.tell_claude { "on" } else { "off" })
        }
        other => bail!("unknown setting '{other}': use mute, unmute, tell-claude or show"),
    };
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_settings_and_the_environment_combine() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Config::load_saved(tmp.path()), Config::default());
        let c = Config { mute: ["deps.added".to_string()].into(), tell_claude: true };
        c.save(tmp.path()).unwrap();
        assert_eq!(Config::load_saved(tmp.path()), c);
        let both = Config::from_parts(c.clone(), Some("code.stub, tests.skipped"), Some("0"));
        assert!(both.is_muted("deps.added") && both.is_muted("code.stub") && both.is_muted("tests.skipped"));
        assert!(!both.tell_claude, "TATTLE_TELL_CLAUDE=0 overrides the project");
        assert!(Config::from_parts(Config::default(), None, Some("on")).tell_claude);
        assert!(!Config::from_parts(c, None, Some("maybe")).mute.is_empty());
    }

    #[test]
    fn the_command_validates_rules_and_saves() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(command(tmp.path(), "mute", Some("no.such_rule")).is_err());
        assert!(command(tmp.path(), "mute", None).is_err());
        let msg = command(tmp.path(), "mute", Some("`deps.added`")).unwrap();
        assert!(msg.starts_with("Muted deps.added"), "{msg}");
        assert!(Config::load_saved(tmp.path()).is_muted("deps.added"));
        assert!(command(tmp.path(), "unmute", Some("deps.added")).unwrap().contains("shown again"));
        assert!(!Config::load_saved(tmp.path()).is_muted("deps.added"));
        assert!(command(tmp.path(), "tell-claude", Some("on")).unwrap().contains("also sends new alerts"));
        assert!(Config::load_saved(tmp.path()).tell_claude);
        assert!(command(tmp.path(), "tell-claude", Some("sometimes")).is_err());
        assert!(command(tmp.path(), "show", None).unwrap().contains("Sending alerts to Claude: on"));
    }
}
