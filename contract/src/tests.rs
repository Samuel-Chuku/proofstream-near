//! State-machine tests, one per rule, named after the rule. Callbacks run with the promise
//! results the test supplies. Cross-contract behaviour (gas, real token, OutLayer) is
//! covered by the sandbox tests and the testnet evidence run.

use super::*;
use near_sdk::test_utils::{get_logs, VMContextBuilder};
use near_sdk::{testing_env, PromiseResult, RuntimeFeesConfig};

const REPO_ID: u64 = 777;
const PAYEE_ID: u64 = 34985055;
const BOT_ID: u64 = 41898282;
const PR: u64 = 9;
const NOW_S: u64 = 1_793_900_000; // 2026-11-05
const MERGED_AT: &str = "2026-11-06T12:00:00Z";
const BUDGET: u128 = 10_000_000;
const FEE: u128 = 20_000;
// Enough for every default attempt.
const FEE_BUDGET: u128 = FEE * DEFAULT_MAX_PAID_ATTEMPTS as u128;
const MIN_DEADLINE_S: u64 = 600;
const DEADLINE_S: u64 = NOW_S + 3_600;
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HASH: &str = "09bf18c97a79112222bac6309efa44c60b2a6a30c8a7df632d8f3eb6b67ab969";

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn context(predecessor: &str, deposit: NearToken, now_s: u64) -> VMContextBuilder {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("contract.testnet"))
        .signer_account_id(acc(predecessor))
        .predecessor_account_id(acc(predecessor))
        .block_timestamp(now_s * NANOS_PER_SECOND)
        .attached_deposit(deposit)
        .account_balance(NearToken::from_near(100))
        .prepaid_gas(Gas::from_tgas(300));
    b
}

fn set(predecessor: &str, deposit: NearToken) {
    testing_env!(context(predecessor, deposit, NOW_S).build());
}

fn set_at(predecessor: &str, now_s: u64) {
    testing_env!(context(predecessor, NearToken::from_near(0), now_s).build());
}

/// Runs the next call as the contract itself, with these promise results available.
fn set_callback(results: Vec<PromiseResult>) {
    let c = context("contract.testnet", NearToken::from_near(0), NOW_S).build();
    testing_env!(c, near_sdk::test_vm_config(), RuntimeFeesConfig::test(), Default::default(), results);
}

fn ok(v: &Value) -> PromiseResult {
    PromiseResult::Successful(v.to_string().into_bytes())
}

fn pin(name: &str) -> Pin {
    Pin { project_id: format!("owner.testnet/{name}"), wasm_hash: HASH.into() }
}

fn params() -> CreateMilestone {
    CreateMilestone {
        repo_id: REPO_ID,
        repo_owner: "Samuel-Chuku".into(),
        repo_name: "proofstream-test".into(),
        base_branch: "main".into(),
        payee_github_id: PAYEE_ID,
        milestone_text: "Add a function that returns a greeting string.".into(),
        required_checks: vec!["test".into()],
        budget: U128(BUDGET),
        fee_budget: U128(FEE_BUDGET),
        deadline_seconds: DEADLINE_S,
        max_paid_attempts: None,
        diff_cap_bytes: None,
        allow_self_merge: None,
        pins: Pins {
            checker: pin("checker"),
            binder: pin("binder"),
            judge: pin("judge"),
            verifier: VerifierPin {
                project_id: "verifier-owner.testnet/verifier".into(),
                wasm_hash: HASH.into(),
                fee: U128(FEE),
                public_key: VERIFIER_KEY.into(),
            },
        },
    }
}

fn new_contract() -> Contract {
    set("deployer.testnet", NearToken::from_near(0));
    Contract::new(acc("token.testnet"), acc("outlayer.testnet"), U128(1_000_000), MIN_DEADLINE_S)
}

fn create(c: &mut Contract, p: CreateMilestone) -> u64 {
    set("funder.testnet", NearToken::from_millinear(100));
    c.create_milestone(p)
}

