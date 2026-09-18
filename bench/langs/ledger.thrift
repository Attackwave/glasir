// Keeps the running balance for one account.
namespace java bench.ledger

// One entry against an account.
struct Entry {
  1: string owner,
  2: i64 amount
}

// What a charge returned.
struct Result {
  1: i64 remaining,
  2: bool refused
}

// Bills accounts and reports what is left.
service Ledger {
  Result charge(1: Entry entry),
  Result refuse(1: Entry entry)
}
