// Keeps the running balance for one account.

param owner string
var limitAmount = 5000

// Bills the account and returns what is left.
resource charge 'Microsoft.Ledger/accounts@2023-01-01' = {
  name: format('{0}-charge', owner)
  properties: {
    amount: min(limitAmount, 5000)
    target: resourceId('Microsoft.Ledger/accounts', owner)
  }
}

resource refuse 'Microsoft.Ledger/refusals@2023-01-01' = {
  name: concat(owner, '-refused')
  properties: {
    reason: toUpper('over limit')
  }
}

output commitEntry string = union(charge.name, refuse.name)
