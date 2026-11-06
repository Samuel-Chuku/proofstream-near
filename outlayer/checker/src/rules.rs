//! The checker's decisions, kept free of I/O so each rule has a test.

use serde::{Deserialize, Serialize};

const GITHUB_DIR: &str = ".github/";
/// The `app.slug` GitHub reports on check runs created by GitHub Actions.
pub const ACTIONS_APP_SLUG: &str = "github-actions";
const MAX_OWNER_LEN: usize = 39;
const MAX_REPO_LEN: usize = 100;

#[derive(Deserialize, Debug, Clone)]
pub struct App {
    pub slug: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub completed_at: Option<String>,
    /// Null on check runs from a deleted app, so it never counts as Actions.
    pub app: Option<App>,
}

/// Any other app with Checks: write could post a check with a required name.
pub fn only_actions(runs: Vec<CheckRun>) -> Vec<CheckRun> {
    runs.into_iter()
        .filter(|r| r.app.as_ref().is_some_and(|a| a.slug == ACTIONS_APP_SLUG))
        .collect()
}

#[derive(Deserialize, Debug, Clone)]
pub struct ChangedFile {
    pub filename: String,
    pub previous_filename: Option<String>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Checks {
    Success,
    Failure,
    Pending,
    None,
    /// GitHub refused to show the check runs. The contract must treat this as not passed.
    Unavailable,
}

/// A contributor can edit CI, so any change under `.github/` voids "CI green".
pub fn touches_github(files: &[ChangedFile]) -> bool {
    files.iter().any(|f| {
        f.filename.starts_with(GITHUB_DIR)
            || f.previous_filename
                .as_deref()
                .is_some_and(|p| p.starts_with(GITHUB_DIR))
    })
}

fn latest_completed<'a>(
    runs: &'a [CheckRun],
    name: &str,
    cutoff: Option<&str>,
) -> Option<&'a CheckRun> {
    runs.iter()
        .filter(|r| r.name == name && r.status == "completed")
        .filter(|r| match (r.completed_at.as_deref(), cutoff) {
            // GitHub's timestamps are fixed-width UTC, so string order is time order.
            (Some(done), Some(limit)) => done <= limit,
            (Some(_), None) => true,
            (None, _) => false,
        })
        .max_by(|a, b| a.completed_at.cmp(&b.completed_at))
}

fn one_required(runs: &[CheckRun], name: &str, cutoff: Option<&str>) -> Checks {
    let named: Vec<&CheckRun> = runs.iter().filter(|r| r.name == name).collect();
    if named.is_empty() {
        return Checks::None;
    }
    match latest_completed(runs, name, cutoff) {
        Some(run) if run.conclusion.as_deref() == Some("success") => Checks::Success,
        Some(_) => Checks::Failure,
        None if named.iter().any(|r| r.status != "completed") => Checks::Pending,
        // Every run finished after the merge, so nothing was green at merge time.
        None => Checks::None,
    }
}

/// Every required check must have succeeded on the head SHA by `merged_at`.
pub fn head_conclusion(runs: &[CheckRun], required: &[String], merged_at: Option<&str>) -> Checks {
    if required.is_empty() {
        return Checks::None;
    }
    worst(required.iter().map(|name| one_required(runs, name, merged_at)))
}

/// The merge commit counts only for required checks that actually ran on it.
pub fn merge_conclusion(runs: &[CheckRun], required: &[String]) -> Checks {
    worst(
        required
            .iter()
            .filter(|name| runs.iter().any(|r| &r.name == *name))
            .map(|name| one_required(runs, name, None)),
    )
}

pub fn combine(head: Checks, merge: Checks) -> Checks {
    worst([head, merge].into_iter())
}

fn rank(c: Checks) -> u8 {
    match c {
        Checks::Unavailable => 4,
        Checks::Failure => 3,
        Checks::Pending => 2,
        Checks::None => 1,
        Checks::Success => 0,
    }
}

fn worst(items: impl Iterator<Item = Checks>) -> Checks {
    items.max_by_key(|c| rank(*c)).unwrap_or(Checks::Success)
}

