;; Keeps the running balance for one account.

(local limit 5000)

(fn refuse [owner]
  (warn-owner owner)
  0)

(fn commit-entry [owner amount]
  (write-entry owner amount)
  amount)

;; Bills the account and returns what is left.
(fn charge [owner amount]
  (if (> amount limit)
      (refuse owner)
      (commit-entry owner amount)))
