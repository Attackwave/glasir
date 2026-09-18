% Keeps the running balance for one account.

function out = charge(owner, amount)
    LIMIT = 5000;
    if amount > LIMIT
        out = refuse(owner);
    else
        out = commit_entry(owner, amount);
    end
end

function out = refuse(owner)
    warn_owner(owner);
    out = 0;
end

function out = commit_entry(owner, amount)
    write_entry(owner, amount);
    out = amount;
end
