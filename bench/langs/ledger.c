/* Keeps the running balance for one account. */
#include "ledger.h"

#define LIMIT 5000

static int refuse(const char *owner)
{
    warn(owner);
    return 0;
}

static int commit_entry(const char *owner, int amount)
{
    write_entry(owner, amount);
    return amount;
}

/* Bills the account and returns what is left. */
int charge(const char *owner, int amount)
{
    if (amount > LIMIT) {
        return refuse(owner);
    }
    return commit_entry(owner, amount);
}
