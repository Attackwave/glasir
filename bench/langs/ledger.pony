// Keeps the running balance for one account.

primitive Limit
  fun apply(): I64 => 5000

class Ledger
  var owner: String = ""

  // Bills the account and returns what is left.
  fun ref charge(amount: I64): I64 =>
    if amount > Limit() then
      refuse(amount)
    else
      commit_entry(amount)
    end

  fun ref refuse(amount: I64): I64 =>
    warn_owner(owner)
    0

  fun ref commit_entry(amount: I64): I64 =>
    write_entry(owner, amount)
    amount
