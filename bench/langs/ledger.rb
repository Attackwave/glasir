# Keeps the running balance for one account.
class Ledger
  LIMIT = 5000

  def initialize(owner)
    @owner = owner
    @entries = []
  end

  # Adds an entry and returns the new balance.
  def charge(amount)
    return refuse(amount) if amount > LIMIT
    @entries.push(amount)
    total
  end

  def total
    @entries.sum
  end

  def refuse(amount)
    warn("over limit")
    nil
  end
end
