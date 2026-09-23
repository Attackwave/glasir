//! Command-line surface.
//!
//! Hand-rolled rather than a parser crate: the whole surface is a verb, a path
//! and a handful of flags, which is less code than wiring up a derive macro and
//! keeps the binary free of another dependency.

/// One parsed invocation.
pub struct Args {
    pub command: String,
    /// First positional argument, defaulting to the working directory.
    pub path: String,
    /// Remaining positional arguments. `token add <name>` is the only user of
    /// this so far — a verb with a sub-verb, which one field cannot carry.
    pub rest: Vec<String>,
    flags: Vec<String>,
    values: Vec<(String, String)>,
}

impl Args {
    pub fn parse(argv: impl Iterator<Item = String>) -> Args {
        let mut argv = argv.skip(1).peekable();
        // A leading flag means no command was given: `--help` and `-h` are the
        // whole invocation, not a verb.
        let command = match argv.peek() {
            Some(a) if a.starts_with('-') => String::new(),
            _ => argv.next().unwrap_or_default(),
        };
        let mut path = None;
        let mut flags = Vec::new();
        let mut values = Vec::new();
        let mut rest = Vec::new();

        while let Some(arg) = argv.next() {
            if let Some(name) = arg.strip_prefix("--") {
                // `--key value` and `--key=value` are both accepted; a flag
                // with no value is a boolean.
                if let Some((k, v)) = name.split_once('=') {
                    values.push((k.to_string(), v.to_string()));
                } else if matches!(
                    name,
                    "platform"
                        | "output"
                        | "as"
                        | "http"
                        | "token"
                        | "port"
                        | "questions"
                        | "floors"
                        | "days"
                        | "public-url"
                        | "tls-cert"
                        | "tls-key"
                        | "control-plane-client-ca"
                        | "control-plane-cidr"
                        | "language-rules"
                        | "max-index-age"
                ) {
                    if let Some(v) = argv.next() {
                        values.push((name.to_string(), v));
                    } else {
                        flags.push(name.to_string());
                    }
                } else {
                    flags.push(name.to_string());
                }
            } else if path.is_none() {
                path = Some(arg);
            } else {
                rest.push(arg);
            }
        }

        Args {
            command,
            path: path.unwrap_or_else(|| ".".into()),
            rest,
            flags,
            values,
        }
    }

    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

pub const HELP: &str = "\
glasir — a queryable graph of a codebase, served over MCP

USAGE
  glasir <command> [path] [options]

SETUP
  install [path]        Register with detected assistants, add git hooks
  uninstall [path]      Remove those registrations and hooks
  status [path]         Show registration and graph status

RUNNING
  serve [path]          Serve the graph over MCP on stdio (what an editor runs)
    --http <port|addr>  Serve over HTTP instead, for a remote agent
    --token <secret>    Require `Authorization: Bearer <secret>`
    --behind-control-plane
                      Private data-plane mode: loopback and per-service token only
    --control-plane-cidr <IPv4/prefix>
                      Permit a non-loopback control plane only from this CIDR
    --control-plane-client-ca <file>
                      PEM CA that must sign the control-plane mTLS client cert
    --max-index-age <seconds>
                      Return 503 from /ready when the loaded graph is older
    --watch             Re-index in the background as the tree changes
    --public-url <url>  The name clients reach this server under, behind a proxy
    --tls-cert <file>   PEM certificate chain; serves HTTPS when given with a key
    --tls-key <file>    PEM private key for --tls-cert
  token <verb> [name]   Per-user access tokens for a served tree
    add <name>          Mint a token and print it once — it is not recoverable
    rotate <name>       Atomically replace all tokens for this name
    revoke <name>       Remove that person's tokens; takes effect immediately
    list                Who has a token, and until when
    --days <n>          add: expire after n days (default: never)
  impact-of [rev]       What a change reaches: git diff -> symbols -> dependents
    --depth <n>         How many hops back to report (default 3)
  guard [path]          Check the architecture contract in glasir-rules.txt
    --rules <file>      Use a different contract
  analyse [path]        Refresh the stored graph and exit (what the git hooks run)
  watch [path]          Index the tree and follow edits, printing what changes
  view [path]           Open the graph as a map in the browser
    --port <n>          Port for the map (default 7878)

GRAPH
  import <graph.json>   Import a pre-built graph and report its cost
  why <question>        Why a question scores what it does: seeds and results
  benchmark [path]      Recall and token cost against bench/questions.txt
    --questions <file>  Use a different question set
    --check             Fail if recall fell below bench/baseline.txt

DIAGNOSTICS
  selfcheck             Every assert-based check, in one process
  lspcheck [path]       Probe a language server: does live tier 1 work here?
  lspfile <path> <file> The same for one file, printing what it resolved

OPTIONS
  --language-rules <dir> Load explicit TOML overrides for Go, Rust and Ruby
  --platform <name>     Registration target: claude, cursor, mcp (neutral)
  --user                Install for the user rather than the project
  --dry-run             Print what would be written, change nothing
  --no-hook             install: skip the git hooks that keep the graph current
  --purge               uninstall: also delete the stored graph
  --help                This text

Run `glasir install` in a repository, then configure an MCP-compatible client
with the generated local registration.
";
