extends Node
# Keeps the running balance for one account.

## A signal is a node's public surface — what another node connects to — and an
## enum and a class-level `var` are declarations too. The fixture carried
## `const` and `var` from the start and measured 0% either way, because
## attribution cannot see a missing *definition*: a real Godot project is what
## showed 449 of them absent against 475 functions found.
signal charged(amount)

enum Mode { OPEN, CLOSED }

const LIMIT = 5000

var owner_name = ""

@export var rate = 2

static var opened = 0

# Bills the account and returns what is left.
func charge(amount):
	if amount > LIMIT:
		return refuse(amount)
	return commit_entry(amount)

func refuse(amount):
	warn(owner_name)
	return 0

func commit_entry(amount):
	write_entry(owner_name, amount)
	return amount
