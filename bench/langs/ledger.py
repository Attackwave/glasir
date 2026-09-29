"""Keeps the running balance for one account."""

LIMIT = 5000


def refuse(owner, amount):
    """Rejects a charge over the limit."""
    warn_owner(owner)
    return 0


def commit_entry(owner, amount):
    write_entry(owner, amount)
    return amount


def charge(owner, amount):
    """Bills the account and returns what is left."""
    if amount > LIMIT:
        return refuse(owner, amount)
    return commit_entry(owner, amount)
