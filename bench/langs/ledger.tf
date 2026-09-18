# Keeps the running balance for one account.

variable "limit" {
  type    = number
  default = 5000
}

resource "ledger_account" "owner" {
  name  = var.owner_name
  limit = var.limit
}

resource "ledger_entry" "charge" {
  account = ledger_account.owner.id
  amount  = var.amount
}

output "remaining" {
  value = ledger_entry.charge.remaining
}
