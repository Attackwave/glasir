---- MODULE ledger ----
\* Keeps the running balance for one account.

Limit == 5000

Refuse(owner) == WarnOwner(owner)

CommitEntry(owner, amount) == WriteEntry(owner, amount)

\* Bills the account and returns what is left.
Charge(owner, amount) ==
    IF amount > Limit
    THEN Refuse(owner)
    ELSE CommitEntry(owner, amount)

====
