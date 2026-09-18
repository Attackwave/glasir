# Keeps the running balance for one account.

local limit = 5000;

{
  refuse:: function(owner) warnOwner(owner),

  commitEntry:: function(owner, amount) writeEntry(owner, amount),

  # Bills the account and returns what is left.
  charge:: function(owner, amount)
    if amount > limit then self.refuse(owner) else self.commitEntry(owner, amount),
}
