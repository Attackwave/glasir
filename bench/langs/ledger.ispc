// Keeps the running balance for one account.

static const uniform int LIMIT = 5000;

static int refuse(uniform int amount) {
    warn_owner(amount);
    return 0;
}

static int commit_entry(uniform int amount) {
    write_entry(amount);
    return amount;
}

// Bills the account and returns what is left.
export uniform int charge(uniform int amount) {
    if (amount > LIMIT) {
        return refuse(amount);
    }
    return commit_entry(amount);
}
