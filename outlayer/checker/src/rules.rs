//! The checker's decisions, kept free of I/O so each rule has a test.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const GITHUB_DIR: &str = ".github/";
/// The `app.slug` GitHub reports on check runs created by GitHub Actions.
pub const ACTIONS_APP_SLUG: &str = "github-actions";
const MAX_OWNER_LEN: usize = 39;
const MAX_REPO_LEN: usize = 100;
/// GitHub's PR files API returns at most this many files.
pub const MAX_PR_FILES: u64 = 3000;
/// The judge's own `MAX_DIFF_BYTES`. A diff past this can never be reviewed, so the
/// checker stops reading there.
const MAX_REVIEWABLE_DIFF_BYTES: u64 = 180_000;

/// True once a diff has grown past anything the AIs could review.
pub fn diff_over_limit(bytes_so_far: u64) -> bool {
    bytes_so_far > MAX_REVIEWABLE_DIFF_BYTES
}
/// GitHub's account `type` for a person. Bots and organizations never count as review.
const HUMAN: &str = "User";

/// A GitHub account as it appears on a PR or review.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Account {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Review {
    /// Null when the reviewer's account was deleted.
    pub user: Option<Account>,
    pub state: String,
    pub commit_id: Option<String>,
}

/// D30: a human other than the author approved this exact head commit, and hasn't
/// since withdrawn it. Only each reviewer's latest decisive review counts, so a later
/// "changes requested" or a dismissal cancels an approval; plain comments don't.
pub fn independent_approval(reviews: &[Review], author_id: u64, head_sha: &str) -> bool {
    let mut latest: HashMap<u64, &Review> = HashMap::new();
    for review in reviews {
        let Some(user) = review.user.as_ref() else { continue };
        if user.kind != HUMAN || user.id == author_id {
            continue;
        }
        if matches!(review.state.as_str(), "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED") {
            latest.insert(user.id, review);
        }
    }
    latest
        .values()
        .any(|r| r.state == "APPROVED" && r.commit_id.as_deref() == Some(head_sha))
}

/// D31: the listed files are the whole PR only if their count equals GitHub's own
/// `changed_files`, and the PR is within what the files API can list.
pub fn file_list_complete(listed: u64, changed_files: u64) -> bool {
    listed == changed_files && changed_files <= MAX_PR_FILES
}

