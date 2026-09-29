unit Ledger;
// Keeps the running balance for one account.

interface

const
  Limit = 5000;

implementation

function Refuse(Amount: Integer): Integer;
begin
  WarnOwner(Amount);
  Refuse := 0;
end;

function CommitEntry(Amount: Integer): Integer;
begin
  WriteEntry(Amount);
  CommitEntry := Amount;
end;

// Bills the account and returns what is left.
function Charge(Amount: Integer): Integer;
begin
  if Amount > Limit then
    Charge := Refuse(Amount)
  else
    Charge := CommitEntry(Amount);
end;

end.
