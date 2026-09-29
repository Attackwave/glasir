// Keeps the running balance for one account.
$version: "2"

namespace ledger

service Ledger {
    operations: [Charge, Refuse]
}

/// Bills the account and returns what is left.
operation Charge {
    input: ChargeInput
    output: CommitEntry
}

operation Refuse {
    input: ChargeInput
}

structure ChargeInput {
    owner: String
    amount: Integer
}

structure CommitEntry {
    written: Boolean
}
