//! What enclave output must say before state or money moves. No state, no I/O,
//! so every rule has a unit test.

use near_sdk::serde::Deserialize;
use near_sdk::AccountId;

const SHA256_HEX_LEN: usize = 64;
const GIT_SHA_LEN: usize = 40;
const IMPLICIT_ACCOUNT_LEN: usize = 64;
const ETH_IMPLICIT_LEN: usize = 42;
const SECONDS_PER_DAY: i64 = 86_400;
const HUMAN: &str = "User";
const MAX_PROJECT_ID_BYTES: usize = 128;
pub const FULL_PCT: u8 = 100;
/// D45: two progress numbers further apart than this are a disagreement, not a difference.
pub const MAX_PCT_GAP: u8 = 30;

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(crate = "near_sdk::serde")]
pub struct Account {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
}

/// The checker's output. Every field is required, so `{"error": ...}` fails to parse and refuses.
#[derive(Deserialize, Debug, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct CheckerOutput {
    pub repo_id: u64,
    pub pr_number: u64,
    pub merged: bool,
    pub base_sha: String,
    pub head_sha: String,
    pub author_id: u64,
    pub merged_by: Option<Account>,
    pub independent_approval: bool,
    pub base_branch: String,
    pub checks_conclusion: String,
    pub files_complete: bool,
    pub touches_github: bool,
    pub merged_at: Option<String>,
    pub diff_bytes: u64,
    pub diff_too_large: bool,
    /// GitHub's own count; the AIs check their diff lists exactly this many files (D31).
    pub changed_files: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    CheckerFailed,
    UnreadableOutput,
    WrongRepo,
    WrongPr,
    BadSha,
    NotMerged,
    WrongBaseBranch,
    WrongAuthor,
    NoIndependentApproval,
    ChecksNotSuccessful,
    IncompleteFileList,
    EmptyPr,
    TouchesGithub,
    MergedBeforeMilestone,
    NotTheGithubIdHolder,
    WrongContract,
    WrongMilestone,
    WrongNonce,
    PayeeIsThisContract,
    UnsupportedAccount,
    AccountMissing,
    StorageFailed,
    AlreadyBound,
    WrongState,
}

/// What the milestone requires of a PR, from stored state only (D15 rule 2).
pub struct Terms<'a> {
    pub pr_number: u64,
    pub repo_id: u64,
    pub base_branch: &'a str,
    pub payee_github_id: u64,
    pub created_at_seconds: u64,
    pub allow_self_merge: bool,
}

/// The facts that passed, for the next stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub merged: bool,
    pub base_sha: String,
    pub head_sha: String,
    pub diff_bytes: u64,
    pub diff_too_large: bool,
    pub changed_files: u64,
}

/// Every hard fact except the merge ones holds for open PRs too (D36). The merge
/// facts, D30 included, apply whenever the PR is merged.
pub fn evaluate(output: &CheckerOutput, terms: &Terms) -> Result<Facts, Refusal> {
    if output.repo_id != terms.repo_id {
        return Err(Refusal::WrongRepo);
    }
    if output.pr_number != terms.pr_number {
        return Err(Refusal::WrongPr);
    }
    if !is_git_sha(&output.base_sha) || !is_git_sha(&output.head_sha) {
        return Err(Refusal::BadSha);
    }
    if output.base_branch != terms.base_branch {
        return Err(Refusal::WrongBaseBranch);
    }
    if output.author_id != terms.payee_github_id {
        return Err(Refusal::WrongAuthor);
    }
    if output.checks_conclusion != "success" {
        return Err(Refusal::ChecksNotSuccessful);
    }
    if !output.files_complete {
        return Err(Refusal::IncompleteFileList);
    }
    if output.changed_files == 0 {
        return Err(Refusal::EmptyPr);
    }
    if output.touches_github {
        return Err(Refusal::TouchesGithub);
    }
    if output.merged {
        match output.merged_at.as_deref().and_then(parse_utc_seconds) {
            // Strictly after: a merge in the creation second can't be proven to come later.
            Some(at) if at > terms.created_at_seconds => {}
            _ => return Err(Refusal::MergedBeforeMilestone),
        }
        if !terms.allow_self_merge && !independently_merged(output) {
            return Err(Refusal::NoIndependentApproval);
        }
    }
    Ok(Facts {
        merged: output.merged,
        base_sha: output.base_sha.clone(),
        head_sha: output.head_sha.clone(),
        diff_bytes: output.diff_bytes,
        diff_too_large: output.diff_too_large,
        changed_files: output.changed_files,
    })
}

