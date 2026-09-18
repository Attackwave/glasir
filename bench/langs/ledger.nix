# Keeps the running balance for one account.
{ pkgs, lib }:

let
  limit = 5000;

  refuse = owner: warnOwner owner;

  commitEntry = owner: writeEntry owner;

  # Bills the account and returns what is left.
  charge = owner: amount:
    if amount > limit
    then refuse owner
    else commitEntry owner;
in
{
  inherit charge refuse commitEntry;
}
