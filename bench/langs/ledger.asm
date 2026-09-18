; Keeps the running balance for one account.

section .data
LIMIT equ 5000

section .text

refuse:
    call warn_owner
    ret

commit_entry:
    call write_entry
    ret

; Bills the account and returns what is left.
charge:
    call commit_entry
    call refuse
    ret
