//! Test double for `outlayer.testnet`, for sandbox tests only. It returns a scripted
//! output per project instead of running an enclave, and keeps a stablecoin balance the way
//! OutLayer's contract does (read from out-layer/outlayer a3a94e3d, 2026-11-08):
//! `deposit_balance` through `ft_transfer_call` credits the sender; `attached_usd` is
//! debited at admission and refused when the balance is short; a failed run (`null`)
//! credits it back to the sender.

use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, Value};
use near_sdk::store::{IterableMap, LookupMap};
use near_sdk::{env, near, AccountId, PanicOnDefault};

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub enum Script {
    /// Returned as the run's output, byte for byte.
    Output(String),
    /// A failed or timed-out run: the result is `null` and any `attached_usd` comes back.
    Null,
    /// Admission refused: the call panics, so the caller's promise fails.
    Refuse,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockOutlayer {
    token: AccountId,
    scripts: LookupMap<String, Script>,
    balances: IterableMap<AccountId, u128>,
    calls: Vec<String>,
}

#[near]
impl MockOutlayer {
    #[init]
    pub fn new(token: AccountId) -> Self {
        Self { token, scripts: LookupMap::new(b"s"), balances: IterableMap::new(b"b"), calls: Vec::new() }
    }

    /// What the next runs of `project_id` return, until set again.
    pub fn set_script(&mut self, project_id: String, script: Script) {
        self.scripts.insert(project_id, script);
    }

    /// The argument JSON of every `request_execution` so far, oldest first.
    pub fn get_calls(&self) -> Vec<String> {
        self.calls.clone()
    }

    pub fn stable_balance(&self, account_id: AccountId) -> U128 {
        U128(self.balances.get(&account_id).copied().unwrap_or(0))
    }

    #[payable]
    pub fn request_execution(&mut self) {
        let raw = env::input().unwrap_or_default();
        let args: Value = serde_json::from_slice(&raw).unwrap_or_else(|_| env::panic_str("bad args"));
        let project = args["source"]["Project"]["project_id"].as_str().unwrap_or_else(|| env::panic_str("no project"));
        let script = self.scripts.get(project).cloned().unwrap_or_else(|| env::panic_str("no script for project"));
        if matches!(script, Script::Refuse) {
            env::panic_str("admission refused");
        }
        let sender = env::predecessor_account_id();
        let usd: u128 = match args["params"]["attached_usd"].as_str() {
            Some(s) => s.parse().unwrap_or_else(|_| env::panic_str("bad attached_usd")),
            None => 0,
        };
        let balance = self.balances.get(&sender).copied().unwrap_or(0);
        if usd > balance {
            env::panic_str("insufficient stablecoin balance");
        }
        self.calls.push(String::from_utf8(raw).unwrap_or_default());
        match script {
            Script::Output(output) => {
                self.balances.insert(sender, balance - usd);
                env::value_return(output.as_bytes());
            }
            // Debited at admission, returned on failure: the balance is unchanged.
            Script::Null => env::value_return(b"null"),
            Script::Refuse => unreachable!(),
        }
    }

    /// NEP-141 receiver: only `{"action":"deposit_balance"}` from the configured token.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        if env::predecessor_account_id() != self.token {
            env::panic_str("wrong token");
        }
        let ok = serde_json::from_str::<Value>(&msg).ok().is_some_and(|v| v["action"] == "deposit_balance");
        if !ok {
            return amount;
        }
        let balance = self.balances.get(&sender_id).copied().unwrap_or(0);
        self.balances.insert(sender_id, balance + amount.0);
        U128(0)
    }
}
