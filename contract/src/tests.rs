//! One test per rule, named after the rule. Refusals assert state and the PR lock.

use super::*;
use near_sdk::test_utils::{get_logs, VMContextBuilder};
use near_sdk::{testing_env, PromiseResult, RuntimeFeesConfig};

const VERSION: &str = "09bf18c97a79112222bac6309efa44c60b2a6a30c8a7df632d8f3eb6b67ab969";
const REPO_ID: u64 = 777;
const PAYEE_ID: u64 = 34985055;
const BOT_ID: u64 = 41898282;
const PR: u64 = 9;
const CREATED_S: u64 = 1_793_900_000; // 2026-11-05T...Z
const AFTER_CREATION: &str = "2026-11-06T12:00:00Z";
const BEFORE_CREATION: &str = "2020-01-01T00:00:00Z";
const BUDGET: u128 = 10_000_000;
const FEE: u128 = 500_000;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn context(predecessor: &str, deposit: NearToken) -> VMContextBuilder {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("contract.testnet"))
        .signer_account_id(acc(predecessor))
        .predecessor_account_id(acc(predecessor))
        .block_timestamp(CREATED_S * NANOS_PER_SECOND)
        .attached_deposit(deposit)
        .prepaid_gas(Gas::from_tgas(300));
    b
}

fn set(predecessor: &str, deposit: NearToken) {
    testing_env!(context(predecessor, deposit).build());
}

/// Runs the next call as the contract itself, with this promise result available.
fn set_callback(result: PromiseResult) {
    let c = context("contract.testnet", NearToken::from_near(0)).build();
    testing_env!(
        c,
        near_sdk::test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        vec![result]
    );
}

fn new_contract() -> Contract {
    set("deployer.testnet", NearToken::from_near(0));
    Contract::new(
        acc("token.testnet"),
        acc("outlayer.testnet"),
        "owner.testnet/checker".to_string(),
        VERSION.to_string(),
        acc("owner.testnet"),
        "default".to_string(),
    )
}

fn msg(budget: u128, fee: u128) -> String {
    json!({
        "repo_owner": "Samuel-Chuku", "repo": "proofstream-test", "repo_id": REPO_ID,
        "base_branch": "main", "payee_github_id": PAYEE_ID, "payee_account": "payee.testnet",
        "required_checks": ["ci"], "milestone_text": "do the thing",
        "budget": budget.to_string(), "fee": fee.to_string(),
    })
    .to_string()
}

fn funded(c: &mut Contract) -> u64 {
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE), msg(BUDGET, FEE));
    c.next_id - 1
}

fn claim(c: &mut Contract, id: u64, pr: u64) {
    set("anyone.testnet", MIN_CLAIM_DEPOSIT);
    let _ = c.claim(id, pr);
}

fn good_output() -> serde_json::Value {
    json!({
        "owner": "Samuel-Chuku", "repo": "proofstream-test", "repo_id": REPO_ID, "pr_number": PR,
        "merged": true, "merge_commit_sha": "abc", "head_sha": "def",
        "author_login": "Samuel-Chuku", "author_id": PAYEE_ID, "merged_by_id": BOT_ID,
        "base_branch": "main", "checks_conclusion": "success", "ci_source": "check_runs",
        "touches_github": false, "merged_at": AFTER_CREATION,
    })
}

fn deliver(c: &mut Contract, id: u64, pr: u64, result: PromiseResult) {
    set_callback(result);
    c.on_checked(id, pr, acc("anyone.testnet"), MIN_CLAIM_DEPOSIT);
}

fn deliver_json(c: &mut Contract, id: u64, pr: u64, output: serde_json::Value) {
    deliver(c, id, pr, PromiseResult::Successful(output.to_string().into_bytes()));
}

/// The refusal path: back to `Funded`, lock released, reason logged.
fn assert_refused(c: &Contract, id: u64, pr: u64, reason: &str) {
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.status, Status::Funded, "status after refusal");
    assert_eq!(m.active_pr, None);
    assert_eq!(c.get_pr_lock(REPO_ID, pr), None, "lock must be released");
    assert!(get_logs().iter().any(|l| l.contains(reason)), "expected {reason} in {:?}", get_logs());
}

