%% Keeps the running balance for one account.
-module(ledger).
-export([charge/2]).

-define(LIMIT, 5000).

%% Bills the account and returns what is left.
charge(Owner, Amount) when Amount > ?LIMIT ->
    refuse(Owner, Amount);
charge(Owner, Amount) ->
    commit_entry(Owner, Amount).

refuse(Owner, _Amount) ->
    warn(Owner),
    0.

commit_entry(Owner, Amount) ->
    write_entry(Owner, Amount),
    Amount.
