//! Sandbox tests: the milestone contract on a local NEAR node, talking to a mock OutLayer
//! (`tests/mocks/outlayer`) and a mock NEP-141 token (`tests/mocks/ft`) through real
//! cross-contract promises. The unit tests cover each rule; these cover what only real
//! receipts show: token movements, deposits at OutLayer, callbacks, gas, and that the
//! contract's ledger matches both counterparties after every flow.
//!
//! Run: `cargo test --test sandbox` (builds the three wasm files with `cargo near` first).
//! near-workspaces' RPC client links OpenSSL; without `pkg-config`, point it at the system
//! library: `OPENSSL_LIB_DIR=/usr/lib/x86_64-linux-gnu OPENSSL_INCLUDE_DIR=/usr/include`.

use near_workspaces::network::Sandbox;
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::{Gas, NearToken};
use near_workspaces::{Account, Contract, Worker};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

const BUDGET: u128 = 10_000_000;
const FEE: u128 = 20_000;
const FEE_BUDGET: u128 = 60_000;
const SUPPLY: u128 = 1_000_000_000_000;
const PAYEE_GITHUB_ID: u64 = 34_985_055;
const REPO_ID: u64 = 1_085_000_000;
const PR: u64 = 7;
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const REVISION: &str = "dddddddddddddddddddddddddddddddddddddddd";
const HASH: &str = "f011de2ce02906146b31fc24690c9f1a5969d9f3b2856dad5a0671d4d2097d68";
const CHECKER: &str = "checker-owner.testnet/proofstream-checker";
const BINDER: &str = "checker-owner.testnet/proofstream-binder";
const JUDGE: &str = "checker-owner.testnet/proofstream-judge";
const VERIFIER: &str = "verifier-owner.testnet/proofstream-verifier";
const MIN_DEADLINE_S: u64 = 60;
const DEADLINE_AFTER_S: u64 = 120;
const MAX_GAS: Gas = Gas::from_tgas(300);
const NANOS: u64 = 1_000_000_000;

struct Wasm {
    milestones: Vec<u8>,
    outlayer: Vec<u8>,
    ft: Vec<u8>,
}

fn build(dir: &str, name: &str) -> Vec<u8> {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let status = Command::new("cargo")
        .args(["near", "build", "non-reproducible-wasm", "--no-abi"])
        .current_dir(&crate_dir)
        .status()
        .unwrap_or_else(|e| panic!("cargo near build in {dir}: {e}"));
    assert!(status.success(), "cargo near build failed in {dir}");
    std::fs::read(crate_dir.join("target/near").join(format!("{name}.wasm"))).unwrap_or_else(|e| panic!("{name}.wasm: {e}"))
}

fn wasm() -> &'static Wasm {
    static WASM: OnceLock<Wasm> = OnceLock::new();
    WASM.get_or_init(|| Wasm {
        milestones: build(".", "proofstream_milestones"),
        outlayer: build("tests/mocks/outlayer", "mock_outlayer"),
        ft: build("tests/mocks/ft", "mock_ft"),
    })
}

struct Env {
    worker: Worker<Sandbox>,
    funder: Account,
    payee: Account,
    relayer: Account,
    ft: Contract,
    outlayer: Contract,
    ps: Contract,
}

fn ok(result: ExecutionFinalResult, what: &str) -> ExecutionFinalResult {
    assert!(result.is_success(), "{what} failed: {:?}", result.failures());
    result
}

fn pass(result: ExecutionFinalResult, what: &str) {
    let _ = ok(result, what);
}

