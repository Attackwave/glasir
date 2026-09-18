# Keeps the running balance for one account.

LIMIT = 5000

class Ledger
  # Bills the account and returns what is left.
  def charge(amount : Int32)
    return refuse(amount) if amount > LIMIT
    commit_entry(amount)
  end

  def refuse(amount : Int32)
    warn_owner(@owner)
    0
  end

  def commit_entry(amount : Int32)
    write_entry(@owner, amount)
    amount
  end
end
