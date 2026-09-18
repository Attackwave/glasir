// Keeps the running balance for one account.

class Action<string name> {
  string Name = name;
}

def Refuse : Action<"refuse">;

def CommitEntry : Action<"commit_entry">;

// Bills the account and returns what is left.
def Charge : Action<"charge"> {
  Action fallback = Refuse;
  Action success = CommitEntry;
}