async fn setup() -> Env {
    let w = wasm();
    let worker = near_workspaces::sandbox().await.unwrap();
    let root = worker.root_account().unwrap();
    let mut accounts = Vec::new();
    for name in ["funder", "payee", "relayer"] {
        let a = root.create_subaccount(name).initial_balance(NearToken::from_near(50)).transact().await.unwrap().into_result().unwrap();
        accounts.push(a);
    }
    let (funder, payee, relayer) = (accounts[0].clone(), accounts[1].clone(), accounts[2].clone());

    let ft = worker.dev_deploy(&w.ft).await.unwrap();
    pass(ft.call("new").args_json(json!({ "owner_id": funder.id(), "total_supply": SUPPLY.to_string() })).transact().await.unwrap(), "ft new");
    let outlayer = worker.dev_deploy(&w.outlayer).await.unwrap();
    pass(outlayer.call("new").args_json(json!({ "token": ft.id() })).transact().await.unwrap(), "outlayer new");
    // A fixed name: the verifier's signed verdict names the contract (D48), and the test
    // signatures in tests/fixtures are made for this one.
    let ps_account = root.create_subaccount("ps").initial_balance(NearToken::from_near(50)).transact().await.unwrap().into_result().unwrap();
    let ps = ps_account.deploy(&w.milestones).await.unwrap().into_result().unwrap();
    pass(
        ps.call("new")
            .args_json(json!({ "token": ft.id(), "outlayer": outlayer.id(), "min_budget": "1000000", "min_deadline_seconds": MIN_DEADLINE_S }))
            .transact()
            .await
            .unwrap(),
        "milestones new",
    );
    // The contract and OutLayer hold the token, so both are registered with it.
    for holder in [ps.id(), outlayer.id()] {
        pass(
            funder
                .call(ft.id(), "storage_deposit")
                .args_json(json!({ "account_id": holder }))
                .deposit(NearToken::from_millinear(10))
                .transact()
                .await
                .unwrap(),
            "storage_deposit",
        );
    }
    Env { worker, funder, payee, relayer, ft, outlayer, ps }
}

async fn now_s(env: &Env) -> u64 {
    env.worker.view_block().await.unwrap().timestamp() / NANOS
}

fn pins() -> Value {
    json!({
        "checker": { "project_id": CHECKER, "wasm_hash": HASH },
        "binder": { "project_id": BINDER, "wasm_hash": HASH },
        "judge": { "project_id": JUDGE, "wasm_hash": HASH },
        "verifier": { "project_id": VERIFIER, "wasm_hash": HASH, "fee": FEE.to_string(), "public_key": VERIFIER_KEY },
    })
}

async fn create(env: &Env) -> u64 {
    let deadline = now_s(env).await + DEADLINE_AFTER_S;
    let params = json!({
        "repo_id": REPO_ID, "repo_owner": "Samuel-Chuku", "repo_name": "proofstream-test", "base_branch": "main",
        "payee_github_id": PAYEE_GITHUB_ID, "milestone_text": "Add a function that returns a greeting string.",
        "required_checks": ["test"], "budget": BUDGET.to_string(), "fee_budget": FEE_BUDGET.to_string(),
        "deadline_seconds": deadline, "max_paid_attempts": null, "diff_cap_bytes": null, "allow_self_merge": null,
        "pins": pins(),
    });
    let r = env.funder.call(env.ps.id(), "create_milestone").args_json(json!({ "params": params }))
        .deposit(NearToken::from_near(1)).gas(MAX_GAS).transact().await.unwrap();
    ok(r, "create_milestone").json::<u64>().unwrap()
}

async fn fund_with(env: &Env, id: u64, amount: u128) -> ExecutionFinalResult {
    env.funder
        .call(env.ft.id(), "ft_transfer_call")
        .args_json(json!({ "receiver_id": env.ps.id(), "amount": amount.to_string(), "msg": json!({ "milestone_id": id }).to_string() }))
        .deposit(NearToken::from_yoctonear(1))
        .gas(MAX_GAS)
        .transact()
        .await
        .unwrap()
}

async fn script(env: &Env, project: &str, s: Value) {
    pass(env.relayer.call(env.outlayer.id(), "set_script").args_json(json!({ "project_id": project, "script": s })).transact().await.unwrap(), "set_script");
}

