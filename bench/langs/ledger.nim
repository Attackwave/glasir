## Keeps the running balance for one account.

const
  ## Nim writes a group as the keyword alone with the names indented under it —
  ## a third of all constants in its own tree, and none was found before.
  LIMIT* = 5000
  RETRY_LIMIT* = 3

proc refuse(owner: string, amount: int): int =
  warnOwner(owner)
  return 0

proc commitEntry(owner: string, amount: int): int =
  writeEntry(owner, amount)
  return amount

## Bills the account and returns what is left.
proc charge*(owner: string, amount: int): int =
  if amount > LIMIT:
    return refuse(owner, amount)
  return commitEntry(owner, amount)
