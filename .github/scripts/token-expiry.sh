#!/usr/bin/env bash
# Called by .github/workflows/token-expiry.yml. `CURL` and `GH` can be pointed
# at stand-ins, which is how this was tested without a real token.
set -euo pipefail

WARN_DAYS=${WARN_DAYS:-30}
CURL=${CURL:-curl}
GH=${GH:-gh}
repo=${GITHUB_REPOSITORY:-Attackwave/glasir}
owner=${repo%%/*}
title="PACKAGES_TOKEN needs renewing"

if [ -z "${PACKAGES_TOKEN:-}" ]; then
  days=-1
  state="is not set"
else
  # GitHub reports a token's expiry in a response header on any request it
  # authenticates; the tap is a request the token must be able to make anyway.
  headers=$($CURL -sS -o /dev/null -D - \
    -H "Authorization: Bearer ${PACKAGES_TOKEN}" \
    "https://api.github.com/repos/${owner}/homebrew-glasir" | tr -d '\r')
  status=$(printf '%s\n' "$headers" | awk 'NR == 1 { print $2 }')
  expires=$(printf '%s\n' "$headers" |
    awk -F': ' 'tolower($1) == "github-authentication-token-expiration" { print $2 }')
  if [ "$status" != "200" ]; then
    days=-1
    state="was refused by GitHub (HTTP ${status:-no response})"
  elif [ -z "$expires" ]; then
    echo "PACKAGES_TOKEN works and does not expire"
    exit 0
  else
    days=$(( ($(date -d "$expires" +%s) - $(date +%s)) / 86400 ))
    state="expires on ${expires}, in ${days} day(s)"
  fi
fi

echo "PACKAGES_TOKEN ${state}"
if [ "$days" -gt "$WARN_DAYS" ]; then
  exit 0
fi

body="PACKAGES_TOKEN ${state}.

The release workflow uses it to push the Homebrew formula to
\`${owner}/homebrew-glasir\` and the Scoop manifest to \`${owner}/scoop-glasir\`.
Without it a release still publishes, but \`brew\` and \`scoop\` keep offering
the previous version.

To renew:
1. https://github.com/settings/personal-access-tokens — regenerate
   \`glasir-packages\`, or create a new fine-grained token with access to
   \`homebrew-glasir\` and \`scoop-glasir\` and **Contents: Read and write**.
2. \`gh secret set PACKAGES_TOKEN -R ${repo}\` and paste it.
3. Run the *token expiry* workflow once by hand; it closes nothing itself,
   so close this issue when it reports the new date."

existing=$($GH issue list -R "$repo" --state open --search "\"${title}\" in:title" \
  --json number --jq '.[0].number // empty')
if [ -n "$existing" ]; then
  $GH issue comment "$existing" -R "$repo" --body "$body"
else
  $GH issue create -R "$repo" --title "$title" --assignee "$owner" --body "$body"
fi