fn output(v: Value) -> Value {
    json!({ "Output": v.to_string() })
}

async fn bind_to(env: &Env, id: u64, account: &str) -> ExecutionFinalResult {
    let nonce: String = env.ps.view("get_bind_nonce").args_json(json!({ "milestone_id": id })).await.unwrap().json().unwrap();
    let out = json!({ "owner_id": PAYEE_GITHUB_ID, "contract": env.ps.id(), "milestone_id": id, "nonce": nonce, "account": account, "revision": REVISION });
    script(env, BINDER, output(out)).await;
    env.relayer.call(env.ps.id(), "bind_payee").args_json(json!({ "milestone_id": id, "gist_id": "a1b2c3" }))
        .deposit(NearToken::from_millinear(100)).gas(MAX_GAS).transact().await.unwrap()
}

/// Created, funded exactly, and bound to `payee`.
async fn ready(env: &Env) -> u64 {
    let id = create(env).await;
    pass(fund_with(env, id, BUDGET + FEE_BUDGET).await, "fund");
    pass(bind_to(env, id, env.payee.id().as_str()).await, "bind");
    assert!(milestone(env, id).await["payee"]["account"] == env.payee.id().as_str(), "payee bound");
    id
}

fn checker(merged: bool, head: &str) -> Value {
    json!({
        "repo_id": REPO_ID, "pr_number": PR, "merged": merged, "base_sha": BASE, "head_sha": head,
        "author_id": PAYEE_GITHUB_ID,
        "merged_by": if merged { json!({ "id": 41_898_282, "type": "Bot" }) } else { Value::Null },
        "independent_approval": true, "base_branch": "main", "checks_conclusion": "success",
        "files_complete": true, "touches_github": false,
        "merged_at": if merged { json!("2099-01-01T00:00:00Z") } else { Value::Null },
        "diff_bytes": 1_000, "diff_too_large": false, "changed_files": 2,
    })
}

async fn claim(env: &Env, id: u64, checker_out: Value) -> ExecutionFinalResult {
    script(env, CHECKER, output(checker_out)).await;
    env.relayer.call(env.ps.id(), "claim").args_json(json!({ "milestone_id": id, "pr_number": PR }))
        .deposit(NearToken::from_millinear(50)).gas(MAX_GAS).transact().await.unwrap()
}

fn ai_pct(role: &str, verdict: &str, head: &str, pct: u8) -> Value {
    output(json!({ "status": "verified", "role": role, "head_sha": head, "verdict": verdict, "percent_done": pct }))
}

fn ai(role: &str, verdict: &str, head: &str) -> Value {
    ai_pct(role, verdict, head, 100)
}

/// D48: the verifier's test key and the judge pin's owner, who pays for its run.
const VERIFIER_KEY: &str = "f5c3e2136054b44445215634ef19540c9cae7d22199d2b6595d12063dca403fe";
const JUDGE_OWNER: &str = "checker-owner.testnet";
const SIGS: &str = include_str!("fixtures/verifier-sigs.json");

/// A real ed25519 signature by the test key (tests/fixtures/sign-verifier.mjs). A message
/// not there yet is written down for that script to sign.
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

