//! Test double for a NEP-141 stablecoin, for sandbox tests only. It does what the
//! milestone contract relies on: registration by `storage_deposit`, `ft_transfer` (1 yocto,
//! registered receiver), and `ft_transfer_call` with the receiver's refund honoured. One
//! switch makes `ft_transfer` fail, so a payout failure can be tested.

use near_sdk::json_types::U128;
use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue};

const STORAGE_COST: NearToken = NearToken::from_micronear(1_250);
/// As in near-contract-standards: the receiver gets the prepaid gas minus this, and the
/// resolve step runs on 5 Tgas, so gas behaves as with a standard token.
const GAS_FOR_FT_TRANSFER_CALL: Gas = Gas::from_tgas(30);
const GAS_FOR_RESOLVE_TRANSFER: Gas = Gas::from_tgas(5);
const MAX_RESULT_BYTES: usize = 64;

#[near(serializers = [json])]
pub struct StorageBalance {
    pub total: NearToken,
    pub available: NearToken,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockFt {
    balances: LookupMap<AccountId, u128>,
    transfers_fail: bool,
}

#[near]
impl MockFt {
    #[init]
    pub fn new(owner_id: AccountId, total_supply: U128) -> Self {
        let mut balances = LookupMap::new(b"b");
        balances.insert(owner_id, total_supply.0);
        Self { balances, transfers_fail: false }
    }

    pub fn set_transfers_fail(&mut self, fail: bool) {
        self.transfers_fail = fail;
    }

    pub fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        U128(self.balances.get(&account_id).copied().unwrap_or(0))
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.balances
            .contains_key(&account_id)
            .then_some(StorageBalance { total: STORAGE_COST, available: NearToken::from_yoctonear(0) })
    }

    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>) -> StorageBalance {
        let _ = registration_only;
        let account = account_id.unwrap_or_else(env::predecessor_account_id);
        let deposit = env::attached_deposit();
        if self.balances.contains_key(&account) {
            if !deposit.is_zero() {
                let _ = Promise::new(env::predecessor_account_id()).transfer(deposit);
            }
        } else {
            require!(deposit >= STORAGE_COST, "attach the storage cost");
            self.balances.insert(account, 0);
            let change = deposit.saturating_sub(STORAGE_COST);
            if !change.is_zero() {
                let _ = Promise::new(env::predecessor_account_id()).transfer(change);
            }
        }
        StorageBalance { total: STORAGE_COST, available: NearToken::from_yoctonear(0) }
    }

    #[payable]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        let _ = memo;
        require!(!self.transfers_fail, "transfers are switched off");
        require!(env::attached_deposit() == NearToken::from_yoctonear(1), "attach exactly 1 yoctoNEAR");
        self.move_tokens(&env::predecessor_account_id(), &receiver_id, amount.0);
    }

    #[payable]
    pub fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>, msg: String) -> Promise {
        let _ = memo;
        require!(env::attached_deposit() == NearToken::from_yoctonear(1), "attach exactly 1 yoctoNEAR");
        let sender = env::predecessor_account_id();
        self.move_tokens(&sender, &receiver_id, amount.0);
        ext_receiver::ext(receiver_id.clone())
            .with_static_gas(env::prepaid_gas().saturating_sub(GAS_FOR_FT_TRANSFER_CALL))
            .with_unused_gas_weight(0)
            .ft_on_transfer(sender.clone(), amount, msg)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_RESOLVE_TRANSFER)
                    .with_unused_gas_weight(0)
                    .ft_resolve_transfer(sender, receiver_id, amount),
            )
    }

    /// Returns the amount used, as NEP-141 does; the unused part goes back to the sender.
    #[private]
    pub fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        let unused = match env::promise_result_checked(0, MAX_RESULT_BYTES) {
            Ok(bytes) => near_sdk::serde_json::from_slice::<U128>(&bytes).map(|u| u.0.min(amount.0)).unwrap_or(amount.0),
            Err(_) => amount.0,
        };
        let receiver_balance = self.balances.get(&receiver_id).copied().unwrap_or(0);
        let refund = unused.min(receiver_balance);
        if refund > 0 {
            self.move_tokens(&receiver_id, &sender_id, refund);
        }
        U128(amount.0 - refund)
    }
}

impl MockFt {
    fn move_tokens(&mut self, from: &AccountId, to: &AccountId, amount: u128) {
        require!(amount > 0, "amount must be positive");
        let from_balance = self.balances.get(from).copied().unwrap_or_else(|| env::panic_str("sender not registered"));
        let to_balance = self.balances.get(to).copied().unwrap_or_else(|| env::panic_str("receiver not registered"));
        require!(from_balance >= amount, "not enough balance");
        self.balances.insert(from.clone(), from_balance - amount);
        let to_balance = if from == to { from_balance - amount } else { to_balance };
        self.balances.insert(to.clone(), to_balance + amount);
    }
}

#[near_sdk::ext_contract(ext_receiver)]
#[allow(dead_code)]
trait Receiver {
    fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128>;
}
