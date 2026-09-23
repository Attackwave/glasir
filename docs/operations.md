# Core operations and data boundaries

Glasir Core is a single-repository data plane. It indexes one source tree and
serves that tree's graph. Glasir Control provides public identity, policy,
cross-repository orchestration, and central audit functions for enterprise
deployments.

## Local state

Core stores its snapshot, token hashes, and audit log beside the indexed tree.
Snapshots are a cache: they are validated against source and extraction
configuration before reuse and are rebuilt when validation fails. Token and
audit files can contain operationally sensitive information; restrict access,
exclude them from source control, and back them up only under the organisation's
approved retention policy.

The Core audit writer is bounded and may drop records when storage cannot keep
up. That protects request availability but makes a local Core audit log
insufficient as the sole enterprise forensic record.

## Deployment boundary

Do not expose a Core process directly to the public internet. In a governed
deployment, run it behind Glasir Control with per-service credentials and mTLS.
Use a network policy as an additional containment control, not as identity.

Core does not provide multi-tenant public identity administration, central
policy storage, or cross-repository authorization on its own. See
[architecture.md](architecture.md) for processing boundaries and the Control
repository for policy, audit, and Kubernetes operations.

## Recovery

The source repository remains the source of truth. Restore source and approved
configuration first, then allow Core to rebuild a snapshot. Test backup and
restore procedures with the same credentials, certificate rotation process,
and network policies used in production.
