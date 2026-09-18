// Keeps the running balance for one account.
module ledger::account {

    const LIMIT: u64 = 5000;

    fun refuse(owner: address): u64 {
        warn_owner(owner);
        0
    }

    fun commit_entry(owner: address, amount: u64): u64 {
        write_entry(owner, amount);
        amount
    }

    /// Bills the account and returns what is left.
    public fun charge(owner: address, amount: u64): u64 {
        if (amount > LIMIT) {
            return refuse(owner)
        };
        commit_entry(owner, amount)
    }
}
