# Keeps the running balance for one account.

let limit = 5000 in

let refuse = fun owner => warn_owner owner in

let commit_entry = fun owner amount => write_entry owner amount in

# Bills the account and returns what is left.
let charge = fun owner amount =>
  if amount > limit then refuse owner else commit_entry owner amount in

charge
