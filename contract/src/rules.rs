//! What the checker's output must say before money moves. No state, no I/O.

use near_sdk::serde::Deserialize;

const SHA256_HEX_LEN: usize = 64;
const GIT_SHA_LEN: usize = 40;
const SECONDS_PER_DAY: i64 = 86_400;

/// Every field is required, so an `{"error": ...}` output fails to parse and refuses.
#[derive(Deserialize, Debug, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct CheckerOutput {
    pub repo_id: u64,
    pub pr_number: u64,
    pub merged: bool,
    pub merged_at: Option<String>,
    pub author_id: u64,
    pub merged_by_id: Option<u64>,
    pub base_branch: String,
    pub checks_conclusion: String,
    pub touches_github: bool,
}

/// Output of the verifier. `status` must be `verified`, so errors and `too_large` give no verdict.
#[derive(Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
struct VerifierOutput {
    status: String,
    head_sha: String,
    verdict: String,
}

/// The verdict, only if the signature was verified and it binds to this commit.
pub fn read_verifier(bytes: &[u8], head_sha: &str) -> Option<String> {
    let output: VerifierOutput = near_sdk::serde_json::from_slice(bytes).ok()?;
    let known = matches!(output.verdict.as_str(), "meets" | "does_not_meet" | "unclear");
    (output.status == "verified" && output.head_sha == head_sha && known).then_some(output.verdict)
}

pub fn is_git_sha(s: &str) -> bool {
    s.len() == GIT_SHA_LEN && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    CheckerFailed,
    CheckerOutputTooLong,
    UnreadableOutput,
    WrongRepo,
    WrongPr,
    NotMerged,
    WrongBaseBranch,
    WrongAuthor,
    SelfMerged,
    ChecksNotSuccessful,
    TouchesGithub,
    MergedBeforeMilestone,
}

pub fn evaluate(
    output: &CheckerOutput,
    pr_number: u64,
    repo_id: u64,
    base_branch: &str,
    payee_github_id: u64,
    created_at_seconds: u64,
) -> Result<(), Refusal> {
    if output.repo_id != repo_id {
        return Err(Refusal::WrongRepo);
    }
    if output.pr_number != pr_number {
        return Err(Refusal::WrongPr);
    }
    if !output.merged {
        return Err(Refusal::NotMerged);
    }
    if output.base_branch != base_branch {
        return Err(Refusal::WrongBaseBranch);
    }
    if output.author_id != payee_github_id {
        return Err(Refusal::WrongAuthor);
    }
    match output.merged_by_id {
        Some(merger) if merger != output.author_id => {}
        _ => return Err(Refusal::SelfMerged),
    }
    if output.checks_conclusion != "success" {
        return Err(Refusal::ChecksNotSuccessful);
    }
    if output.touches_github {
        return Err(Refusal::TouchesGithub);
    }
    match output.merged_at.as_deref().and_then(parse_utc_seconds) {
        // Strictly after: a merge in the creation second can't be proven to come later.
        Some(merged) if merged > created_at_seconds => Ok(()),
        _ => Err(Refusal::MergedBeforeMilestone),
    }
}

pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == SHA256_HEX_LEN && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Parses GitHub's `YYYY-MM-DDTHH:MM:SSZ` only. Anything else is `None`, which refuses.
pub fn parse_utc_seconds(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    let shape_ok = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z';
    if !shape_ok {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let part = s.get(from..to)?;
        part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(days * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second).ok()
}

/// Days since 1970-01-01 (Howard Hinnant's civil calendar algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
