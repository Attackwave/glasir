// Keeps the running balance for one account.

static const int LIMIT = 5000;

int refuse(int amount) {
    warnOwner(amount);
    return 0;
}

int commitEntry(int amount) {
    writeEntry(amount);
    return amount;
}

// Bills the account and returns what is left.
int charge(int amount) {
    if (amount > LIMIT) {
        return refuse(amount);
    }
    return commitEntry(amount);
}
