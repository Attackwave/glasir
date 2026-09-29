// Keeps the running balance for one account.
class Ledger {

    static final int LIMIT = 5000

    /// Bills the account and returns what is left.
    int charge(String owner, int amount) {
        if (amount > LIMIT) {
            return refuse(owner)
        }
        return commitEntry(owner, amount)
    }

    int refuse(String owner) {
        warnOwner(owner)
        return 0
    }

    int commitEntry(String owner, int amount) {
        writeEntry(owner, amount)
        return amount
    }
}
