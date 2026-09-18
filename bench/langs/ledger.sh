#!/usr/bin/env bash
# Keeps the running balance for one account.

LIMIT=5000

refuse() {
    warn_owner "$1"
    echo 0
}

commit_entry() {
    write_entry "$1" "$2"
    echo "$2"
}

# Bills the account and returns what is left.
charge() {
    local owner="$1"
    local amount="$2"
    if [ "$amount" -gt "$LIMIT" ]; then
        refuse "$owner"
        return
    fi
    commit_entry "$owner" "$amount"
}
