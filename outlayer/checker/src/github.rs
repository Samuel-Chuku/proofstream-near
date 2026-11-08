//! GitHub REST calls, and the mapping from their results to the checker's output.

use crate::rules::{self, Account, App, ChangedFile, CheckRun, Checks, CiSource, Output, Review};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use wasi_http_client::Client;

const API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const USER_AGENT: &str = "proofstream-checker";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PER_PAGE: usize = 100;
/// GitHub lists at most 3000 files for a PR.
const MAX_FILE_PAGES: usize = 30;
const MAX_CHECK_PAGES: usize = 10;
const MAX_REVIEW_PAGES: usize = 10;

#[derive(Deserialize)]
struct Input {
    owner: String,
    repo: String,
    pr_number: u64,
    required_checks: Vec<String>,
}

#[derive(Deserialize)]
struct Pull {
    merged: bool,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
    merged_by: Option<Account>,
    user: User,
    head: Head,
    base: Base,
    additions: u64,
    deletions: u64,
    changed_files: u64,
}

#[derive(Deserialize)]
struct User {
    login: String,
    id: u64,
}

#[derive(Deserialize)]
struct Head {
    sha: String,
}

#[derive(Deserialize)]
struct Base {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
    repo: Repo,
}

#[derive(Deserialize)]
struct Repo {
    id: u64,
    private: bool,
}

#[derive(Deserialize)]
struct WorkflowRunsPage {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
}

#[derive(Deserialize)]
struct JobsPage {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    name: String,
    status: String,
    conclusion: Option<String>,
    completed_at: Option<String>,
}

#[derive(Deserialize)]
struct CheckRunsPage {
    check_runs: Vec<CheckRun>,
}

fn get(token: &str, path: &str) -> Result<(u16, Vec<u8>), &'static str> {
    let response = Client::new()
        .get(&format!("{API}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("User-Agent", USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .send()
        .map_err(|_| "github_unreachable")?;
    let status = response.status();
    let body = response.body().map_err(|_| "github_bad_body")?;
    Ok((status, body))
}

fn get_ok(token: &str, path: &str) -> Result<Vec<u8>, &'static str> {
    match get(token, path)? {
        (200, body) => Ok(body),
        (401, _) => Err("github_bad_token"),
        (403, _) | (429, _) => Err("github_forbidden_or_rate_limited"),
        (404, _) => Err("github_not_found"),
        _ => Err("github_unexpected_status"),
    }
}

/// Reads every page of the PR's files: how many GitHub listed, and whether any
/// of them is under `.github/`. D31 compares the count with `changed_files`.
fn pr_files(token: &str, base: &str, pr: u64) -> Result<(u64, bool), &'static str> {
    let (mut listed, mut touches) = (0u64, false);
    for page in 1..=MAX_FILE_PAGES {
        let path = format!("{base}/pulls/{pr}/files?per_page={PER_PAGE}&page={page}");
        let files: Vec<ChangedFile> =
            serde_json::from_slice(&get_ok(token, &path)?).map_err(|_| "github_bad_json")?;
        listed += files.len() as u64;
        touches |= rules::touches_github(&files);
        if files.len() < PER_PAGE {
            break;
        }
    }
    Ok((listed, touches))
}

/// Every review on the PR, oldest first, as GitHub lists them.
fn reviews(token: &str, base: &str, pr: u64) -> Result<Vec<Review>, &'static str> {
    let mut all = Vec::new();
    for page in 1..=MAX_REVIEW_PAGES {
        let path = format!("{base}/pulls/{pr}/reviews?per_page={PER_PAGE}&page={page}");
        let batch: Vec<Review> =
            serde_json::from_slice(&get_ok(token, &path)?).map_err(|_| "github_bad_json")?;
        let count = batch.len();
        all.extend(batch);
        if count < PER_PAGE {
            return Ok(all);
        }
    }
    Err("too_many_reviews")
}

/// `Ok(None)` means GitHub refused this token. The caller reports `unavailable`.
fn check_runs(token: &str, base: &str, sha: &str) -> Result<Option<Vec<CheckRun>>, &'static str> {
    let mut runs = Vec::new();
    for page in 1..=MAX_CHECK_PAGES {
        // `filter=all` keeps re-runs, so a result from before the merge stays visible.
        let path = format!("{base}/commits/{sha}/check-runs?filter=all&per_page={PER_PAGE}&page={page}");
        let body = match get(token, &path)? {
            (200, body) => body,
            (403, _) => return Ok(None),
            (429, _) => return Err("github_forbidden_or_rate_limited"),
            _ => return Err("github_unexpected_status"),
        };
        let parsed: CheckRunsPage = serde_json::from_slice(&body).map_err(|_| "github_bad_json")?;
        let count = parsed.check_runs.len();
        runs.extend(parsed.check_runs);
        if count < PER_PAGE {
            return Ok(Some(rules::only_actions(runs)));
        }
    }
    Err("too_many_check_runs")
}