/// An unseen file could be under `.github/`, so an incomplete list reports `true`.
pub fn reported_touches_github(seen_touch: bool, complete: bool) -> bool {
    seen_touch || !complete
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
// Built only by the WASI GitHub path, which host tests don't compile.
#[cfg_attr(not(target_os = "wasi"), allow(dead_code))]
pub enum CiSource {
    CheckRuns,
    /// Fine-grained tokens can't read check runs on private repos.
    /// This path sees GitHub Actions only, never third-party CI.
    ActionsJobs,
}

/// What the checker prints. OutLayer returns at most 900 bytes, which a test guards.
/// Owner and repo aren't echoed: the contract wrote the input and checks `repo_id`.
#[derive(Serialize)]
pub struct Output {
    pub repo_id: u64,
    pub pr_number: u64,
    pub merged: bool,
    pub base_sha: String,
    pub head_sha: String,
    pub merge_commit_sha: Option<String>,
    pub author_login: String,
    pub author_id: u64,
    pub merged_by: Option<Account>,
    pub independent_approval: bool,
    pub base_branch: String,
    pub checks_conclusion: Checks,
    pub ci_source: CiSource,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub files_complete: bool,
    pub touches_github: bool,
    pub merged_at: Option<String>,
    /// Bytes of the compare diff the AIs will read; `diff_too_large` when GitHub won't render it.
    pub diff_bytes: u64,
    pub diff_too_large: bool,
}

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

    const AUTHOR: u64 = 1;
    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn review(id: u64, kind: &str, state: &str, commit: &str) -> Review {
        Review {
            user: Some(Account { id, kind: kind.into() }),
            state: state.into(),
            commit_id: Some(commit.into()),
        }
    }

    #[test]
    fn approval_of_head_by_another_human_counts() {
        assert!(independent_approval(&[review(2, "User", "APPROVED", HEAD)], AUTHOR, HEAD));
    }

    #[test]
    fn approval_by_the_author_does_not_count() {
        assert!(!independent_approval(&[review(AUTHOR, "User", "APPROVED", HEAD)], AUTHOR, HEAD));
    }

    #[test]
    fn approval_by_a_bot_does_not_count() {
        assert!(!independent_approval(&[review(2, "Bot", "APPROVED", HEAD)], AUTHOR, HEAD));
    }

    #[test]
    fn approval_of_an_older_commit_does_not_count() {
        let old = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(!independent_approval(&[review(2, "User", "APPROVED", old)], AUTHOR, HEAD));
    }

    #[test]
    fn later_changes_requested_withdraws_an_approval() {
        let reviews = [review(2, "User", "APPROVED", HEAD), review(2, "User", "CHANGES_REQUESTED", HEAD)];
        assert!(!independent_approval(&reviews, AUTHOR, HEAD));
    }

    #[test]
    fn dismissed_approval_does_not_count() {
        assert!(!independent_approval(&[review(2, "User", "DISMISSED", HEAD)], AUTHOR, HEAD));
    }

    #[test]
    fn a_later_comment_keeps_an_approval() {
        let reviews = [review(2, "User", "APPROVED", HEAD), review(2, "User", "COMMENTED", HEAD)];
        assert!(independent_approval(&reviews, AUTHOR, HEAD));
    }

    #[test]
    fn approval_from_a_deleted_account_does_not_count() {
        let mut r = review(2, "User", "APPROVED", HEAD);
        r.user = None;
        assert!(!independent_approval(&[r], AUTHOR, HEAD));
    }

    #[test]
    fn no_reviews_means_no_approval() {
        assert!(!independent_approval(&[], AUTHOR, HEAD));
    }

    #[test]
    fn matching_file_count_is_complete() {
        assert!(file_list_complete(250, 250));
        assert!(file_list_complete(0, 0));
    }

    #[test]
    fn fewer_files_listed_than_changed_is_incomplete() {
        assert!(!file_list_complete(300, 301));
    }

    #[test]
    fn more_than_github_can_list_is_incomplete() {
        assert!(file_list_complete(MAX_PR_FILES, MAX_PR_FILES));
        assert!(!file_list_complete(MAX_PR_FILES, MAX_PR_FILES + 1));
    }

    #[test]
    fn incomplete_list_reports_touching_github() {
        assert!(reported_touches_github(false, false));
        assert!(!reported_touches_github(false, true));
        assert!(reported_touches_github(true, true));
    }

    #[test]
    fn diff_reading_stops_just_past_the_judges_limit() {
        assert!(!diff_over_limit(MAX_REVIEWABLE_DIFF_BYTES));
        assert!(diff_over_limit(MAX_REVIEWABLE_DIFF_BYTES + 1));
    }

    #[test]
    fn largest_output_fits_outlayers_limit() {
        let sha = "f".repeat(40);
        let out = Output {
            repo_id: u64::MAX,
            pr_number: u64::MAX,
            merged: true,
            base_sha: sha.clone(),
            head_sha: sha.clone(),
            merge_commit_sha: Some(sha),
            author_login: "a".repeat(MAX_OWNER_LEN),
            author_id: u64::MAX,
            merged_by: Some(Account { id: u64::MAX, kind: "Organization".into() }),
            independent_approval: false,
            base_branch: "b".repeat(100),
            checks_conclusion: Checks::Unavailable,
            ci_source: CiSource::ActionsJobs,
            additions: u64::MAX,
            deletions: u64::MAX,
            changed_files: u64::MAX,
            files_complete: false,
            touches_github: false,
            merged_at: Some("2026-11-06T12:00:00Z".into()),
            diff_bytes: u64::MAX,
            diff_too_large: false,
        };
        let len = serde_json::to_string(&out).unwrap().len();
        assert!(len <= 900, "output is {len} bytes");
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
