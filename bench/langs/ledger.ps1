# Keeps the running balance for one account.

$LIMIT = 5000

function Refuse {
    param([string]$Owner)
    Warn-Owner $Owner
    return 0
}

function Commit-Entry {
    param([string]$Owner, [int]$Amount)
    Write-Entry $Owner $Amount
    return $Amount
}

# Bills the account and returns what is left.
function Charge {
    param([string]$Owner, [int]$Amount)
    if ($Amount -gt $LIMIT) {
        return Refuse $Owner
    }
    return Commit-Entry $Owner $Amount
}
