package bench;

/** Keeps the running balance for one account. */
public class Ledger {
    static final int LIMIT = 5000;

    private final String owner;

    public Ledger(String owner) {
        this.owner = owner;
    }

    /** Bills the account and returns what is left. */
    public int charge(int amount) {
        if (amount > LIMIT) {
            return refuse(amount);
        }
        return commit(amount);
    }

    private int refuse(int amount) {
        warn(owner);
        return 0;
    }

    private int commit(int amount) {
        write(owner, amount);
        return amount;
    }
}