/// Owner and repo go into URLs sent with our token, so only GitHub's own characters pass.
pub fn valid_slug(owner: &str, repo: &str) -> bool {
    let ok = |s: &str, max: usize| {
        !s.is_empty()
            && s.len() <= max
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    ok(owner, MAX_OWNER_LEN) && ok(repo, MAX_REPO_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MERGED_AT: &str = "2026-11-06T12:00:00Z";

    fn run(name: &str, status: &str, conclusion: Option<&str>, at: Option<&str>) -> CheckRun {
        CheckRun {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(Into::into),
            completed_at: at.map(Into::into),
            app: Some(App { slug: ACTIONS_APP_SLUG.into() }),
        }
    }

    fn file(name: &str, previous: Option<&str>) -> ChangedFile {
        ChangedFile {
            filename: name.into(),
            previous_filename: previous.map(Into::into),
        }
    }

    fn ci() -> Vec<String> {
        vec!["ci".into()]
    }

    #[test]
    fn success_before_merge_passes() {
        let runs = [run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::Success);
    }

    #[test]
    fn failure_is_refused() {
        let runs = [run("ci", "completed", Some("failure"), Some("2026-11-06T11:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::Failure);
    }

    #[test]
    fn rerun_after_merge_cannot_turn_a_failure_green() {
        let runs = [
            run("ci", "completed", Some("failure"), Some("2026-11-06T11:00:00Z")),
            run("ci", "completed", Some("success"), Some("2026-11-06T13:00:00Z")),
        ];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::Failure);
    }

    #[test]
    fn green_only_after_merge_is_not_green() {
        let runs = [run("ci", "completed", Some("success"), Some("2026-11-06T13:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::None);
    }

    #[test]
    fn rerun_before_merge_uses_the_latest_result() {
        let runs = [
            run("ci", "completed", Some("failure"), Some("2026-11-06T10:00:00Z")),
            run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z")),
        ];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::Success);
    }

    #[test]
    fn skipped_or_neutral_is_not_success() {
        let runs = [run("ci", "completed", Some("skipped"), Some("2026-11-06T11:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::Failure);
    }

    #[test]
    fn running_check_is_pending() {
        let runs = [run("ci", "in_progress", None, None)];
        assert_eq!(head_conclusion(&runs, &ci(), None), Checks::Pending);
    }

    #[test]
    fn a_different_check_name_does_not_count() {
        let runs = [run("lint", "completed", Some("success"), Some("2026-11-06T11:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &ci(), Some(MERGED_AT)), Checks::None);
    }

    #[test]
    fn every_required_check_must_pass() {
        let runs = [run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"))];
        let required = vec!["ci".to_string(), "lint".to_string()];
        assert_eq!(head_conclusion(&runs, &required, Some(MERGED_AT)), Checks::None);
    }

    #[test]
    fn no_required_checks_never_passes() {
        let runs = [run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"))];
        assert_eq!(head_conclusion(&runs, &[], Some(MERGED_AT)), Checks::None);
    }

    #[test]
    fn merge_commit_without_checks_is_ignored() {
        assert_eq!(merge_conclusion(&[], &ci()), Checks::Success);
    }

    #[test]
    fn failing_merge_commit_blocks_a_green_head() {
        let runs = [run("ci", "completed", Some("failure"), Some("2026-11-06T12:05:00Z"))];
        let merge = merge_conclusion(&runs, &ci());
        assert_eq!(combine(Checks::Success, merge), Checks::Failure);
    }

    #[test]
    fn unavailable_outranks_everything() {
        assert_eq!(combine(Checks::Success, Checks::Unavailable), Checks::Unavailable);
        assert_eq!(combine(Checks::Failure, Checks::Unavailable), Checks::Unavailable);
    }

    #[test]
    fn workflow_edit_is_detected() {
        assert!(touches_github(&[file(".github/workflows/ci.yml", None)]));
    }

    #[test]
    fn local_action_edit_is_detected() {
        assert!(touches_github(&[file(".github/actions/test/action.yml", None)]));
    }

    #[test]
    fn file_moved_out_of_github_is_detected() {
        assert!(touches_github(&[file("old/ci.yml", Some(".github/workflows/ci.yml"))]));
    }

    #[test]
    fn ordinary_files_are_fine() {
        assert!(!touches_github(&[file("src/main.rs", None), file("github/notes.md", None)]));
    }

    #[test]
    fn check_from_another_app_does_not_count() {
        let mut other = run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"));
        other.app = Some(App { slug: "someone-elses-app".into() });
        let kept = only_actions(vec![other]);
        assert_eq!(head_conclusion(&kept, &ci(), Some(MERGED_AT)), Checks::None);
    }

    #[test]
    fn check_with_no_app_does_not_count() {
        let mut orphan = run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"));
        orphan.app = None;
        assert!(only_actions(vec![orphan]).is_empty());
    }

    #[test]
    fn actions_check_is_kept() {
        let kept = only_actions(vec![run("ci", "completed", Some("success"), Some("2026-11-06T11:00:00Z"))]);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn slugs_reject_url_tricks() {
        assert!(valid_slug("Samuel-Chuku", "proofstream-test"));
        assert!(!valid_slug("a/b", "c"));
        assert!(!valid_slug("a", "c?x=1"));
        assert!(!valid_slug("a", ".."));
        assert!(!valid_slug("", "c"));
    }
}
