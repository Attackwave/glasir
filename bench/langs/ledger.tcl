# Keeps the running balance for one account.

set LIMIT 5000

proc refuse {owner} {
    warn_owner $owner
    return 0
}

proc commit_entry {owner amount} {
    write_entry $owner $amount
    return $amount
}

# Bills the account and returns what is left.
proc charge {owner amount} {
    global LIMIT
    if {$amount > $LIMIT} {
        return [refuse $owner]
    }
    return [commit_entry $owner $amount]
}
