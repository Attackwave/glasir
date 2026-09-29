; Keeps the running balance for one account.

@LIMIT = global i64 5000

define i64 @refuse(i64 %owner) {
  %1 = call i64 @warn_owner(i64 %owner)
  ret i64 0
}

define i64 @commit_entry(i64 %owner, i64 %amount) {
  %1 = call i64 @write_entry(i64 %owner, i64 %amount)
  ret i64 %amount
}

; Bills the account and returns what is left.
define i64 @charge(i64 %owner, i64 %amount) {
  %1 = call i64 @refuse(i64 %owner)
  %2 = call i64 @commit_entry(i64 %owner, i64 %amount)
  ret i64 %2
}