fn refused_with(change: impl FnOnce(&mut serde_json::Value), reason: &str) {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    let mut output = good_output();
    change(&mut output);
    deliver_json(&mut c, id, PR, output);
    assert_refused(&c, id, PR, reason);
}

// Funding

#[test]
fn funding_creates_a_funded_milestone_with_fee_held() {
    let mut c = new_contract();
    let id = funded(&mut c);
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.status, Status::Funded);
    assert_eq!(m.budget.0, BUDGET);
    assert_eq!(m.fee_remaining.0, FEE);
    assert_eq!(m.funder, acc("funder.testnet"));
    assert_eq!(m.created_at_ns, CREATED_S * NANOS_PER_SECOND);
}

#[test]
#[should_panic(expected = "wrong token")]
fn funding_refuses_another_token() {
    let mut c = new_contract();
    set("fake-token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE), msg(BUDGET, FEE));
}

#[test]
#[should_panic(expected = "amount must equal budget plus fee")]
fn funding_refuses_underfunding() {
    let mut c = new_contract();
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET), msg(BUDGET, FEE));
}

#[test]
#[should_panic(expected = "amount must equal budget plus fee")]
fn funding_refuses_overfunding() {
    let mut c = new_contract();
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE + 1), msg(BUDGET, FEE));
}

#[test]
#[should_panic(expected = "budget and fee must be positive")]
fn funding_refuses_zero_budget() {
    let mut c = new_contract();
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(FEE), msg(0, FEE));
}

#[test]
#[should_panic(expected = "overflow")]
fn funding_refuses_overflow() {
    let mut c = new_contract();
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(u128::MAX), msg(u128::MAX, 1));
}

#[test]
fn max_paid_attempts_defaults_to_three() {
    let mut c = new_contract();
    let id = funded(&mut c);
    assert_eq!(c.get_milestone(id).unwrap().max_paid_attempts, 3);
}

#[test]
#[should_panic(expected = "max_paid_attempts out of range")]
fn max_paid_attempts_refuses_zero() {
    let mut c = new_contract();
    set("token.testnet", NearToken::from_near(0));
    let mut m: serde_json::Value = serde_json::from_str(&msg(BUDGET, FEE)).unwrap();
    m["max_paid_attempts"] = json!(0);
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE), m.to_string());
}

// Configuration

#[test]
#[should_panic(expected = "checker_version must be a wasm sha256")]
fn checker_must_be_pinned_to_a_hash() {
    set("deployer.testnet", NearToken::from_near(0));
    Contract::new(
        acc("token.testnet"),
        acc("outlayer.testnet"),
        "owner.testnet/checker".to_string(),
        "latest".to_string(),
        acc("owner.testnet"),
        "default".to_string(),
    );
}

// Claim start

#[test]
#[should_panic(expected = "attach at least 0.01 NEAR")]
fn claim_refuses_too_small_a_deposit() {
    let mut c = new_contract();
    let id = funded(&mut c);
    set("anyone.testnet", NearToken::from_millinear(9));
    let _ = c.claim(id, PR);
}

#[test]
#[should_panic(expected = "no such milestone")]
fn claim_refuses_unknown_milestone() {
    let mut c = new_contract();
    claim(&mut c, 99, PR);
}

#[test]
fn claim_locks_the_milestone_and_the_pr_at_start() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.status, Status::Checking);
    assert_eq!(m.active_pr, Some(PR));
    assert_eq!(c.get_pr_lock(REPO_ID, PR), Some(PrLock::Locked(id)));
}

#[test]
#[should_panic(expected = "milestone is not claimable")]
fn second_claim_refused_while_first_is_in_flight() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    claim(&mut c, id, PR);
}

#[test]
#[should_panic(expected = "PR is locked or already paid")]
fn same_pr_on_another_milestone_refused_while_in_flight() {
    let mut c = new_contract();
    let first = funded(&mut c);
    let second = funded(&mut c);
    claim(&mut c, first, PR);
    claim(&mut c, second, PR);
}

// Hard-fact refusals