fn fund(c: &mut Contract, id: u64) -> U128 {
    set("token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE_BUDGET), json!({ "milestone_id": id }).to_string())
}

fn bind(c: &mut Contract, id: u64) {
    c.milestones.get_mut(&id).unwrap().payee =
        Some(Bound { account: acc("payee.testnet"), gist_id: "abc".into(), gist_revision: "d".repeat(40) });
}

/// Created, funded and bound.
fn ready(c: &mut Contract) -> u64 {
    let id = create(c, params());
    fund(c, id);
    bind(c, id);
    id
}

fn state(c: &Contract, id: u64) -> State {
    c.get_milestone(id).unwrap().state
}

fn logged(what: &str) -> bool {
    get_logs().iter().any(|l| l.contains(what))
}

fn claim(c: &mut Contract, id: u64, pr: u64) {
    set("anyone.testnet", MIN_CLAIM_DEPOSIT);
    let _ = c.claim(id, pr);
}

fn checker(merged: bool) -> Value {
    json!({
        "repo_id": REPO_ID, "pr_number": PR, "merged": merged, "base_sha": BASE, "head_sha": HEAD,
        "merge_commit_sha": if merged { Some("e".repeat(40)) } else { None },
        "author_id": PAYEE_ID, "author_login": "Samuel-Chuku",
        "merged_by": if merged { json!({ "id": BOT_ID, "type": "Bot" }) } else { Value::Null },
        "independent_approval": true, "base_branch": "main", "checks_conclusion": "success",
        "ci_source": "check_runs", "additions": 3, "deletions": 0,
        "files_complete": true, "touches_github": false,
        "merged_at": if merged { Some(MERGED_AT) } else { None },
        "diff_bytes": 1_000, "diff_too_large": false, "changed_files": 3,
    })
}

fn checked(c: &mut Contract, id: u64, approved: Option<&str>, result: PromiseResult) {
    set_callback(vec![result]);
    c.on_checked(id, PR, approved.map(|h| (h.to_string(), 100)), acc("anyone.testnet"), MIN_CLAIM_DEPOSIT);
}

fn checked_json(c: &mut Contract, id: u64, output: Value) {
    checked(c, id, None, ok(&output));
}

/// Claimed, checked and passed: HardFactsPassed.
fn passed(c: &mut Contract, merged: bool) -> u64 {
    let id = ready(c);
    claim(c, id, PR);
    checked_json(c, id, checker(merged));
    id
}

fn review(c: &mut Contract, id: u64) {
    set("anyone.testnet", MIN_AI_DEPOSIT.saturating_mul(2));
    let _ = c.review(id);
}

fn fee_ready(c: &mut Contract, id: u64, taken: Vec<(u64, U128)>, new_deposit: u128, result: Option<PromiseResult>) {
    set_callback(result.into_iter().collect());
    c.on_fee_ready(id, acc("anyone.testnet"), MIN_AI_DEPOSIT.saturating_mul(2), taken, U128(new_deposit));
}

fn ai_pct(role: &str, verdict: &str, head: &str, pct: u8) -> Value {
    json!({ "status": "verified", "role": role, "head_sha": head, "verdict": verdict, "percent_done": pct })
}

fn ai(role: &str, verdict: &str, head: &str) -> Value {
    ai_pct(role, verdict, head, 100)
}

fn judge_ai(verdict: &str, head: &str) -> Value {
    ai("judge", verdict, head)
}

fn verifier_ai(verdict: &str, head: &str) -> Value {
    ai("verifier", verdict, head)
}

/// D48: the verifier's test key (tests/fixtures/sign-verifier.mjs, a public test seed).
const VERIFIER_KEY: &str = "f5c3e2136054b44445215634ef19540c9cae7d22199d2b6595d12063dca403fe";
/// The judge pin's owner, who pays for the verifier's run.
const JUDGE_OWNER: &str = "owner.testnet";
const SIGS: &str = include_str!("../tests/fixtures/verifier-sigs.json");

/// A real ed25519 signature by the test key, from the fixture. A message not there yet is
/// written down for `node tests/fixtures/sign-verifier.mjs` to sign.
fn sig_for(message: &str) -> String {
    let fixture: Value = serde_json::from_str(SIGS).unwrap();
    if let Some(sig) = fixture["sigs"][message].as_str() {
        return sig.into();
    }
    use std::io::Write;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/missing-verifier-sigs.txt");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(f, "{message}").unwrap();
    panic!("no test signature for {message}: run `node tests/fixtures/sign-verifier.mjs`");
}

/// The verifier's claim as its enclave would sign it, for this review.
fn signed_claim(id: u64, attempt: u32, head: &str, verdict: &str, pct: u64) -> Value {
    let pct8 = u8::try_from(pct).unwrap_or(255);
    let message = rules::verdict_message("contract.testnet", id, attempt, head, verdict, pct8, JUDGE_OWNER);
    json!({ "verdict": verdict, "percent_done": pct, "caller": JUDGE_OWNER, "signature": sig_for(&message), "call_id": "call-1" })
}

/// What the judge's run returns: its own answer, carrying the verifier's signed claim
/// when the verifier gave one (D48). The two test answers keep their old meaning.
fn judge_result(c: &Contract, id: u64, judge: PromiseResult, verifier: PromiseResult) -> PromiseResult {
    let PromiseResult::Successful(bytes) = &judge else { return judge };
    let Ok(mut out) = serde_json::from_slice::<Value>(bytes) else { return judge };
    if !out.is_object() {
        return judge;
    }
    let attempt = c.get_milestone(id).unwrap().paid_attempts_used;
    let v = match &verifier {
        PromiseResult::Successful(b) => serde_json::from_slice::<Value>(b).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let block = match (v["role"].as_str(), v["status"].as_str()) {
        (Some("verifier"), Some("verified")) if v["verdict"].is_string() => {
            signed_claim(id, attempt, v["head_sha"].as_str().unwrap(), v["verdict"].as_str().unwrap(), v["percent_done"].as_u64().unwrap_or(0))
        }
        (Some("verifier"), Some(early @ ("too_large" | "incomplete_diff"))) => signed_claim(id, attempt, v["head_sha"].as_str().unwrap(), early, 0),
        // Another role's answer can't carry the verifier's signature.
        (Some(_), Some("verified")) => {
            json!({ "verdict": v["verdict"], "percent_done": 100, "caller": JUDGE_OWNER, "signature": "00".repeat(64), "call_id": "call-1" })
        }
        _ => json!({ "error": "verifier_not_completed" }),
    };
    out["verifier"] = block;
    ok(&out)
}

fn reviewed(c: &mut Contract, id: u64, judge: PromiseResult, verifier: PromiseResult) {
    let result = judge_result(c, id, judge, verifier);
    set_callback(vec![result]);
    c.on_reviewed(id, acc("anyone.testnet"), MIN_AI_DEPOSIT.saturating_mul(2));
}

/// Through review with both answers given, the fee deposited fresh.
fn full_review(c: &mut Contract, id: u64, judge: PromiseResult, verifier: PromiseResult) {
    review(c, id);
    fee_ready(c, id, vec![], FEE, Some(ok(&json!(FEE.to_string()))));
    reviewed(c, id, judge, verifier);
}

fn assert_reconciles(c: &Contract) {
    let r = c.reconcile();
    assert_eq!(r.tokens_held, r.owed, "tokens held must equal what is owed");
}

// ---------------------------------------------------------------- create and fund

#[test]
fn create_starts_in_funding_and_charges_storage() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    assert_eq!(state(&c, id), State::Funding);
    assert!(logged("milestone_created"));
}

#[test]
#[should_panic(expected = "attach at least")]
fn create_refuses_unpaid_storage() {
    let mut c = new_contract();
    set("funder.testnet", NearToken::from_yoctonear(1));
    c.create_milestone(params());
}

#[test]
#[should_panic(expected = "budget below min_budget")]
fn create_refuses_a_budget_below_the_minimum() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { budget: U128(999_999), ..params() });
}

#[test]
#[should_panic(expected = "deadline too soon")]
fn create_refuses_a_deadline_inside_min_deadline() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { deadline_seconds: NOW_S + MIN_DEADLINE_S - 1, ..params() });
}

#[test]
#[should_panic(expected = "fee_budget must cover")]
fn create_refuses_a_fee_budget_below_one_review() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { fee_budget: U128(FEE - 1), ..params() });
}

#[test]
#[should_panic(expected = "bad pins")]
fn create_refuses_a_pin_that_is_not_a_wasm_hash() {
    let mut c = new_contract();
    let mut p = params();
    p.pins.judge.wasm_hash = "latest".into();
    create(&mut c, p);
}

#[test]
#[should_panic(expected = "bad pins")]
fn create_refuses_a_project_without_an_owner() {
    let mut c = new_contract();
    let mut p = params();
    p.pins.checker.project_id = "checker".into();
    create(&mut c, p);
}

#[test]
#[should_panic(expected = "judge and verifier pins must have different owners and projects")]
fn create_refuses_judge_and_verifier_with_one_owner() {
    let mut c = new_contract();
    let mut p = params();
    // Different project names, same owner: one account's key and secrets would serve both.
    p.pins.verifier.project_id = "owner.testnet/proofstream-verifier".into();
    create(&mut c, p);
}

#[test]
#[should_panic(expected = "judge and verifier pins must have different owners and projects")]
fn create_refuses_judge_and_verifier_on_one_project() {
    let mut c = new_contract();
    let mut p = params();
    p.pins.verifier.project_id = p.pins.judge.project_id.clone();
    create(&mut c, p);
}

#[test]
fn create_accepts_one_build_for_both_ais_under_different_owners() {
    // D37: the same wasm hash is fine; independence is the owner and the project.
    let mut c = new_contract();
    let p = params();
    assert_eq!(p.pins.judge.wasm_hash, p.pins.verifier.wasm_hash);
    create(&mut c, p);
}

