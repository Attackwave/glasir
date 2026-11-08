# Keeps the running balance for one account.
FROM alpine:3.19@sha256:6baf43584bcb78f2e5847d1de515f23499913ac9f12bdf834811a3145eb11ca1 AS refuse
RUN warn_owner

FROM refuse AS commit_entry
RUN write_entry

# Bills the account and returns what is left.
FROM commit_entry AS charge
RUN check_limit 5000
ENTRYPOINT ["charge"]
