use crate::{Account, LedgerError, LedgerStore, PostOutcome, Posting, check_balanced};
use microinsurance_engine::money::Money;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
struct Books {
    by_key: BTreeMap<String, Vec<Posting>>,
    balances: BTreeMap<Account, i64>,
}

/// In-memory ledger with the same semantics as the PostgreSQL store.
#[derive(Debug, Default)]
pub struct MemoryLedger {
    books: Mutex<Books>,
}

impl MemoryLedger {
    pub fn new() -> MemoryLedger {
        MemoryLedger::default()
    }
}

impl LedgerStore for MemoryLedger {
    async fn post(
        &self,
        idempotency_key: &str,
        entries: &[Posting],
    ) -> Result<PostOutcome, LedgerError> {
        check_balanced(entries)?;
        let mut books = self
            .books
            .lock()
            .map_err(|_| LedgerError::Store("ledger lock poisoned".into()))?;
        if let Some(existing) = books.by_key.get(idempotency_key) {
            return if existing.as_slice() == entries {
                Ok(PostOutcome::AlreadyPosted)
            } else {
                Err(LedgerError::KeyReused(idempotency_key.to_string()))
            };
        }
        for p in entries {
            *books.balances.entry(p.account).or_default() += p.amount.0;
        }
        books
            .by_key
            .insert(idempotency_key.to_string(), entries.to_vec());
        Ok(PostOutcome::Posted)
    }

    async fn balance(&self, account: Account) -> Result<Money, LedgerError> {
        let books = self
            .books
            .lock()
            .map_err(|_| LedgerError::Store("ledger lock poisoned".into()))?;
        Ok(Money(books.balances.get(&account).copied().unwrap_or(0)))
    }
}