#[test]
#[should_panic(expected = "diff_cap_bytes out of range")]
fn create_refuses_a_diff_cap_the_judge_cannot_review() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { diff_cap_bytes: Some(MAX_DIFF_CAP_BYTES + 1), ..params() });
}

#[test]
#[should_panic(expected = "required_checks")]
fn create_refuses_no_required_checks() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { required_checks: vec![], ..params() });
}

#[test]
fn exact_funding_from_the_funder_funds_it() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    assert_eq!(fund(&mut c, id), U128(0));
    assert_eq!(state(&c, id), State::Funded);
    assert_eq!(c.reconcile().owed, U128(BUDGET + FEE_BUDGET));
    assert_reconciles(&c);
}

#[test]
#[should_panic(expected = "wrong token")]
fn funding_from_another_token_is_refused() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    set("fake-token.testnet", NearToken::from_near(0));
    c.ft_on_transfer(acc("funder.testnet"), U128(BUDGET + FEE_BUDGET), json!({ "milestone_id": id }).to_string());
}

#[test]
fn funding_from_someone_else_is_returned() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    set("token.testnet", NearToken::from_near(0));
    let back = c.ft_on_transfer(acc("stranger.testnet"), U128(BUDGET + FEE_BUDGET), json!({ "milestone_id": id }).to_string());
    assert_eq!(back, U128(BUDGET + FEE_BUDGET));
    assert_eq!(state(&c, id), State::Funding);
}

#[test]
fn under_or_over_funding_is_returned() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    for amount in [BUDGET + FEE_BUDGET - 1, BUDGET + FEE_BUDGET + 1] {
        set("token.testnet", NearToken::from_near(0));
        let back = c.ft_on_transfer(acc("funder.testnet"), U128(amount), json!({ "milestone_id": id }).to_string());
        assert_eq!(back, U128(amount));
    }
    assert_eq!(state(&c, id), State::Funding);
    assert_eq!(c.reconcile().owed, U128(0));
}

#[test]
fn funding_twice_returns_the_second() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    fund(&mut c, id);
    assert_eq!(fund(&mut c, id), U128(BUDGET + FEE_BUDGET));
    assert_reconciles(&c);
}

#[test]
fn unreadable_funding_msg_is_returned() {
    let mut c = new_contract();
    create(&mut c, params());
    set("token.testnet", NearToken::from_near(0));
    assert_eq!(c.ft_on_transfer(acc("funder.testnet"), U128(5), "{}".into()), U128(5));
}

// ---------------------------------------------------------------- bind

fn binder_out(c: &Contract, id: u64, account: &str) -> Value {
    json!({
        "owner_id": PAYEE_ID, "contract": "contract.testnet", "milestone_id": id,
        "nonce": c.get_bind_nonce(id).unwrap(), "account": account, "revision": "d".repeat(40),
    })
}

fn start_bind(c: &mut Contract, id: u64) {
    set("relayer.testnet", NearToken::from_millinear(100));
    let _ = c.bind_payee(id, "abc123".into());
}

fn bound_result(c: &mut Contract, id: u64, result: PromiseResult) {
    set_callback(vec![result]);
    let reserve = TOKEN_STORAGE.saturating_add(IMPLICIT_PROBE);
    c.on_bound(id, "abc123".into(), acc("relayer.testnet"), NearToken::from_millinear(100).saturating_sub(reserve), reserve);
}

#[test]
fn a_binding_in_flight_blocks_a_second() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    assert!(c.get_milestone(id).unwrap().binding_in_flight);
    set("relayer.testnet", NearToken::from_millinear(100));
    let second = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.bind_payee(id, "abc123".into())));
    assert!(second.is_err());
}

#[test]
fn a_gist_from_another_github_id_binds_nothing() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    let mut out = binder_out(&c, id, "alice.testnet");
    out["owner_id"] = 1.into();
    bound_result(&mut c, id, ok(&out));
    let m = c.get_milestone(id).unwrap();
    assert!(m.payee.is_none() && !m.binding_in_flight);
    assert!(logged("NotTheGithubIdHolder"));
}

#[test]
fn an_eth_style_account_is_refused_for_now() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    let out = binder_out(&c, id, &format!("0x{}", "a".repeat(40)));
    bound_result(&mut c, id, ok(&out));
    assert!(c.get_milestone(id).unwrap().payee.is_none());
    assert!(logged("UnsupportedAccount"));
}

#[test]
fn a_good_gist_probes_before_recording_anything() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    let out = binder_out(&c, id, "alice.testnet");
    bound_result(&mut c, id, ok(&out));
    let m = c.get_milestone(id).unwrap();
    assert!(m.payee.is_none(), "nothing is bound before the probe succeeds");
    assert!(m.binding_in_flight);
}

#[test]
fn a_missing_named_account_is_refused() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    set_callback(vec![PromiseResult::Failed]);
    c.on_probed(id, acc("typo.testnet"), "abc123".into(), "d".repeat(40), acc("relayer.testnet"), NearToken::from_millinear(1));
    let m = c.get_milestone(id).unwrap();
    assert!(m.payee.is_none() && !m.binding_in_flight);
    assert!(logged("AccountMissing"));
}

#[test]
fn a_registered_account_is_bound_with_its_gist_revision() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    start_bind(&mut c, id);
    set_callback(vec![ok(&Value::Null)]);
    c.on_registered(id, acc("alice.testnet"), "abc123".into(), "d".repeat(40));
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.payee.unwrap().account, acc("alice.testnet"));
    assert!(!m.binding_in_flight);
}

#[test]
#[should_panic(expected = "bad gist id")]
fn a_gist_id_with_url_characters_is_refused() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    set("relayer.testnet", NearToken::from_millinear(100));
    let _ = c.bind_payee(id, "../x".into());
}

// ---------------------------------------------------------------- claim

#[test]
#[should_panic(expected = "payee is not bound")]
fn claim_needs_a_bound_payee() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    fund(&mut c, id);
    claim(&mut c, id, PR);
}

#[test]
#[should_panic(expected = "not claimable")]
fn claim_needs_a_funded_milestone() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    bind(&mut c, id);
    claim(&mut c, id, PR);
}

#[test]
fn claim_locks_the_pr_and_the_milestone() {
    let mut c = new_contract();
    let id = ready(&mut c);
    claim(&mut c, id, PR);
    assert_eq!(state(&c, id), State::Checking { pr: PR });
    assert_eq!(c.get_lock(REPO_ID, PR), Some(PrLock::Locked(id)));
}

#[test]
#[should_panic(expected = "PR is locked")]
fn the_same_pr_cannot_race_onto_a_second_milestone() {
    let mut c = new_contract();
    let a = ready(&mut c);
    let b = ready(&mut c);
    claim(&mut c, a, PR);
    claim(&mut c, b, PR);
}

#[test]
fn merged_pr_with_good_facts_passes() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    assert_eq!(
        state(&c, id),
        State::HardFactsPassed { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), merged: true, changed_files: 3 }
    );
}

