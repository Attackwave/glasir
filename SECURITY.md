# Security

## Reporting a vulnerability

Open a [private security advisory](https://github.com/Attackwave/glasir/security/advisories/new)
on GitHub. Please do not open a public issue for anything exploitable.

Expect an acknowledgement within a week. There is no bounty programme.

## Supported versions

Versions follow Semantic Versioning; before 1.0 a minor release may change
behaviour, as SemVer allows. Fixes land on the latest release. Before 1.0 there
are no backports: upgrade to the newest `0.x` to receive a fix. The release
notes name every vulnerability a release fixes, with its advisory. Each release
names its version (`glasir --version`), ships a SHA-256 and build provenance
per archive and a CycloneDX SBOM, and the container image is signed with Cosign
and carries build provenance.

## What Glasir touches

Worth knowing before an evaluation, because it bounds what a vulnerability here
could reach:

- **It reads source files and writes beside the tree**: `.glasir-graph` (the
  analysed graph), `.glasir-layout-*` (map coordinates, written by `view`),
  `.glasir-tokens` (SHA-256 hashes of access tokens, mode 0600) and
  `.glasir-audit.jsonl` (mode 0600). `install` adds them to `.gitignore`.
- **`install` writes editor configuration and git hooks**: an MCP registration
  for each assistant it detects (`.mcp.json`, `.codex/config.toml`,
  `.gemini/settings.json`, `.qwen/settings.json`, `.agents/mcp_config.json`, `.cursor/mcp.json`,
  `.vscode/mcp.json`, `opencode.json`, or the neutral `glasir-mcp.json`) holding the absolute
  path to the binary and the tree, `post-merge`, `post-checkout` and
  `post-rewrite` hooks that run `glasir analyse`, and for Claude Code a
  `SessionStart` hook in `.claude/settings.local.json` that runs `glasir hook`
  (reads the event from stdin, prints a note). Existing files are merged
  into, never replaced; a registration file it creates is gitignored.
  `uninstall` removes exactly what it wrote. A hook is appended to only when
  it is a shell script; the block runs in a subshell, so it cannot end the
  rest of the hook.
- **It never writes through a symbolic link inside the tree.** Every file
  above, and every hook, is replaced through a sibling file and a rename or
  opened with `O_NOFOLLOW`; a link planted where one of them goes is refused,
  so a tree cannot redirect a write to a file outside it. User-level
  registrations (`install --user`) still follow links, as dotfile managers
  expect.
- **The Claude Code plugin runs in every project you open.** It starts
  `glasir serve --plugin` on the session's directory, which refuses a directory
  outside a git repository, and adds the generated file names to that
  repository's `.git/info/exclude`. A session started at the root of a
  repository indexes that repository.
- **It makes no network connections of its own.** No telemetry, no model API, no
  update check. The only network sockets are the ones you ask it to listen on.
- **`serve` over stdio shares one local backend per tree.** On Unix the process
  a client starts relays to a backend on a Unix socket under
  `~/.local/state/glasir/sockets/` (or `$XDG_STATE_HOME`): the directory is
  mode 0700 and the socket 0600, so only the same user can connect. The backend
  runs detached, logs beside its socket, and ends a minute after its last
  client. It runs with the environment, `PATH` included, of the client that
  started it, so a language server or indexer is found where that client finds
  it. Each server also records its tree, parent and last request under
  `~/.local/state/glasir/servers/` for `glasir servers`. Where the socket
  cannot be reached, as in a container without a writable home, `serve` serves
  the client itself. `--own-process` keeps the old one-server-per-client
  behaviour; Windows always does.
- **No model anywhere in the index or query path**, so no code is sent anywhere
  for embedding or completion. This holds for documentation too, not only code.
- **It does start three external programs**, all from the system path: `git`
  (to locate the hooks directory, to read the diff for `detect_changes`,
  `affected_tests` and `impact-of`, and the commit history for `co_changes`),
  an indexer such as `rust-analyzer scip` when one is
  configured for the language, and a language server for tier-1 upgrades
  during `watch`. The last two are optional.
  Whoever can write to those binaries can already run code as the user, but the
  dependency belongs stated rather than discovered. A revision a client passes
  to `detect_changes` or `affected_tests` is refused when it starts with `-`,
  so git can never read it as an option (fixed in 0.3.0). A repository is
  refused, by name, when its own configuration — `.git/config`, any file it
  includes, or a worktree's own file — names a command git would run on those
  operations (`core.fsmonitor` other than git's daemon, `core.sshCommand`,
  `core.gitProxy`, `diff.external`, a diff driver's `command` or `textconv`,
  `filter.*`, `gpg.program`, a remote's `uploadpack`/`receivepack`/`proxy`,
  `extensions.partialClone`): a served tree is not trusted to run anything.
  Independently of that check, every git child runs with `core.fsmonitor`
  and `log.showSignature` off, without lazy fetching of missing objects,
  without external diffs or text conversion, and with any `GIT_DIR`-style
  variable of the calling environment removed. Each has a deadline and a
  bound on the output read.

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
is not a guarantee of correct extraction; the fixture checks are
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
  cannot leave an anonymous door open beside them. Once in effect, per-user
  auth stays in effect for the life of the process: an emptied, unreadable or
  deleted token file admits nobody. A `.glasir-tokens` that this user owns but
  others may read is ignored with a warning: `token issue` writes mode 0600,
  and a file that came with a clone or an archive does not look like that. The
  shared token is compared in constant time.

Also enforced: read and write timeouts per operation and a 30-second deadline
for a whole request to arrive, a bounded connection pool, a 1 MiB body cap, a
16 KiB line cap and a cap of 100 header lines — the caps before
authentication, since `Content-Length` is attacker-controlled and arrives first.

`glasir view` is unauthenticated. On loopback it answers only a `Host` naming
loopback, which a page reached through a rebound DNS name does not send; bound
to another address it warns that anyone reaching it can read the graph.

**A `GET` opens an event stream, and it holds a connection slot for as long as
it is open.** The pool is 32, so streams are subject to the same cap as any
other connection and a client that stops reading is disconnected by the write
timeout rather than holding its slot forever. The stream is only opened for a
caller that authenticated, and it carries no graph content — a re-index sends
`notifications/resources/updated`, which says that something changed and
nothing about what.

Four paths are answered without a token, because the callers they exist for
have none to offer: `GET /health` (liveness: node and symbol counts, which any
authorised `overview` already returns), `GET /ready` (the loaded graph's
`indexed_at`, publish generation and last re-index error), `GET /metrics`
(Prometheus counters per tool, refusals, a latency histogram and the graph's
size, with labels from a fixed list so a caller cannot mint series) and
`/.well-known/oauth-protected-resource` (RFC 9728 metadata a refused client
reads to learn how to authenticate). None of them carries source, symbol names
or questions. `/mcp` and every tool stay behind the token.

**TLS terminates here when you supply a certificate**: `--tls-cert` and
`--tls-key`, both PEM, both or neither. Without them the server speaks plain
HTTP and warns on a non-loopback address, because a bearer token in clear is
readable by anything on the path. Terminating at a reverse proxy instead
remains fine — that is why this warns rather than refuses. In this mode there
are no client certificates: the credential is the token, and TLS is what keeps
it unreadable on the wire.

**Behind Glasir Control** (`--behind-control-plane`) the Core is a private data
plane and is stricter. It requires a revocable per-service token from
`.glasir-tokens` and refuses a shared `--token`. It binds to loopback unless
`--control-plane-cidr` names the network the control plane connects from; a
peer outside that range is dropped without an HTTP response, so it does not
learn a graph service exists. With `--control-plane-client-ca` plus a
certificate and key, the listener requires mutual TLS and verifies the control
plane's client certificate against that CA. Token, network range and client
certificate are checked independently.

## Threat model

Glasir is a local, read-only index. It defends the machine against the tree it
reads: no MCP tool edits the tree or runs a command a client chooses, a
repository whose git configuration names a command is refused, files are
bounded in size and depth, and a stored graph is validated before use.

It cannot defend the model against the tree. Tools return source, comments and
documentation verbatim, so text written to steer an assistant ("ignore previous
instructions …") reaches the model exactly as it would through reading the file
directly. Treat answers about a tree you do not trust as untrusted input, and
do not give an assistant working on such a tree permission to run commands or
write files unattended. Glasir neither detects nor filters such text: any
filter would also hide real code.

Trust in the binary rests on what can be checked rather than on the author:
build provenance and SHA-256 per archive (`gh attestation verify <archive>
--repo Attackwave/glasir`), a signed container image, a CycloneDX SBOM, pinned
dependencies audited on every build, and the source under Apache-2.0.

## Known limits

Stated rather than discovered during an evaluation:

- **Indexing cost per file is bounded, not free.** Files over 1 MiB are skipped,
  and a file is read only up to where its brackets nest 1024 deep, so
  definitions past that point are missing. A crafted file of that size can still
  take seconds.
- **No RBAC and no multi-tenancy.** A valid token reaches every tool and the
  whole tree; one process serves one tree.
- **Token revocation is checked per request via the file's mtime.** On a
  filesystem with one-second timestamps, a revocation landing in the same second
  as the previous read can be missed until the file is touched again.
- **A corrupt `.glasir-graph` is rejected and rebuilt.** After deserialization
  the graph arrays are checked against each other (offsets, parallel edge
  arrays, targets, names, communities, hubs), so a file placed in a tree
  cannot make the server index out of bounds. The lower-level CSR import
  path owns its bytes rather than holding an mmap, so an external truncate
  cannot invalidate references inside a running process.
- **The audit log drops records rather than blocking**, and says so in the next
  written line (`dropped_before`). That trade is deliberate: an agent must not
  hang on a full disk.
