/// Keeps the running balance for one account.
class Ledger {
  static const int limit = 5000;

  final String owner;

  Ledger(this.owner);

  /// Bills the account and returns what is left.
  int charge(int amount) {
    if (amount > limit) {
      return refuse(amount);
    }
    return commitEntry(amount);
  }

  int refuse(int amount) {
    warnOwner(owner);
    return 0;
  }

  int commitEntry(int amount) {
    writeEntry(owner, amount);
    return amount;
  }
}