#[test]
fn open_pr_with_good_facts_passes_for_premerge_review() {
    let mut c = new_contract();
    let id = passed(&mut c, false);
    assert!(matches!(state(&c, id), State::HardFactsPassed { merged: false, .. }));
}

fn refused_with(change: impl FnOnce(&mut Value), reason: &str) {
    let mut c = new_contract();
    let id = ready(&mut c);
    claim(&mut c, id, PR);
    let mut out = checker(true);
    change(&mut out);
    checked_json(&mut c, id, out);
    assert_eq!(state(&c, id), State::Funded);
    assert_eq!(c.get_lock(REPO_ID, PR), None, "a refusal releases the lock");
    assert!(logged(reason), "expected {reason} in {:?}", get_logs());
}

#[test]
fn bot_merge_without_independent_approval_is_refused() {
    refused_with(|o| o["independent_approval"] = false.into(), "NoIndependentApproval");
}

#[test]
fn incomplete_file_list_is_refused() {
    refused_with(|o| o["files_complete"] = false.into(), "IncompleteFileList");
}

#[test]
fn wrong_author_is_refused() {
    refused_with(|o| o["author_id"] = 1.into(), "WrongAuthor");
}

#[test]
fn checker_error_output_is_refused() {
    refused_with(|o| *o = json!({ "error": "github_not_found" }), "UnreadableOutput");
}

#[test]
fn a_timed_out_checker_is_refused() {
    refused_with(|o| *o = Value::Null, "UnreadableOutput");
}

#[test]
fn a_refused_admission_returns_the_claim_deposit() {
    let mut c = new_contract();
    let id = ready(&mut c);
    claim(&mut c, id, PR);
    checked(&mut c, id, None, PromiseResult::Failed);
    assert_eq!(state(&c, id), State::Funded);
    assert!(logged("CheckerFailed"));
}

#[test]
#[should_panic(expected = "does not match the claim in flight")]
fn a_stale_checker_result_cannot_touch_the_milestone() {
    let mut c = new_contract();
    let id = ready(&mut c);
    checked_json(&mut c, id, checker(true));
}

#[test]
fn diff_over_the_cap_is_held_without_spending_a_fee() {
    let mut c = new_contract();
    let id = ready(&mut c);
    claim(&mut c, id, PR);
    let mut out = checker(true);
    out["diff_bytes"] = (DEFAULT_DIFF_CAP_BYTES + 1).into();
    checked_json(&mut c, id, out);
    let m = c.get_milestone(id).unwrap();
    assert!(matches!(m.state, State::Held { ref reason, .. } if reason.starts_with("too_large")));
    assert_eq!((m.fee_remaining, m.paid_attempts_used), (U128(FEE_BUDGET), 0));
}

#[test]
#[should_panic(expected = "used on this milestone")]
fn a_rejected_pr_cannot_be_claimed_again_here() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai("does_not_meet", HEAD)), ok(&verifier_ai("meets", HEAD)));
    set("funder.testnet", NearToken::from_near(0));
    c.funder_reject(id, "not what was asked".into());
    claim(&mut c, id, PR);
}

// ---------------------------------------------------------------- review

#[test]
#[should_panic(expected = "not ready for review")]
fn review_needs_passed_hard_facts() {
    let mut c = new_contract();
    let id = ready(&mut c);
    review(&mut c, id);
}

#[test]
#[should_panic(expected = "attach at least 0.06 NEAR")]
fn review_needs_compute_for_both_runs() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    set("anyone.testnet", MIN_AI_DEPOSIT);
    let _ = c.review(id);
}

#[test]
fn review_spends_one_fee_and_counts_one_attempt() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review(&mut c, id);
    let m = c.get_milestone(id).unwrap();
    assert_eq!((m.fee_remaining, m.paid_attempts_used), (U128(FEE_BUDGET - FEE), 1));
    assert_eq!(m.budget, U128(BUDGET), "the fee never comes out of the budget");
    assert_reconciles(&c);
}

#[test]
fn a_failed_fee_deposit_undoes_the_attempt() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review(&mut c, id);
    fee_ready(&mut c, id, vec![], FEE, Some(ok(&json!("0"))));
    let m = c.get_milestone(id).unwrap();
    assert!(matches!(m.state, State::HardFactsPassed { .. }));
    assert_eq!((m.fee_remaining, m.paid_attempts_used), (U128(FEE_BUDGET), 0));
    assert_reconciles(&c);
}

#[test]
fn both_meet_on_a_merged_pr_starts_the_payout() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai("meets", HEAD)), ok(&verifier_ai("meets", HEAD)));
    assert_eq!(state(&c, id), State::PayingOut { part: Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 100 }, amount: U128(BUDGET) });
}

#[test]
fn both_meet_on_an_open_pr_records_approval_and_moves_no_money() {
    let mut c = new_contract();
    let id = passed(&mut c, false);
    full_review(&mut c, id, ok(&judge_ai("meets", HEAD)), ok(&verifier_ai("meets", HEAD)));
    assert_eq!(state(&c, id), State::Approved { pr: PR, head_sha: HEAD.into(), pct: 100 });
    assert_eq!(c.get_lock(REPO_ID, PR), Some(PrLock::Locked(id)));
    assert!(logged("verdict_approved_premerge"));
    assert_eq!(c.get_milestone(id).unwrap().budget, U128(BUDGET));
}

fn held_by(judge: &str, verifier: &str) {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai(judge, HEAD)), ok(&verifier_ai(verifier, HEAD)));
    assert!(matches!(state(&c, id), State::Held { .. }), "{judge}/{verifier}");
}

#[test]
fn a_judge_objection_holds_for_the_funder() {
    held_by("does_not_meet", "meets");
}

#[test]
fn an_unclear_verifier_holds_for_the_funder() {
    held_by("meets", "unclear");
}

#[test]
fn an_incomplete_diff_holds_for_the_funder() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    // The enclave's own D31 pre-check: a status, with no model verdict.
    let incomplete = json!({ "status": "incomplete_diff", "role": "verifier", "head_sha": HEAD, "diff_files": 2, "changed_files": 3 });
    full_review(&mut c, id, ok(&judge_ai("meets", HEAD)), ok(&incomplete));
    assert!(matches!(state(&c, id), State::Held { ref reason, .. } if reason == "verifier:incomplete_diff"));
}

#[test]
fn a_verdict_from_the_wrong_role_counts_as_no_verdict() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    // Both pins pointing at the judge's project: the second "meets" isn't the verifier's.
    full_review(&mut c, id, ok(&judge_ai("meets", HEAD)), ok(&judge_ai("meets", HEAD)));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
    assert!(logged("review_retry"));
}

