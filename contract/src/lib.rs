//! ProofStream milestones on NEAR. A funder escrows a budget in a NEP-141 token. It pays
//! the GitHub id holder's bound account only when the hard facts read in an OutLayer TDX
//! enclave pass, and two independent AI reviews (judge and verifier, different models and
//! owners) both say the work meets the milestone. Every enclave build is pinned per
//! milestone by wasm hash (D28).
//!
//! No owner, admin, setter or upgrade method exists. `new` fixes the configuration.

mod events;
mod rules;

use events::emit;
use near_sdk::json_types::U128;
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json, Value};
use near_sdk::store::{LookupMap, LookupSet};
use near_sdk::{
    env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, GasWeight, NearToken, PanicOnDefault,
    Promise, PromiseError, PromiseOrValue,
};
pub use rules::Refusal;
use rules::{AfterCheck, AiAnswer, CheckerOutput, Probe, ReviewOutcome, Terms};

const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);
const NANOS_PER_SECOND: u64 = 1_000_000_000;

// Deposits. OutLayer refunds whatever a run doesn't use to the payer.
const MIN_CLAIM_DEPOSIT: NearToken = NearToken::from_millinear(10);
const MIN_AI_DEPOSIT: NearToken = NearToken::from_millinear(30);
const MIN_BIND_COMPUTE: NearToken = NearToken::from_millinear(10);
/// Token storage for the payout account, as `storage_balance_bounds` reports for the test token.
const TOKEN_STORAGE: NearToken = NearToken::from_micronear(1_250);
/// D29: sent to a 64-hex account, which creates it and pays its first transactions.
const IMPLICIT_PROBE: NearToken = NearToken::from_millinear(50);
/// Charged at creation on top of the milestone's own bytes, for what it grows into later
/// (state data, binding, used PRs, locks, and up to MAX_PARTS paid parts with their lock and
/// used-PR records, ~900 bytes, D45). The excess is never refunded; it's a few mNEAR.
const STORAGE_HEADROOM_BYTES: u64 = 2_500;

// Gas. Requests share unused gas by weight; the figures are the static minimums.
const CHECKER_REQUEST_GAS: Gas = Gas::from_tgas(100);
const AI_REQUEST_GAS: Gas = Gas::from_tgas(60);
const ON_CHECKED_GAS: Gas = Gas::from_tgas(40);
const FEE_DEPOSIT_GAS: Gas = Gas::from_tgas(50);
const ON_FEE_READY_GAS: Gas = Gas::from_tgas(5);
const ON_REVIEWED_GAS: Gas = Gas::from_tgas(50);
const FT_TRANSFER_GAS: Gas = Gas::from_tgas(15);
const ON_TRANSFERRED_GAS: Gas = Gas::from_tgas(10);
const ON_BOUND_GAS: Gas = Gas::from_tgas(60);
const ON_PROBED_GAS: Gas = Gas::from_tgas(40);
const STORAGE_DEPOSIT_GAS: Gas = Gas::from_tgas(10);
const ON_REGISTERED_GAS: Gas = Gas::from_tgas(10);

// Enclave limits (BUILD_BRIEF 2.4).
const FACTS_MAX_SECONDS: u64 = 20;
const FACTS_MAX_INSTRUCTIONS: u64 = 10_000_000_000;
const FACTS_MAX_MEMORY_MB: u32 = 128;
const AI_MAX_SECONDS: u64 = 60;
const AI_MAX_INSTRUCTIONS: u64 = 100_000_000_000;
const AI_MAX_MEMORY_MB: u32 = 256;
const SECRETS_PROFILE: &str = "default";

// Result size caps. The OutLayer output limit is 900 bytes; a larger result refuses.
const MAX_RESULT_BYTES: usize = 1024;
const MAX_FT_RESULT_BYTES: usize = 64;

// Milestone limits.
/// Shared by every PR of the milestone (D45), so several partial PRs need room to retry.
const DEFAULT_MAX_PAID_ATTEMPTS: u32 = 10;
const MAX_PAID_ATTEMPTS_LIMIT: u32 = 25;
const DEFAULT_DIFF_CAP_BYTES: u64 = 100_000;
/// The judge's own `MAX_DIFF_BYTES`: a larger cap could never be reviewed.
const MAX_DIFF_CAP_BYTES: u64 = 180_000;
const MAX_TEXT_BYTES: usize = 2_000;
const MAX_FIELD_BYTES: usize = 100;
const MAX_REQUIRED_CHECKS: usize = 8;
const MAX_REASON_BYTES: usize = 280;
const MAX_GIST_ID_BYTES: usize = 64;
const NONCE_HASH_BYTES: usize = 16;
/// D45: paid parts per milestone; the AI sees every earlier one, so this bounds its input.
const MAX_PARTS: usize = 5;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Milestones,
    PrLocks,
    UsedPrs,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// `owner.testnet/name`. The owner's OutLayer secrets are the ones the run reads.
    pub project_id: String,
    pub wasm_hash: String,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifierPin {
    pub project_id: String,
    pub wasm_hash: String,
    /// Paid to the verifier's owner per review, in the token's minimal units (D22).
    pub fee: U128,
}

/// D28: fixed at creation, never changed for that milestone.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    pub checker: Pin,
    pub binder: Pin,
    pub judge: Pin,
    pub verifier: VerifierPin,
}

/// D45: one PR's certified share of the milestone. `pct` is the milestone's total
/// certified share once this PR is paid, not this PR's own increment.
#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub pr: u64,
    pub base_sha: String,
    pub head_sha: String,
    pub pct: u8,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Funding,
    Funded,
    Checking { pr: u64 },
    HardFactsPassed { pr: u64, base_sha: String, head_sha: String, merged: bool, changed_files: u64 },
    Reviewing { pr: u64, base_sha: String, head_sha: String, merged: bool, changed_files: u64 },
    /// D36: both AIs certified `pct` on the open PR at `head_sha`. Pays once it's merged there.
    Approved { pr: u64, head_sha: String, pct: u8 },
    Held { pr: u64, base_sha: String, head_sha: String, merged: bool, reason: String },
    PayingOut { part: Part, amount: U128 },
    PayoutFailed { part: Part, amount: U128 },
    Paid { pr: u64 },
    Refunded,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub account: AccountId,
    pub gist_id: String,
    pub gist_revision: String,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub funder: AccountId,
    pub repo_id: u64,
    pub repo_owner: String,
    pub repo_name: String,
    pub base_branch: String,
    pub payee_github_id: u64,
    pub milestone_text: String,
    pub required_checks: Vec<String>,
    /// Budget not yet paid out.
    pub budget: U128,
    /// The budget as funded; shares are computed on it (D45).
    pub budget_total: U128,
    /// Share of the milestone certified and paid so far, 0-100 (D45). Only rises.
    pub certified_pct: u8,
    /// The PRs paid so far, oldest first (D45).
    pub parts: Vec<Part>,
    pub fee_budget: U128,
    /// Fee not yet spent on reviews.
    pub fee_remaining: U128,
    /// Fee owned by this milestone that another milestone's held fee now covers (ledger 0.3).
    pub fee_refundable: U128,
    /// This milestone's stablecoin sitting in the contract's OutLayer balance after failed runs.
    pub outlayer_credit: U128,
    pub created_at_ns: u64,
    pub deadline_ns: u64,
    pub max_paid_attempts: u32,
    pub paid_attempts_used: u32,
    pub diff_cap_bytes: u64,
    pub allow_self_merge: bool,
    pub pins: Pins,
    pub payee: Option<Bound>,
    pub binding_in_flight: bool,
    pub state: State,
}

