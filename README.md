# Glasir

**Deterministic code intelligence for repositories.** Glasir builds a local,
queryable graph of definitions, references, dependencies, and change impact,
then serves bounded evidence over [MCP](https://modelcontextprotocol.io).
It gives engineering tools structural answers instead of an unfiltered file
dump: what calls this, what breaks if it changes, and how two symbols connect.

*Glasir is the tree with golden leaves that stands before Valhalla, "the fairest
among gods and men" — a tree whose every branch is visible from the ground.*

One Rust binary. No Python, no Node, no database, no GPU, and **no model
anywhere in the index or query path** — the same tree and the same question
always produce the same answer.

> Production deployments pair the Core with Glasir Control for tenant policy,
> mTLS boundaries, audited administration, cross-repository review, and
> provider pull-request checks.

```
glasir serve . --watch  # index the tree, follow edits, serve on stdio
```

## Why a graph

Search finds text. A graph preserves the relationships that make a repository
behave as a system.

For example, answering "what is affected if `authorize` changes?" with search
requires finding candidate files, interpreting each call site, and repeatedly
reconstructing transitive dependencies. `impact` follows stored,
provenance-labelled edges and returns affected symbols grouped by distance.
Likewise, `shortest_path` shows the evidence-backed route between two symbols
instead of asking a caller to infer one from unrelated file excerpts.

| Need | Search and file reading | Glasir |
|---|---|---|
| Find text mentioning a name | Useful | Useful |
| Find direct callers | Manual interpretation required | Direct graph query |
| Assess transitive impact | Reconstruct dependencies repeatedly | Bounded traversal with edge provenance |
| Explain how A reaches B | Infer a path from excerpts | Return a concrete path |

The graph does not claim to translate arbitrary natural language. Search uses
deterministic identifier and text matching; unresolved or ambiguous symbols
remain visible as such rather than being guessed.

## Benchmarking

Glasir ships regression benchmarks, not vendor-comparison claims. They compare
the checked-in corpus with a documented grep-and-read baseline and fail when a
configured recall floor regresses. Results change as the corpus, questions and
implementation change, so this README intentionally does not publish static
percentages or token counts.

```sh
glasir benchmark .
glasir benchmark . --check
```

[Benchmark methodology](docs/benchmarking.md) defines the corpus, baseline,
token estimate, limits, and interpretation. It also explains why results from
this repository are not a substitute for an evaluation on a customer's code.

## Performance

Indexing and query latency depend on source shape, parser tier, storage,
hardware, and the requested graph traversal. The `analyse` command reports the
measured cold-index time and graph size for a repository; running it a second
time measures snapshot reuse. Evaluate performance with the exact release,
repository shape, and deployment limits that will be used in production.

Snapshots are stored beside the source tree and validated before reuse. A warm
start maps the stored graph instead of reparsing the full tree, provided the
source and extraction configuration still match.

## What it serves

Nine MCP tools, over stdio for a local editor or Streamable HTTP for a remote
agent:

- `overview` — the subsystems of an unfamiliar tree and the way into each
- `query_graph` — the subgraph relevant to a question
- `shortest_path` — how one symbol reaches another
- `explain_node` — callers, callees and subsystem of one symbol
- `impact` — what breaks if this changes, grouped by distance
- `cycles` — dependency cycles, compiler-resolved edges only by default
- `get_code_snippet` — the source a symbol names, from the range the parser
  recorded, so nothing has to open the file or guess a line
- `find_callers` — the direct callers of one symbol, as one list
- `detect_changes` — a git diff mapped to the symbols it touches and what
  depends on them; the only tool that reads the working tree

**Every answer comes twice: as prose and as `structuredContent`.** The text is
laid out for a person to read; the structured half is the same facts as data,
against a schema each tool declares, so an agent parses fields instead of
matching on wording that may be reworded. The text is rendered from that value,
so the two cannot disagree.

`GET /health` answers `{"status":"ok",...}` without a token, so a container
orchestrator can probe liveness; everything else needs one.

Under `serve --watch` a `GET` opens an event stream that pushes
`notifications/resources/updated` when the tree is re-indexed, so a client
holding cached answers learns they describe a tree that has moved on.

**The same commit answers the same way, on any machine.** No model and no
sampling in the index path: embedding signatures are hashed, every ranking
breaks ties on node id, and `glasir selfcheck` analyses a fixture twice from
cold and requires ids, edges, partition, embeddings, layout and search
rankings to match. That is what makes a retrieved answer citable rather than
merely plausible.

**Every edge says how it was resolved.** `extracted` is compiler-verified
(SCIP/LSP), `inferred` is syntax from native scanners, `ambiguous` is a name match.
An answer is therefore citable: a caller can tell what the tool knows from what
it guessed.

## Languages

119 language variants are registered in `parsers/`, with native scanners and
no external grammar dependency. Among them Rust, Python, JavaScript,
TypeScript, Go, Java, C, C++, C#, Ruby, PHP, Swift, Scala, Kotlin, Bash, Lua,
Elixir, Dart, Haskell, Zig, Perl, SQL, HCL, R, Julia, OCaml, Solidity and
Erlang. The checked-in language fixtures exercise extraction floors for every
supported language variant, so an empty or materially degraded scanner fails
the build rather than quietly yielding nothing. Markdown is indexed as a source
in its own right — sections are nodes and backticked names are edges to the
code they name.

Bundled extraction rules work without setup. The public override interface is
currently limited to Go, Rust, and Ruby; other bundled rule files are internal
implementation detail and are not a compatibility promise. To replace a
supported language rule explicitly:

```sh
glasir analyse . --language-rules /path/to/rules
glasir serve . --watch --language-rules /path/to/rules
```

Rules are validated before indexing, stay fixed until restart, and participate
in snapshot validity. See [language rule maintenance](parsers/README.md) for the
schema, examples, tests and current extraction limits.

## Running it for a team

```
glasir serve /srv/repo --http 0.0.0.0:8899   # refuses to start: no auth
glasir token add anna --days 90              # prints the token once
glasir serve /srv/repo --http 0.0.0.0:8899 --watch
```

- **Per-user tokens**, stored as SHA-256 only. Revoking takes effect on the next
  request, without a restart.
- **Audit log** in JSONL: timestamp, identity, tool, question and result size,
  written off the request path so a full disk costs records, never requests.
- **A non-loopback address with no authentication refuses to start.** Loopback
  stays open — that is the single-machine case.
- `Origin` validation and a bounded connection pool, per the MCP transport spec.

## Deployment boundary

For an enterprise deployment, run the separate
[Glasir Control](https://github.com/Attackwave/glasir-control) service at the
public edge. It supplies OIDC resource-server validation, repository routing,
code-host permission mirroring, central audit and the public TLS boundary.
Glasir Core remains the private data plane for an individual source tree and
does not expose those control-plane capabilities itself. Run every Core process
without a public shared secret:

```sh
cd /srv/alpha && CONTROL_TOKEN="$(glasir token add control)"
glasir serve /srv/alpha --http 7001 --behind-control-plane --watch
# rights.tsv in glasir-control contains the one-time CONTROL_TOKEN for alpha.
```

`--behind-control-plane` refuses `--token` or a missing per-service token and
remains loopback-only unless the explicit CIDR gate below is supplied. The
public proxy validates the browser origin and removes it before forwarding;
the Core keeps its loopback DNS-rebinding defence.

For Kubernetes, the Core may bind to a pod interface only with an explicit
network boundary. Pair the Core's Service with a default-deny NetworkPolicy and
pass the exact pod CIDR of the control plane:

```sh
glasir serve /srv/alpha --http 0.0.0.0:7001 --behind-control-plane \
  --control-plane-cidr 10.1.0.0/16 \
  --tls-cert /run/tls/core.crt --tls-key /run/tls/core.key \
  --control-plane-client-ca /run/tls/ca.crt --watch
```

The CIDR is an additional connection gate, not an authentication substitute:
the per-service token remains mandatory. Connections outside it are dropped
before HTTP parsing and receive no oracle response. IPv6 is intentionally not
accepted by this first networked data-plane mode.

For a production control plane, CIDR plus the per-service token is only a
first boundary: use mutual TLS. `--control-plane-client-ca` is accepted only
with `--behind-control-plane` and a server certificate/key. The Core then
requires the forwarding control-plane client certificate to chain to that CA;
the certificate name must match the Core Service DNS name. Keep the CA,
server certificate/key and control-plane client certificate in a workload
secret (or an external-secrets provider), rotate them independently, and do
not terminate this hop at a plaintext sidecar.

The Core's standalone scope excludes public identity administration and
cross-repository orchestration. TLS is available (`--tls-cert` / `--tls-key`)
for standalone use or terminates at the reverse proxy. Current Core boundaries
and guarantees are documented in `docs/architecture.md`.

## Building

Tagged releases publish signed, multi-architecture (`linux/amd64`,
`linux/arm64`) Core images to GHCR. Release assets include a CycloneDX SBOM;
the release workflow also creates build provenance and a keyless Cosign
signature for the published image manifest. Deploy by digest, not a mutable
tag. CI also rebuilds the production Dockerfile and verifies its non-root
runtime on every change.

```
cargo build --release
cargo test          # root checks and CLI integration tests
cargo test --manifest-path parsers/Cargo.toml --target-dir target/parser-tests
glasir selfcheck   # the same checks in one pass, in order
glasir benchmark . --check          # recall against bench/baseline.txt
glasir langcheck bench/langs --check # extraction against bench/languages.txt
```

The benchmark is a guard, not a report: a set falling below its floor fails the
build. Two silent regressions in one session are what put it there.

`langcheck` guards the other half. The recall floors read this repository, which
is Rust, so they cannot see a scanner that stopped extracting from Julia; this
reports definitions and edges per 1,000 lines per language, plus the share of
references attributed to the file rather than to an enclosing definition — the
number that moves from 3% to 99% when a language keyword stops being
recognised.

## Operating it

`/health` answers without a credential for liveness. `/ready` is the deployment
probe: it proves a graph is loaded and returns its `indexed_at`, publish
`generation` and any last re-index error. `/metrics` answers without a
credential with Prometheus counters — calls, errors and time per tool, refusals,
a latency histogram, and the graph's size. `/mcp` stays behind the token either
way.

Set `--max-index-age 900` on a watched production server to make `/ready`
return `503` once its loaded graph is older than the agreed 15-minute SLO. The
last valid graph remains available while the orchestrator replaces or repairs
the instance.

## Licence

Apache-2.0. See `LICENSE` and `NOTICE`.

The dependency inventory is recorded in `Cargo.lock`. Dependencies are permissively licensed — MIT, Apache-2.0,
BSD, ISC, Zlib, Unlicense — with no copyleft and no unclear terms. CI generates
a CycloneDX SBOM from `Cargo.lock` on every run, so that claim is checkable
rather than asserted.
