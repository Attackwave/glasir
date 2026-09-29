package ledger

// Limit caps a single charge.
const Limit = 5000

// Charge bills an account and returns what is left.
func Charge(account string, amount int) int {
	balance := lookup(account)
	if balance < amount {
		return refuse(account)
	}
	return commit(account, balance-amount)
}

func lookup(account string) int {
	return read(account)
}

func refuse(account string) int {
	warn(account)
	return 0
}

func commit(account string, left int) int {
	write(account, left)
	return left
}
