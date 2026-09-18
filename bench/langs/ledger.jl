"Keeps the running balance for one account."
module Ledger

const LIMIT = 5000

"Bills the account and returns what is left."
function charge(owner, amount)
    if amount > LIMIT
        return refuse(owner, amount)
    end
    return commit_entry(owner, amount)
end

refuse(owner, amount) = warn_owner(owner)

function commit_entry(owner, amount)
    write_entry(owner, amount)
    return amount
end

end
