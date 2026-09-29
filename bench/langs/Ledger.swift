import Foundation

/// Keeps the running balance for one account.
class Ledger {
    let limit = 5000

    private let owner: String

    init(owner: String) {
        self.owner = owner
    }

    /// Bills the account and returns what is left.
    func charge(amount: Int) -> Int {
        if amount > limit {
            return refuse(amount: amount)
        }
        return commit(amount: amount)
    }

    private func refuse(amount: Int) -> Int {
        warn(owner)
        return 0
    }

    private func commit(amount: Int) -> Int {
        write(owner, amount)
        return amount
    }
}