#[test]
fn refuses_wrong_repo_id() {
    refused_with(|o| o["repo_id"] = json!(REPO_ID + 1), "WrongRepo");
}

#[test]
fn refuses_wrong_pr_number() {
    refused_with(|o| o["pr_number"] = json!(PR + 1), "WrongPr");
}

#[test]
fn refuses_unmerged_pr() {
    refused_with(|o| o["merged"] = json!(false), "NotMerged");
}

#[test]
fn refuses_wrong_base_branch() {
    refused_with(|o| o["base_branch"] = json!("develop"), "WrongBaseBranch");
}

#[test]
fn refuses_wrong_author() {
    refused_with(|o| o["author_id"] = json!(1), "WrongAuthor");
}

#[test]
fn refuses_self_merge() {
    refused_with(|o| o["merged_by_id"] = json!(PAYEE_ID), "SelfMerged");
}

#[test]
fn refuses_unknown_merger() {
    refused_with(|o| o["merged_by_id"] = serde_json::Value::Null, "SelfMerged");
}

#[test]
fn refuses_every_conclusion_except_success() {
    for conclusion in ["failure", "pending", "none", "unavailable", "neutral", "skipped", "SUCCESS", ""] {
        refused_with(|o| o["checks_conclusion"] = json!(conclusion), "ChecksNotSuccessful");
    }
}

#[test]
fn refuses_pr_touching_github_dir() {
    refused_with(|o| o["touches_github"] = json!(true), "TouchesGithub");
}

#[test]
fn refuses_pr_merged_before_the_milestone() {
    refused_with(|o| o["merged_at"] = json!(BEFORE_CREATION), "MergedBeforeMilestone");
}

#[test]
fn refuses_merge_in_the_creation_second() {
    let same_second = format_utc_for_test(CREATED_S);
    refused_with(|o| o["merged_at"] = json!(same_second), "MergedBeforeMilestone");
}

#[test]
fn refuses_missing_or_malformed_merge_time() {
    refused_with(|o| o["merged_at"] = serde_json::Value::Null, "MergedBeforeMilestone");
    refused_with(|o| o["merged_at"] = json!("yesterday"), "MergedBeforeMilestone");
    refused_with(|o| o["merged_at"] = json!("2026-13-01T00:00:00Z"), "MergedBeforeMilestone");
}

// Fail closed

#[test]
fn refuses_checker_error_output() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver_json(&mut c, id, PR, json!({ "error": "github_not_found" }));
    assert_refused(&c, id, PR, "UnreadableOutput");
}

#[test]
fn refuses_empty_output_as_after_a_yield_timeout() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver(&mut c, id, PR, PromiseResult::Successful(vec![]));
    assert_refused(&c, id, PR, "UnreadableOutput");
}

#[test]
fn refuses_null_output_as_measured_on_a_real_timeout() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver(&mut c, id, PR, PromiseResult::Successful(b"null".to_vec()));
    assert_refused(&c, id, PR, "UnreadableOutput");
}

#[test]
fn refuses_output_missing_a_field() {
    refused_with(|o| { o.as_object_mut().unwrap().remove("touches_github"); }, "UnreadableOutput");
}

#[test]
fn refuses_oversized_output() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver(&mut c, id, PR, PromiseResult::Successful(vec![b' '; MAX_CHECKER_OUTPUT_BYTES + 1]));
    assert_refused(&c, id, PR, "CheckerOutputTooLong");
}

#[test]
fn checker_failure_refuses_and_returns_the_deposit_to_the_caller() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver(&mut c, id, PR, PromiseResult::Failed);
    assert_refused(&c, id, PR, "CheckerFailed");
    let receipts = near_sdk::test_utils::get_created_receipts();
    assert!(
        receipts.iter().any(|r| r.receiver_id == acc("anyone.testnet")),
        "deposit must go back to the caller"
    );
}

#[test]
fn refused_pr_can_be_claimed_again() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    let mut pending = good_output();
    pending["checks_conclusion"] = json!("pending");
    deliver_json(&mut c, id, PR, pending);
    claim(&mut c, id, PR);
    assert_eq!(c.get_milestone(id).unwrap().status, Status::Checking);
}

