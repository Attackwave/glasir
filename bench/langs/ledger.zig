// Keeps the running balance for one account.

const LIMIT = 5000;

fn refuse(owner: []const u8, amount: i64) i64 {
    warnOwner(owner);
    return 0;
}

fn commitEntry(owner: []const u8, amount: i64) i64 {
    writeEntry(owner, amount);
    return amount;
}

// Bills the account and returns what is left.
pub fn charge(owner: []const u8, amount: i64) i64 {
    if (amount > LIMIT) {
        return refuse(owner, amount);
    }
    return commitEntry(owner, amount);
}
