# Glasir Core Architecture

Glasir builds a deterministic, queryable graph from a source tree. The Core is
deliberately self-contained: it stores graph state locally and exposes a small
MCP-compatible interface. Authentication, tenant policy, and deployment
orchestration belong to the separate Control Plane.

## Processing pipeline

1. **Discovery** walks the source tree with explicit size, path, and symlink
   limits. Generated files and local build output are excluded before parsing.
2. **Extraction** uses native language scanners, optional SCIP input, and an
   optional language-server tier. Every edge carries provenance so callers can
   distinguish extracted facts from bounded inference.
3. **Resolution** links unqualified references only when the evidence is
   unique. Ambiguous references remain explicit graph nodes instead of being
   guessed.
4. **Storage** persists a versioned snapshot and applies incremental changes
   through a compact delta layer. Snapshot publication is atomic, so readers
   always observe a coherent graph.
5. **Queries** provide search, impact analysis, paths, callers, cycles, and
   bounded source snippets. Results are deterministic for the same tree,
   revision, and request.

## Trust boundary

The Core can serve locally over standard input or through its HTTP endpoint.
When it runs behind Glasir Control, it accepts only the configured control-plane
network and mTLS client identity, and uses an isolated service token for each
tree. The Core never evaluates caller credentials from the public boundary.

## Cross-repository contracts

Each repository may publish versioned exported symbols and declared imports.
The Core resolves only exact package and version matches, and retains unresolved
imports as evidence. This prevents a cross-repository impact report from
inventing a dependency. The exchange schema is documented in
[cross-repo-contracts.md](cross-repo-contracts.md).

## Operational properties

- Parsing and graph updates are bounded by input size and concurrency limits.
- Snapshot files include schema and integrity checks before they are loaded.
- A request never changes graph state.
- Incremental updates remove stale symbols before publishing the next snapshot.
- The Core has no network dependency in the extraction or query path.
