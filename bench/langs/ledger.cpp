// Keeps the running balance for one account.
#include "ledger.hpp"

namespace bench {

const int Limit = 5000;

class Ledger {
public:
    explicit Ledger(std::string owner) : owner_(std::move(owner)) {}

    // Bills the account and returns what is left.
    int charge(int amount) {
        if (amount > Limit) {
            return refuse(amount);
        }
        return commit(amount);
    }

private:
    int refuse(int amount) {
        warn(owner_);
        return 0;
    }

    int commit(int amount) {
        write(owner_, amount);
        return amount;
    }

    std::string owner_;
};

}
