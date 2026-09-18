;; Keeps the running balance for one account.

(define limit 5000)

(define (refuse owner amount)
  (warn-owner owner)
  0)

(define (commit-entry owner amount)
  (write-entry owner amount)
  amount)

;; Bills the account and returns what is left.
;;
;; The nesting is deliberate: a body one level deep is what a wrong depth on
;; the parameter list gets wrong, and a flat body does not show it.
(define (charge owner amount)
  (if (> amount limit)
      (refuse owner amount)
      (let ((left (commit-entry owner amount)))
        (record-total left)
        left)))

(define-macro (with-ledger owner body)
  `(let ((ledger (open-ledger ,owner))) ,body))
