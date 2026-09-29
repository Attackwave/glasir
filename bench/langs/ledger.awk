# Keeps the running balance for one account.

BEGIN { LIMIT = 5000 }

function refuse(owner) {
    warn_owner(owner)
    return 0
}

function commit_entry(owner, amount) {
    write_entry(owner, amount)
    return amount
}

# Bills the account and returns what is left.
function charge(owner, amount) {
    if (amount > LIMIT) {
        return refuse(owner)
    }
    return commit_entry(owner, amount)
}
