# Keeps the running balance for one account.

LIMIT = 5000

refuse:
	warn_owner $(LIMIT)

commit_entry:
	write_entry $(LIMIT)

# Bills the account and returns what is left.
charge: commit_entry
	check_limit $(LIMIT)