/// Arguments of `create_milestone`.
#[near(serializers = [json])]
#[derive(Debug, Clone)]
pub struct CreateMilestone {
    pub repo_id: u64,
    pub repo_owner: String,
    pub repo_name: String,
    pub base_branch: String,
    pub payee_github_id: u64,
    pub milestone_text: String,
    pub required_checks: Vec<String>,
    pub budget: U128,
    pub fee_budget: U128,
    pub deadline_seconds: u64,
    pub max_paid_attempts: Option<u32>,
    pub diff_cap_bytes: Option<u64>,
    pub allow_self_merge: Option<bool>,
    pub pins: Pins,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct FundMsg {
    milestone_id: u64,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrLock {
    Locked(u64),
    Spent(u64),
}

#[near(serializers = [json])]
pub struct Config {
    pub token: AccountId,
    pub outlayer: AccountId,
    pub min_budget: U128,
    pub min_deadline_seconds: u64,
}

/// `tokens_held` is what this contract has received minus what it has sent, by its own
/// accounting. `owed` is the sum of every milestone's unpaid budget and fees. They must be
/// equal, and both must equal the token's `ft_balance_of(this contract)`.
#[near(serializers = [json])]
pub struct Reconcile {
    pub tokens_held: U128,
    pub owed: U128,
    pub outlayer_credit: U128,
}

/// Why money left. Lets `on_transferred` apply exactly one outcome.
#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Payout,
    Refund,
}

#[ext_contract(ext_ft)]
#[allow(dead_code)]
trait FungibleToken {
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
    fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>, msg: String);
    fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>);
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    token: AccountId,
    outlayer: AccountId,
    min_budget: u128,
    min_deadline_seconds: u64,
    next_id: u64,
    milestones: LookupMap<u64, Milestone>,
    pr_locks: LookupMap<String, PrLock>,
    used_prs: LookupSet<String>,
    /// Milestones with a nonzero `outlayer_credit`, spent oldest first.
    credit_owners: Vec<u64>,
    tokens_held: u128,
    owed: u128,
}

fn pr_key(repo_id: u64, pr: u64) -> String {
    format!("{repo_id}:{pr}")
}

fn used_key(milestone_id: u64, pr: u64) -> String {
    format!("{milestone_id}:{pr}")
}

