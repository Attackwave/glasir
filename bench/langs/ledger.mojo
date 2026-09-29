# Keeps the running balance for one account.

alias LIMIT = 5000


fn refuse(owner: String) -> Int:
    warn_owner(owner)
    return 0


fn commit_entry(owner: String, amount: Int) -> Int:
    write_entry(owner, amount)
    return amount


# Bills the account and returns what is left.
fn charge(owner: String, amount: Int) -> Int:
    if amount > LIMIT:
        return refuse(owner)
    return commit_entry(owner, amount)
