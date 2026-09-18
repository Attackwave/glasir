// Keeps the running balance for one account.

def LIMIT: int = 5000;

fn refuse(owner: str) int = {
	warn_owner(owner);
	return 0;
};

fn commit_entry(owner: str, amount: int) int = {
	write_entry(owner, amount);
	return amount;
};

// Bills the account and returns what is left.
fn charge(owner: str, amount: int) int = {
	if (amount > LIMIT) {
		return refuse(owner);
	};
	return commit_entry(owner, amount);
};
