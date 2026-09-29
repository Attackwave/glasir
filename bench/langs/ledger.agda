-- Keeps the running balance for one account.
module Ledger where

limit : Nat
limit = 5000

refuse : Nat -> Nat
refuse owner = warnOwner owner

commitEntry : Nat -> Nat -> Nat
commitEntry owner amount = writeEntry owner amount

-- Bills the account and returns what is left.
charge : Nat -> Nat -> Nat
charge owner amount = commitEntry owner amount
