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
  "imports": [{"package": "ledger", "version": "4.0.0", "contract": "event:payment.created.v1"}]
}
```

Contract changes are analysis changes. Run `glasir analyse` after changing the
file and publish the resulting snapshot together with the same source revision;
never reuse a contract report across a different commit.

Global symbol identity is `package@version::symbol`. An import edge is emitted
only when package, **exact version**, and contract agree. Version ranges are not
resolved. The current report emits contract evidence and unresolved imports; it
does not infer runtime edges or upgrade an edge from compiler telemetry.

The current `glasir.cross-repo-report.v1` output records source package, target
package, contract, target symbol, and `contract` evidence. Persist the source
revision that produced a report alongside the report itself when a historical
review must be reproducible.
