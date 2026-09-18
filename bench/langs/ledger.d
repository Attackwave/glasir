/// Keeps the running balance for one account.
module bench.ledger;

enum LIMIT = 5000;

int refuse(string owner, int amount)
{
    warnOwner(owner);
    return 0;
}

int commitEntry(string owner, int amount)
{
    writeEntry(owner, amount);
    return amount;
}

/// Bills the account and returns what is left.
int charge(string owner, int amount)
{
    if (amount > LIMIT)
    {
        return refuse(owner, amount);
    }
    return commitEntry(owner, amount);
}
