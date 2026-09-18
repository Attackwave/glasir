-- Keeps the running balance for one account.

CREATE OR REPLACE PACKAGE BODY ledger IS

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
    -- `:=` is PL/SQL's assignment and appears on nearly every line of real
    -- code, not only on a constant. The fixture carried exactly one, in a
    -- declaration, so it could not have caught a scanner that mishandled the
    -- token — which became a live risk once `:=` was lexed as one symbol.
    v_total NUMBER := 0;
    v_left  NUMBER;
  BEGIN
    v_total := lookup(owner);
    v_left  := v_total - amount;
    IF v_left > limit_amount THEN
      v_left := refuse(owner);
    END IF;
    RETURN commit_entry(owner, v_left);
  END;

END ledger;
