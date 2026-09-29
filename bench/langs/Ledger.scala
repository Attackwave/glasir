package bench

/** Keeps the running balance for one account. */
class Ledger(owner: String) {

  /** Bills the account and returns what is left. */
  def charge(amount: Int): Int = {
    if (amount > Ledger.Limit) {
      refuse(amount)
    } else {
      commit(amount)
    }
  }

  private def refuse(amount: Int): Int = {
    warn(owner)
    0
  }

  private def commit(amount: Int): Int = {
    write(owner, amount)
    amount
  }
}

object Ledger {
  val Limit = 5000
}
