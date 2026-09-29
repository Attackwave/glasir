// Keeps the running balance for one account.
component {

    variables.limit = 5000;

    /// Bills the account and returns what is left.
    function charge(owner, amount) {
        if (amount > variables.limit) {
            return refuse(owner);
        }
        return commitEntry(owner, amount);
    }

    function refuse(owner) {
        warnOwner(owner);
        return 0;
    }

    function commitEntry(owner, amount) {
        writeEntry(owner, amount);
        return amount;
    }
}