fn bind_nonce(milestone_id: u64, created_at_ns: u64) -> String {
    let seed = format!("{}:{milestone_id}:{created_at_ns}", env::current_account_id());
    env::sha256(seed.as_bytes())[..NONCE_HASH_BYTES].iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a promise succeeded, whatever it returned. Reading a result with a size cap
/// reports an oversized success as an error, and treating that as a failed payout would
/// let `retry_payout` pay twice.
fn succeeded(index: u64) -> bool {
    !matches!(env::promise_result_checked(index, 0), Err(PromiseError::Failed))
}

/// The AIs' input, from stored state only (D15 rule 2). Earlier paid parts go with it, so
/// redoing paid work earns nothing (D45).
fn ai_input(m: &Milestone, base_sha: &str, head_sha: &str, changed_files: u64) -> Value {
    json!({
        "owner": m.repo_owner,
        "repo": m.repo_name,
        "base_sha": base_sha,
        "head_sha": head_sha,
        "changed_files": changed_files,
        "milestone_text": m.milestone_text,
        "certified_pct": m.certified_pct,
        "prior": m.parts.iter().map(|p| json!({ "base_sha": p.base_sha, "head_sha": p.head_sha })).collect::<Vec<_>>(),
    })
}

/// The PR an Approved or PayoutFailed milestone keeps locked.
fn locked_pr(state: &State) -> Option<u64> {
    match state {
        State::Approved { pr, .. } => Some(*pr),
        State::PayoutFailed { part, .. } => Some(part.pr),
        _ => None,
    }
}

fn valid_pin(project_id: &str, wasm_hash: &str) -> bool {
    rules::project_owner(project_id).is_some() && rules::is_sha256_hex(wasm_hash)
}

fn short_field(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_FIELD_BYTES
}

/// `request_execution` arguments for a pinned project. The input is always built from
/// stored state (D15 rule 2); callers never supply it.
fn request_args(pin: &Pin, input: &Value, ai: bool, payer: &AccountId, attached_usd: Option<u128>) -> Vec<u8> {
    let owner = rules::project_owner(&pin.project_id).unwrap_or_else(|| env::panic_str("bad pin"));
    let (seconds, instructions, memory) = if ai {
        (AI_MAX_SECONDS, AI_MAX_INSTRUCTIONS, AI_MAX_MEMORY_MB)
    } else {
        (FACTS_MAX_SECONDS, FACTS_MAX_INSTRUCTIONS, FACTS_MAX_MEMORY_MB)
    };
    let mut args = json!({
        "source": { "Project": { "project_id": pin.project_id, "version_key": pin.wasm_hash }},
        "resource_limits": {
            "max_instructions": instructions,
            "max_memory_mb": memory,
            "max_execution_seconds": seconds,
        },
        "input_data": input.to_string(),
        "secrets_ref": { "profile": SECRETS_PROFILE, "account_id": owner },
        "response_format": "Json",
        "payer_account_id": payer,
    });
    if let Some(usd) = attached_usd {
        args["params"] = json!({ "attached_usd": U128(usd) });
    }
    args.to_string().into_bytes()
}

#[near]
impl Contract {
    #[init]
    pub fn new(token: AccountId, outlayer: AccountId, min_budget: U128, min_deadline_seconds: u64) -> Self {
        require!(min_budget.0 > 0, "min_budget must be positive");
        Self {
            token,
            outlayer,
            min_budget: min_budget.0,
            min_deadline_seconds,
            next_id: 0,
            milestones: LookupMap::new(StorageKey::Milestones),
            pr_locks: LookupMap::new(StorageKey::PrLocks),
            used_prs: LookupSet::new(StorageKey::UsedPrs),
            credit_owners: Vec::new(),
            tokens_held: 0,
            owed: 0,
        }
    }

    // ---------------------------------------------------------------- create and fund

    /// Payable: the funder pays this milestone's storage. Puts it in Funding; it becomes
    /// Funded when exactly `budget + fee_budget` arrives through `ft_transfer_call`.
    #[payable]
    pub fn create_milestone(&mut self, params: CreateMilestone) -> u64 {
        let p = params;
        let now_ns = env::block_timestamp();
        require!(p.budget.0 >= self.min_budget, "budget below min_budget");
        p.budget.0.checked_add(p.fee_budget.0).unwrap_or_else(|| env::panic_str("overflow"));
        require!(p.fee_budget.0 >= p.pins.verifier.fee.0, "fee_budget must cover at least one review");
        let deadline_ns = p.deadline_seconds.saturating_mul(NANOS_PER_SECOND);
        let min_deadline_ns = now_ns.saturating_add(self.min_deadline_seconds.saturating_mul(NANOS_PER_SECOND));
        require!(deadline_ns >= min_deadline_ns, "deadline too soon");
        require!(!p.milestone_text.is_empty() && p.milestone_text.len() <= MAX_TEXT_BYTES, "bad milestone_text");
        require!(
            !p.required_checks.is_empty() && p.required_checks.len() <= MAX_REQUIRED_CHECKS,
            "required_checks must have 1 to 8 names"
        );
        require!(
            [&p.repo_owner, &p.repo_name, &p.base_branch].iter().all(|s| short_field(s))
                && p.required_checks.iter().all(|s| short_field(s)),
            "bad field length"
        );
        let attempts = p.max_paid_attempts.unwrap_or(DEFAULT_MAX_PAID_ATTEMPTS);
        require!((1..=MAX_PAID_ATTEMPTS_LIMIT).contains(&attempts), "max_paid_attempts out of range");
        let diff_cap = p.diff_cap_bytes.unwrap_or(DEFAULT_DIFF_CAP_BYTES);
        require!((1..=MAX_DIFF_CAP_BYTES).contains(&diff_cap), "diff_cap_bytes out of range");
        let pins = &p.pins;
        require!(
            valid_pin(&pins.checker.project_id, &pins.checker.wasm_hash)
                && valid_pin(&pins.binder.project_id, &pins.binder.wasm_hash)
                && valid_pin(&pins.judge.project_id, &pins.judge.wasm_hash)
                && valid_pin(&pins.verifier.project_id, &pins.verifier.wasm_hash)
                && pins.verifier.fee.0 > 0,
            "bad pins"
        );
        require!(
            rules::independent_ai_pins(&pins.judge.project_id, &pins.verifier.project_id),
            "judge and verifier pins must have different owners and projects"
        );

        let before = env::storage_usage();
        let id = self.next_id;
        self.next_id += 1;
        let funder = env::predecessor_account_id();
        self.milestones.insert(
            id,
            Milestone {
                funder: funder.clone(),
                repo_id: p.repo_id,
                repo_owner: p.repo_owner,
                repo_name: p.repo_name,
                base_branch: p.base_branch,
                payee_github_id: p.payee_github_id,
                milestone_text: p.milestone_text,
                required_checks: p.required_checks,
                budget: p.budget,
                budget_total: p.budget,
                certified_pct: 0,
                parts: Vec::new(),
                fee_budget: p.fee_budget,
                fee_remaining: p.fee_budget,
                fee_refundable: U128(0),
                outlayer_credit: U128(0),
                created_at_ns: now_ns,
                deadline_ns,
                max_paid_attempts: attempts,
                paid_attempts_used: 0,
                diff_cap_bytes: diff_cap,
                allow_self_merge: p.allow_self_merge.unwrap_or(false),
                pins: p.pins,
                payee: None,
                binding_in_flight: false,
                state: State::Funding,
            },
        );
        self.milestones.flush();
        let bytes = env::storage_usage().saturating_sub(before) + STORAGE_HEADROOM_BYTES;
        let cost = env::storage_byte_cost().saturating_mul(u128::from(bytes));
        let deposit = env::attached_deposit();
        require!(deposit >= cost, format!("attach at least {} yoctoNEAR for storage", cost.as_yoctonear()));
        let change = deposit.saturating_sub(cost);
        if !change.is_zero() {
            let _ = Promise::new(funder.clone()).transfer(change);
        }
        let m = &self.milestones[&id];
        emit(
            "milestone_created",
            json!({
                "milestone_id": id, "funder": funder, "repo_id": m.repo_id,
                "repo": format!("{}/{}", m.repo_owner, m.repo_name), "base_branch": m.base_branch,
                "payee_github_id": m.payee_github_id, "budget": m.budget, "fee_budget": m.fee_budget,
                "deadline_ns": U128(u128::from(m.deadline_ns)), "allow_self_merge": m.allow_self_merge,
                "pins": m.pins,
            }),
        );
        id
    }

    /// NEP-141 receiver. Only the configured token, only the milestone's funder, only the
    /// exact total while Funding. Anything else is returned in full.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        if env::predecessor_account_id() != self.token {
            env::panic_str("wrong token");
        }
        let Some(id) = serde_json::from_str::<FundMsg>(&msg).ok().map(|f| f.milestone_id) else {
            emit("funding_refused", json!({ "reason": "bad_msg", "sender": sender_id }));
            return amount;
        };
        let Some(m) = self.milestones.get_mut(&id) else {
            emit("funding_refused", json!({ "milestone_id": id, "reason": "no_milestone" }));
            return amount;
        };
        let total = m.budget.0 + m.fee_budget.0;
        let reason = if m.state != State::Funding {
            Some("not_funding")
        } else if sender_id != m.funder {
            Some("not_the_funder")
        } else if amount.0 != total {
            Some("wrong_amount")
        } else {
            None
        };
        if let Some(reason) = reason {
            emit("funding_refused", json!({ "milestone_id": id, "reason": reason, "amount": amount }));
            return amount;
        }
        m.state = State::Funded;
        self.tokens_held += total;
        self.owed += total;
        emit("milestone_funded", json!({ "milestone_id": id, "amount": amount }));
        U128(0)
    }

    // ---------------------------------------------------------------- bind the payee

    /// Anyone can submit: the gist proves the choice, not the caller (D19). The caller pays
    /// NEAR for the binder run, the D29 probe and the payee's token storage.
    #[payable]
    pub fn bind_payee(&mut self, milestone_id: u64, gist_id: String) -> Promise {
        let reserve = TOKEN_STORAGE.saturating_add(IMPLICIT_PROBE);
        let deposit = env::attached_deposit();
        require!(deposit >= reserve.saturating_add(MIN_BIND_COMPUTE), "attach at least 0.06125 NEAR");
        require!(
            !gist_id.is_empty()
                && gist_id.len() <= MAX_GIST_ID_BYTES
                && gist_id.chars().all(|c| c.is_ascii_alphanumeric()),
            "bad gist id"
        );
        let m = self.milestone_mut(milestone_id);
        require!(matches!(m.state, State::Funding | State::Funded), "milestone is not bindable");
        require!(m.payee.is_none(), "already bound");
        require!(!m.binding_in_flight, "a binding is in flight");
        m.binding_in_flight = true;
        let input = json!({
            "gist_id": gist_id,
            "contract": env::current_account_id(),
            "milestone_id": milestone_id,
            "nonce": bind_nonce(milestone_id, m.created_at_ns),
        });
        let caller = env::predecessor_account_id();
        let compute = deposit.saturating_sub(reserve);
        let args = request_args(&m.pins.binder, &input, false, &caller, None);
        emit("bind_started", json!({ "milestone_id": milestone_id, "gist_id": gist_id }));
        Promise::new(self.outlayer.clone())
            .function_call_weight("request_execution", args, compute, CHECKER_REQUEST_GAS, GasWeight(1))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_BOUND_GAS)
                    .on_bound(milestone_id, gist_id, caller, compute, reserve),
            )
    }

    #[private]
    pub fn on_bound(
        &mut self,
        milestone_id: u64,
        gist_id: String,
        caller: AccountId,
        compute: NearToken,
        reserve: NearToken,
    ) {
        let result = env::promise_result_checked(0, MAX_RESULT_BYTES);
        let contract = env::current_account_id();
        let m = self.milestone_mut(milestone_id);
        // Admission failed: NEAR returned the compute deposit here, not to the caller.
        let admission_failed = matches!(result, Err(PromiseError::Failed));
        let outcome = result.map_err(|_| Refusal::CheckerFailed).and_then(|bytes| {
            rules::evaluate_binding(
                &bytes,
                contract.as_str(),
                milestone_id,
                &bind_nonce(milestone_id, m.created_at_ns),
                m.payee_github_id,
            )
        });
        let outcome = outcome.and_then(|b| rules::probe_for(b.account.as_str()).map(|probe| (b, probe)));
        match outcome {
            Err(refusal) => {
                m.binding_in_flight = false;
                let back = if admission_failed { reserve.saturating_add(compute) } else { reserve };
                let _ = Promise::new(caller).transfer(back);
                emit("bind_refused", json!({ "milestone_id": milestone_id, "reason": format!("{refusal:?}") }));
            }
            Ok((binding, probe)) => {
                // D29: prove the account exists (or create it) before anything is recorded.
                let amount = match probe {
                    Probe::Named => ONE_YOCTO,
                    Probe::Implicit => IMPLICIT_PROBE,
                };
                let leftover = reserve.saturating_sub(TOKEN_STORAGE).saturating_sub(amount);
                let _ = Promise::new(binding.account.clone()).transfer(amount).then(
                    Self::ext(contract)
                        .with_static_gas(ON_PROBED_GAS)
                        .on_probed(milestone_id, binding.account, gist_id, binding.revision, caller, leftover),
                );
            }
        }
    }

    #[private]
    pub fn on_probed(
        &mut self,
        milestone_id: u64,
        account: AccountId,
        gist_id: String,
        revision: String,
        caller: AccountId,
        leftover: NearToken,
    ) {
        let probed = succeeded(0);
        let m = self.milestone_mut(milestone_id);
        if !probed {
            // A transfer to a missing named account fails and NEAR refunds it here.
            m.binding_in_flight = false;
            let _ = Promise::new(caller).transfer(leftover.saturating_add(TOKEN_STORAGE).saturating_add(ONE_YOCTO));
            emit("bind_refused", json!({ "milestone_id": milestone_id, "reason": "AccountMissing", "account": account }));
            return;
        }
        if !leftover.is_zero() {
            let _ = Promise::new(caller.clone()).transfer(leftover);
        }
        // Only now: `storage_deposit` succeeds even for a missing account (the D23 strand).
        let _ = ext_ft::ext(self.token.clone())
            .with_attached_deposit(TOKEN_STORAGE)
            .with_static_gas(STORAGE_DEPOSIT_GAS)
            .storage_deposit(Some(account.clone()), Some(true))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_REGISTERED_GAS)
                    .on_registered(milestone_id, account, gist_id, revision),
            );
    }

    #[private]
    pub fn on_registered(&mut self, milestone_id: u64, account: AccountId, gist_id: String, revision: String) {
        let ok = succeeded(0);
        let m = self.milestone_mut(milestone_id);
        m.binding_in_flight = false;
        if !ok {
            emit("bind_refused", json!({ "milestone_id": milestone_id, "reason": "StorageFailed" }));
            return;
        }
        if m.payee.is_some() || !matches!(m.state, State::Funding | State::Funded) {
            emit("bind_refused", json!({ "milestone_id": milestone_id, "reason": "WrongState" }));
            return;
        }
        emit(
            "payee_bound",
            json!({ "milestone_id": milestone_id, "account": account, "gist_id": gist_id, "gist_revision": revision }),
        );
        m.payee = Some(Bound { account, gist_id, gist_revision: revision });
    }

    // ---------------------------------------------------------------- claim: hard facts

    /// Anyone may call. The caller pays the checker's NEAR and gets the unused part back.
    /// From Funded it takes any unused PR; from Approved, only the approved PR (D36).
    #[payable]
    pub fn claim(&mut self, milestone_id: u64, pr_number: u64) -> Promise {
        let deposit = env::attached_deposit();
        require!(deposit >= MIN_CLAIM_DEPOSIT, "attach at least 0.01 NEAR");
        let caller = env::predecessor_account_id();
        let key = {
            let m = self.milestone(milestone_id);
            require!(m.payee.is_some(), "payee is not bound yet");
            require!(m.parts.len() < MAX_PARTS, "milestone has its most paid parts");
            pr_key(m.repo_id, pr_number)
        };
        let m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        match &m.state {
            State::Funded => {
                require!(!self.pr_locks.contains_key(&key), "PR is locked or already paid");
                require!(!self.used_prs.contains(&used_key(milestone_id, pr_number)), "PR was used on this milestone");
                self.pr_locks.insert(key, PrLock::Locked(milestone_id));
            }
            State::Approved { pr, .. } => require!(*pr == pr_number, "only the approved PR can be claimed"),
            _ => env::panic_str("milestone is not claimable"),
        }
        let input = json!({
            "owner": m.repo_owner,
            "repo": m.repo_name,
            "pr_number": pr_number,
            "required_checks": m.required_checks,
        });
        let args = request_args(&m.pins.checker, &input, false, &caller, None);
        let from_approved = matches!(m.state, State::Approved { .. });
        let approved = match &m.state {
            State::Approved { head_sha, pct, .. } => Some((head_sha.clone(), *pct)),
            _ => None,
        };
        self.milestone_mut(milestone_id).state = State::Checking { pr: pr_number };
        emit("claim_started", json!({ "milestone_id": milestone_id, "pr": pr_number, "from_approved": from_approved }));
        Promise::new(self.outlayer.clone())
            .function_call_weight("request_execution", args, deposit, CHECKER_REQUEST_GAS, GasWeight(1))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_CHECKED_GAS)
                    .on_checked(milestone_id, pr_number, approved, caller, deposit),
            )
    }

    /// Must not panic after the stage check: a panic would leave the lock held.
    #[private]
    pub fn on_checked(
        &mut self,
        milestone_id: u64,
        pr_number: u64,
        approved: Option<(String, u8)>,
        caller: AccountId,
        deposit: NearToken,
    ) {
        let m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        if m.state != (State::Checking { pr: pr_number }) {
            env::panic_str("callback does not match the claim in flight");
        }
        let result = env::promise_result_checked(0, MAX_RESULT_BYTES);
        if matches!(result, Err(PromiseError::Failed)) {
            let _ = Promise::new(caller).transfer(deposit);
        }
        let terms = Terms {
            pr_number,
            repo_id: m.repo_id,
            base_branch: &m.base_branch,
            payee_github_id: m.payee_github_id,
            created_at_seconds: m.created_at_ns / NANOS_PER_SECOND,
            allow_self_merge: m.allow_self_merge,
        };
        let facts = match result {
            Err(_) => Err(Refusal::CheckerFailed),
            Ok(bytes) => match serde_json::from_slice::<CheckerOutput>(&bytes) {
                Err(_) => Err(Refusal::UnreadableOutput),
                Ok(output) => rules::evaluate(&output, &terms),
            },
        };
        let back_to = match &approved {
            Some((head, pct)) => State::Approved { pr: pr_number, head_sha: head.clone(), pct: *pct },
            None => State::Funded,
        };
        let facts = match facts {
            Ok(f) => f,
            Err(refusal) => {
                self.refuse_claim(milestone_id, pr_number, back_to, &format!("{refusal:?}"));
                return;
            }
        };
        match rules::after_check(&facts, approved.as_ref().map(|(head, _)| head.as_str()), m.diff_cap_bytes) {
            AfterCheck::StillOpen => {
                self.refuse_claim(milestone_id, pr_number, back_to, "NotMerged");
            }
            AfterCheck::PayNow => {
                emit("hard_facts_passed", json!({ "milestone_id": milestone_id, "pr": pr_number, "head_sha": facts.head_sha, "merged": true, "approved_premerge": true }));
                let pct = approved.map_or(0, |(_, pct)| pct);
                let part = Part { pr: pr_number, base_sha: facts.base_sha, head_sha: facts.head_sha, pct };
                self.certify(milestone_id, part, true);
            }
            AfterCheck::TooLarge => {
                let reason = format!("too_large:{}", facts.diff_bytes);
                self.hold(milestone_id, pr_number, facts.base_sha, facts.head_sha, facts.merged, reason);
            }
            AfterCheck::Review => {
                emit("hard_facts_passed", json!({ "milestone_id": milestone_id, "pr": pr_number, "head_sha": facts.head_sha, "merged": facts.merged }));
                self.milestone_mut(milestone_id).state = State::HardFactsPassed {
                    pr: pr_number,
                    base_sha: facts.base_sha,
                    head_sha: facts.head_sha,
                    merged: facts.merged,
                    changed_files: facts.changed_files,
                };
            }
        }
    }

    // ---------------------------------------------------------------- review: both AIs

    /// D33: anyone may call once the hard facts pass. The attached NEAR is split between the
    /// judge and the verifier runs; unused NEAR goes back to the caller. The verifier's fee
    /// comes from this milestone's fee budget, using pooled OutLayer credit first (0.3).
    #[payable]
    pub fn review(&mut self, milestone_id: u64) -> Promise {
        let deposit = env::attached_deposit();
        require!(deposit >= MIN_AI_DEPOSIT.saturating_mul(2), "attach at least 0.06 NEAR");
        let caller = env::predecessor_account_id();
        let m = self.milestone(milestone_id).clone();
        let State::HardFactsPassed { pr, base_sha, head_sha, merged, changed_files } = m.state.clone() else {
            env::panic_str("milestone is not ready for review");
        };
        let fee = m.pins.verifier.fee.0;
        require!(m.paid_attempts_used < m.max_paid_attempts, "no paid attempts left");
        require!(m.fee_remaining.0 >= fee, "fee exhausted");

        let taken = self.take_credit(fee);
        let credited: u128 = taken.iter().map(|(_, a)| a.0).sum();
        let m = self.milestone_mut(milestone_id);
        m.paid_attempts_used += 1;
        m.fee_remaining = U128(m.fee_remaining.0 - fee);
        m.state = State::Reviewing { pr, base_sha, head_sha, merged, changed_files };
        // The fee left this milestone; `credited` of it is still in the contract, now owed
        // to the credit owners (moved in `take_credit`), so only the new deposit leaves.
        let new_deposit = fee - credited;
        self.owed -= new_deposit;
        self.tokens_held -= new_deposit;
        let attempt = self.milestone(milestone_id).paid_attempts_used;
        emit("review_started", json!({ "milestone_id": milestone_id, "pr": pr, "attempt": attempt, "fee": U128(fee), "from_credit": U128(credited) }));

        let next = Self::ext(env::current_account_id())
            .with_static_gas(ON_FEE_READY_GAS)
            .with_unused_gas_weight(1)
            .on_fee_ready(milestone_id, caller, deposit, taken, U128(new_deposit));
        if new_deposit == 0 {
            return next;
        }
        // Weight 0: the deposit runs on its static gas, and all unused gas goes to
        // `on_fee_ready`, which must fund both AI runs and `on_reviewed` (measured in sandbox:
        // an even split left it ~120 Tgas for ~170 Tgas of calls).
        ext_ft::ext(self.token.clone())
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FEE_DEPOSIT_GAS)
            .with_unused_gas_weight(0)
            .ft_transfer_call(
                self.outlayer.clone(),
                U128(new_deposit),
                None,
                json!({ "action": "deposit_balance" }).to_string(),
            )
            .then(next)
    }

    /// Starts both AI runs once the fee sits in this contract's OutLayer balance.
    #[private]
    pub fn on_fee_ready(
        &mut self,
        milestone_id: u64,
        caller: AccountId,
        deposit: NearToken,
        taken: Vec<(u64, U128)>,
        new_deposit: U128,
    ) {
        let m = self.milestone(milestone_id).clone();
        let State::Reviewing { pr, base_sha, head_sha, merged, changed_files } = m.state.clone() else {
            env::panic_str("callback does not match the review in flight");
        };
        let deposited = new_deposit.0 == 0
            || env::promise_result_checked(0, MAX_FT_RESULT_BYTES)
                .ok()
                .and_then(|b| serde_json::from_slice::<U128>(&b).ok())
                .is_some_and(|used| used == new_deposit);
        if !deposited {
            // The token refunded the deposit: undo the attempt, the fee and the credit move.
            let fee = m.pins.verifier.fee.0;
            self.restore_credit(&taken);
            let m = self.milestone_mut(milestone_id);
            m.paid_attempts_used -= 1;
            m.fee_remaining = U128(m.fee_remaining.0 + fee);
            m.state = State::HardFactsPassed { pr, base_sha, head_sha, merged, changed_files };
            self.owed += new_deposit.0;
            self.tokens_held += new_deposit.0;
            let _ = Promise::new(caller).transfer(deposit);
            emit("review_refused", json!({ "milestone_id": milestone_id, "reason": "FeeDepositFailed" }));
            return;
        }
        let input = ai_input(&m, &base_sha, &head_sha, changed_files);
        let half = NearToken::from_yoctonear(deposit.as_yoctonear() / 2);
        let verifier_pin = Pin { project_id: m.pins.verifier.project_id.clone(), wasm_hash: m.pins.verifier.wasm_hash.clone() };
        let judge = Promise::new(self.outlayer.clone()).function_call_weight(
            "request_execution",
            request_args(&m.pins.judge, &input, true, &caller, None),
            half,
            AI_REQUEST_GAS,
            GasWeight(1),
        );
        let verifier = Promise::new(self.outlayer.clone()).function_call_weight(
            "request_execution",
            request_args(&verifier_pin, &input, true, &caller, Some(m.pins.verifier.fee.0)),
            deposit.saturating_sub(half),
            AI_REQUEST_GAS,
            GasWeight(1),
        );
        let _ = judge.and(verifier).then(
            Self::ext(env::current_account_id())
                .with_static_gas(ON_REVIEWED_GAS)
                .on_reviewed(milestone_id, caller, half, deposit.saturating_sub(half)),
        );
    }

    #[private]
    pub fn on_reviewed(
        &mut self,
        milestone_id: u64,
        caller: AccountId,
        judge_deposit: NearToken,
        verifier_deposit: NearToken,
    ) {
        let m = self.milestone(milestone_id).clone();
        let State::Reviewing { pr, base_sha, head_sha, merged, changed_files } = m.state.clone() else {
            env::panic_str("callback does not match the review in flight");
        };
        let judge_raw = env::promise_result_checked(0, MAX_RESULT_BYTES);
        let verifier_raw = env::promise_result_checked(1, MAX_RESULT_BYTES);
        // Admission failures return NEAR here; OutLayer refunds every other path itself.
        let mut back = NearToken::from_yoctonear(0);
        if matches!(judge_raw, Err(PromiseError::Failed)) {
            back = back.saturating_add(judge_deposit);
        }
        if matches!(verifier_raw, Err(PromiseError::Failed)) {
            back = back.saturating_add(verifier_deposit);
        }
        if !back.is_zero() {
            let _ = Promise::new(caller).transfer(back);
        }
        // OutLayer returns `null` for a failed or timed-out run and puts the fee back in this
        // contract's balance; a refused admission never took it. Either way it's credit.
        let fee_back = match &verifier_raw {
            Err(PromiseError::Failed) => true,
            Ok(bytes) => bytes.as_slice() == b"null",
            Err(_) => false,
        };
        if fee_back {
            self.add_credit(milestone_id, m.pins.verifier.fee.0);
        }
        let read = |r: &Result<Vec<u8>, PromiseError>, role: &str| match r {
            Ok(bytes) => rules::read_ai(bytes, &head_sha, role),
            Err(_) => AiAnswer::Failed,
        };
        let (judge, verifier) = (read(&judge_raw, "judge"), read(&verifier_raw, "verifier"));
        emit(
            "review_result",
            json!({ "milestone_id": milestone_id, "pr": pr, "head_sha": head_sha,
                    "judge": format!("{judge:?}"), "verifier": format!("{verifier:?}"), "fee_credited": fee_back }),
        );
        match rules::combine(&judge, &verifier, m.certified_pct) {
            ReviewOutcome::Certify(pct) => {
                let part = Part { pr, base_sha, head_sha, pct };
                self.certify(milestone_id, part, merged);
            }
            ReviewOutcome::Held(reason) => self.hold(milestone_id, pr, base_sha, head_sha, merged, reason),
            ReviewOutcome::Retry => {
                let fee = m.pins.verifier.fee.0;
                let (used, max) = (m.paid_attempts_used, m.max_paid_attempts);
                if used >= max || m.fee_remaining.0 < fee {
                    self.hold(milestone_id, pr, base_sha, head_sha, merged, "attempts_exhausted".into());
                } else {
                    self.milestone_mut(milestone_id).state = State::HardFactsPassed { pr, base_sha, head_sha, merged, changed_files };
                    emit("review_retry", json!({ "milestone_id": milestone_id, "pr": pr, "attempts_left": max - used }));
                }
            }
        }
    }

    // ---------------------------------------------------------------- funder decisions

    /// Held can only be released (pay the bound payee) or rejected (0.2). The funder sets
    /// the milestone's new certified total, above what is already certified (D45).
    pub fn funder_release(&mut self, milestone_id: u64, percent: u8) -> Promise {
        let m = self.milestone(milestone_id).clone();
        require!(env::predecessor_account_id() == m.funder, "only the funder");
        let State::Held { pr, base_sha, head_sha, .. } = m.state else { env::panic_str("milestone is not held") };
        require!(percent > m.certified_pct && percent <= rules::FULL_PCT, "percent must be above the certified share and at most 100");
        let part = Part { pr, base_sha, head_sha, pct: percent };
        let amount = rules::share_to_pay(m.budget_total.0, m.budget.0, percent)
            .unwrap_or_else(|| env::panic_str("that share pays nothing"));
        emit("funder_released", json!({ "milestone_id": milestone_id, "pr": pr, "pct": percent }));
        self.begin_payout(milestone_id, part, amount)
    }

    /// Held or Approved back to Funded. The PR can't be used on this milestone again.
    pub fn funder_reject(&mut self, milestone_id: u64, reason: String) {
        require!(!reason.is_empty() && reason.len() <= MAX_REASON_BYTES, "reason must be 1 to 280 bytes");
        let m = self.milestone(milestone_id);
        require!(env::predecessor_account_id() == m.funder, "only the funder");
        let (pr, approved) = match &m.state {
            State::Held { pr, .. } => (*pr, false),
            State::Approved { pr, .. } => (*pr, true),
            _ => env::panic_str("milestone is not held or approved"),
        };
        let repo_id = m.repo_id;
        self.pr_locks.remove(&pr_key(repo_id, pr));
        self.used_prs.insert(used_key(milestone_id, pr));
        self.milestone_mut(milestone_id).state = State::Funded;
        emit("funder_rejected", json!({ "milestone_id": milestone_id, "pr": pr, "reason": reason, "was_approved_premerge": approved }));
    }

    /// The funder, after the deadline, from Funding, Funded, PayoutFailed or Approved (0.2,
    /// D36). Returns the unpaid budget, unspent fee and the fee others now cover for this
    /// milestone. A Paid milestone can refund only its leftover fee, at any time.
    pub fn refund(&mut self, milestone_id: u64) -> PromiseOrValue<()> {
        let m = self.milestone(milestone_id).clone();
        require!(env::predecessor_account_id() == m.funder, "only the funder");
        let after_deadline = env::block_timestamp() >= m.deadline_ns;
        let budget = match &m.state {
            State::Funding => {
                // Nothing was deposited: close it without a transfer.
                require!(after_deadline, "deadline has not passed");
                self.milestone_mut(milestone_id).state = State::Refunded;
                emit("milestone_refunded", json!({ "milestone_id": milestone_id, "amount": U128(0) }));
                return PromiseOrValue::Value(());
            }
            State::Funded | State::PayoutFailed { .. } | State::Approved { .. } => {
                require!(after_deadline, "deadline has not passed");
                m.budget.0
            }
            State::Paid { .. } => 0,
            _ => env::panic_str("milestone cannot be refunded in this state"),
        };
        let amount = budget + m.fee_remaining.0 + m.fee_refundable.0;
        require!(amount > 0, "nothing to refund");
        if let State::Approved { pr, head_sha, .. } = &m.state {
            emit("refund_after_approval", json!({ "milestone_id": milestone_id, "pr": pr, "head_sha": head_sha }));
        }
        if let Some(pr) = locked_pr(&m.state) {
            self.pr_locks.remove(&pr_key(m.repo_id, pr));
        }
        let was = m.state.clone();
        let m = self.milestone_mut(milestone_id);
        if !matches!(was, State::Paid { .. }) {
            m.state = State::Refunded;
        }
        let funder = m.funder.clone();
        emit("refund_started", json!({ "milestone_id": milestone_id, "amount": U128(amount) }));
        PromiseOrValue::Promise(self.send(milestone_id, funder, amount, Transfer::Refund, was))
    }

    /// Anyone may retry a failed payout.
    pub fn retry_payout(&mut self, milestone_id: u64) -> Promise {
        let State::PayoutFailed { part, amount } = self.milestone(milestone_id).state.clone() else {
            env::panic_str("payout has not failed");
        };
        self.begin_payout(milestone_id, part, amount.0)
    }

    #[private]
    pub fn on_transferred(&mut self, milestone_id: u64, amount: U128, kind: Transfer, was: State) {
        let ok = succeeded(0);
        let m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        match (kind, ok) {
            (Transfer::Payout, true) => {
                let State::PayingOut { part, .. } = m.state else { env::panic_str("not paying out") };
                self.tokens_held -= amount.0;
                self.owed -= amount.0;
                let (pr, pct) = (part.pr, part.pct);
                self.pr_locks.insert(pr_key(m.repo_id, pr), PrLock::Spent(milestone_id));
                self.used_prs.insert(used_key(milestone_id, pr));
                let mm = self.milestone_mut(milestone_id);
                mm.budget = U128(mm.budget.0 - amount.0);
                mm.certified_pct = pct;
                mm.parts.push(part);
                // D45: a partial share leaves the rest open for the next PR.
                mm.state = if pct == rules::FULL_PCT { State::Paid { pr } } else { State::Funded };
                emit("milestone_paid", json!({ "milestone_id": milestone_id, "pr": pr, "amount": amount, "pct": pct, "to": m.payee.map(|p| p.account) }));
            }
            (Transfer::Payout, false) => {
                let State::PayingOut { part, amount } = m.state else { env::panic_str("not paying out") };
                emit("payout_failed", json!({ "milestone_id": milestone_id, "pr": part.pr }));
                self.milestone_mut(milestone_id).state = State::PayoutFailed { part, amount };
            }
            (Transfer::Refund, true) => {
                self.tokens_held -= amount.0;
                self.owed -= amount.0;
                let mm = self.milestone_mut(milestone_id);
                mm.budget = U128(0);
                mm.fee_remaining = U128(0);
                mm.fee_refundable = U128(0);
                emit("milestone_refunded", json!({ "milestone_id": milestone_id, "amount": amount }));
            }
            (Transfer::Refund, false) => {
                // Put the milestone back as it was, so the funder can try again.
                if let Some(pr) = locked_pr(&was) {
                    self.pr_locks.insert(pr_key(m.repo_id, pr), PrLock::Locked(milestone_id));
                }
                self.milestone_mut(milestone_id).state = was;
                emit("refund_failed", json!({ "milestone_id": milestone_id }));
            }
        }
    }

    // ---------------------------------------------------------------- views

    pub fn get_milestone(&self, milestone_id: u64) -> Option<Milestone> {
        self.milestones.get(&milestone_id).cloned()
    }

    pub fn get_bind_nonce(&self, milestone_id: u64) -> Option<String> {
        self.milestones.get(&milestone_id).map(|m| bind_nonce(milestone_id, m.created_at_ns))
    }

    pub fn get_lock(&self, repo_id: u64, pr_number: u64) -> Option<PrLock> {
        self.pr_locks.get(&pr_key(repo_id, pr_number)).copied()
    }

    pub fn is_pr_used(&self, milestone_id: u64, pr_number: u64) -> bool {
        self.used_prs.contains(&used_key(milestone_id, pr_number))
    }

    pub fn get_config(&self) -> Config {
        Config {
            token: self.token.clone(),
            outlayer: self.outlayer.clone(),
            min_budget: U128(self.min_budget),
            min_deadline_seconds: self.min_deadline_seconds,
        }
    }

    pub fn reconcile(&self) -> Reconcile {
        let credit = self.credit_owners.iter().filter_map(|id| self.milestones.get(id)).map(|m| m.outlayer_credit.0).sum();
        Reconcile { tokens_held: U128(self.tokens_held), owed: U128(self.owed), outlayer_credit: U128(credit) }
    }
}

