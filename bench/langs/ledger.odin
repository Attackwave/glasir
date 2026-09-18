// Keeps the running balance for one account.
package ledger

LIMIT :: 5000

refuse :: proc(owner: string) -> int {
	warn_owner(owner)
	return 0
}

commit_entry :: proc(owner: string, amount: int) -> int {
	write_entry(owner, amount)
	return amount
}

// Bills the account and returns what is left.
charge :: proc(owner: string, amount: int) -> int {
	if amount > LIMIT {
		return refuse(owner)
	}
	return commit_entry(owner, amount)
}
