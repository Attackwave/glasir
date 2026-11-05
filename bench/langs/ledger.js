// Keeps the running balance for one account.

const LIMIT = 5000;

function refuse(owner, amount) {
    warnOwner(owner);
    return 0;
}

function commitEntry(owner, amount) {
    writeEntry(owner, amount);
    return amount;
}

// Bills the account and returns what is left.
function charge(owner, amount) {
    if (amount > LIMIT) {
        return refuse(owner, amount);
    }
    return commitEntry(owner, amount);
}

// A destructuring parameter opens a brace inside the parameter list, and the
// matching one used to close the constant it belongs to — every call in the
// body was then attributed to the file.
const describeEntry = ({ owner, amount }) => formatEntry(owner, amount);

// Assigned to a member, a function is named by the member.
exports.settle = function (owner) {
    return commitEntry(owner, 0);
};

Ledger.prototype.close = async (owner) => {
    await refuse(owner, 0);
};
