// Keeps the running balance for one account.

__constant__ int LIMIT = 5000;

__device__ int refuse(int amount) {
    warn_owner(amount);
    return 0;
}

__device__ int commit_entry(int amount) {
    write_entry(amount);
    return amount;
}

// Bills the account and returns what is left.
__global__ void charge(int *out, int amount) {
    if (amount > LIMIT) {
        *out = refuse(amount);
        return;
    }
    *out = commit_entry(amount);
}
