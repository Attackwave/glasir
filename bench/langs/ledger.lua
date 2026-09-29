-- Keeps the running balance for one account.

local LIMIT = 5000

local function refuse(owner, amount)
    warn_owner(owner)
    return 0
end

local function commit_entry(owner, amount)
    write_entry(owner, amount)
    return amount
end

-- Bills the account and returns what is left.
function charge(owner, amount)
    if amount > LIMIT then
        return refuse(owner, amount)
    end
    return commit_entry(owner, amount)
end
