# Cross-repository evidence contracts

Glasir joins repositories only through explicit, versioned evidence. A guessed
name match is never promoted to a cross-repository edge.

Each repository publishes `.glasir/contracts.json` in the same revision as its
code:

```json
{
  "schema": "glasir.contracts.v1",
  "package": {"name": "payments-api", "version": "2026.09.18"},
  "exports": [{"symbol": "src/api.rs#create_payment", "contract": "openapi:payments.v2"}],
  "imports": [{"package": "ledger", "version": "^4", "contract": "event:payment.created.v1"}]
}
```

Contract changes are analysis changes. Run `glasir analyse` after changing the
file and publish the resulting snapshot together with the same source revision;
never reuse a contract report across a different commit.

Global symbol identity is `package@version::symbol`. An import edge is emitted
only when package/version and contract agree. Its evidence is `contract`; SCIP
or compiler resolution upgrades it to `precise`; runtime telemetry may add an
`observed` edge but never replace static evidence.

Every persisted edge carries `source`, `target`, `relation`, `evidence`,
`confidence`, `repo`, `revision`, `valid_from`, and `valid_to`. This makes a
PR report explain *why* an impact crosses repositories and prevents stale
dependency metadata from silently changing a historical answer.
