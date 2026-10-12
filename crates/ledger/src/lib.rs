//! Double-entry money ledger. Every posting set balances to zero and carries an idempotency key
//! (R27), so a retried payout can never pay twice. `MemoryLedger` is the reference store for
//! tests and demo mode; `PgLedger` is the PostgreSQL store (a primary key on the idempotency key
//! and `ON CONFLICT DO NOTHING`). Both pass the same tests (`tests/ledger.rs`).
//!
//! The ledger never decides or rounds an amount: it records the engine's decisions as they are
//! (ADR-004). `postings` maps each decision to its entries.
#![allow(async_fn_in_trait)]

mod memory;
mod postgres;
pub mod postings;

pub use memory::MemoryLedger;
pub use postgres::PgLedger;

use microinsurance_engine::money::Money;
use serde::{Deserialize, Serialize};

/// Positive balances are debits, negative credits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// Money held by the insurer's bank account.
    Cash,
    /// Premiums billed and not yet received from customers.
    CustomerPremiums,
    /// The insurer's risk account: premium earned (credit), benefits paid (debit).
    InsurerRisk,
    /// Payouts decided and not yet settled.
    PayoutsPayable,
    /// Reserved for in-kind orders: owed to the supplier once delivered, or back to the customer
    /// (R28, R56).
    InKindPayable,
    /// Customers' reward balances (R16).
    RewardsPayable,
    PartnerCommission,
    PlatformFee,
    /// Prevention and loyalty rewards granted: a loss-prevention expense, not claims (R40).
    RewardExpense,
}

impl Account {
    pub const ALL: [Account; 9] = [
        Account::Cash,
        Account::CustomerPremiums,
        Account::InsurerRisk,
        Account::PayoutsPayable,
        Account::InKindPayable,
        Account::RewardsPayable,
        Account::PartnerCommission,
        Account::PlatformFee,
        Account::RewardExpense,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Account::Cash => "cash",
            Account::CustomerPremiums => "customer_premiums",
            Account::InsurerRisk => "insurer_risk",
            Account::PayoutsPayable => "payouts_payable",
            Account::InKindPayable => "in_kind_payable",
            Account::RewardsPayable => "rewards_payable",
            Account::PartnerCommission => "partner_commission",
            Account::PlatformFee => "platform_fee",
            Account::RewardExpense => "reward_expense",
        }
    }

    pub fn parse(s: &str) -> Option<Account> {
        Account::ALL.into_iter().find(|a| a.as_str() == s)
    }
}

/// Positive = debit, negative = credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    pub account: Account,
    pub amount: Money,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostOutcome {
    Posted,
    /// The same key and the same postings were already recorded; nothing changed.
    AlreadyPosted,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("postings do not balance: off by {0} cents")]
    Unbalanced(i64),
    #[error("idempotency key {0} was already used for different postings")]
    KeyReused(String),
    #[error("store error: {0}")]
    Store(String),
}

pub fn check_balanced(entries: &[Posting]) -> Result<(), LedgerError> {
    let sum: i64 = entries.iter().map(|p| p.amount.0).sum();
    if sum == 0 {
        Ok(())
    } else {
        Err(LedgerError::Unbalanced(sum))
    }
}

/// The postings in order, as text: two posting sets are the same exactly when this is.
pub(crate) fn fingerprint(entries: &[Posting]) -> String {
    entries
        .iter()
        .map(|p| format!("{}:{}", p.account.as_str(), p.amount.0))
        .collect::<Vec<_>>()
        .join(";")
}

pub trait LedgerStore {
    async fn post(
        &self,
        idempotency_key: &str,
        entries: &[Posting],
    ) -> Result<PostOutcome, LedgerError>;

    /// Posts many entries, each with its own key and outcome; one failing never stops the rest.
    async fn post_many(
        &self,
        batch: &[(String, Vec<Posting>)],
    ) -> Vec<Result<PostOutcome, LedgerError>> {
        let mut out = Vec::with_capacity(batch.len());
        for (key, entries) in batch {
            out.push(self.post(key, entries).await);
        }
        out
    }

    async fn balance(&self, account: Account) -> Result<Money, LedgerError>;
}