/// D30: a human other than the author merged it, or approved this exact head.
fn independently_merged(output: &CheckerOutput) -> bool {
    let human_merger = output
        .merged_by
        .as_ref()
        .is_some_and(|m| m.kind == HUMAN && m.id != output.author_id);
    human_merger || output.independent_approval
}

/// Where a claim goes once its hard facts pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterCheck {
    /// Merged at the head both AIs already approved (D36): pay, no review, no fee.
    PayNow,
    /// Still open while an approval stands: stay Approved.
    StillOpen,
    /// Over the diff cap: Held, no fee spent (D33).
    TooLarge,
    /// On to `review`.
    Review,
}

pub fn after_check(facts: &Facts, approved_head: Option<&str>, diff_cap_bytes: u64) -> AfterCheck {
    match approved_head {
        Some(head) if facts.merged && facts.head_sha == head => return AfterCheck::PayNow,
        Some(_) if !facts.merged => return AfterCheck::StillOpen,
        _ => {}
    }
    if facts.diff_too_large || facts.diff_bytes > diff_cap_bytes {
        AfterCheck::TooLarge
    } else {
        AfterCheck::Review
    }
}

/// One AI's answer, read from its enclave output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiAnswer {
    /// `meets` (100) or `partial` (1-99): the share of the whole milestone done (D45).
    Progress(u8),
    /// `does_not_meet`, `unclear` (D17), `incomplete_diff` (D31) or `too_large`.
    Objects(String),
    /// Error, timeout, bad signature, wrong commit or unreadable: no verdict.
    Failed,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AiOutput {
    status: String,
    role: String,
    head_sha: String,
    verdict: Option<String>,
    percent_done: Option<u8>,
}

/// D48: the verifier's verdict as the judge passed it on, signed in the verifier's enclave.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(crate = "near_sdk::serde")]
pub struct VerifierClaim {
    pub verdict: String,
    pub percent_done: u8,
    pub caller: String,
    pub signature: String,
    pub call_id: String,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct JudgeOutput {
    verifier: Option<VerifierClaim>,
}

/// Prefix of every message the verifier signs; the ai-review crate composes the same.
pub const VERDICT_PREFIX: &str = "proofstream-verdict:v1";

/// What the verifier signed for this review (D48). Built from the contract's own state:
/// only `verdict`, `percent` and `caller` come from the claim, and they are checked below.
pub fn verdict_message(contract: &str, milestone_id: u64, attempt: u32, head_sha: &str, verdict: &str, percent: u8, caller: &str) -> String {
    format!("{VERDICT_PREFIX}:{contract}:{milestone_id}:{attempt}:{head_sha}:{verdict}:{percent}:{caller}")
}

/// The verifier's claim, if the judge's output carries one. Absent, `null` or an error
/// block (the judge objected first, or couldn't reach the verifier) is no claim.
pub fn verifier_claim(judge_bytes: &[u8]) -> Option<VerifierClaim> {
    near_sdk::serde_json::from_slice::<JudgeOutput>(judge_bytes).ok()?.verifier
}

/// Which review a signed verdict must name.
pub struct ReviewRef<'a> {
    pub contract: &'a str,
    pub milestone_id: u64,
    pub attempt: u32,
    pub head_sha: &'a str,
    /// The judge's project owner: the account that pays for the verifier's run.
    pub judge_owner: &'a str,
    /// The verifier's in-enclave signing key, pinned in the milestone (hex, 32 bytes).
    pub verifier_key: &'a str,
}

