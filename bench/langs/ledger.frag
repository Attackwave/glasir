// Keeps the running balance for one account.
#version 450

const float LIMIT = 5000.0;

float refuse(float amount) {
    warnOwner(amount);
    return 0.0;
}

float commitEntry(float amount) {
    writeEntry(amount);
    return amount;
}

// Bills the account and returns what is left.
float charge(float amount) {
    if (amount > LIMIT) {
        return refuse(amount);
    }
    return commitEntry(amount);
}
