-- Keeps the running balance for one account.

CREATE TABLE ledger_entries (
  id     INT PRIMARY KEY,
  owner  TEXT NOT NULL,
  amount BIGINT NOT NULL
);

-- An index names the table it covers; the reference belongs to the index.
CREATE INDEX idx_ledger_owner ON ledger_entries (owner);

CREATE SEQUENCE ledger_entry_id START 1;

-- Bills the account and returns what is left.
CREATE FUNCTION charge(p_owner TEXT, p_amount BIGINT) RETURNS BIGINT AS $$
BEGIN
  INSERT INTO ledger_entries (owner, amount) VALUES (p_owner, p_amount);
  RETURN p_amount;
END;
$$ LANGUAGE plpgsql;