// ---------------------------------------------------------------- internals

impl Contract {
    fn milestone(&self, id: u64) -> &Milestone {
        self.milestones.get(&id).unwrap_or_else(|| env::panic_str("no such milestone"))
    }

    fn milestone_mut(&mut self, id: u64) -> &mut Milestone {
        self.milestones.get_mut(&id).unwrap_or_else(|| env::panic_str("no such milestone"))
    }

    fn refuse_claim(&mut self, milestone_id: u64, pr: u64, back_to: State, reason: &str) {
        let approved = matches!(back_to, State::Approved { .. });
        let repo_id = self.milestone(milestone_id).repo_id;
        // An Approved milestone keeps its PR locked; a Funded one releases it.
        if !approved {
            self.pr_locks.remove(&pr_key(repo_id, pr));
        }
        self.milestone_mut(milestone_id).state = back_to;
        emit("claim_refused", json!({ "milestone_id": milestone_id, "pr": pr, "reason": reason, "still_approved": approved }));
    }

    fn hold(&mut self, milestone_id: u64, pr: u64, base_sha: String, head_sha: String, merged: bool, reason: String) {
        emit("held", json!({ "milestone_id": milestone_id, "pr": pr, "head_sha": head_sha, "reason": reason }));
        self.milestone_mut(milestone_id).state = State::Held { pr, base_sha, head_sha, merged, reason };
    }

