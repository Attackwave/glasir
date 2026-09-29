;; Keeps the running balance for one account.
(ns bench.ledger
  (:require [clojure.test :refer [deftest is]]))

(def limit 5000)

(defmulti describe :kind)

(defmethod describe :entry [entry]
  (format-entry entry))

(defn- refuse [owner amount]
  (warn owner)
  0)

(defn- commit-entry [owner amount]
  (write-entry owner amount)
  amount)

;; Bills the account and returns what is left.
(defn charge [owner amount]
  (if (> amount limit)
    (refuse owner amount)
    (commit-entry owner amount)))

(deftest charge-refuses-over-limit
  (is (= 0 (charge "a" 9000))))