/// The judge's run as it would come back: its own answer, carrying the verifier's verdict
/// signed for this contract, milestone and attempt when the verifier gave one. A verifier
/// script of `Null` or `Refuse` means the judge couldn't get a second opinion.
async fn review(env: &Env, id: u64, judge: Value, verifier: Value) -> ExecutionFinalResult {
    let attempt = milestone(env, id).await["paid_attempts_used"].as_u64().unwrap() + 1;
    let judge = match judge["Output"].as_str().map(|s| serde_json::from_str::<Value>(s).unwrap()) {
        Some(mut out) => {
            out["verifier"] = match verifier["Output"].as_str().map(|s| serde_json::from_str::<Value>(s).unwrap()) {
                Some(v) => {
                    let (verdict, pct) = (v["verdict"].as_str().unwrap().to_string(), v["percent_done"].as_u64().unwrap());
                    let message = format!("proofstream-verdict:v1:{}:{id}:{attempt}:{}:{verdict}:{pct}:{JUDGE_OWNER}", env.ps.id(), v["head_sha"].as_str().unwrap());
                    json!({ "verdict": verdict, "percent_done": pct, "caller": JUDGE_OWNER, "signature": sig_for(&message), "call_id": "call-1" })
                }
                None => json!({ "error": "verifier_not_completed" }),
            };
            output(out)
        }
        None => judge,
    };
    script(env, JUDGE, judge).await;
    env.relayer.call(env.ps.id(), "review").args_json(json!({ "milestone_id": id }))
        .deposit(NearToken::from_millinear(100)).gas(MAX_GAS).transact().await.unwrap()
}

async fn milestone(env: &Env, id: u64) -> Value {
    env.ps.view("get_milestone").args_json(json!({ "milestone_id": id })).await.unwrap().json().unwrap()
}

/// The state's variant name, e.g. `Funded` or `Held`.
async fn state(env: &Env, id: u64) -> String {
    match &milestone(env, id).await["state"] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.keys().next().unwrap().clone(),
        other => panic!("odd state {other}"),
    }
}

async fn balance(env: &Env, account: &str) -> u128 {
    let v: String = env.ft.view("ft_balance_of").args_json(json!({ "account_id": account })).await.unwrap().json().unwrap();
    v.parse().unwrap()
}

async fn outlayer_balance(env: &Env) -> u128 {
    let v: String = env.outlayer.view("stable_balance").args_json(json!({ "account_id": env.ps.id() })).await.unwrap().json().unwrap();
    v.parse().unwrap()
}

async fn ai_calls(env: &Env) -> usize {
    let calls: Vec<String> = env.outlayer.view("get_calls").await.unwrap().json().unwrap();
    calls.iter().filter(|c| c.contains("proofstream-judge") || c.contains("proofstream-verifier")).count()
}

/// The ledger matches both counterparties: what the token says the contract holds, and
/// what OutLayer says is sitting in the contract's stablecoin balance.
async fn assert_reconciles(env: &Env) {
    let r: Value = env.ps.view("reconcile").await.unwrap().json().unwrap();
    let field = |k: &str| r[k].as_str().unwrap().parse::<u128>().unwrap();
    assert_eq!(field("tokens_held"), field("owed"), "tokens held = owed: {r}");
    assert_eq!(field("tokens_held"), balance(env, env.ps.id().as_str()).await, "ledger = token balance: {r}");
    assert_eq!(field("outlayer_credit"), outlayer_balance(env).await, "credit = OutLayer balance: {r}");
}

fn logged(result: &ExecutionFinalResult, event: &str) -> bool {
    result.logs().iter().any(|l| l.contains(&format!("\"event\":\"{event}\"")))
}

// ---------------------------------------------------------------- funding

#[tokio::test]
async fn exact_funding_is_accepted_and_any_other_amount_comes_back() {
    let env = setup().await;
    let id = create(&env).await;
    let before = balance(&env, env.funder.id().as_str()).await;

    let r = ok(fund_with(&env, id, BUDGET + FEE_BUDGET + 1).await, "overfund");
    assert!(logged(&r, "funding_refused"));
    assert_eq!(balance(&env, env.funder.id().as_str()).await, before, "the whole amount came back");
    assert_eq!(state(&env, id).await, "Funding");

    pass(fund_with(&env, id, BUDGET + FEE_BUDGET).await, "fund");
    assert_eq!(state(&env, id).await, "Funded");
    assert_eq!(balance(&env, env.ps.id().as_str()).await, BUDGET + FEE_BUDGET);
    assert_reconciles(&env).await;
}

// ---------------------------------------------------------------- binding

