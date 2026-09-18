# Keeps the running balance for one account.

LIMIT = "5000"

do_refuse() {
    warn_owner ${LIMIT}
}

do_commit_entry() {
    write_entry ${LIMIT}
}

# Bills the account and returns what is left.
do_charge() {
    do_refuse
    do_commit_entry
}

addtask charge after do_configure
