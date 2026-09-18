" Keeps the running balance for one account.

let s:limit = 5000

function! s:Refuse(owner)
    call WarnOwner(a:owner)
    return 0
endfunction

function s:CommitEntry(owner, amount)
    call WriteEntry(a:owner, a:amount)
    return a:amount
endfunction

" Bills the account and returns what is left.
function! s:Charge(owner, amount)
    if a:amount > s:limit
        return s:Refuse(a:owner)
    endif
    return s:CommitEntry(a:owner, a:amount)
endfunction
