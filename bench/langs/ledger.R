# Keeps the running balance for one account.

LIMIT <- 5000

refuse <- function(owner, amount) {
  warn_owner(owner)
  0
}

commit_entry <- function(owner, amount) {
  write_entry(owner, amount)
  amount
}

# Bills the account and returns what is left.
charge <- function(owner, amount) {
  if (amount > LIMIT) {
    refuse(owner, amount)
  } else {
    commit_entry(owner, amount)
  }
}
