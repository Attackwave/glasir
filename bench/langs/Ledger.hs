-- | Keeps the running balance for one account.
module Bench.Ledger (charge) where

limit :: Int
limit = 5000

-- | Bills the account and returns what is left.
charge :: String -> Int -> Int
charge owner amount =
  if amount > limit
    then refuse owner amount
    else commitEntry owner amount

refuse :: String -> Int -> Int
refuse owner _ = warnOwner owner

commitEntry :: String -> Int -> Int
commitEntry owner amount = writeEntry owner amount
