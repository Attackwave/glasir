// Keeps the running balance for one account.
module Ledger

let limit = 5000

let refuse owner =
    warnOwner owner
    0

let commitEntry owner amount =
    writeEntry owner amount
    amount

// Bills the account and returns what is left.
let charge owner amount =
    if amount > limit then
        refuse owner
    else
        commitEntry owner amount

// F# implements an interface with `override`, and a member is named after the
// receiver rather than before it. The fixture wrote only `let`, so the corpus
// is what found this: a file of interface implementations measured 42% on
// `<module>` with every member invisible.
type Ledger() =
    member this.Charge(owner, amount) =
        charge owner amount

    override this.ToString() =
        refuse 0