    /// Both AIs certified `part.pct` (D45). Merged: pay the new share. Open: wait for the
    /// merge at this head (D36). A share that pays nothing goes to the funder.
    fn certify(&mut self, milestone_id: u64, part: Part, merged: bool) {
        let m = self.milestone(milestone_id);
        let Some(amount) = rules::share_to_pay(m.budget_total.0, m.budget.0, part.pct) else {
            let reason = format!("no_new_progress:{}", part.pct);
            self.hold(milestone_id, part.pr, part.base_sha, part.head_sha, merged, reason);
            return;
        };
        emit("certified", json!({ "milestone_id": milestone_id, "pr": part.pr, "head_sha": part.head_sha, "pct": part.pct, "merged": merged }));
        if merged {
            let _ = self.begin_payout(milestone_id, part, amount);
        } else {
            emit("verdict_approved_premerge", json!({ "milestone_id": milestone_id, "pr": part.pr, "head_sha": part.head_sha, "pct": part.pct }));
            self.milestone_mut(milestone_id).state = State::Approved { pr: part.pr, head_sha: part.head_sha, pct: part.pct };
        }
    }

    fn begin_payout(&mut self, milestone_id: u64, part: Part, amount: u128) -> Promise {
        let m = self.milestone(milestone_id).clone();
        let payee = m.payee.clone().unwrap_or_else(|| env::panic_str("payee is not bound")).account;
        let was = m.state.clone();
        emit("payout_started", json!({ "milestone_id": milestone_id, "pr": part.pr, "pct": part.pct, "to": payee, "amount": U128(amount) }));
        self.milestone_mut(milestone_id).state = State::PayingOut { part, amount: U128(amount) };
        self.send(milestone_id, payee, amount, Transfer::Payout, was)
    }

