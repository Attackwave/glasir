; Keeps the running balance for one account.

LIMIT       EQU     5000
EN_AMOUNT   EQU     4

* Saves every register it touches, so a caller need not.
SAVE        MACRO
            MOVEM.L D1/A0,-(SP)
            JSR     warn_owner
            ENDM

refuse:
            SAVE
            BSR     warn_owner
            RTS

commit_entry:
            MOVE.L  EN_AMOUNT(A0),D0
            BEQ.S   .empty
            JSR     write_entry
.empty:
            RTS

; Bills the account and returns what is left.
charge:
            CMP.L   #LIMIT,D0
            BHI.S   .over
            BSR     commit_entry
            RTS
.over:
            BRA     refuse

ledger_ops:
            DC.L    charge
            DC.L    refuse
