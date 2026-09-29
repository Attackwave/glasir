//! Tier 1, live half: a language server over stdio.
//!
//! The batch half (`import_scip`) reads an index someone built earlier, so its
//! edges age the moment a file is saved. This keeps a language server alive
//! beside the watcher instead, and re-asks it about the files that changed, so
//! a save keeps compiler-grade `Extracted` edges rather than degrading to
//! tier 2's syntactic guesses.
//!
//! Measured against `rust-analyzer` on this repository: the handshake answers
//! in ~15 ms, cache priming finishes at ~5 s, and a reference query then costs
//! ~850 ms. Before priming ends the server answers references **successfully
//! with an empty list** — indistinguishable from "nothing calls this", and
//! enough to strip real edges out of the graph. Readiness is therefore tracked
//! from the `cachePriming` progress token and `references` refuses until then.
//!
//! `run_watch` holds one server per language for the life of the watch and
//! re-queries a saved file's symbols through it, replacing tier 2's guesses
//! wholesale. `glasir lspcheck` exercises the same path on demand.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long to wait for the server to finish indexing before trusting it.
/// Priming took ~6 s here; the margin covers a colder cache and a larger tree.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Per-request ceiling once the server is up, so one wedged query cannot stall
/// the watcher.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Language servers we know how to start. Adding one is a command and the
/// extensions it covers.
const SERVERS: &[(&str, &str, &[&str])] = &[
    ("rust-analyzer", "rust", &["rs"]),
    ("gopls", "go", &["go"]),
    (
        "typescript-language-server",
        "typescript",
        &["ts", "tsx", "js", "jsx"],
    ),
];

pub struct LspClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    /// Which language id this server was started for, for didOpen.
    language: &'static str,
    pub ready: bool,
}

/// The server that handles `path`, if one is configured and on PATH.
pub fn server_for(path: &Path) -> Option<(&'static str, &'static str)> {
    let ext = path.extension()?.to_str()?;
    SERVERS
        .iter()
        .find(|(_, _, exts)| exts.contains(&ext))
        .map(|&(cmd, lang, _)| (cmd, lang))
}