    fn send(&self, milestone_id: u64, to: AccountId, amount: u128, kind: Transfer, was: State) -> Promise {
        ext_ft::ext(self.token.clone())
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FT_TRANSFER_GAS)
            .ft_transfer(to, U128(amount), Some(format!("proofstream milestone {milestone_id}")))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_TRANSFERRED_GAS)
                    .on_transferred(milestone_id, U128(amount), kind, was),
            )
    }

    /// Ledger 0.3: spend pooled OutLayer credit first. Each owner's credit becomes fee it can
    /// refund, paid for by the held fee of the milestone now spending it.
    fn take_credit(&mut self, need: u128) -> Vec<(u64, U128)> {
        let mut taken = Vec::new();
        let mut left = need;
        let mut i = 0;
        while left > 0 && i < self.credit_owners.len() {
            let owner = self.credit_owners[i];
            let m = self.milestone_mut(owner);
            let take = m.outlayer_credit.0.min(left);
            m.outlayer_credit = U128(m.outlayer_credit.0 - take);
            m.fee_refundable = U128(m.fee_refundable.0 + take);
            left -= take;
            taken.push((owner, U128(take)));
            if self.milestone(owner).outlayer_credit.0 == 0 {
                self.credit_owners.remove(i);
            } else {
                i += 1;
            }
        }
        taken
    }

    fn restore_credit(&mut self, taken: &[(u64, U128)]) {
        for (owner, amount) in taken {
            let m = self.milestone_mut(*owner);
            m.outlayer_credit = U128(m.outlayer_credit.0 + amount.0);
            m.fee_refundable = U128(m.fee_refundable.0 - amount.0);
            if !self.credit_owners.contains(owner) {
                self.credit_owners.push(*owner);
            }
        }
    }

    /// The fee went back to the OutLayer balance: it's no longer in the contract, and this
    /// milestone owns it as credit until another review spends it.
    fn add_credit(&mut self, milestone_id: u64, amount: u128) {
        let m = self.milestone_mut(milestone_id);
        m.outlayer_credit = U128(m.outlayer_credit.0 + amount);
        if !self.credit_owners.contains(&milestone_id) {
            self.credit_owners.push(milestone_id);
        }
        emit("fee_credited", json!({ "milestone_id": milestone_id, "amount": U128(amount) }));
    }
}

#[cfg(test)]
mod tests;
