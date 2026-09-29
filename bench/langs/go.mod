// Keeps the running balance for one account.
module ledger/charge

go 1.22

require (
	ledger/refuse v1.0.0
	ledger/commit_entry v1.2.0
)

replace ledger/refuse => ../refuse