#[tokio::test]
async fn binding_probes_the_account_and_registers_it_with_the_token() {
    let env = setup().await;
    let id = create(&env).await;
    let r = ok(bind_to(&env, id, env.payee.id().as_str()).await, "bind");
    assert!(logged(&r, "payee_bound"), "{:?}", r.logs());
    let m = milestone(&env, id).await;
    assert_eq!(m["payee"]["gist_revision"], REVISION);
    let reg: Value = env.ft.view("storage_balance_of").args_json(json!({ "account_id": env.payee.id() })).await.unwrap().json().unwrap();
    assert!(!reg.is_null(), "payee is registered with the token");
}

#[tokio::test]
async fn binding_a_named_account_that_does_not_exist_is_refused() {
    let env = setup().await;
    let id = create(&env).await;
    let r = ok(bind_to(&env, id, "nobody-here.test.near").await, "bind tx");
    assert!(logged(&r, "bind_refused"), "{:?}", r.logs());
    let m = milestone(&env, id).await;
    assert!(m["payee"].is_null());
    assert_eq!(m["binding_in_flight"], false);
}

#[tokio::test]
async fn binding_an_implicit_account_creates_it_before_binding() {
    let env = setup().await;
    let id = create(&env).await;
    let implicit = "7e57".repeat(16);
    let r = ok(bind_to(&env, id, &implicit).await, "bind");
    assert!(logged(&r, "payee_bound"), "{:?}", r.logs());
    let account: near_workspaces::AccountId = implicit.parse().unwrap();
    let details = env.worker.view_account(&account).await.unwrap();
    assert_eq!(details.balance, NearToken::from_millinear(50), "D29: the probe created it with 0.05 NEAR");
    assert_eq!(milestone(&env, id).await["payee"]["account"], implicit);
}

// ---------------------------------------------------------------- the happy path

#[tokio::test]
async fn merged_pr_with_both_ais_meeting_pays_the_payee() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    assert_eq!(state(&env, id).await, "HardFactsPassed");

    let r = ok(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    println!("review gas burnt: {} Tgas", r.total_gas_burnt.as_tgas());
    assert_eq!(state(&env, id).await, "Paid");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, BUDGET);
    // The fee went to OutLayer and was spent at admission.
    assert_eq!(balance(&env, env.outlayer.id().as_str()).await, FEE);
    assert_eq!(outlayer_balance(&env).await, 0);
    let lock: Value = env.ps.view("get_lock").args_json(json!({ "repo_id": REPO_ID, "pr_number": PR })).await.unwrap().json().unwrap();
    assert_eq!(lock, json!({ "Spent": id }));
    assert_reconciles(&env).await;
}

#[tokio::test]
async fn the_one_ai_call_is_the_judges_paid_its_fee_with_its_own_owners_secrets() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    pass(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    let calls: Vec<String> = env.outlayer.view("get_calls").await.unwrap().json().unwrap();
    let ai: Vec<Value> = calls.iter().map(|c| serde_json::from_str::<Value>(c).unwrap())
        .filter(|c| [JUDGE, VERIFIER].contains(&c["source"]["Project"]["project_id"].as_str().unwrap())).collect();
    assert_eq!(ai.len(), 1, "D48: the judge asks the verifier itself");
    let call = &ai[0];
    assert_eq!(call["source"]["Project"]["project_id"], JUDGE);
    assert_eq!(call["secrets_ref"]["account_id"], JUDGE_OWNER);
    let input: Value = serde_json::from_str(call["input_data"].as_str().unwrap()).unwrap();
    assert_eq!(input["changed_files"], 2);
    assert_eq!(input["verifier"]["project_id"], VERIFIER);
    assert_eq!(call["params"]["attached_usd"], FEE.to_string(), "the judge is paid the second opinion's fee");
}

// ---------------------------------------------------------------- objections and the funder

