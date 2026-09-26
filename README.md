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

## Quickstart

Install with the package manager you already use:

```sh
brew install attackwave/glasir/glasir                      # macOS, Linux
scoop bucket add glasir https://github.com/Attackwave/scoop-glasir
scoop install glasir                                       # Windows
```

Or download the archive for your platform from
[Releases](https://github.com/Attackwave/glasir/releases), check it against
the `.sha256` beside it, and put `glasir` on your `PATH`:

| Platform | Archive |
|---|---|
| Linux x86_64 | `glasir-<version>-linux-x86_64.tar.gz` |
| Linux ARM64 | `glasir-<version>-linux-arm64.tar.gz` |
| macOS Apple Silicon | `glasir-<version>-macos-arm64.tar.gz` |
| macOS Intel | `glasir-<version>-macos-x86_64.tar.gz` |
| Windows x86_64 | `glasir-<version>-windows-x86_64.zip` |

The Linux binaries are statically linked and run on any distribution,
including Alpine and older enterprise releases. A binary downloaded by hand on
macOS is not yet notarized, so macOS refuses its first start; clear the flag
once, after checking the checksum: `xattr -d com.apple.quarantine glasir`.
Homebrew installs are not affected.

Then, in the repository you want to ask about:

```sh
glasir install .
```

That finds the coding assistants installed on your machine — by their program
on `PATH`, their settings in your home directory, or their files in the
project — and registers Glasir with each, in the project's own configuration:

| Assistant | File |
|---|---|
| Claude Code | `.mcp.json` |
| Codex | `.codex/config.toml` |
| Gemini CLI | `.gemini/settings.json` |
| Qwen Code | `.qwen/settings.json` |
| Cursor | `.cursor/mcp.json` |
| VS Code (Copilot) | `.vscode/mcp.json` |
| OpenCode | `opencode.json` |

It also installs git hooks that keep the graph current after a pull, checkout
or rebase. A registration file it creates holds absolute paths to your machine,
so it is added to `.gitignore`; one your team already tracks is merged into,
never replaced, and nothing in your home directory is touched. With no
assistant detected it writes a neutral `glasir-mcp.json` for any MCP client.
`--platform <name>` picks one explicitly.

Most assistants ask before they start a new server: Claude Code and Qwen Code
want the server approved, Codex and Gemini CLI read project settings only in a
folder you have trusted. `install` prints the step for each one it wrote.

Restart the assistant and ask it something about the code: *"what breaks if I
change `parse_config`?"*, *"how does a request reach the database?"*. The
answers come from `impact`, `query_graph`, `shortest_path` and ten more tools
listed under [What it serves](#what-it-serves).

```sh
glasir status .             # what is registered, what the graph holds
glasir why "your question"  # what the search saw, when an answer surprises you
glasir uninstall .          # remove the registration and the hooks
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
python3 bench/scale.py target/release/glasir --check   # 100k vs 1M lines
bench/foreign/fetch.sh /tmp/foreign          # eight foreign repos at pinned commits
glasir benchmark . --foreign /tmp/foreign --check
```

The foreign sets ask the structural questions — who calls this, who uses this
type, which tests to run — on eight repositories in Go, Java, Python, Kotlin,
C#, TypeScript, Rust and GDScript, with
answers read out of their source by hand.

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

Fourteen MCP tools, over stdio for a local editor or Streamable HTTP for a remote
agent:

- `overview` — the subsystems of an unfamiliar tree and the way into each
- `query_graph` — the subgraph relevant to a question
- `shortest_path` — how one symbol reaches another
- `explain_node` — callers, callees and subsystem of one symbol
- `impact` — what breaks if this changes, grouped by distance
- `cycles` — dependency cycles, compiler-resolved edges only by default
- `get_code_snippet` — the source a symbol names, from the range the parser
  recorded, so nothing has to open the file or guess a line
- `find_callers` — the direct callers of one symbol, as one list, and the
  definitions that use it by name without calling it (a type in a signature,
  a constant read, a base class)
- `detect_changes` — a git diff mapped to the definitions whose lines it
  touches (new files included) and what depends on them, with a risk level
  and its reasons: changed code that has callers and no test reaching it, and
  files the history says usually change too that the change leaves out

Three answer what to do *before* a commit, which is when an answer is
cheapest to act on:

- `affected_tests` — the tests that reach a symbol or the uncommitted diff,
  nearest first, with the files to run. Recognises the test conventions of
  each language (`tests/`, `spec/`, `__tests__/`, `test_x`, `x_test`,
  `x.spec`, `FooTest`, `TestX`)
- `co_changes` — the files that change in the same commits as one file, from
  the git history, marking those no edge in the graph connects: the hidden
  coupling — a template, a migration, a client in another language — that
  `impact` cannot see because no code names it
- `check_architecture` — the architecture contract in `glasir-rules.txt`
  (`deny <path> -> <path>`, `no-cycles`) checked against the graph, every
  broken rule with the edges that break it; pass rules ad hoc to test a
  boundary before writing it down. Under `serve --watch` the graph follows
  the working tree, so a change is checked before it is committed
- `find_unused` — definitions nothing in the tree refers to, confirmed against
  every text file rather than the call graph alone, with tests, annotated and
  generated code, overrides and framework conventions left out. On six real
  repositories in five languages, every name it reported appeared nowhere but
  in its own definition; what it cannot see — a library's public API, a name
  built at run time — is stated in each answer

One is for joining trees:

- `http_surface` — the HTTP routes this tree declares with their handlers, and
  the requests it sends with their senders, as matchable segments. Within a
  tree `find_callers` and `impact` already follow a request to its handler;
  Glasir Control uses this to do the same across the repositories of a
  workspace, where the client and its server live apart

Every tool declares itself read-only, idempotent and closed-world
(`readOnlyHint`, `openWorldHint: false`), so a client in a planning or
read-only mode may call all of them. The three that list without bound —
`impact`, `find_callers`, `find_unused` — take `limit` and `offset` and return
`next_offset` while more remain; the order is stable, so pages add up to the
whole answer.

**"Nothing calls this" is checked against the source.** The graph records
calls; a type in a signature, a constant read or a handler wired up in a
template is a use it has no edge for. `impact`, `find_callers` and
`affected_tests` therefore also name the files whose text names the symbol
without an edge connecting them (`named_elsewhere`), for every name the tree
defines exactly once — so an interface injected into three controllers is no
longer reported as breaking nothing.

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
it guessed. In Python, Go and JavaScript/TypeScript a call through an import —
`util.Do()`, `from pkg.b import helper as h; h()`, `import * as x from './x'` —
resolves to the file or package the import names, so a name defined in several
places still links to the right one instead of to none.

## Languages

147 languages are registered in `parsers/`, with native scanners and no
external grammar dependency. Among them Rust, Python, JavaScript,
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

## Asking in other languages

**English is the preferred language for questions.** Code is almost always
named in English, and `query_graph` matches the words of a question against
identifiers and documentation — it does not translate.

Everything structural is independent of language: `impact`, `find_callers`,
`shortest_path`, `explain_node`, `cycles` and `overview` work on symbols and
edges, and documentation and comments in any language are indexed.

A question in another language still finds English code when its technical
words share a stem with the code's — which technical vocabulary often does:
*server*, *serveur*, *servidor*; *compaction*, *compactage*, *compactación*;
German *Konfiguration* reaches `configuration` and *Aktion* reaches `action`,
because German `k` and `z` are matched against English `c` and `t`. Asked in
German, French and Spanish, the same questions about this repository land in
the right file, though English ranks the exact symbol highest.

What does not work is a word pair with no common stem — *Raum* and `room`,
*Leinwand* and `canvas`. Bridging those needs a dictionary or a language model,
and Glasir deliberately has neither in its query path, so the same question
always returns the same answer. When a question misses, the reply lists the
repository's own vocabulary so the assistant can ask again in its words.

| Question language | Status |
|---|---|
| English | Preferred; the reference for all benchmarks |
| German | Supported: question words are filtered, spelling differences to English are bridged |
| Other Latin-script languages | Works through shared technical stems; question words such as *comment* or *qué* are not filtered and count as search terms |
| Other scripts (Cyrillic, CJK, …) | Not measured; free-text questions are unlikely to match English code, structural tools are unaffected |

In practice the assistant in front of Glasir usually phrases its tool calls in
the code's own terms, whatever language you ask it in.

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
build.

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

Every dependency linked into the binary is permissively licensed — MIT,
Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0 or Unlicense — with no copyleft.
Each release archive carries their copyright notices and licence texts in
`THIRD-PARTY-LICENSES.txt`, generated by `cargo about` from the tagged
`Cargo.lock`; `about.toml` lists the accepted licences and the release fails on
any other. CI also generates a CycloneDX SBOM on every run, so the claim is
checkable rather than asserted.
