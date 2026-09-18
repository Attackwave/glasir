-- Keeps the running balance for one account.
--
-- A package rather than an architecture: an architecture header names its
-- entity before any routine exists, and those references genuinely have no
-- enclosing name. A fixture must measure the scanner, not the shape.

package body ledger is

    function limit_amount return integer is
    begin
        return 5000;
    end function;

    function refuse(owner : integer) return integer is
    begin
        warn_owner(owner);
        return 0;
    end function;

    function commit_entry(owner : integer; amount : integer) return integer is
    begin
        write_entry(owner, amount);
        return amount;
    end function;

    -- Bills the account and returns what is left.
    function charge(owner : integer; amount : integer) return integer is
    begin
        if amount > limit_amount then
            return refuse(owner);
        end if;
        return commit_entry(owner, amount);
    end function;

end package body;