/// D48: a verifier verdict counts only if its signature checks against the pinned key over
/// the message this review should produce, and the run was paid by the judge's owner.
/// Then it is read as `read_ai` reads a verdict: meets is 100, partial 1-99, the rest object.
pub fn read_verifier_claim(claim: Option<&VerifierClaim>, review: &ReviewRef) -> AiAnswer {
    let Some(c) = claim else { return AiAnswer::Failed };
    if c.caller != review.judge_owner {
        return AiAnswer::Failed;
    }
    let (Some(signature), Some(key)) = (hex_array::<64>(&c.signature), hex_array::<32>(review.verifier_key)) else {
        return AiAnswer::Failed;
    };
    let message = verdict_message(review.contract, review.milestone_id, review.attempt, review.head_sha, &c.verdict, c.percent_done, &c.caller);
    if !near_sdk::env::ed25519_verify(&signature, message.as_bytes(), &key) {
        return AiAnswer::Failed;
    }
    match (c.verdict.as_str(), c.percent_done) {
        ("meets", FULL_PCT) => AiAnswer::Progress(FULL_PCT),
        ("partial", p) if (1..FULL_PCT).contains(&p) => AiAnswer::Progress(p),
        (v @ ("does_not_meet" | "unclear" | "too_large" | "incomplete_diff"), _) => AiAnswer::Objects(v.into()),
        _ => AiAnswer::Failed,
    }
}

/// Lowercase hex of exactly `N` bytes.
pub fn hex_array<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// A verdict counts only if the enclave verified the model's signature, it names this
/// commit, and it came from the expected role. One build serves both roles, chosen by
/// the OutLayer project, so a pin naming the wrong project can't stand in for the other AI.
pub fn read_ai(bytes: &[u8], head_sha: &str, role: &str) -> AiAnswer {
    let Ok(out) = near_sdk::serde_json::from_slice::<AiOutput>(bytes) else {
        return AiAnswer::Failed;
    };
    if out.head_sha != head_sha || out.role != role {
        return AiAnswer::Failed;
    }
    match (out.status.as_str(), out.verdict.as_deref(), out.percent_done) {
        // A verdict and its percent must agree (the enclave checks this too).
        ("verified", Some("meets"), Some(FULL_PCT)) => AiAnswer::Progress(FULL_PCT),
        ("verified", Some("partial"), Some(p)) if (1..FULL_PCT).contains(&p) => AiAnswer::Progress(p),
        ("verified", Some(v @ ("does_not_meet" | "unclear")), _) => AiAnswer::Objects(v.into()),
        // Decided in the enclave before any model call, so there's no verdict field.
        ("too_large", _, _) => AiAnswer::Objects("too_large".into()),
        ("incomplete_diff", _, _) => AiAnswer::Objects("incomplete_diff".into()),
        _ => AiAnswer::Failed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    /// Both report progress: certify this total share of the milestone (the lower one).
    Certify(u8),
    /// Which AI objected and how, e.g. `judge:does_not_meet`.
    Held(String),
    /// Neither objected, but at least one gave no verdict: try again (attempt counted).
    Retry,
}

/// No AI verdict releases money alone: both must report progress, and the lower share
/// counts (D45). Any objection holds, even when the other AI failed, because the objection
/// is a real verdict. Two shares far apart hold, and so does one that adds nothing.
pub fn combine(judge: &AiAnswer, verifier: &AiAnswer, certified_pct: u8) -> ReviewOutcome {
    if let AiAnswer::Objects(v) = judge {
        return ReviewOutcome::Held(format!("judge:{v}"));
    }
    if let AiAnswer::Objects(v) = verifier {
        return ReviewOutcome::Held(format!("verifier:{v}"));
    }
    let (AiAnswer::Progress(j), AiAnswer::Progress(v)) = (judge, verifier) else {
        return ReviewOutcome::Retry;
    };
    if j.abs_diff(*v) > MAX_PCT_GAP {
        return ReviewOutcome::Held(format!("disagree:{j}/{v}"));
    }
    let lower = (*j).min(*v);
    if lower <= certified_pct {
        return ReviewOutcome::Held(format!("no_new_progress:{lower}"));
    }
    ReviewOutcome::Certify(lower)
}

/// D45: what to pay when the certified share rises to `new_pct`. Whatever stays in the
/// milestone is the budget's uncertified share, so rounding never pays more than certified,
/// and at 100 % exactly the rest goes.
pub fn share_to_pay(budget_total: u128, remaining: u128, new_pct: u8) -> Option<u128> {
    let stays = budget_total - budget_total.checked_mul(u128::from(new_pct))? / u128::from(FULL_PCT);
    remaining.checked_sub(stays).filter(|pay| *pay > 0)
}

/// The binder's output. `{"error": ...}` fails to parse and refuses.
#[derive(Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
struct BinderOutput {
    owner_id: u64,
    contract: String,
    milestone_id: u64,
    nonce: String,
    account: AccountId,
    revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub account: AccountId,
    pub revision: String,
}

/// The account a gist chose, only if its owner holds this milestone's GitHub id (by
/// numeric id) and it answers this contract's nonce for this milestone (D15 rule 7).
pub fn evaluate_binding(
    bytes: &[u8],
    contract: &str,
    milestone_id: u64,
    nonce: &str,
    payee_github_id: u64,
) -> Result<Binding, Refusal> {
    let out: BinderOutput = near_sdk::serde_json::from_slice(bytes).map_err(|_| Refusal::UnreadableOutput)?;
    if out.owner_id != payee_github_id {
        return Err(Refusal::NotTheGithubIdHolder);
    }
    if out.contract != contract {
        return Err(Refusal::WrongContract);
    }
    if out.milestone_id != milestone_id {
        return Err(Refusal::WrongMilestone);
    }
    if out.nonce != nonce {
        return Err(Refusal::WrongNonce);
    }
    if out.account.as_str() == contract {
        return Err(Refusal::PayeeIsThisContract);
    }
    if !is_git_sha(&out.revision) {
        return Err(Refusal::UnreadableOutput);
    }
    Ok(Binding { account: out.account, revision: out.revision })
}

/// D29: how to prove a payout account exists before binding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// A named account: 1 yoctoNEAR fails if it doesn't exist.
    Named,
    /// A 64-hex account: the transfer creates it.
    Implicit,
}

