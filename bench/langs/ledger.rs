//! Keeps the running balance for one account.

/// Caps a single charge.
pub const LIMIT: i64 = 5000;

pub struct Ledger {
    owner: String,
    entries: Vec<i64>,
}

impl Ledger {
    /// Bills the account and returns what is left.
    pub fn charge(&mut self, amount: i64) -> Option<i64> {
        if amount > LIMIT {
            return self.refuse(amount);
        }
        self.entries.push(amount);
        Some(self.total())
    }

    fn total(&self) -> i64 {
        sum_of(&self.entries)
    }

    fn refuse(&mut self, amount: i64) -> Option<i64> {
        warn(&self.owner);
        None
    }
}
