//! Every finding the checker can report, with a stable id.
//!
//! Ids are part of the scoring contract: mutations and hand labels name
//! the finding they expect by id. Changing an id is a breaking change to
//! the test corpus, so add new ids rather than renaming.

pub struct CheckInfo {
    pub id: &'static str,
    /// Build phase that implements it.
    pub phase: &'static str,
    pub description: &'static str,
}

pub const CHECKS: &[CheckInfo] = &[
    CheckInfo { id: "capture.sequence_gap", phase: "2", description: "Events are missing from the log (sequence numbers skip)" },
    CheckInfo { id: "capture.incomplete", phase: "2", description: "The recorder reported a problem, or a turn has no closing snapshot" },
    CheckInfo { id: "events.changed_by_command", phase: "2", description: "A file changed through a shell command, not an edit tool" },
    CheckInfo { id: "events.unexplained_change", phase: "2", description: "A file changed with no recorded action that explains it" },
    CheckInfo { id: "tests.edited_after_failure", phase: "3", description: "A test file was edited between a failing and a passing test run" },
    CheckInfo { id: "claims.tests_pass_false", phase: "3", description: "The agent said tests pass, but the last recorded test run failed" },
    CheckInfo { id: "events.edited_without_reading", phase: "3", description: "An existing project file was edited without the agent reading it first" },
    CheckInfo { id: "tests.not_rerun_after_edit", phase: "3", description: "Code the tests import changed after the last test run, and the tests weren't run again" },
    CheckInfo { id: "commands.risky", phase: "3", description: "A dangerous shell command was run (force push, skipped hooks, deleting outside the project, …)" },
    CheckInfo { id: "events.edit_outside_project", phase: "3", description: "A file outside the project was changed" },
    CheckInfo { id: "tests.removed", phase: "4", description: "A test case was deleted" },
    CheckInfo { id: "tests.assertion_weakened", phase: "4", description: "An assertion was replaced by one that always passes" },
    CheckInfo { id: "tests.skipped", phase: "4", description: "A test was marked as skipped" },
    CheckInfo { id: "code.stub", phase: "4", description: "A function body was replaced by a stub (throw / TODO / fixed value)" },
    CheckInfo { id: "code.swallowed_error", phase: "4", description: "An error is caught and ignored" },
    CheckInfo { id: "deps.added", phase: "4", description: "A new dependency was added" },
    CheckInfo { id: "secrets.hardcoded", phase: "4", description: "A secret is written into the code" },
    CheckInfo { id: "scope.unrelated_file", phase: "4", description: "A file unrelated to the rest of the change was edited" },
    CheckInfo { id: "intent.deps_unchanged", phase: "6", description: "You said no new packages, and one was added" },
    CheckInfo { id: "intent.tests_pass", phase: "6", description: "You asked for passing tests, and the last test run failed" },
    CheckInfo { id: "intent.scope_only", phase: "6", description: "A file outside the scope you asked for was changed" },
    CheckInfo { id: "intent.target_unchanged", phase: "6", description: "Something you said not to touch was changed" },
    CheckInfo { id: "intent.rename_incomplete", phase: "6", description: "A rename you asked for wasn't finished" },
    CheckInfo { id: "intent.param_missing", phase: "6", description: "A specific value you asked for isn't in the changed code" },
    CheckInfo { id: "intent.route_missing", phase: "6", description: "An endpoint you asked for isn't in the code" },
    CheckInfo { id: "intent.target_not_changed", phase: "6", description: "An existing route you asked to change wasn't changed" },
    CheckInfo { id: "manifest.claim_unsupported", phase: "6", description: "The agent's manifest lists a file, name, route or test the code doesn't bear out" },
    CheckInfo { id: "manifest.undeclared_dependency", phase: "6", description: "A package was added but not declared in the manifest" },
    CheckInfo { id: "manifest.false_test_claim", phase: "6", description: "The manifest says tests passed; the last recorded run failed" },
    CheckInfo { id: "manifest.missing", phase: "6", description: "A manifest was requested but not written" },
    // Recall study (2026-10-26): hallucinations real agents made.
    CheckInfo { id: "claims.tests_pass_unverified", phase: "6", description: "The agent said tests pass, but no test run was recorded" },
    CheckInfo { id: "manifest.tests_unrun_passed", phase: "6", description: "The manifest says tests passed and also that they weren't run" },
    CheckInfo { id: "intent.phantom_reference", phase: "6", description: "You named something that didn't exist, and the agent worked on it without saying so" },
    CheckInfo { id: "intent.unrequested_change", phase: "6", description: "You said not to change anything, and files changed" },
    CheckInfo { id: "claims.fix_unreproduced", phase: "6", description: "The agent says it fixed a reported bug, but tests passed before its change and nothing ever showed the bug" },
    // Phase 4b: from an execution record written by `tattle verify --write`.
    CheckInfo { id: "exec.regression", phase: "4b", description: "Running the tests: more fail on the final code than at the start of the session" },
    CheckInfo { id: "exec.tests_fail", phase: "4b", description: "Running the tests on the final code: some fail (no baseline to compare)" },
    CheckInfo { id: "exec.build_fails", phase: "4b", description: "The build or type check fails on the final code but didn't at the start" },
    CheckInfo { id: "exec.app_does_not_start", phase: "4b", description: "The app crashes, never listens or answers 5xx when started on the final code" },
    CheckInfo { id: "exec.fix_not_demonstrated", phase: "4b", description: "The agent says it fixed a problem and names tests as proof, but those tests pass on the code from before the change too" },
    CheckInfo { id: "exec.tests_bent", phase: "4b", description: "On a fix request, tests that passed at the start fail against the final code, and the session edited them" },
    CheckInfo { id: "exec.claim_contradicted", phase: "4b", description: "The agent said the tests pass; running them says otherwise" },
    CheckInfo { id: "claims.flaky_unobserved", phase: "6", description: "The prompt says a test is flaky; the agent says it's fixed, but no run before its change ever failed" },
    CheckInfo { id: "claims.tests_pass_other_command", phase: "6", description: "The agent says tests pass, but the project's test command last failed; only a different command passed" },
];

pub fn is_known(id: &str) -> bool {
    CHECKS.iter().any(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for c in CHECKS {
            assert!(seen.insert(c.id), "duplicate id {}", c.id);
            let (area, name) = c.id.split_once('.').expect("area.name");
            assert!(!area.is_empty() && !name.is_empty());
            assert!(c.id.chars().all(|ch| ch.is_ascii_lowercase() || ch == '.' || ch == '_'));
        }
    }
}
