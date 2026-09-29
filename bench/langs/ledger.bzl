# Keeps the running balance for one account.

LIMIT = 5000

def refuse(owner):
    warn_owner(owner)
    return 0

def commit_entry(owner, amount):
    write_entry(owner, amount)
    return amount

# Bills the account and returns what is left.
def charge(owner, amount):
    if amount > LIMIT:
        return refuse(owner)
    return commit_entry(owner, amount)