// Callbacks

// `#[private]` exists only in the wasm entry point, so testnet attack transactions cover it.

#[test]
#[should_panic(expected = "callback does not match the claim in flight")]
fn stale_checker_result_cannot_touch_a_funded_milestone() {
    let mut c = new_contract();
    let id = funded(&mut c);
    deliver_json(&mut c, id, PR, good_output());
}

// Payout

fn paying(c: &mut Contract) -> u64 {
    let id = funded(c);
    claim(c, id, PR);
    deliver_json(c, id, PR, good_output());
    id
}

#[test]
fn passing_facts_start_a_payout_to_the_fixed_payee_only() {
    let mut c = new_contract();
    let id = paying(&mut c);
    assert_eq!(c.get_milestone(id).unwrap().status, Status::Paying);
    let receipts = near_sdk::test_utils::get_created_receipts();
    assert!(receipts.iter().any(|r| r.receiver_id == acc("token.testnet")));
    assert!(!receipts.iter().any(|r| r.receiver_id == acc("anyone.testnet")), "the caller gets nothing");
    let transfer = format!("{:?}", receipts.iter().find(|r| r.receiver_id == acc("token.testnet")).unwrap());
    let as_receipt_bytes = |s: &str| format!("{:?}", s.as_bytes()).trim_matches(['[', ']']).to_string();
    assert!(transfer.contains(&as_receipt_bytes("ft_transfer")), "{transfer}");
    assert!(transfer.contains(&as_receipt_bytes("payee.testnet")), "{transfer}");
}

#[test]
fn successful_payout_is_final_and_spends_the_pr() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(PromiseResult::Successful(vec![]));
    c.on_paid(id, PR);
    assert_eq!(c.get_milestone(id).unwrap().status, Status::Paid);
    assert_eq!(c.get_pr_lock(REPO_ID, PR), Some(PrLock::Spent(id)));
}

#[test]
#[should_panic(expected = "milestone is not claimable")]
fn paid_milestone_cannot_be_claimed_again() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(PromiseResult::Successful(vec![]));
    c.on_paid(id, PR);
    claim(&mut c, id, PR + 1);
}

#[test]
#[should_panic(expected = "PR is locked or already paid")]
fn paid_pr_cannot_pay_another_milestone() {
    let mut c = new_contract();
    let first = paying(&mut c);
    set_callback(PromiseResult::Successful(vec![]));
    c.on_paid(first, PR);
    let second = funded(&mut c);
    claim(&mut c, second, PR);
}

#[test]
fn failed_transfer_returns_to_funded_and_releases_the_pr() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(PromiseResult::Failed);
    c.on_paid(id, PR);
    assert_refused(&c, id, PR, "payout_failed");
}

#[test]
fn accounting_is_unchanged_by_a_refusal() {
    let mut c = new_contract();
    let id = funded(&mut c);
    claim(&mut c, id, PR);
    deliver_json(&mut c, id, PR, json!({ "error": "x" }));
    let m = c.get_milestone(id).unwrap();
    assert_eq!((m.budget.0, m.fee_remaining.0), (BUDGET, FEE));
}

// Verifier fee (Q6)

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn pay_verifier(c: &mut Contract, id: u64, caller: &str) {
    set(caller, MIN_VERIFIER_DEPOSIT);
    let _ = c.pay_verifier(id, SHA_A.to_string(), SHA_B.to_string());
}

#[test]
fn fee_is_spent_from_the_milestone_never_the_budget() {
    let mut c = new_contract();
    let id = funded(&mut c);
    pay_verifier(&mut c, id, "funder.testnet");
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.fee_remaining.0, FEE - VERIFIER_FEE);
    assert_eq!(m.budget.0, BUDGET);
    assert_eq!(m.status, Status::Funded);
}

#[test]
#[should_panic(expected = "only the funder can spend the fee")]
fn only_the_funder_can_spend_the_fee() {
    let mut c = new_contract();
    let id = funded(&mut c);
    pay_verifier(&mut c, id, "anyone.testnet");
}

