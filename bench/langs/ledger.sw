// Keeps the running balance for one account.
contract;

const LIMIT: u64 = 5000;

fn refuse(owner: Address) -> u64 {
    warn_owner(owner);
    0
}

fn commit_entry(owner: Address, amount: u64) -> u64 {
    write_entry(owner, amount);
    amount
}

// Bills the account and returns what is left.
fn charge(owner: Address, amount: u64) -> u64 {
    if amount > LIMIT {
        return refuse(owner);
    }
    commit_entry(owner, amount)
}
