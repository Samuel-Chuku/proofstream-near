use super::*;

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const AUTHOR: u64 = 7;
const CREATED: u64 = 1_762_000_000; // 2025-11-01

fn merged() -> CheckerOutput {
    CheckerOutput {
        repo_id: 100,
        pr_number: 5,
        merged: true,
        base_sha: BASE.into(),
        head_sha: HEAD.into(),
        author_id: AUTHOR,
        merged_by: Some(Account { id: 9, kind: "User".into() }),
        independent_approval: false,
        base_branch: "main".into(),
        checks_conclusion: "success".into(),
        files_complete: true,
        touches_github: false,
        merged_at: Some("2026-11-06T12:00:00Z".into()),
        diff_bytes: 1000,
        diff_too_large: false,
        changed_files: 3,
    }
}

fn open() -> CheckerOutput {
    CheckerOutput { merged: false, merged_by: None, merged_at: None, ..merged() }
}

fn terms() -> Terms<'static> {
    Terms {
        pr_number: 5,
        repo_id: 100,
        base_branch: "main",
        payee_github_id: AUTHOR,
        created_at_seconds: CREATED,
        allow_self_merge: false,
    }
}

fn refused(change: impl FnOnce(&mut CheckerOutput), want: Refusal) {
    let mut o = merged();
    change(&mut o);
    assert_eq!(evaluate(&o, &terms()), Err(want));
}

#[test]
fn merged_by_another_human_passes() {
    let facts = evaluate(&merged(), &terms()).unwrap();
    assert!(facts.merged);
    assert_eq!(facts.head_sha, HEAD);
}

#[test]
fn wrong_repo_is_refused() {
    refused(|o| o.repo_id = 101, Refusal::WrongRepo);
}

#[test]
fn wrong_pr_is_refused() {
    refused(|o| o.pr_number = 6, Refusal::WrongPr);
}

#[test]
fn malformed_sha_is_refused() {
    refused(|o| o.head_sha = "ABC".into(), Refusal::BadSha);
}

#[test]
fn wrong_base_branch_is_refused() {
    refused(|o| o.base_branch = "dev".into(), Refusal::WrongBaseBranch);
}

#[test]
fn wrong_author_is_refused() {
    refused(|o| o.author_id = 8, Refusal::WrongAuthor);
}

#[test]
fn every_conclusion_except_success_is_refused() {
    for c in ["failure", "pending", "none", "unavailable", "skipped", ""] {
        refused(|o| o.checks_conclusion = c.into(), Refusal::ChecksNotSuccessful);
    }
}

#[test]
fn incomplete_file_list_is_refused() {
    refused(|o| o.files_complete = false, Refusal::IncompleteFileList);
}

#[test]
fn pr_with_no_changed_files_is_refused() {
    refused(|o| o.changed_files = 0, Refusal::EmptyPr);
}

#[test]
fn changed_files_go_on_to_the_review() {
    assert_eq!(evaluate(&merged(), &terms()).unwrap().changed_files, 3);
}

#[test]
fn touching_github_is_refused() {
    refused(|o| o.touches_github = true, Refusal::TouchesGithub);
}

#[test]
fn merge_before_or_in_the_creation_second_is_refused() {
    refused(|o| o.merged_at = Some("2025-10-31T00:00:00Z".into()), Refusal::MergedBeforeMilestone);
    refused(|o| o.merged_at = Some("2025-11-01T12:26:40Z".into()), Refusal::MergedBeforeMilestone);
    refused(|o| o.merged_at = None, Refusal::MergedBeforeMilestone);
    refused(|o| o.merged_at = Some("2026-11-06 12:00:00".into()), Refusal::MergedBeforeMilestone);
}

#[test]
fn author_merge_without_approval_is_refused() {
    refused(|o| o.merged_by = Some(Account { id: AUTHOR, kind: "User".into() }), Refusal::NoIndependentApproval);
}

#[test]
fn bot_merge_without_approval_is_refused() {
    refused(|o| o.merged_by = Some(Account { id: 41898282, kind: "Bot".into() }), Refusal::NoIndependentApproval);
}

#[test]
fn unknown_merger_without_approval_is_refused() {
    refused(|o| o.merged_by = None, Refusal::NoIndependentApproval);
}

#[test]
fn bot_merge_with_independent_approval_passes() {
    let mut o = merged();
    o.merged_by = Some(Account { id: 41898282, kind: "Bot".into() });
    o.independent_approval = true;
    assert!(evaluate(&o, &terms()).is_ok());
}

#[test]
fn self_merge_passes_only_when_the_funder_opted_in() {
    let mut o = merged();
    o.merged_by = Some(Account { id: AUTHOR, kind: "User".into() });
    let mut t = terms();
    t.allow_self_merge = true;
    assert!(evaluate(&o, &t).is_ok());
}

