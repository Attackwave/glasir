# Keeps the running balance for one account.
#
# No `set` at file level: a top-level command genuinely has no enclosing
# function, so counting it measures the shape of the script rather than the
# scanner. The corpus is where a legitimately high share belongs.

function limit_amount
    echo 5000
end

function refuse
    warn_owner $argv[1]
    echo 0
end

function commit_entry
    write_entry $argv[1] $argv[2]
    echo $argv[2]
end

# Bills the account and returns what is left.
function charge
    set -l cap (limit_amount)
    if test $argv[2] -gt $cap
        refuse $argv[1]
        return
    end
    commit_entry $argv[1] $argv[2]
end
