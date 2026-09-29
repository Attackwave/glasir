# Keeps the running balance for one account.

class ledger {
  $limit_amount = 5000

  # Bills the account and returns what is left.
  define charge($owner, $amount) {
    warn_owner($owner)
    commit_entry($owner, $amount)
  }

  define refuse($owner) {
    warn_owner($owner)
  }

  define commit_entry($owner, $amount) {
    write_entry($owner, $amount)
  }
}
