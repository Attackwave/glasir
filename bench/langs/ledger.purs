-- | Keeps the running balance for one account.
module Bench.Ledger (charge) where

limit :: Int
limit = 5000

refuse :: String -> Int
refuse owner = warnOwner owner

commitEntry :: String -> Int -> Int
commitEntry owner amount = writeEntry owner amount

-- | Bills the account and returns what is left.
charge :: String -> Int -> Int
charge owner amount =
  if amount > limit then refuse owner
  else commitEntry owner amount
