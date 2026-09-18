# Keeps the running balance for one account.

limit := "5000"

refuse:
	warn_owner {{limit}}

commit_entry:
	write_entry {{limit}}

# Bills the account and returns what is left.
charge: commit_entry
	check_limit {{limit}}