#[test]
#[should_panic(expected = "attach at least 0.03 NEAR")]
fn verifier_needs_a_deposit_for_compute() {
    let mut c = new_contract();
    let id = funded(&mut c);
    set("funder.testnet", MIN_CLAIM_DEPOSIT);
    let _ = c.pay_verifier(id, SHA_A.to_string(), SHA_B.to_string());
}

#[test]
#[should_panic(expected = "bad sha")]
fn verifier_refuses_a_bad_sha() {
    let mut c = new_contract();
    let id = funded(&mut c);
    set("funder.testnet", MIN_VERIFIER_DEPOSIT);
    let _ = c.pay_verifier(id, "main".to_string(), SHA_B.to_string());
}

#[test]
#[should_panic(expected = "fee exhausted")]
fn fee_cannot_be_overspent() {
    let mut c = new_contract();
    let id = funded(&mut c);
    for _ in 0..=(FEE / VERIFIER_FEE) {
        pay_verifier(&mut c, id, "funder.testnet");
    }
}

fn fee_deposit_result(result: PromiseResult, id: u64, c: &mut Contract) {
    set_callback(result);
    c.on_fee_deposited(id, SHA_A.to_string(), SHA_B.to_string(), acc("funder.testnet"), MIN_VERIFIER_DEPOSIT);
}

#[test]
fn failed_fee_deposit_restores_the_fee_and_returns_the_near() {
    for result in [PromiseResult::Failed, PromiseResult::Successful(b"\"0\"".to_vec())] {
        let mut c = new_contract();
        let id = funded(&mut c);
        pay_verifier(&mut c, id, "funder.testnet");
        fee_deposit_result(result, id, &mut c);
        assert_eq!(c.get_milestone(id).unwrap().fee_remaining.0, FEE);
        let receipts = near_sdk::test_utils::get_created_receipts();
        assert!(receipts.iter().any(|r| r.receiver_id == acc("funder.testnet")));
        assert!(!receipts.iter().any(|r| r.receiver_id == acc("outlayer.testnet")), "no execution");
    }
}

#[test]
fn deposited_fee_calls_the_pinned_verifier_with_attached_usd() {
    let mut c = new_contract();
    let id = funded(&mut c);
    pay_verifier(&mut c, id, "funder.testnet");
    fee_deposit_result(PromiseResult::Successful(format!("\"{VERIFIER_FEE}\"").into_bytes()), id, &mut c);
    let receipts = near_sdk::test_utils::get_created_receipts();
    let call = format!("{:?}", receipts.iter().find(|r| r.receiver_id == acc("outlayer.testnet")).unwrap());
    let bytes = |s: &str| format!("{:?}", s.as_bytes()).trim_matches(['[', ']']).to_string();
    assert!(call.contains(&bytes(VERIFIER_VERSION)), "pinned hash");
    assert!(call.contains(&bytes("attached_usd")));
}

fn verifier_output(status: &str, head: &str, verdict: &str) -> Vec<u8> {
    json!({ "status": status, "head_sha": head, "verdict": verdict }).to_string().into_bytes()
}

#[test]
fn verifier_verdict_counts_only_if_verified_and_bound_to_the_commit() {
    assert_eq!(rules::read_verifier(&verifier_output("verified", SHA_B, "meets"), SHA_B).as_deref(), Some("meets"));
    assert_eq!(rules::read_verifier(&verifier_output("verified", SHA_A, "meets"), SHA_B), None, "other commit");
    assert_eq!(rules::read_verifier(&verifier_output("too_large", SHA_B, "meets"), SHA_B), None);
    assert_eq!(rules::read_verifier(&verifier_output("verified", SHA_B, "approved"), SHA_B), None);
    assert_eq!(rules::read_verifier(br#"{"error":"signature_text_mismatch"}"#, SHA_B), None);
    assert_eq!(rules::read_verifier(b"null", SHA_B), None);
}

#[test]
fn verifier_result_moves_no_money() {
    let mut c = new_contract();
    let id = funded(&mut c);
    set_callback(PromiseResult::Successful(verifier_output("verified", SHA_B, "meets")));
    c.on_verified(id, SHA_B.to_string(), acc("funder.testnet"), MIN_VERIFIER_DEPOSIT);
    assert_eq!(c.get_milestone(id).unwrap().status, Status::Funded);
    assert!(near_sdk::test_utils::get_created_receipts().is_empty());
    assert!(get_logs().iter().any(|l| l.contains("verdict=meets")));
}

// Binding the payout account (Q3)

const CONTRACT: &str = "contract.testnet";
const IMPLICIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn unbound(c: &mut Contract) -> u64 {
    let mut m: serde_json::Value = serde_json::from_str(&msg(BUDGET, FEE)).unwrap();
    m["payee_account"] = json!(CONTRACT);
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE), m.to_string());
    c.next_id - 1
}

