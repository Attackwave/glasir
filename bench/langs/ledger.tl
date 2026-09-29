-- Keeps the running balance for one account.

local LIMIT = 5000

local function refuse(owner: string): number
   warn_owner(owner)
   return 0
end

local function commit_entry(owner: string, amount: number): number
   write_entry(owner, amount)
   return amount
end

-- Bills the account and returns what is left.
local function charge(owner: string, amount: number): number
   if amount > LIMIT then
      return refuse(owner)
   end
   return commit_entry(owner, amount)
end
