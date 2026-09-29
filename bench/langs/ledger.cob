      * Keeps the running balance for one account.
       IDENTIFICATION DIVISION.
       PROGRAM-ID. LEDGER.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-LIMIT PIC 9(5) VALUE 5000.
       PROCEDURE DIVISION.
       CHARGE-PARA.
           PERFORM REFUSE-PARA
           PERFORM COMMIT-ENTRY-PARA.
       REFUSE-PARA.
           PERFORM WARN-OWNER-PARA.
       COMMIT-ENTRY-PARA.
           PERFORM WRITE-ENTRY-PARA.
       WARN-OWNER-PARA.
           DISPLAY "refused".
       WRITE-ENTRY-PARA.
           DISPLAY "committed".