const BIND_DEPOSIT: NearToken = NearToken::from_millinear(25);

fn start_bind(c: &mut Contract, id: u64) {
    set("relayer.testnet", BIND_DEPOSIT);
    let _ = c.bind_payee(id, "abc123".to_string());
}

fn binder_output(c: &Contract, id: u64, account: &str) -> serde_json::Value {
    json!({ "gist_id": "abc123", "gist_version": "v1", "owner_id": PAYEE_ID, "contract": CONTRACT,
            "milestone_id": id, "nonce": c.get_bind_nonce(id).unwrap(), "account": account })
}

fn deliver_bind(c: &mut Contract, id: u64, result: PromiseResult) {
    set_callback(result);
    c.on_bind_checked(id, acc("relayer.testnet"), NearToken::from_micronear(13_750), NearToken::from_micronear(11_250));
}

fn deliver_bind_json(c: &mut Contract, id: u64, v: serde_json::Value) {
    deliver_bind(c, id, PromiseResult::Successful(v.to_string().into_bytes()));
}

fn bind_refused_with(change: impl FnOnce(&mut serde_json::Value), reason: &str) {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    let mut v = binder_output(&c, id, "me.testnet");
    change(&mut v);
    deliver_bind_json(&mut c, id, v);
    assert_eq!(c.get_bound_payee(id), None, "nothing may be bound");
    assert!(get_logs().iter().any(|l| l.contains(reason)), "expected {reason} in {:?}", get_logs());
    let receipts = near_sdk::test_utils::get_created_receipts();
    assert!(receipts.iter().any(|r| r.receiver_id == acc("relayer.testnet")), "reserve goes back to the caller");
    assert!(!receipts.iter().any(|r| r.receiver_id == acc("token.testnet")), "no registration");
}

#[test]
fn gist_owner_must_hold_the_github_id() {
    bind_refused_with(|v| v["owner_id"] = json!(1), "NotTheGithubIdHolder");
}

#[test]
fn gist_for_another_contract_is_refused() {
    bind_refused_with(|v| v["contract"] = json!("other.testnet"), "WrongContract");
}

#[test]
fn gist_for_another_milestone_is_refused() {
    bind_refused_with(|v| v["milestone_id"] = json!(99), "WrongMilestone");
}

#[test]
fn stale_nonce_is_refused() {
    bind_refused_with(|v| v["nonce"] = json!("old"), "WrongNonce");
}

#[test]
fn invalid_account_name_is_refused() {
    bind_refused_with(|v| v["account"] = json!("NOT VALID!"), "UnreadableOutput");
}

#[test]
fn binder_error_binds_nothing() {
    bind_refused_with(|v| *v = json!({ "error": "gist_not_found" }), "UnreadableOutput");
}

#[test]
fn paying_the_contract_itself_is_refused() {
    bind_refused_with(|v| v["account"] = json!(CONTRACT), "UnreadableOutput");
}

#[test]
fn nonce_differs_per_milestone() {
    let mut c = new_contract();
    let (a, b) = (unbound(&mut c), unbound(&mut c));
    assert_ne!(c.get_bind_nonce(a), c.get_bind_nonce(b));
}

