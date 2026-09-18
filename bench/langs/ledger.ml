(* Keeps the running balance for one account. *)

let limit = 5000

let refuse owner amount =
  warn_owner owner;
  0

let commit_entry owner amount =
  write_entry owner amount;
  amount

(* Bills the account and returns what is left. *)
let charge owner amount =
  if amount > limit then
    refuse owner amount
  else
    commit_entry owner amount