#[test]
fn the_ai_input_carries_the_prs_file_count() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review(&mut c, id);
    fee_ready(&mut c, id, vec![], FEE, Some(ok(&json!(FEE.to_string()))));
    let calls = ai_calls();
    assert_eq!(calls.len(), 1, "D48: one run, the judge's; it asks the verifier itself");
    let input: Value = near_sdk::serde_json::from_str(calls[0]["input_data"].as_str().unwrap()).unwrap();
    assert_eq!((input["changed_files"].clone(), input["head_sha"].clone()), (json!(3), json!(HEAD)));
}

#[test]
fn the_judge_is_told_which_review_this_is_and_which_verifier_build_to_pay() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review(&mut c, id);
    fee_ready(&mut c, id, vec![], FEE, Some(ok(&json!(FEE.to_string()))));
    let call = &ai_calls()[0];
    let input: Value = near_sdk::serde_json::from_str(call["input_data"].as_str().unwrap()).unwrap();
    assert_eq!(input["review"], json!({ "contract": "contract.testnet", "milestone_id": id, "attempt": 1 }));
    let pins = params().pins;
    assert_eq!(input["verifier"], json!({ "project_id": pins.verifier.project_id, "version_key": HASH, "fee": FEE }));
    // The judge's owner is paid the fee, to pass on to the verifier.
    assert_eq!(call["params"]["attached_usd"], FEE.to_string());
    assert_eq!(call["source"]["Project"]["project_id"], pins.judge.project_id);
}

/// The `request_execution` calls the last callback created, as JSON.
fn ai_calls() -> Vec<Value> {
    near_sdk::test_utils::get_created_receipts()
        .iter()
        .flat_map(|r| r.actions.iter())
        .filter_map(|a| match a {
            near_sdk::mock::MockAction::FunctionCallWeight { method_name, args, .. }
                if method_name == b"request_execution" =>
            {
                near_sdk::serde_json::from_slice(args).ok()
            }
            _ => None,
        })
        .collect()
}

#[test]
fn each_ai_call_opens_the_secrets_of_its_own_pins_owner() {
    // D37 condition 1: the secrets row comes from the pin's project_id, never another field.
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review(&mut c, id);
    fee_ready(&mut c, id, vec![], FEE, Some(ok(&json!(FEE.to_string()))));
    let calls = ai_calls();
    let pins = params().pins;
    // D48: only the judge runs on chain; the verifier's row is opened by the judge's call.
    let expected = [(pins.judge.project_id, "owner.testnet")];
    assert_eq!(calls.len(), expected.len());
    for (call, (project, owner)) in calls.iter().zip(expected) {
        assert_eq!(call["source"]["Project"]["project_id"], project);
        assert_eq!(call["secrets_ref"]["account_id"], owner);
        assert_eq!(call["secrets_ref"]["profile"], "default");
    }
}

#[test]
fn a_success_that_returns_a_value_still_counts_as_paid() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(vec![PromiseResult::Successful(vec![b'x'; 100])]);
    c.on_transferred(id, U128(BUDGET), Transfer::Payout, State::Funded);
    assert_eq!(state(&c, id), State::Paid { pr: PR });
}

#[test]
fn a_verdict_for_another_commit_is_not_a_verdict() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai("meets", NEW_HEAD)), ok(&verifier_ai("meets", HEAD)));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
}

#[test]
fn a_failed_run_retries_until_attempts_run_out() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    for _ in 0..DEFAULT_MAX_PAID_ATTEMPTS - 1 {
        full_review(&mut c, id, ok(&json!({ "error": "signature_text_mismatch" })), ok(&verifier_ai("meets", HEAD)));
        assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
    }
    full_review(&mut c, id, ok(&json!({ "error": "signature_text_mismatch" })), ok(&verifier_ai("meets", HEAD)));
    assert!(matches!(state(&c, id), State::Held { ref reason, .. } if reason == "attempts_exhausted"));
    assert_reconciles(&c);
}

#[test]
fn a_failed_judge_run_records_its_fee_as_credit() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&Value::Null), ok(&verifier_ai("meets", HEAD)));
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.outlayer_credit, U128(FEE));
    assert_eq!(c.reconcile().outlayer_credit, U128(FEE));
    assert_reconciles(&c);
}

#[test]
fn the_next_review_spends_credit_before_new_money() {
    let mut c = new_contract();
    let a = passed(&mut c, true);
    full_review(&mut c, a, ok(&Value::Null), ok(&verifier_ai("meets", HEAD)));
    // A second milestone's review spends A's credit, and A can refund that much.
    let b = ready(&mut c);
    claim(&mut c, b, PR + 1);
    let mut out = checker(true);
    out["pr_number"] = (PR + 1).into();
    set_callback(vec![ok(&out)]);
    c.on_checked(b, PR + 1, None, acc("anyone.testnet"), MIN_CLAIM_DEPOSIT);
    review(&mut c, b);
    assert!(logged(r#""from_credit":"20000""#), "{:?}", get_logs());
    let ma = c.get_milestone(a).unwrap();
    assert_eq!((ma.outlayer_credit, ma.fee_refundable), (U128(0), U128(FEE)));
    assert_eq!(c.get_milestone(b).unwrap().fee_remaining, U128(FEE_BUDGET - FEE));
    assert_reconciles(&c);
}

// ---------------------------------------------------------------- D36: approved before merge

fn approved(c: &mut Contract) -> u64 {
    let id = passed(c, false);
    full_review(c, id, ok(&judge_ai("meets", HEAD)), ok(&verifier_ai("meets", HEAD)));
    id
}

#[test]
#[should_panic(expected = "only the approved PR")]
fn an_approved_milestone_takes_only_its_pr() {
    let mut c = new_contract();
    let id = approved(&mut c);
    claim(&mut c, id, PR + 1);
}

#[test]
fn merged_at_the_approved_head_pays_without_another_review() {
    let mut c = new_contract();
    let id = approved(&mut c);
    claim(&mut c, id, PR);
    checked(&mut c, id, Some(HEAD), ok(&checker(true)));
    assert_eq!(state(&c, id), State::PayingOut { part: Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 100 }, amount: U128(BUDGET) });
    assert_eq!(c.get_milestone(id).unwrap().paid_attempts_used, 1, "no second fee");
}

#[test]
fn still_open_stays_approved_and_locked() {
    let mut c = new_contract();
    let id = approved(&mut c);
    claim(&mut c, id, PR);
    checked(&mut c, id, Some(HEAD), ok(&checker(false)));
    assert_eq!(state(&c, id), State::Approved { pr: PR, head_sha: HEAD.into(), pct: 100 });
    assert_eq!(c.get_lock(REPO_ID, PR), Some(PrLock::Locked(id)));
}

#[test]
fn merged_at_a_new_head_is_reviewed_again() {
    let mut c = new_contract();
    let id = approved(&mut c);
    claim(&mut c, id, PR);
    let mut out = checker(true);
    out["head_sha"] = NEW_HEAD.into();
    checked(&mut c, id, Some(HEAD), ok(&out));
    assert!(matches!(state(&c, id), State::HardFactsPassed { ref head_sha, merged: true, .. } if head_sha == NEW_HEAD));
}

#[test]
fn an_approval_still_needs_the_merge_facts() {
    let mut c = new_contract();
    let id = approved(&mut c);
    claim(&mut c, id, PR);
    let mut out = checker(true);
    out["independent_approval"] = false.into();
    checked(&mut c, id, Some(HEAD), ok(&out));
    assert_eq!(state(&c, id), State::Approved { pr: PR, head_sha: HEAD.into(), pct: 100 });
    assert!(logged("NoIndependentApproval"));
}

#[test]
fn the_funder_can_reject_an_approval_with_a_reason() {
    let mut c = new_contract();
    let id = approved(&mut c);
    set("funder.testnet", NearToken::from_near(0));
    c.funder_reject(id, "changed our mind".into());
    assert_eq!(state(&c, id), State::Funded);
    assert!(c.is_pr_used(id, PR));
    assert!(logged(r#""was_approved_premerge":true"#));
}

#[test]
fn refund_after_an_approval_is_on_the_record() {
    let mut c = new_contract();
    let id = approved(&mut c);
    set_at("funder.testnet", DEADLINE_S);
    let _ = c.refund(id);
    assert_eq!(state(&c, id), State::Refunded);
    assert!(logged("refund_after_approval"));
    assert_eq!(c.get_lock(REPO_ID, PR), None);
}

// ---------------------------------------------------------------- funder

#[test]
#[should_panic(expected = "only the funder")]
fn only_the_funder_can_release() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai("unclear", HEAD)), ok(&verifier_ai("meets", HEAD)));
    set("payee.testnet", NearToken::from_near(0));
    let _ = c.funder_release(id, 100);
}