#[test]
fn open_pr_passes_hard_facts_without_merge_facts() {
    let facts = evaluate(&open(), &terms()).unwrap();
    assert!(!facts.merged);
}

#[test]
fn open_pr_still_needs_every_other_fact() {
    let mut o = open();
    o.checks_conclusion = "pending".into();
    assert_eq!(evaluate(&o, &terms()), Err(Refusal::ChecksNotSuccessful));
    let mut o = open();
    o.author_id = 8;
    assert_eq!(evaluate(&o, &terms()), Err(Refusal::WrongAuthor));
}

fn facts(merged: bool, head: &str, bytes: u64) -> Facts {
    Facts { merged, base_sha: BASE.into(), head_sha: head.into(), diff_bytes: bytes, diff_too_large: false, changed_files: 3 }
}

#[test]
fn merged_at_the_approved_head_pays_without_review() {
    assert_eq!(after_check(&facts(true, HEAD, 10), Some(HEAD), 100), AfterCheck::PayNow);
}

#[test]
fn approved_and_still_open_stays_approved() {
    assert_eq!(after_check(&facts(false, HEAD, 10), Some(HEAD), 100), AfterCheck::StillOpen);
}

#[test]
fn merged_at_a_new_head_is_reviewed_again() {
    assert_eq!(after_check(&facts(true, BASE, 10), Some(HEAD), 100), AfterCheck::Review);
}

#[test]
fn diff_over_the_cap_is_too_large() {
    assert_eq!(after_check(&facts(true, HEAD, 101), None, 100), AfterCheck::TooLarge);
    let mut f = facts(false, HEAD, 0);
    f.diff_too_large = true;
    assert_eq!(after_check(&f, None, 100), AfterCheck::TooLarge);
}

#[test]
fn diff_at_the_cap_is_reviewed() {
    assert_eq!(after_check(&facts(false, HEAD, 100), None, 100), AfterCheck::Review);
}

fn ai(status: &str, role: &str, head: &str, verdict: Option<&str>) -> Vec<u8> {
    near_sdk::serde_json::json!({ "status": status, "role": role, "head_sha": head, "verdict": verdict })
        .to_string()
        .into_bytes()
}

#[test]
fn verified_meets_is_meets() {
    assert_eq!(read_ai(&ai("verified", "judge", HEAD, Some("meets")), HEAD, "judge"), AiAnswer::Meets);
}

#[test]
fn every_objection_is_an_objection() {
    for v in ["does_not_meet", "unclear"] {
        assert_eq!(read_ai(&ai("verified", "judge", HEAD, Some(v)), HEAD, "judge"), AiAnswer::Objects(v.into()));
    }
    for status in ["too_large", "incomplete_diff"] {
        assert_eq!(read_ai(&ai(status, "verifier", HEAD, None), HEAD, "verifier"), AiAnswer::Objects(status.into()));
    }
}

#[test]
fn a_verdict_for_another_commit_is_no_verdict() {
    assert_eq!(read_ai(&ai("verified", "judge", BASE, Some("meets")), HEAD, "judge"), AiAnswer::Failed);
}

#[test]
fn a_verdict_from_the_other_role_is_no_verdict() {
    // A verifier pin pointing at the judge's project would get GLM twice.
    assert_eq!(read_ai(&ai("verified", "judge", HEAD, Some("meets")), HEAD, "verifier"), AiAnswer::Failed);
    let no_role = br#"{"status":"verified","head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","verdict":"meets"}"#;
    assert_eq!(read_ai(no_role, HEAD, "judge"), AiAnswer::Failed);
}

