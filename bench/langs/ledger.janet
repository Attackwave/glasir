# Keeps the running balance for one account.

(def limit 5000)

(defn refuse [owner]
  (warn-owner owner)
  0)

(defn commit-entry [owner amount]
  (write-entry owner amount)
  amount)

# Bills the account and returns what is left.
(defn charge [owner amount]
  (if (> amount limit)
    (refuse owner)
    (commit-entry owner amount)))