#[test]
fn the_funder_can_release_a_held_milestone() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    full_review(&mut c, id, ok(&judge_ai("unclear", HEAD)), ok(&verifier_ai("meets", HEAD)));
    set("funder.testnet", NearToken::from_near(0));
    let _ = c.funder_release(id, 100);
    assert_eq!(state(&c, id), State::PayingOut { part: Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 100 }, amount: U128(BUDGET) });
}

#[test]
#[should_panic(expected = "not held or approved")]
fn reject_needs_held_or_approved() {
    let mut c = new_contract();
    let id = ready(&mut c);
    set("funder.testnet", NearToken::from_near(0));
    c.funder_reject(id, "no".into());
}

#[test]
#[should_panic(expected = "reason must be")]
fn reject_needs_a_short_reason() {
    let mut c = new_contract();
    let id = approved(&mut c);
    set("funder.testnet", NearToken::from_near(0));
    c.funder_reject(id, "x".repeat(MAX_REASON_BYTES + 1));
}

#[test]
#[should_panic(expected = "deadline has not passed")]
fn refund_waits_for_the_deadline() {
    let mut c = new_contract();
    let id = ready(&mut c);
    set_at("funder.testnet", DEADLINE_S - 1);
    let _ = c.refund(id);
}

#[test]
fn refund_is_impossible_while_held_or_in_flight() {
    let mut c = new_contract();
    let held = passed(&mut c, true);
    full_review(&mut c, held, ok(&judge_ai("unclear", HEAD)), ok(&verifier_ai("meets", HEAD)));
    let checking = ready(&mut c);
    claim(&mut c, checking, PR + 1);
    for id in [held, checking] {
        set_at("funder.testnet", DEADLINE_S + 1);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(id)));
        assert!(r.is_err(), "milestone {id} must not refund");
    }
}

#[test]
fn refund_returns_budget_and_unspent_fee() {
    let mut c = new_contract();
    let id = ready(&mut c);
    set_at("funder.testnet", DEADLINE_S);
    let _ = c.refund(id);
    assert!(logged(&format!(r#""amount":"{}""#, BUDGET + FEE_BUDGET)));
    set_callback(vec![ok(&Value::Null)]);
    c.on_transferred(id, U128(BUDGET + FEE_BUDGET), Transfer::Refund, State::Funded);
    assert_eq!(c.reconcile().owed, U128(0));
    assert_reconciles(&c);
}

#[test]
fn a_failed_refund_puts_the_milestone_back() {
    let mut c = new_contract();
    let id = ready(&mut c);
    set_at("funder.testnet", DEADLINE_S);
    let _ = c.refund(id);
    set_callback(vec![PromiseResult::Failed]);
    c.on_transferred(id, U128(BUDGET + FEE_BUDGET), Transfer::Refund, State::Funded);
    assert_eq!(state(&c, id), State::Funded);
    assert_reconciles(&c);
}

// ---------------------------------------------------------------- payout

fn paying(c: &mut Contract) -> u64 {
    let id = passed(c, true);
    full_review(c, id, ok(&judge_ai("meets", HEAD)), ok(&verifier_ai("meets", HEAD)));
    id
}

#[test]
fn a_confirmed_payout_is_final_and_spends_the_pr() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(vec![ok(&Value::Null)]);
    c.on_transferred(id, U128(BUDGET), Transfer::Payout, State::Reviewing { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), merged: true, changed_files: 3 });
    assert_eq!(state(&c, id), State::Paid { pr: PR });
    assert_eq!(c.get_lock(REPO_ID, PR), Some(PrLock::Spent(id)));
    assert_eq!(c.reconcile().owed, U128(FEE_BUDGET - FEE));
    assert_reconciles(&c);
}

#[test]
#[should_panic(expected = "PR is locked or already paid")]
fn a_paid_pr_cannot_pay_another_milestone() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(vec![ok(&Value::Null)]);
    c.on_transferred(id, U128(BUDGET), Transfer::Payout, State::Funded);
    let other = ready(&mut c);
    claim(&mut c, other, PR);
}

#[test]
fn a_failed_payout_can_be_retried_by_anyone() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(vec![PromiseResult::Failed]);
    c.on_transferred(id, U128(BUDGET), Transfer::Payout, State::Funded);
    assert_eq!(state(&c, id), State::PayoutFailed { part: Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 100 }, amount: U128(BUDGET) });
    set("stranger.testnet", NearToken::from_near(0));
    let _ = c.retry_payout(id);
    assert_eq!(state(&c, id), State::PayingOut { part: Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 100 }, amount: U128(BUDGET) });
}

