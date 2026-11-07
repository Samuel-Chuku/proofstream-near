//! Holds funded milestones and pays one only when an OutLayer checker, pinned to an
//! exact wasm hash, reports facts that satisfy every rule in `rules`.
//!
//! No owner, admin, setter or upgrade method exists: `new` fixes the configuration.

mod rules;

use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{
    env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, GasWeight, NearToken,
    PanicOnDefault, Promise, PromiseError,
};
pub use rules::{CheckerOutput, Refusal};

const MIN_CLAIM_DEPOSIT: NearToken = NearToken::from_millinear(10);
const REQUEST_MIN_GAS: Gas = Gas::from_tgas(100);
const ON_CHECKED_GAS: Gas = Gas::from_tgas(60);
const FT_TRANSFER_GAS: Gas = Gas::from_tgas(15);
const ON_PAID_GAS: Gas = Gas::from_tgas(10);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);

const MAX_INSTRUCTIONS: u64 = 10_000_000_000;
const MAX_MEMORY_MB: u32 = 128;
const MAX_EXECUTION_SECONDS: u64 = 60;
/// The checker's output is measured at under 500 bytes. A larger result refuses.
const MAX_CHECKER_OUTPUT_BYTES: usize = 4096;

/// The verifier is pinned in code here because adding it to `new` would change stored state.
const VERIFIER_PROJECT: &str = "proofstream-verifier-owner.testnet/proofstream-verifier";
const VERIFIER_VERSION: &str = "9c6eebed7b7f99e68bf239b61083c8c652a630befc97a4eb05cb046ca1efea52";
const VERIFIER_SECRETS_OWNER: &str = "proofstream-verifier-owner.testnet";
const VERIFIER_SECRETS_PROFILE: &str = "default";
/// In the token's minimal units: 0.05 test USDC.
const VERIFIER_FEE: u128 = 50_000;
const MIN_VERIFIER_DEPOSIT: NearToken = NearToken::from_millinear(30);
const VERIFIER_MAX_EXECUTION_SECONDS: u64 = 85;
const VERIFIER_MAX_INSTRUCTIONS: u64 = 100_000_000_000;
const VERIFIER_MAX_MEMORY_MB: u32 = 256;
const FT_TRANSFER_CALL_GAS: Gas = Gas::from_tgas(60);
const ON_FEE_DEPOSITED_GAS: Gas = Gas::from_tgas(150);
const ON_VERIFIED_GAS: Gas = Gas::from_tgas(20);
const MAX_FEE_RESULT_BYTES: usize = 64;

const DEFAULT_MAX_PAID_ATTEMPTS: u32 = 3;
const MAX_PAID_ATTEMPTS_LIMIT: u32 = 10;
const MAX_TEXT_BYTES: usize = 2000;
const MAX_FIELD_BYTES: usize = 100;
const MAX_REQUIRED_CHECKS: usize = 10;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Milestones,
    PrLocks,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Funded,
    Checking,
    Paying,
    Paid,
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrLock {
    Locked(u64),
    Spent(u64),
}

#[near(serializers = [json, borsh])]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub funder: AccountId,
    pub repo_owner: String,
    pub repo: String,
    pub repo_id: u64,
    pub base_branch: String,
    pub payee_github_id: u64,
    /// Spike-only shortcut (D12). The build binds this through the Q3 proof instead.
    pub payee_account: AccountId,
    pub required_checks: Vec<String>,
    pub milestone_text: String,
    pub budget: U128,
    pub fee_remaining: U128,
    pub max_paid_attempts: u32,
    pub created_at_ns: u64,
    pub status: Status,
    pub active_pr: Option<u64>,
}

/// `msg` of the funder's `ft_transfer_call`.
#[derive(Serialize, Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
struct CreateMilestone {
    repo_owner: String,
    repo: String,
    repo_id: u64,
    base_branch: String,
    payee_github_id: u64,
    payee_account: AccountId,
    required_checks: Vec<String>,
    milestone_text: String,
    budget: U128,
    fee: U128,
    max_paid_attempts: Option<u32>,
}

#[near(serializers = [json])]
pub struct Config {
    pub token: AccountId,
    pub outlayer: AccountId,
    pub checker_project: String,
    pub checker_version: String,
    pub secrets_owner: AccountId,
    pub secrets_profile: String,
}

#[ext_contract(ext_ft)]
#[allow(dead_code)]
trait FungibleToken {
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    token: AccountId,
    outlayer: AccountId,
    checker_project: String,
    checker_version: String,
    secrets_owner: AccountId,
    secrets_profile: String,
    next_id: u64,
    milestones: LookupMap<u64, Milestone>,
    pr_locks: LookupMap<String, PrLock>,
}

