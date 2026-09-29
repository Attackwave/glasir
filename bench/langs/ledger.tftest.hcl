# Terraform's own test format: `run` blocks with nested assertions.
#
# A separate fixture because `.tftest.hcl` and `.tfcomponent.hcl` carry block
# types the `.tf` grammar does not, and a scanner that misses them attributes
# every assertion to the file instead.

run "refuses_over_limit" {
  command = plan

  variables {
    amount = 9000
  }

  assert {
    condition     = output.remaining == 0
    error_message = "a charge over the limit must be refused"
  }
}

run "commits_under_limit" {
  command = apply

  assert {
    condition     = output.remaining > 0
    error_message = "a charge under the limit must commit"
  }
}