impl LspClient {
    /// Starts a server for `root` and waits until it can actually answer.
    ///
    /// Returns `None` when the binary is not installed — that is the cascade
    /// working as intended, not a failure: tier 2 covers the file instead.
    pub fn start(command: &str, language: &'static str, root: &Path) -> Option<LspClient> {
        let mut child = Command::new(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The server's own logging is not ours to relay.
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        let mut client = LspClient {
            child,
            stdin,
            stdout,
            next_id: 1,
            language,
            ready: false,
        };

        let root_uri = format!("file://{}", root.display());
        let id = client.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    // Asking for progress is what makes readiness observable.
                    "window": {"workDoneProgress": true},
                    "textDocument": {"references": {}, "documentSymbol": {}}
                }
            }),
        )?;
        client.await_response(id, READY_TIMEOUT)?;
        client.notify("initialized", json!({}))?;
        Some(client)
    }

    fn send(&mut self, msg: &Value) -> Option<()> {
        let body = serde_json::to_vec(msg).ok()?;
        // LSP frames messages with a Content-Length header, unlike MCP's
        // line-delimited stdio.
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).ok()?;
        self.stdin.write_all(&body).ok()?;
        self.stdin.flush().ok()
    }

    fn request(&mut self, method: &str, params: Value) -> Option<i64> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        Some(id)
    }

    fn notify(&mut self, method: &str, params: Value) -> Option<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn read_message(&mut self) -> Option<Value> {
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).ok()? == 0 {
                return None; // server exited
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = rest.trim().parse().ok()?;
            }
        }
        if length == 0 {
            return None;
        }
        let mut buf = vec![0u8; length];
        std::io::Read::read_exact(&mut self.stdout, &mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    /// Reads until the reply to `id` arrives, answering the server's own
    /// requests along the way.
    fn await_response(&mut self, id: i64, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let msg = self.read_message()?;
            // The server asks us to create a progress token before reporting
            // against it, and blocks on the answer.
            if msg["method"] == "window/workDoneProgress/create" {
                let reply_id = msg["id"].clone();
                self.send(&json!({"jsonrpc": "2.0", "id": reply_id, "result": null}))?;
                continue;
            }
            if msg["method"] == "$/progress" {
                // Cache priming is the point at which references stop coming
                // back empty. Anything earlier is not readiness.
                if msg["params"]["value"]["kind"] == "end"
                    && msg["params"]["token"]
                        .as_str()
                        .is_some_and(|t| t.contains("cachePriming"))
                {
                    self.ready = true;
                }
                continue;
            }
            if msg["id"] == json!(id) {
                return msg.get("result").cloned().or(Some(Value::Null));
            }
        }
        None
    }

    /// Tells the server a file's current contents.
    pub fn open(&mut self, path: &Path, text: &str) -> Option<()> {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": format!("file://{}", path.display()),
                "languageId": self.language,
                "version": 1,
                "text": text
            }}),
        )
    }

    /// Reports an edit. Full-document sync: sending the whole text is a few
    /// kilobytes and avoids tracking incremental ranges for no measurable gain
    /// at this file size.
    pub fn change(&mut self, path: &Path, text: &str, version: i64) -> Option<()> {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": format!("file://{}", path.display()), "version": version},
                "contentChanges": [{"text": text}]
            }),
        )
    }

    /// Symbols a file declares, as (name, line).
    pub fn document_symbols(&mut self, path: &Path) -> Option<Vec<(String, u32)>> {
        let id = self.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": format!("file://{}", path.display())}}),
        )?;
        let result = self.await_response(id, REQUEST_TIMEOUT)?;
        let mut out = Vec::new();
        collect_symbols(&result, &mut out);
        Some(out)
    }

    /// Everything referencing the symbol at `line`/`character`, as
    /// (file path, line).
    ///
    /// **Only meaningful once `ready`.** Before priming completes the server
    /// answers successfully with an empty list, which is indistinguishable
    /// from "nothing calls this" and would quietly delete real edges.
    pub fn references(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<(String, u32)>, &'static str> {
        // An unready server answers successfully with an empty list, which is
        // indistinguishable from "nothing calls this" — so refuse instead.
        if !self.ready {
            return Err("server has not finished indexing");
        }
        let id = self
            .request(
                "textDocument/references",
                json!({
                    "textDocument": {"uri": format!("file://{}", path.display())},
                    "position": {"line": line, "character": character},
                    "context": {"includeDeclaration": false}
                }),
            )
            .ok_or("could not send request")?;
        let result = self
            .await_response(id, REQUEST_TIMEOUT)
            .ok_or("no reply within the timeout")?;
        Ok(result
            .as_array()
            .ok_or("reply was not a location list")?
            .iter()
            .filter_map(|loc| {
                let uri = loc["uri"].as_str()?.strip_prefix("file://")?.to_string();
                let line = loc["range"]["start"]["line"].as_u64()? as u32;
                Some((uri, line))
            })
            .collect())
    }

    /// Drains messages until priming ends or the deadline passes.
    ///
    /// Called once after startup: the server answers queries long before its
    /// index exists, so a client that does not wait gets empty results that
    /// look like facts.
    pub fn wait_until_ready(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !self.ready && Instant::now() < deadline {
            // A ping we do not care about the answer to; reading its reply
            // pumps the progress notifications that precede it.
            let Some(id) = self.request("$/ping", json!({})) else {
                break;
            };
            if self.await_response(id, Duration::from_secs(2)).is_none() && !self.ready {
                continue;
            }
        }
        self.ready
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // The server is a child process; leaving one per watched tree behind
        // would leak a rust-analyzer per run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Re-parses one file at compiler grade: every symbol it defines, and every
/// call into those symbols, as (caller_file, caller_line, definition_name).
///
/// The direction is inverted from tier 2 on purpose. LSP answers "who
/// references this definition", not "what does this function call", so edges
/// are collected per definition and the caller is whoever the server names.
/// That is also why this cannot be scoped to one file the way tier 2 is: a
/// reference to a saved file's function usually lives somewhere else.
pub fn file_facts(
    client: &mut LspClient,
    path: &Path,
    source: &str,
) -> Option<Vec<(String, u32, String)>> {
    client.open(path, source);
    let symbols = client.document_symbols(path)?;
    let mut out = Vec::new();

    for (name, line) in symbols {
        // An impl block is not a callable and has no references of its own.
        if name.starts_with("impl") {
            continue;
        }
        let Some(col) = source
            .lines()
            .nth(line as usize)
            .and_then(|l| l.find(name.as_str()))
            .map(|c| c as u32)
        else {
            continue;
        };
        // A refusal means the server is not ready, and an empty answer from an
        // unready server would look like "no callers" — so give up on the file
        // rather than record a lie.
        let refs = client.references(path, line, col).ok()?;
        for (file, ref_line) in refs {
            out.push((file, ref_line, name.clone()));
        }
    }
    Some(out)
}

/// Flattens a documentSymbol reply.
///
/// The reply comes in one of two shapes and the server picks: a nested
/// `DocumentSymbol` tree with `selectionRange`, or a flat `SymbolInformation`
/// list whose position sits under `location.range`. rust-analyzer sends the
/// latter, so reading only the former yields zero symbols with no error.
fn collect_symbols(value: &Value, out: &mut Vec<(String, u32)>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for item in items {
        let line = item["selectionRange"]["start"]["line"]
            .as_u64()
            .or_else(|| item["range"]["start"]["line"].as_u64())
            .or_else(|| item["location"]["range"]["start"]["line"].as_u64());
        if let (Some(name), Some(line)) = (item["name"].as_str(), line) {
            out.push((name.to_string(), line as u32));
        }
        if let Some(children) = item.get("children") {
            collect_symbols(children, out);
        }
    }
}
