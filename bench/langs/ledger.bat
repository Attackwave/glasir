REM Keeps the running balance for one account.
set LIMIT=5000

:refuse
call :warn_owner %1
exit /b 0

:commit_entry
call :write_entry %1 %2
exit /b 0

REM Bills the account and returns what is left.
:charge
call :refuse %1
call :commit_entry %1 %2
exit /b 0
