# Keeps the running balance for one account.
FROM alpine:3.19 AS refuse
RUN warn_owner

FROM refuse AS commit_entry
RUN write_entry

# Bills the account and returns what is left.
FROM commit_entry AS charge
RUN check_limit 5000
ENTRYPOINT ["charge"]