#[test]
fn unverified_errors_and_garbage_are_no_verdict() {
    assert_eq!(read_ai(&ai("unverified", "judge", HEAD, Some("meets")), HEAD, "judge"), AiAnswer::Failed);
    assert_eq!(read_ai(&ai("verified", "judge", HEAD, Some("approve")), HEAD, "judge"), AiAnswer::Failed);
    // The model can't answer `incomplete_diff`; only the enclave's pre-check can.
    assert_eq!(read_ai(&ai("verified", "judge", HEAD, Some("incomplete_diff")), HEAD, "judge"), AiAnswer::Failed);
    assert_eq!(read_ai(br#"{"error":"signature_text_mismatch"}"#, HEAD, "judge"), AiAnswer::Failed);
    assert_eq!(read_ai(b"null", HEAD, "judge"), AiAnswer::Failed);
    assert_eq!(read_ai(b"", HEAD, "judge"), AiAnswer::Failed);
}

#[test]
fn both_must_meet() {
    assert_eq!(combine(&AiAnswer::Meets, &AiAnswer::Meets), ReviewOutcome::BothMeet);
    assert_eq!(combine(&AiAnswer::Meets, &AiAnswer::Failed), ReviewOutcome::Retry);
    assert_eq!(combine(&AiAnswer::Failed, &AiAnswer::Meets), ReviewOutcome::Retry);
    assert_eq!(combine(&AiAnswer::Failed, &AiAnswer::Failed), ReviewOutcome::Retry);
}

#[test]
fn any_objection_holds_even_beside_a_failure() {
    let no = AiAnswer::Objects("unclear".into());
    assert_eq!(combine(&no, &AiAnswer::Meets), ReviewOutcome::Held("judge:unclear".into()));
    assert_eq!(combine(&AiAnswer::Meets, &no), ReviewOutcome::Held("verifier:unclear".into()));
    assert_eq!(combine(&AiAnswer::Failed, &no), ReviewOutcome::Held("verifier:unclear".into()));
}

fn binder(owner: u64, contract: &str, milestone: u64, nonce: &str, account: &str) -> Vec<u8> {
    near_sdk::serde_json::json!({
        "owner_id": owner, "contract": contract, "milestone_id": milestone,
        "nonce": nonce, "account": account, "revision": "d".repeat(40),
    })
    .to_string()
    .into_bytes()
}

#[test]
fn binding_needs_the_github_id_contract_milestone_and_nonce() {
    let ok = evaluate_binding(&binder(AUTHOR, "c.testnet", 1, "n", "alice.testnet"), "c.testnet", 1, "n", AUTHOR);
    assert_eq!(ok.unwrap().account.as_str(), "alice.testnet");
    let call = |b: Vec<u8>| evaluate_binding(&b, "c.testnet", 1, "n", AUTHOR);
    assert_eq!(call(binder(8, "c.testnet", 1, "n", "a.testnet")), Err(Refusal::NotTheGithubIdHolder));
    assert_eq!(call(binder(AUTHOR, "x.testnet", 1, "n", "a.testnet")), Err(Refusal::WrongContract));
    assert_eq!(call(binder(AUTHOR, "c.testnet", 2, "n", "a.testnet")), Err(Refusal::WrongMilestone));
    assert_eq!(call(binder(AUTHOR, "c.testnet", 1, "m", "a.testnet")), Err(Refusal::WrongNonce));
    assert_eq!(call(binder(AUTHOR, "c.testnet", 1, "n", "c.testnet")), Err(Refusal::PayeeIsThisContract));
    assert_eq!(call(binder(AUTHOR, "c.testnet", 1, "n", "Not An Account")), Err(Refusal::UnreadableOutput));
    assert_eq!(call(br#"{"error":"gist_not_found"}"#.to_vec()), Err(Refusal::UnreadableOutput));
}

#[test]
fn binding_needs_a_gist_revision() {
    let mut v: near_sdk::serde_json::Value =
        near_sdk::serde_json::from_slice(&binder(AUTHOR, "c.testnet", 1, "n", "a.testnet")).unwrap();
    v["revision"] = "latest".into();
    let r = evaluate_binding(v.to_string().as_bytes(), "c.testnet", 1, "n", AUTHOR);
    assert_eq!(r, Err(Refusal::UnreadableOutput));
}

#[test]
fn probe_matches_the_account_kind() {
    assert_eq!(probe_for("alice.testnet"), Ok(Probe::Named));
    assert_eq!(probe_for(&"a".repeat(64)), Ok(Probe::Implicit));
    assert_eq!(probe_for(&format!("0x{}", "a".repeat(40))), Err(Refusal::UnsupportedAccount));
    assert_eq!(probe_for(&format!("0s{}", "a".repeat(40))), Err(Refusal::UnsupportedAccount));
}

#[test]
fn project_owner_is_the_account_before_the_slash() {
    assert_eq!(project_owner("proofstream-checker-owner.testnet/proofstream-judge"), Some("proofstream-checker-owner.testnet"));
    assert_eq!(project_owner("no-slash"), None);
    assert_eq!(project_owner("a.testnet/b/c"), None);
    assert_eq!(project_owner("/name"), None);
    assert_eq!(project_owner("Bad Owner/name"), None);
}

#[test]
fn utc_parsing_is_exact() {
    assert_eq!(parse_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_utc_seconds("2026-11-06T12:00:00Z"), Some(1_793_966_400));
    assert_eq!(parse_utc_seconds("2026-13-06T12:00:00Z"), None);
    assert_eq!(parse_utc_seconds("2026-11-06T12:00:00+01:00"), None);
}
