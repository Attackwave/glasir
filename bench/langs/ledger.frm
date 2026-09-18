-- Keeps the running balance for one account.

PACKAGE BODY ledger IS

  limit_amount CONSTANT NUMBER := 5000;

  FUNCTION refuse(owner IN VARCHAR2) RETURN NUMBER IS
  BEGIN
    warn_owner(owner);
    RETURN 0;
  END;

  FUNCTION commit_entry(owner IN VARCHAR2, amount IN NUMBER) RETURN NUMBER IS
  BEGIN
    write_entry(owner, amount);
    RETURN amount;
  END;

  -- Bills the account and returns what is left.
  FUNCTION charge(owner IN VARCHAR2, amount IN NUMBER) RETURN NUMBER IS
    v_left NUMBER;
  BEGIN
    v_left := amount;
    IF v_left > limit_amount THEN
      v_left := refuse(owner);
    END IF;
    RETURN commit_entry(owner, v_left);
  END;

END ledger;
