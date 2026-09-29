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

HTTP calls between repositories are a separate, inferred kind of evidence:
Glasir Control matches a request in one tree to a route in another by verb and
path (see the Core tool `http_surface`) and reports them apart from contract
edges, at the weakest confidence tier. A contract remains the only evidence
that a dependency was declared.

The current `glasir.cross-repo-report.v1` output records source package, target
package, contract, target symbol, and `contract` evidence. Persist the source
revision that produced a report alongside the report itself when a historical
review must be reproducible.
