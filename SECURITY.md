# Security

## Reporting a vulnerability

Open a [private security advisory](../../security/advisories/new) on GitHub.
Please do not open a public issue for anything exploitable.

Expect an acknowledgement within a week. There is no bounty programme.

## What Glasir touches

Worth knowing before an evaluation, because it bounds what a vulnerability here
could reach:

- **It reads source files and writes three files beside the tree**:
  `.glasir-graph` (the analysed graph), `.glasir-tokens` (SHA-256 hashes of
  access tokens, mode 0600) and `.glasir-audit.jsonl` (mode 0600). `install`
  adds all three to `.gitignore`.
- **It makes no network connections of its own.** No telemetry, no model API, no
  update check. The only sockets are the ones you ask it to listen on.
- **No model anywhere in the index or query path**, so no code is sent anywhere
  for embedding or completion. This holds for documentation too, not only code.
- **It does start three external programs**, all of them optional and all from
  the tree being served or the system path: `git` (to locate the hooks
  directory), an indexer such as `rust-analyzer scip` when one is configured
  for the language, and a language server for tier-1 upgrades during `watch`.
  Whoever can write to those binaries can already run code as the user, but the
  dependency belongs stated rather than discovered.

## Language rules

`--language-rules <directory>` explicitly selects local TOML overrides. Files
are data, not scripts: no executable hooks or dynamic libraries are loaded.
Only `go.toml`, `rust.toml` and `ruby.toml` are currently accepted. Unknown TOML
files, fields, unsupported schema versions and invalid tokens are rejected
before indexing. Each file is limited to 64 KiB; lists are limited to 128
entries and tokens to 128 bytes. Rules are loaded once and held immutable until
restart. Their exact contents and the parser implementation version are stored
with the snapshot and compared before reuse.

Rules control which facts appear in the graph. Operators should version and
review overrides with the code that depends on them. A syntactically valid rule
is not a guarantee of correct extraction; the fixtures and corpus checks are
what measure that. No rule directory is discovered automatically.

## Serving over HTTP

The transport spec treats a locally reachable MCP server as a DNS-rebinding
target, and all three of its defences are enforced:

- **Loopback unless told otherwise.** A non-loopback address with no
  authentication configured refuses to start rather than warning.
- **`Origin` validation**, comparing the host exactly — a prefix test accepts
  `localhost.evil.com`, which the self-check asserts against.
- **Bearer tokens.** Per-user tokens take precedence over a shared `--token`:
  once any user token exists the shared one stops working, so issuing tokens
  cannot leave an anonymous door open beside them. The shared token is compared
  in constant time.

Also enforced: read and write timeouts, a bounded connection pool, a 1 MiB body
cap and a header-count cap — the last two before authentication, since
`Content-Length` is attacker-controlled and arrives first.

**A `GET` opens an event stream, and it holds a connection slot for as long as
it is open.** The pool is 32, so streams are subject to the same cap as any
other connection and a client that stops reading is disconnected by the write
timeout rather than holding its slot forever. The stream is only opened for a
caller that authenticated, and it carries no graph content — a re-index sends
`notifications/resources/updated`, which says that something changed and
nothing about what.

`GET /health` is answered without a token: an orchestrator probing liveness has
no credential to offer, and the reply carries only node and symbol counts,
which any authorised `overview` call already returns. Every other path stays
behind the token.

**TLS terminates here when you supply a certificate**: `--tls-cert` and
`--tls-key`, both PEM, both or neither. Without them the server speaks plain
HTTP and warns on a non-loopback address, because a bearer token in clear is
readable by anything on the path. Terminating at a reverse proxy instead
remains fine — that is why this warns rather than refuses. No client
certificates: the credential is the token, and TLS is what keeps it unreadable
on the wire.

## Known limits

Stated rather than discovered during an evaluation:

- **No RBAC and no multi-tenancy.** A valid token reaches every tool and the
  whole tree; one process serves one tree.
- **Token revocation is checked per request via the file's mtime.** On a
  filesystem with one-second timestamps, a revocation landing in the same second
  as the previous read can be missed until the file is touched again.
- **A corrupt `.glasir-graph` is rejected and rebuilt.** Snapshot archives are
  structurally validated before deserialization. The lower-level CSR import
  path owns its bytes rather than holding an mmap, so an external truncate
  cannot invalidate references inside a running process.
- **The audit log drops records rather than blocking**, and says so in the next
  written line (`dropped_before`). That trade is deliberate: an agent must not
  hang on a full disk.
