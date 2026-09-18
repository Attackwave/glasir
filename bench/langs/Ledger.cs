namespace Bench;

/// <summary>Keeps the running balance for one account.</summary>
public class Ledger
{
    const int Limit = 5000;

    private readonly string owner;

    public Ledger(string owner)
    {
        this.owner = owner;
    }

    /// <summary>Bills the account and returns what is left.</summary>
    public int Charge(int amount)
    {
        if (amount > Limit)
        {
            return Refuse(amount);
        }
        return Commit(amount);
    }

    private int Refuse(int amount)
    {
        Warn(owner);
        return 0;
    }

    private int Commit(int amount)
    {
        Write(owner, amount);
        return amount;
    }
}
