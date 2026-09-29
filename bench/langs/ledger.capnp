# Keeps the running balance for one account.
@0xdbb9ad1f14bf0b36;

# One entry against an account.
struct Entry {
  owner @0 :Text;
  amount @1 :Int64;
}

# What a charge returned.
struct Result {
  remaining @0 :Int64;
  refused @1 :Bool;
}

# Bills accounts and reports what is left.
interface Ledger {
  charge @0 (entry :Entry) -> (result :Result);
  refuse @1 (entry :Entry) -> (result :Result);
}
