// Keeps the running balance for one account.

const limit = 5000

fn refuse(owner: String) -> Int {
  warn_owner(owner)
  0
}

fn commit_entry(owner: String, amount: Int) -> Int {
  write_entry(owner, amount)
  amount
}

// Bills the account and returns what is left.
pub fn charge(owner: String, amount: Int) -> Int {
  case amount > limit {
    True -> refuse(owner)
    False -> commit_entry(owner, amount)
  }
}