pub fn probe_for(account: &str) -> Result<Probe, Refusal> {
    if is_lower_hex(account, IMPLICIT_ACCOUNT_LEN) {
        return Ok(Probe::Implicit);
    }
    // `0x` (eth-implicit) and `0s` (deterministic) accounts: refused for now (D29).
    let eth = account.len() == ETH_IMPLICIT_LEN && account.starts_with("0x");
    if eth || account.starts_with("0s") {
        return Err(Refusal::UnsupportedAccount);
    }
    Ok(Probe::Named)
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

pub fn is_git_sha(s: &str) -> bool {
    is_lower_hex(s, GIT_SHA_LEN)
}

pub fn is_sha256_hex(s: &str) -> bool {
    is_lower_hex(s, SHA256_HEX_LEN)
}

/// `owner.testnet/name`: the owner is whose OutLayer secrets the run reads.
pub fn project_owner(project_id: &str) -> Option<&str> {
    if project_id.len() > MAX_PROJECT_ID_BYTES {
        return None;
    }
    let (owner, name) = project_id.split_once('/')?;
    let ok = !owner.is_empty() && !name.is_empty() && !name.contains('/');
    (ok && owner.parse::<AccountId>().is_ok()).then_some(owner)
}

/// D37: the two AIs share one build, so their independence is the project and the owner
/// (whose secrets row, API key and verifier fee are separate). Both must differ.
pub fn independent_ai_pins(judge_project: &str, verifier_project: &str) -> bool {
    match (project_owner(judge_project), project_owner(verifier_project)) {
        (Some(judge_owner), Some(verifier_owner)) => judge_project != verifier_project && judge_owner != verifier_owner,
        _ => false,
    }
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

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
