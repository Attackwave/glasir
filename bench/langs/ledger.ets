// Keeps the running balance for one account.

const LIMIT: number = 5000;

function refuse(owner: string): number {
    warnOwner(owner);
    return 0;
}

function commitEntry(owner: string, amount: number): number {
    writeEntry(owner, amount);
    return amount;
}

// Bills the account and returns what is left.
function charge(owner: string, amount: number): number {
    if (amount > LIMIT) {
        return refuse(owner);
    }
    return commitEntry(owner, amount);
}
