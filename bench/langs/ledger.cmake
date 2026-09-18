# Keeps the running balance for one account.

set(LIMIT 5000)

function(refuse owner)
    warn_owner(${owner})
endfunction()

function(commit_entry owner amount)
    write_entry(${owner} ${amount})
endfunction()

# Bills the account and returns what is left.
function(charge owner amount)
    if(${amount} GREATER ${LIMIT})
        refuse(${owner})
        return()
    endif()
    commit_entry(${owner} ${amount})
endfunction()
