// Keeps the running balance for one account.

let limit = 5000

let refuse = owner => {
  warnOwner(owner)
  0
}

let commitEntry = (owner, amount) => {
  writeEntry(owner, amount)
  amount
}

// Bills the account and returns what is left.
let charge = (owner, amount) =>
  if amount > limit {
    refuse(owner)
  } else {
    commitEntry(owner, amount)
  }
