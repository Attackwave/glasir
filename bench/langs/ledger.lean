-- Keeps the running balance for one account.

def limit : Nat := 5000

def refuse (owner : String) : Nat :=
  Ledger.warnOwner owner

def commitEntry (owner : String) (amount : Nat) : Nat :=
  Ledger.writeEntry owner amount

-- Bills the account and returns what is left.
def charge (owner : String) (amount : Nat) : Nat :=
  if amount > limit then refuse owner
  else commitEntry owner amount
