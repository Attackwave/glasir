package bench

/** Keeps the running balance for one account. */
class Ledger(private val owner: String) {

    /** Bills the account and returns what is left. */
    fun charge(amount: Int): Int {
        if (amount > LIMIT) {
            return refuse(amount)
        }
        return commit(amount)
    }

    private fun refuse(amount: Int): Int {
        warn(owner)
        return 0
    }

    private fun commit(amount: Int): Int {
        write(owner, amount)
        return amount
    }

    companion object {
        const val LIMIT = 5000
    }
}
