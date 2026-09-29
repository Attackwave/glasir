// Keeps the running balance for one account.
public class Ledger {

    public static final Integer LIMIT = 5000;

    // Bills the account and returns what is left.
    public Integer charge(String owner, Integer amount) {
        if (amount > LIMIT) {
            return refuse(owner);
        }
        return commitEntry(owner, amount);
    }

    private Integer refuse(String owner) {
        warnOwner(owner);
        return 0;
    }

    private Integer commitEntry(String owner, Integer amount) {
        writeEntry(owner, amount);
        return amount;
    }
}