#[tokio::test]
async fn one_objection_holds_and_the_funder_can_release() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    pass(review(&env, id, ai("judge", "does_not_meet", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    assert_eq!(state(&env, id).await, "Held");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, 0);

    let stranger = env.relayer.call(env.ps.id(), "funder_release").args_json(json!({ "milestone_id": id, "percent": 100 })).gas(MAX_GAS).transact().await.unwrap();
    assert!(stranger.is_failure(), "only the funder releases");

    pass(env.funder.call(env.ps.id(), "funder_release").args_json(json!({ "milestone_id": id, "percent": 100 })).gas(MAX_GAS).transact().await.unwrap(), "release");
    assert_eq!(state(&env, id).await, "Paid");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, BUDGET);
    assert_reconciles(&env).await;
}

#[tokio::test]
async fn a_partial_review_pays_the_lower_share_through_the_real_token() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    pass(review(&env, id, ai_pct("judge", "partial", HEAD, 70), ai_pct("verifier", "partial", HEAD, 60)).await, "review");
    assert_eq!(state(&env, id).await, "Funded");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, BUDGET * 60 / 100);
    let m = milestone(&env, id).await;
    assert_eq!(m["certified_pct"], 60);
    assert_eq!(m["budget"], (BUDGET - BUDGET * 60 / 100).to_string());
    assert_reconciles(&env).await;
}

#[tokio::test]
async fn a_rejected_pr_cannot_be_claimed_again_on_that_milestone() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    pass(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "unclear", HEAD)).await, "review");
    assert_eq!(state(&env, id).await, "Held");
    pass(env.funder.call(env.ps.id(), "funder_reject").args_json(json!({ "milestone_id": id, "reason": "not what we asked for" }))
        .gas(MAX_GAS).transact().await.unwrap(), "reject");
    assert_eq!(state(&env, id).await, "Funded");
    let again = claim(&env, id, checker(true, HEAD)).await;
    assert!(again.is_failure(), "the same PR is refused");
    assert_reconciles(&env).await;
}

// ---------------------------------------------------------------- fee ledger at OutLayer

#[tokio::test]
async fn a_failed_judge_run_leaves_credit_that_the_next_review_spends() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");

    let r = ok(review(&env, id, json!("Null"), ai("verifier", "meets", HEAD)).await, "review 1");
    assert!(logged(&r, "review_retry"), "{:?}", r.logs());
    assert_eq!(state(&env, id).await, "HardFactsPassed");
    assert_eq!(outlayer_balance(&env).await, FEE, "the fee came back to the contract's OutLayer balance");
    assert_reconciles(&env).await;

    let token_at_outlayer = balance(&env, env.outlayer.id().as_str()).await;
    pass(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review 2");
    assert_eq!(state(&env, id).await, "Paid");
    assert_eq!(balance(&env, env.outlayer.id().as_str()).await, token_at_outlayer, "no new deposit: the credit paid");
    assert_eq!(outlayer_balance(&env).await, 0);
    let m = milestone(&env, id).await;
    assert_eq!(m["fee_remaining"], (FEE_BUDGET - 2 * FEE).to_string());
    assert_reconciles(&env).await;
}

#[tokio::test]
async fn a_refused_admission_also_leaves_the_fee_as_credit() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    let r = ok(review(&env, id, json!("Refuse"), ai("verifier", "meets", HEAD)).await, "review");
    assert!(logged(&r, "review_retry"), "{:?}", r.logs());
    assert_eq!(outlayer_balance(&env).await, FEE);
    assert_reconciles(&env).await;
}

// ---------------------------------------------------------------- D36: approval before merge

#[tokio::test]
async fn approval_on_an_open_pr_pays_once_merged_at_that_head_without_a_second_review() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(false, HEAD)).await, "claim open");
    let r = ok(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    assert!(logged(&r, "verdict_approved_premerge"));
    assert_eq!(state(&env, id).await, "Approved");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, 0, "no money moves before the merge");

    pass(claim(&env, id, checker(false, HEAD)).await, "claim still open");
    assert_eq!(state(&env, id).await, "Approved");

    let calls_before = ai_calls(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim merged");
    assert_eq!(state(&env, id).await, "Paid");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, BUDGET);
    assert_eq!(ai_calls(&env).await, calls_before, "no second review");
    assert_reconciles(&env).await;
}