#[test]
fn a_paid_milestone_refunds_only_its_leftover_fee() {
    let mut c = new_contract();
    let id = paying(&mut c);
    set_callback(vec![ok(&Value::Null)]);
    c.on_transferred(id, U128(BUDGET), Transfer::Payout, State::Funded);
    set("funder.testnet", NearToken::from_near(0));
    let _ = c.refund(id);
    assert!(logged(&format!(r#""amount":"{}""#, FEE_BUDGET - FEE)));
    assert_eq!(state(&c, id), State::Paid { pr: PR });
}

#[test]
#[should_panic(expected = "does not match the review in flight")]
fn a_stale_review_result_cannot_touch_the_milestone() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    reviewed(&mut c, id, ok(&judge_ai("meets", HEAD)), ok(&verifier_ai("meets", HEAD)));
}

// ---------------------------------------------------------------- D45 partial certification

const PR2: u64 = 10;

fn checker_for(pr: u64, head: &str, merged: bool) -> Value {
    let mut v = checker(merged);
    v["pr_number"] = json!(pr);
    v["head_sha"] = json!(head);
    v
}

fn checked_pr(c: &mut Contract, id: u64, pr: u64, output: Value) {
    set_callback(vec![ok(&output)]);
    c.on_checked(id, pr, None, acc("anyone.testnet"), MIN_CLAIM_DEPOSIT);
}

fn pct_of(pct: u128) -> u128 {
    BUDGET * pct / 100
}

fn both(verdict_j: &str, j: u8, verdict_v: &str, v: u8, head: &str) -> (PromiseResult, PromiseResult) {
    (ok(&ai_pct("judge", verdict_j, head, j)), ok(&ai_pct("verifier", verdict_v, head, v)))
}

/// The payout callback for whatever the milestone is paying out now.
fn payout_lands(c: &mut Contract, id: u64, success: bool) {
    let State::PayingOut { amount, .. } = state(c, id) else { panic!("not paying out: {:?}", state(c, id)) };
    set_callback(vec![if success { ok(&Value::Null) } else { PromiseResult::Failed }]);
    c.on_transferred(id, amount, Transfer::Payout, State::Funded);
}

/// PR 9 merged, reviewed at `j`/`v` percent partial, and paid.
fn paid_partial(c: &mut Contract, j: u8, v: u8) -> u64 {
    let id = passed(c, true);
    let (judge, verifier) = both("partial", j, "partial", v, HEAD);
    full_review(c, id, judge, verifier);
    payout_lands(c, id, true);
    id
}

#[test]
fn a_partial_review_pays_the_lower_share_and_reopens_the_milestone() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    let (judge, verifier) = both("partial", 70, "partial", 60, HEAD);
    full_review(&mut c, id, judge, verifier);
    let part = Part { pr: PR, base_sha: BASE.into(), head_sha: HEAD.into(), pct: 60 };
    assert_eq!(state(&c, id), State::PayingOut { part: part.clone(), amount: U128(pct_of(60)) });
    payout_lands(&mut c, id, true);
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.state, State::Funded);
    assert_eq!((m.budget, m.certified_pct, m.parts), (U128(BUDGET - pct_of(60)), 60, vec![part]));
    assert_eq!(c.get_lock(REPO_ID, PR), Some(PrLock::Spent(id)));
    assert!(c.is_pr_used(id, PR));
    assert_reconciles(&c);
}

#[test]
fn a_later_pr_pays_only_the_new_share_and_100_pays_the_rest() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 60, 60);
    claim(&mut c, id, PR2);
    checked_pr(&mut c, id, PR2, checker_for(PR2, NEW_HEAD, true));
    let (judge, verifier) = both("meets", 100, "meets", 100, NEW_HEAD);
    full_review(&mut c, id, judge, verifier);
    let State::PayingOut { amount, part } = state(&c, id) else { panic!() };
    assert_eq!((amount, part.pct), (U128(BUDGET - pct_of(60)), 100));
    payout_lands(&mut c, id, true);
    let m = c.get_milestone(id).unwrap();
    assert_eq!((m.state, m.budget, m.certified_pct, m.parts.len()), (State::Paid { pr: PR2 }, U128(0), 100, 2));
    assert_reconciles(&c);
}

#[test]
fn the_ais_see_every_earlier_paid_part() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 60, 60);
    let m = c.get_milestone(id).unwrap();
    let input = ai_input(&m, (id, 1), BASE, NEW_HEAD, 3);
    assert_eq!(input["certified_pct"], 60);
    assert_eq!(input["prior"], json!([{ "base_sha": BASE, "head_sha": HEAD }]));
}

#[test]
fn shares_far_apart_hold_for_the_funder() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    let (judge, verifier) = both("meets", 100, "partial", 60, HEAD);
    full_review(&mut c, id, judge, verifier);
    assert!(matches!(state(&c, id), State::Held { reason, .. } if reason == "disagree:100/60"));
}

#[test]
fn a_review_that_adds_nothing_holds() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 60, 60);
    claim(&mut c, id, PR2);
    checked_pr(&mut c, id, PR2, checker_for(PR2, NEW_HEAD, true));
    let (judge, verifier) = both("partial", 60, "partial", 55, NEW_HEAD);
    full_review(&mut c, id, judge, verifier);
    assert!(matches!(state(&c, id), State::Held { reason, .. } if reason == "no_new_progress:55"));
    assert_eq!(c.get_milestone(id).unwrap().budget, U128(BUDGET - pct_of(60)));
}

#[test]
fn a_share_too_small_to_pay_anything_holds() {
    let mut c = new_contract();
    // 1 % of the minimum budget is still 10 000 units, so force a budget where 1 % is 0.
    let id = passed(&mut c, true);
    let m = c.milestones.get_mut(&id).unwrap();
    m.budget_total = U128(50);
    m.budget = U128(50);
    let (judge, verifier) = both("partial", 1, "partial", 1, HEAD);
    full_review(&mut c, id, judge, verifier);
    assert!(matches!(state(&c, id), State::Held { reason, .. } if reason == "no_new_progress:1"));
}

#[test]
fn a_premerge_approval_carries_its_share_and_the_merge_pays_it() {
    let mut c = new_contract();
    let id = passed(&mut c, false);
    let (judge, verifier) = both("partial", 60, "partial", 65, HEAD);
    full_review(&mut c, id, judge, verifier);
    assert_eq!(state(&c, id), State::Approved { pr: PR, head_sha: HEAD.into(), pct: 60 });
    claim(&mut c, id, PR);
    set_callback(vec![ok(&checker(true))]);
    c.on_checked(id, PR, Some((HEAD.into(), 60)), acc("anyone.testnet"), MIN_CLAIM_DEPOSIT);
    let State::PayingOut { amount, part } = state(&c, id) else { panic!("{:?}", state(&c, id)) };
    assert_eq!((amount, part.pct), (U128(pct_of(60)), 60));
}

fn held_partial(c: &mut Contract) -> u64 {
    let id = passed(c, true);
    let (judge, verifier) = both("unclear", 0, "partial", 50, HEAD);
    full_review(c, id, judge, verifier);
    assert!(matches!(state(c, id), State::Held { .. }));
    id
}

#[test]
fn the_funder_releases_a_share_of_their_choosing() {
    let mut c = new_contract();
    let id = held_partial(&mut c);
    set("funder.testnet", NearToken::from_near(0));
    let _ = c.funder_release(id, 40);
    let State::PayingOut { amount, part } = state(&c, id) else { panic!() };
    assert_eq!((amount, part.pct), (U128(pct_of(40)), 40));
    payout_lands(&mut c, id, true);
    assert_eq!(state(&c, id), State::Funded);
    assert_reconciles(&c);
}