#[test]
fn valid_gist_binds_the_account_and_registers_it() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    let v = binder_output(&c, id, "me.testnet");
    deliver_bind_json(&mut c, id, v);
    assert_eq!(c.get_bound_payee(id), Some(acc("me.testnet")));
    let receipts = near_sdk::test_utils::get_created_receipts();
    assert!(receipts.iter().any(|r| r.receiver_id == acc("token.testnet")), "token registration");
    assert!(!receipts.iter().any(|r| r.receiver_id == acc("me.testnet")), "a named account isn't created");
    assert!(receipts.iter().any(|r| r.receiver_id == acc("relayer.testnet")), "unused NEAR returns");
}

#[test]
fn new_implicit_account_is_created_with_a_transfer() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    let v = binder_output(&c, id, IMPLICIT);
    deliver_bind_json(&mut c, id, v);
    let receipts = near_sdk::test_utils::get_created_receipts();
    assert!(receipts.iter().any(|r| r.receiver_id == acc(IMPLICIT)));
}

#[test]
fn second_binding_in_flight_cannot_replace_the_first() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    start_bind(&mut c, id);
    let v = binder_output(&c, id, "first.testnet");
    deliver_bind_json(&mut c, id, v);
    let w = binder_output(&c, id, "second.testnet");
    deliver_bind_json(&mut c, id, w);
    assert_eq!(c.get_bound_payee(id), Some(acc("first.testnet")));
    assert!(get_logs().iter().any(|l| l.contains("AlreadyBound")));
}

#[test]
#[should_panic(expected = "already bound")]
fn binding_is_final() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    let v = binder_output(&c, id, "me.testnet");
    deliver_bind_json(&mut c, id, v);
    start_bind(&mut c, id);
}

#[test]
#[should_panic(expected = "this milestone has a fixed payee")]
fn funder_set_payee_cannot_be_rebound() {
    let mut c = new_contract();
    let id = funded(&mut c);
    start_bind(&mut c, id);
}

#[test]
#[should_panic(expected = "attach at least 0.025 NEAR")]
fn binding_needs_a_deposit() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    set("relayer.testnet", MIN_CLAIM_DEPOSIT);
    let _ = c.bind_payee(id, "abc123".to_string());
}

#[test]
#[should_panic(expected = "payee is not bound yet")]
fn unbound_milestone_cannot_be_claimed() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    claim(&mut c, id, PR);
}

#[test]
fn admission_failure_returns_compute_and_reserve() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    deliver_bind(&mut c, id, PromiseResult::Failed);
    assert_eq!(c.get_bound_payee(id), None);
    assert!(get_logs().iter().any(|l| l.contains("CheckerFailed")));
    let receipts = format!("{:?}", near_sdk::test_utils::get_created_receipts());
    assert!(receipts.contains(&BIND_DEPOSIT.as_yoctonear().to_string()), "{receipts}");
}

#[test]
fn bound_milestone_pays_the_bound_account_only() {
    let mut c = new_contract();
    let id = unbound(&mut c);
    start_bind(&mut c, id);
    let v = binder_output(&c, id, "me.testnet");
    deliver_bind_json(&mut c, id, v);
    claim(&mut c, id, PR);
    deliver_json(&mut c, id, PR, good_output());
    assert_eq!(c.get_milestone(id).unwrap().status, Status::Paying);
    let transfer = format!("{:?}", near_sdk::test_utils::get_created_receipts());
    let bytes = format!("{:?}", "me.testnet".as_bytes()).trim_matches(['[', ']']).to_string();
    assert!(transfer.contains(&bytes), "{transfer}");
}

// Time parsing

#[test]
fn parses_known_utc_times() {
    assert_eq!(rules::parse_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(rules::parse_utc_seconds("2000-03-01T00:00:00Z"), Some(951_868_800));
    assert_eq!(rules::parse_utc_seconds("2026-11-06T10:00:22Z"), Some(1_793_959_222));
}

#[test]
fn rejects_time_formats_other_than_github_utc() {
    for bad in ["2026-11-06T10:00:22+01:00", "2026-11-06 10:00:22Z", "2026-11-06T10:00:22", "", "x", "2026-11-06T25:00:00Z"] {
        assert_eq!(rules::parse_utc_seconds(bad), None, "{bad}");
    }
}

fn format_utc_for_test(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let s = seconds % 86_400;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", s / 3600, (s % 3600) / 60, s % 60)
}