#[tokio::test]
async fn new_commits_after_approval_mean_a_fresh_review() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(false, HEAD)).await, "claim open");
    pass(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    pass(claim(&env, id, checker(true, NEW_HEAD)).await, "claim merged at a new head");
    assert_eq!(state(&env, id).await, "HardFactsPassed");
    assert_eq!(milestone(&env, id).await["state"]["HardFactsPassed"]["head_sha"], NEW_HEAD);
}

// ---------------------------------------------------------------- payout failure

#[tokio::test]
async fn a_failed_payout_can_be_retried_and_pays_exactly_once() {
    let env = setup().await;
    let id = ready(&env).await;
    pass(claim(&env, id, checker(true, HEAD)).await, "claim");
    pass(env.relayer.call(env.ft.id(), "set_transfers_fail").args_json(json!({ "fail": true })).transact().await.unwrap(), "switch off");
    pass(review(&env, id, ai("judge", "meets", HEAD), ai("verifier", "meets", HEAD)).await, "review");
    assert_eq!(state(&env, id).await, "PayoutFailed");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, 0);
    assert_reconciles(&env).await;

    pass(env.relayer.call(env.ft.id(), "set_transfers_fail").args_json(json!({ "fail": false })).transact().await.unwrap(), "switch on");
    pass(env.relayer.call(env.ps.id(), "retry_payout").args_json(json!({ "milestone_id": id })).gas(MAX_GAS).transact().await.unwrap(), "retry");
    assert_eq!(state(&env, id).await, "Paid");
    let again = env.relayer.call(env.ps.id(), "retry_payout").args_json(json!({ "milestone_id": id })).gas(MAX_GAS).transact().await.unwrap();
    assert!(again.is_failure(), "a paid milestone can't be paid again");
    assert_eq!(balance(&env, env.payee.id().as_str()).await, BUDGET);
    assert_reconciles(&env).await;
}

// ---------------------------------------------------------------- refusals and refunds

#[tokio::test]
async fn a_refused_claim_releases_the_lock() {
    let env = setup().await;
    let id = ready(&env).await;
    let mut wrong = checker(true, HEAD);
    wrong["author_id"] = json!(1);
    let r = ok(claim(&env, id, wrong).await, "claim");
    assert!(logged(&r, "claim_refused"), "{:?}", r.logs());
    assert_eq!(state(&env, id).await, "Funded");
    let lock: Value = env.ps.view("get_lock").args_json(json!({ "repo_id": REPO_ID, "pr_number": PR })).await.unwrap().json().unwrap();
    assert!(lock.is_null());
}

#[tokio::test]
async fn refund_waits_for_the_deadline_then_returns_everything_unspent() {
    let env = setup().await;
    let id = ready(&env).await;
    let before = balance(&env, env.funder.id().as_str()).await;
    let early = env.funder.call(env.ps.id(), "refund").args_json(json!({ "milestone_id": id })).gas(MAX_GAS).transact().await.unwrap();
    assert!(early.is_failure(), "not before the deadline");

    let deadline_s = milestone(&env, id).await["deadline_ns"].as_u64().unwrap() / NANOS;
    while now_s(&env).await <= deadline_s {
        env.worker.fast_forward(200).await.unwrap();
    }
    pass(env.funder.call(env.ps.id(), "refund").args_json(json!({ "milestone_id": id })).gas(MAX_GAS).transact().await.unwrap(), "refund");
    assert_eq!(state(&env, id).await, "Refunded");
    assert_eq!(balance(&env, env.funder.id().as_str()).await, before + BUDGET + FEE_BUDGET);
    assert_reconciles(&env).await;
}
