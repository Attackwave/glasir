// Keeps the running balance for one account.

const LIMIT = 5000;

function refuse(owner: string, amount: number): number {
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
        return refuse(owner, amount);
    }
    return commitEntry(owner, amount);
}

// A destructuring parameter opens a brace inside the parameter list, and the
// matching one used to close the constant it belongs to — every call in the
// body was then attributed to the file.
const describeEntry = ({ owner, amount }: { owner: string; amount: number }): number => formatEntry(owner, amount);