/// One GET that treats 403 as `None`, for the same reason as `check_runs`.
fn get_or_refused(token: &str, path: &str) -> Result<Option<Vec<u8>>, &'static str> {
    match get(token, path)? {
        (200, body) => Ok(Some(body)),
        (403, _) => Ok(None),
        (429, _) => Err("github_forbidden_or_rate_limited"),
        _ => Err("github_unexpected_status"),
    }
}

/// Every job, on every attempt, from the workflow runs on this SHA.
fn actions_jobs(token: &str, base: &str, sha: &str) -> Result<Option<Vec<CheckRun>>, &'static str> {
    let mut runs = Vec::new();
    for page in 1..=MAX_CHECK_PAGES {
        let path = format!("{base}/actions/runs?head_sha={sha}&per_page={PER_PAGE}&page={page}");
        let Some(body) = get_or_refused(token, &path)? else {
            return Ok(None);
        };
        let parsed: WorkflowRunsPage = serde_json::from_slice(&body).map_err(|_| "github_bad_json")?;
        let count = parsed.workflow_runs.len();
        for run in parsed.workflow_runs {
            if !add_jobs(token, base, run.id, &mut runs)? {
                return Ok(None);
            }
        }
        if count < PER_PAGE {
            return Ok(Some(runs));
        }
    }
    Err("too_many_check_runs")
}

/// `false` when GitHub refused this token.
fn add_jobs(token: &str, base: &str, run_id: u64, out: &mut Vec<CheckRun>) -> Result<bool, &'static str> {
    for page in 1..=MAX_CHECK_PAGES {
        // `filter=all` keeps earlier attempts, like `filter=all` on check runs.
        let path = format!("{base}/actions/runs/{run_id}/jobs?filter=all&per_page={PER_PAGE}&page={page}");
        let Some(body) = get_or_refused(token, &path)? else {
            return Ok(false);
        };
        let parsed: JobsPage = serde_json::from_slice(&body).map_err(|_| "github_bad_json")?;
        let count = parsed.jobs.len();
        out.extend(parsed.jobs.into_iter().map(|job| CheckRun {
            name: job.name,
            status: job.status,
            conclusion: job.conclusion,
            completed_at: job.completed_at,
            app: Some(App { slug: rules::ACTIONS_APP_SLUG.to_string() }),
        }));
        if count < PER_PAGE {
            return Ok(true);
        }
    }
    Err("too_many_check_runs")
}

pub fn run(stdin: &str) -> Result<Value, &'static str> {
    let input: Input = serde_json::from_str(stdin).map_err(|_| "bad_input")?;
    if !rules::valid_slug(&input.owner, &input.repo) {
        return Err("bad_input");
    }
    let token = std::env::var("GITHUB_TOKEN").map_err(|_| "missing_github_token")?;
    let base = format!("/repos/{}/{}", input.owner, input.repo);

    let pull: Pull = serde_json::from_slice(&get_ok(&token, &format!("{base}/pulls/{}", input.pr_number))?)
        .map_err(|_| "github_bad_json")?;
    let (listed, seen_touch) = pr_files(&token, &base, input.pr_number)?;
    let files_complete = rules::file_list_complete(listed, pull.changed_files);
    let independent_approval =
        rules::independent_approval(&reviews(&token, &base, input.pr_number)?, pull.user.id, &pull.head.sha);
    let private = pull.base.repo.private;
    let ci_source = if private { CiSource::ActionsJobs } else { CiSource::CheckRuns };
    let read_ci = |sha: &str| {
        if private {
            actions_jobs(&token, &base, sha)
        } else {
            check_runs(&token, &base, sha)
        }
    };

    let checks_conclusion = match read_ci(&pull.head.sha)? {
        None => Checks::Unavailable,
        Some(head_runs) => {
            let head = rules::head_conclusion(&head_runs, &input.required_checks, pull.merged_at.as_deref());
            let merge = match pull.merge_commit_sha.as_deref().filter(|_| pull.merged) {
                None => Checks::Success,
                Some(sha) => match read_ci(sha)? {
                    None => Checks::Unavailable,
                    Some(runs) => rules::merge_conclusion(&runs, &input.required_checks),
                },
            };
            rules::combine(head, merge)
        }
    };

    serde_json::to_value(Output {
        repo_id: pull.base.repo.id,
        pr_number: input.pr_number,
        merged: pull.merged,
        base_sha: pull.base.sha,
        head_sha: pull.head.sha,
        merge_commit_sha: pull.merge_commit_sha.filter(|_| pull.merged),
        author_login: pull.user.login,
        author_id: pull.user.id,
        merged_by: pull.merged_by,
        independent_approval,
        base_branch: pull.base.name,
        checks_conclusion,
        ci_source,
        additions: pull.additions,
        deletions: pull.deletions,
        changed_files: pull.changed_files,
        files_complete,
        touches_github: rules::reported_touches_github(seen_touch, files_complete),
        merged_at: pull.merged_at,
    })
    .map_err(|_| "encode_failed")
}