fn pr_key(repo_id: u64, pr_number: u64) -> String {
    format!("{repo_id}:{pr_number}")
}

#[near]
impl Contract {
    #[init]
    pub fn new(
        token: AccountId,
        outlayer: AccountId,
        checker_project: String,
        checker_version: String,
        secrets_owner: AccountId,
        secrets_profile: String,
    ) -> Self {
        require!(rules::is_sha256_hex(&checker_version), "checker_version must be a wasm sha256");
        Self {
            token,
            outlayer,
            checker_project,
            checker_version,
            secrets_owner,
            secrets_profile,
            next_id: 0,
            milestones: LookupMap::new(StorageKey::Milestones),
            pr_locks: LookupMap::new(StorageKey::PrLocks),
        }
    }

    /// NEP-141 receiver. A panic here makes the token return the whole amount,
    /// so a milestone can't exist unless it's fully funded.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        require!(env::predecessor_account_id() == self.token, "wrong token");
        let m: CreateMilestone = serde_json::from_str(&msg).unwrap_or_else(|_| env::panic_str("bad msg"));

        let total = m.budget.0.checked_add(m.fee.0).unwrap_or_else(|| env::panic_str("overflow"));
        require!(m.budget.0 > 0 && m.fee.0 > 0, "budget and fee must be positive");
        require!(amount.0 == total, "amount must equal budget plus fee");
        require!(!m.required_checks.is_empty(), "required_checks is empty");
        require!(m.required_checks.len() <= MAX_REQUIRED_CHECKS, "too many required_checks");
        require!(m.milestone_text.len() <= MAX_TEXT_BYTES, "milestone_text too long");
        require!(
            [&m.repo_owner, &m.repo, &m.base_branch]
                .iter()
                .chain(m.required_checks.iter().collect::<Vec<_>>().iter())
                .all(|s| !s.is_empty() && s.len() <= MAX_FIELD_BYTES),
            "bad field length"
        );
        let max_paid_attempts = m.max_paid_attempts.unwrap_or(DEFAULT_MAX_PAID_ATTEMPTS);
        require!(
            (1..=MAX_PAID_ATTEMPTS_LIMIT).contains(&max_paid_attempts),
            "max_paid_attempts out of range"
        );

