// Keeps the running balance for one account.

#let limit = 5000

#let refuse(owner) = {
  warn-owner(owner)
  0
}

#let commit-entry(owner, amount) = {
  write-entry(owner, amount)
  amount
}

// Bills the account and returns what is left.
#let charge(owner, amount) = {
  if amount > limit {
    refuse(owner)
  } else {
    commit-entry(owner, amount)
  }
}