#[test]
#[should_panic(expected = "percent must be above the certified share and at most 100")]
fn the_funder_cannot_release_more_than_100() {
    let mut c = new_contract();
    let id = held_partial(&mut c);
    set("funder.testnet", NearToken::from_near(0));
    let _ = c.funder_release(id, 101);
}

#[test]
#[should_panic(expected = "percent must be above the certified share and at most 100")]
fn the_funder_cannot_release_a_share_already_paid() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 60, 60);
    claim(&mut c, id, PR2);
    checked_pr(&mut c, id, PR2, checker_for(PR2, NEW_HEAD, true));
    let (judge, verifier) = both("unclear", 0, "unclear", 0, NEW_HEAD);
    full_review(&mut c, id, judge, verifier);
    set("funder.testnet", NearToken::from_near(0));
    let _ = c.funder_release(id, 60);
}

#[test]
fn after_the_deadline_the_funder_takes_back_the_uncertified_rest() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 60, 60);
    set_at("funder.testnet", DEADLINE_S);
    let _ = c.refund(id);
    let m = c.get_milestone(id).unwrap();
    assert_eq!(m.state, State::Refunded);
    let rest = BUDGET - pct_of(60) + m.fee_remaining.0;
    assert!(logged(&format!("\"amount\":\"{rest}\"")), "{:?}", get_logs());
}

#[test]
#[should_panic(expected = "milestone has its most paid parts")]
fn a_sixth_part_is_refused() {
    let mut c = new_contract();
    let id = paid_partial(&mut c, 10, 10);
    let part = c.get_milestone(id).unwrap().parts[0].clone();
    c.milestones.get_mut(&id).unwrap().parts = vec![part; MAX_PARTS];
    claim(&mut c, id, PR2);
}

#[test]
fn a_failed_partial_payout_retries_the_same_share() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    let (judge, verifier) = both("partial", 60, "partial", 60, HEAD);
    full_review(&mut c, id, judge, verifier);
    payout_lands(&mut c, id, false);
    let State::PayoutFailed { amount, part } = state(&c, id) else { panic!() };
    assert_eq!((amount, part.pct), (U128(pct_of(60)), 60));
    set("anyone.testnet", NearToken::from_near(0));
    let _ = c.retry_payout(id);
    assert!(matches!(state(&c, id), State::PayingOut { amount, .. } if amount == U128(pct_of(60))));
    assert_reconciles(&c);
}

#[test]
fn review_attempts_default_to_10_and_go_up_to_25() {
    let mut c = new_contract();
    let id = create(&mut c, params());
    assert_eq!(c.get_milestone(id).unwrap().max_paid_attempts, 10);
    let id = create(&mut c, CreateMilestone { max_paid_attempts: Some(25), ..params() });
    assert_eq!(c.get_milestone(id).unwrap().max_paid_attempts, 25);
}

#[test]
#[should_panic(expected = "max_paid_attempts out of range")]
fn more_than_25_review_attempts_are_refused() {
    let mut c = new_contract();
    create(&mut c, CreateMilestone { max_paid_attempts: Some(26), ..params() });
}


// ---------------------------------------------------------------- D48: the signed second opinion

fn judge_out(head: &str, verifier: Value) -> Value {
    let mut out = ai_pct("judge", "meets", head, 100);
    out["verifier"] = verifier;
    out
}

fn review_with(c: &mut Contract, id: u64, judge: Value) {
    review(c, id);
    fee_ready(c, id, vec![], FEE, Some(ok(&json!(FEE.to_string()))));
    set_callback(vec![ok(&judge)]);
    c.on_reviewed(id, acc("anyone.testnet"), MIN_AI_DEPOSIT.saturating_mul(2));
}

#[test]
fn a_correctly_signed_verifier_verdict_counts() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review_with(&mut c, id, judge_out(HEAD, signed_claim(id, 1, HEAD, "meets", 100)));
    assert!(matches!(state(&c, id), State::PayingOut { .. }), "{:?}", state(&c, id));
}

#[test]
fn a_verdict_the_judge_changed_is_no_verdict() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    // Signed as partial 60, passed on as meets 100.
    let mut forged = signed_claim(id, 1, HEAD, "partial", 60);
    forged["verdict"] = json!("meets");
    forged["percent_done"] = json!(100);
    review_with(&mut c, id, judge_out(HEAD, forged));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
    assert!(logged("review_retry"));
}

fn replayed_is_no_verdict(replayed: fn(u64) -> Value) {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review_with(&mut c, id, judge_out(HEAD, replayed(id)));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
}

#[test]
fn a_signature_from_another_attempt_is_no_verdict() {
    replayed_is_no_verdict(|id| signed_claim(id, 2, HEAD, "meets", 100));
}

#[test]
fn a_signature_from_another_milestone_is_no_verdict() {
    replayed_is_no_verdict(|id| signed_claim(id + 1, 1, HEAD, "meets", 100));
}

#[test]
fn a_signature_for_another_commit_is_no_verdict() {
    replayed_is_no_verdict(|id| signed_claim(id, 1, NEW_HEAD, "meets", 100));
}

#[test]
fn a_verdict_run_by_anyone_but_the_judges_owner_is_no_verdict() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    let mut other = signed_claim(id, 1, HEAD, "meets", 100);
    other["caller"] = json!("someone-else.testnet");
    review_with(&mut c, id, judge_out(HEAD, other));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
}

#[test]
fn a_verdict_signed_by_another_key_is_no_verdict() {
    let mut c = new_contract();
    let mut p = params();
    // The real signature, checked against a different pinned key.
    p.pins.verifier.public_key = "11".repeat(32);
    let id = create(&mut c, p);
    fund(&mut c, id);
    bind(&mut c, id);
    claim(&mut c, id, PR);
    checked_json(&mut c, id, checker(true));
    review_with(&mut c, id, judge_out(HEAD, signed_claim(id, 1, HEAD, "meets", 100)));
    assert!(matches!(state(&c, id), State::HardFactsPassed { .. }));
}

#[test]
fn a_signed_objection_from_the_verifier_holds() {
    let mut c = new_contract();
    let id = passed(&mut c, true);
    review_with(&mut c, id, judge_out(HEAD, signed_claim(id, 1, HEAD, "unclear", 0)));
    assert!(matches!(state(&c, id), State::Held { reason, .. } if reason == "verifier:unclear"));
}

#[test]
fn no_second_opinion_means_no_payout() {
    replayed_is_no_verdict(|_| Value::Null);
}

#[test]
fn an_unreachable_verifier_means_no_payout() {
    replayed_is_no_verdict(|_| json!({ "error": "verifier_not_completed" }));
}

#[test]
#[should_panic(expected = "bad pins")]
fn create_refuses_a_verifier_key_that_is_not_32_bytes_of_hex() {
    let mut c = new_contract();
    let mut p = params();
    p.pins.verifier.public_key = "ed25519:abc".into();
    create(&mut c, p);
}