        let id = self.next_id;
        self.next_id += 1;
        self.milestones.insert(
            id,
            Milestone {
                funder: sender_id,
                repo_owner: m.repo_owner,
                repo: m.repo,
                repo_id: m.repo_id,
                base_branch: m.base_branch,
                payee_github_id: m.payee_github_id,
                payee_account: m.payee_account,
                required_checks: m.required_checks,
                milestone_text: m.milestone_text,
                budget: m.budget,
                fee_remaining: m.fee,
                max_paid_attempts,
                created_at_ns: env::block_timestamp(),
                status: Status::Funded,
                active_pr: None,
            },
        );
        env::log_str(&format!("milestone_created id={id}"));
        U128(0)
    }

    /// Anyone may call. The caller pays the checker's NEAR, and gets the unused part back.
    #[payable]
    pub fn claim(&mut self, milestone_id: u64, pr_number: u64) -> Promise {
        let deposit = env::attached_deposit();
        require!(deposit >= MIN_CLAIM_DEPOSIT, "attach at least 0.01 NEAR");
        let caller = env::predecessor_account_id();

        let mut m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        require!(m.status == Status::Funded, "milestone is not claimable");
        let key = pr_key(m.repo_id, pr_number);
        require!(!self.pr_locks.contains_key(&key), "PR is locked or already paid");

        m.status = Status::Checking;
        m.active_pr = Some(pr_number);
        let input = json!({
            "owner": m.repo_owner,
            "repo": m.repo,
            "pr_number": pr_number,
            "required_checks": m.required_checks,
        });
        self.milestones.insert(milestone_id, m);
        self.pr_locks.insert(key, PrLock::Locked(milestone_id));
        env::log_str(&format!("claim_started id={milestone_id} pr={pr_number}"));

        let args = json!({
            "source": { "Project": {
                "project_id": self.checker_project,
                "version_key": self.checker_version,
            }},
            "resource_limits": {
                "max_instructions": MAX_INSTRUCTIONS,
                "max_memory_mb": MAX_MEMORY_MB,
                "max_execution_seconds": MAX_EXECUTION_SECONDS,
            },
            "input_data": input.to_string(),
            "secrets_ref": { "profile": self.secrets_profile, "account_id": self.secrets_owner },
            "response_format": "Json",
            "payer_account_id": caller,
        });

        // OutLayer reserves its own callback gas from this, so unused gas goes to it too.
        Promise::new(self.outlayer.clone())
            .function_call_weight(
                "request_execution".to_string(),
                args.to_string().into_bytes(),
                deposit,
                REQUEST_MIN_GAS,
                GasWeight(1),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_CHECKED_GAS)
                    .on_checked(milestone_id, pr_number, caller, deposit),
            )
    }

    /// Must not panic after the checks at the top: a panic here would leave the lock held.
    #[private]
    pub fn on_checked(&mut self, milestone_id: u64, pr_number: u64, caller: AccountId, deposit: NearToken) {
        let mut m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        if m.status != Status::Checking || m.active_pr != Some(pr_number) {
            env::panic_str("callback does not match the claim in flight");
        }

        let result = env::promise_result_checked(0, MAX_CHECKER_OUTPUT_BYTES);
        // A failed `request_execution` means OutLayer refused at admission, and NEAR
        // returned the deposit to this contract. Every other path refunds through OutLayer.
        if matches!(result, Err(PromiseError::Failed)) {
            let _ = Promise::new(caller).transfer(deposit);
        }
        let verdict = match result {
            Err(PromiseError::Failed) => Err(Refusal::CheckerFailed),
            Err(_) => Err(Refusal::CheckerOutputTooLong),
            Ok(bytes) => match serde_json::from_slice::<CheckerOutput>(&bytes) {
                Err(_) => Err(Refusal::UnreadableOutput),
                Ok(output) => rules::evaluate(
                    &output,
                    pr_number,
                    m.repo_id,
                    &m.base_branch,
                    m.payee_github_id,
                    m.created_at_ns / NANOS_PER_SECOND,
                ),
            },
        };

        if let Err(refusal) = verdict {
            m.status = Status::Funded;
            m.active_pr = None;
            self.pr_locks.remove(&pr_key(m.repo_id, pr_number));
            self.milestones.insert(milestone_id, m);
            env::log_str(&format!("claim_refused id={milestone_id} pr={pr_number} reason={refusal:?}"));
            return;
        }

        m.status = Status::Paying;
        let (payee, budget) = (m.payee_account.clone(), m.budget);
        self.milestones.insert(milestone_id, m);
        env::log_str(&format!("payout_started id={milestone_id} pr={pr_number}"));
        let _ = ext_ft::ext(self.token.clone())
            .with_attached_deposit(ONE_YOCTO)
            .with_static_gas(FT_TRANSFER_GAS)
            .ft_transfer(payee, budget, Some(format!("milestone {milestone_id}")))
            .then(Self::ext(env::current_account_id()).with_static_gas(ON_PAID_GAS).on_paid(milestone_id, pr_number));
    }

    #[private]
    pub fn on_paid(&mut self, milestone_id: u64, pr_number: u64) {
        let mut m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        if m.status != Status::Paying || m.active_pr != Some(pr_number) {
            env::panic_str("callback does not match the payout in flight");
        }
        let key = pr_key(m.repo_id, pr_number);
        if env::promise_result_checked(0, 0).is_ok() {
            m.status = Status::Paid;
            self.pr_locks.insert(key, PrLock::Spent(milestone_id));
            env::log_str(&format!("milestone_paid id={milestone_id} pr={pr_number}"));
        } else {
            m.status = Status::Funded;
            m.active_pr = None;
            self.pr_locks.remove(&key);
            env::log_str(&format!("payout_failed id={milestone_id} pr={pr_number}"));
        }
        self.milestones.insert(milestone_id, m);
    }

    /// Q6: pays the verifier's owner from this milestone's fee, then records its verdict.
    /// Only the milestone's funder can spend its fee. The build calls this from the AI stage,
    /// after the hard facts pass. A verdict is only logged here: it moves no money.
    #[payable]
    pub fn pay_verifier(&mut self, milestone_id: u64, base_sha: String, head_sha: String) -> Promise {
        let deposit = env::attached_deposit();
        require!(deposit >= MIN_VERIFIER_DEPOSIT, "attach at least 0.03 NEAR");
        require!(rules::is_git_sha(&base_sha) && rules::is_git_sha(&head_sha), "bad sha");
        let caller = env::predecessor_account_id();

        let mut m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        require!(caller == m.funder, "only the funder can spend the fee");
        require!(m.fee_remaining.0 >= VERIFIER_FEE, "fee exhausted");
        m.fee_remaining = U128(m.fee_remaining.0 - VERIFIER_FEE);
        self.milestones.insert(milestone_id, m);
        env::log_str(&format!("verifier_fee_spent id={milestone_id} amount={VERIFIER_FEE}"));

        Promise::new(self.token.clone())
            .function_call(
                "ft_transfer_call".to_string(),
                json!({
                    "receiver_id": self.outlayer,
                    "amount": U128(VERIFIER_FEE),
                    "msg": json!({ "action": "deposit_balance" }).to_string(),
                })
                .to_string()
                .into_bytes(),
                ONE_YOCTO,
                FT_TRANSFER_CALL_GAS,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_FEE_DEPOSITED_GAS)
                    .with_unused_gas_weight(1)
                    .on_fee_deposited(milestone_id, base_sha, head_sha, caller, deposit),
            )
    }

    #[private]
    pub fn on_fee_deposited(
        &mut self,
        milestone_id: u64,
        base_sha: String,
        head_sha: String,
        caller: AccountId,
        deposit: NearToken,
    ) {
        let mut m = self.milestones.get(&milestone_id).cloned().unwrap_or_else(|| env::panic_str("no such milestone"));
        let deposited = env::promise_result_checked(0, MAX_FEE_RESULT_BYTES)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<U128>(&bytes).ok())
            .is_some_and(|used| used.0 == VERIFIER_FEE);
        if !deposited {
            m.fee_remaining = U128(m.fee_remaining.0 + VERIFIER_FEE);
            self.milestones.insert(milestone_id, m);
            let _ = Promise::new(caller).transfer(deposit);
            env::log_str(&format!("verifier_fee_restored id={milestone_id}"));
            return;
        }

        let input = json!({
            "owner": m.repo_owner,
            "repo": m.repo,
            "base_sha": base_sha,
            "head_sha": head_sha,
            "milestone_text": m.milestone_text,
        });
        let args = json!({
            "source": { "Project": { "project_id": VERIFIER_PROJECT, "version_key": VERIFIER_VERSION }},
            "resource_limits": {
                "max_instructions": VERIFIER_MAX_INSTRUCTIONS,
                "max_memory_mb": VERIFIER_MAX_MEMORY_MB,
                "max_execution_seconds": VERIFIER_MAX_EXECUTION_SECONDS,
            },
            "input_data": input.to_string(),
            "secrets_ref": { "profile": VERIFIER_SECRETS_PROFILE, "account_id": VERIFIER_SECRETS_OWNER },
            "response_format": "Json",
            "payer_account_id": caller,
            "params": { "attached_usd": U128(VERIFIER_FEE) },
        });
        let _ = Promise::new(self.outlayer.clone())
            .function_call_weight(
                "request_execution".to_string(),
                args.to_string().into_bytes(),
                deposit,
                REQUEST_MIN_GAS,
                GasWeight(1),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(ON_VERIFIED_GAS)
                    .on_verified(milestone_id, head_sha, caller, deposit),
            );
    }

    #[private]
    pub fn on_verified(&mut self, milestone_id: u64, head_sha: String, caller: AccountId, deposit: NearToken) {
        let result = env::promise_result_checked(0, MAX_CHECKER_OUTPUT_BYTES);
        // As in `on_checked`: an admission failure returns the deposit to this contract.
        if matches!(result, Err(PromiseError::Failed)) {
            let _ = Promise::new(caller).transfer(deposit);
        }
        match result.ok().and_then(|bytes| rules::read_verifier(&bytes, &head_sha)) {
            Some(verdict) => env::log_str(&format!("verifier_verdict id={milestone_id} head={head_sha} verdict={verdict}")),
            None => env::log_str(&format!("verifier_unverified id={milestone_id} head={head_sha}")),
        }
    }

    pub fn get_milestone(&self, milestone_id: u64) -> Option<Milestone> {
        self.milestones.get(&milestone_id).cloned()
    }

    pub fn get_pr_lock(&self, repo_id: u64, pr_number: u64) -> Option<PrLock> {
        self.pr_locks.get(&pr_key(repo_id, pr_number)).copied()
    }

    pub fn get_config(&self) -> Config {
        Config {
            token: self.token.clone(),
            outlayer: self.outlayer.clone(),
            checker_project: self.checker_project.clone(),
            checker_version: self.checker_version.clone(),
            secrets_owner: self.secrets_owner.clone(),
            secrets_profile: self.secrets_profile.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
