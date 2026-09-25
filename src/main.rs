// Phase 2 lands the watcher and parts of the delta API that phase 3's parser
// is the first caller of; they are exercised by demo() only indirectly.
#![allow(dead_code)]

use crate::parse_ast::LangExt;
use serde_json::json;

/// See the musl note in Cargo.toml.
#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod arena;
mod audit;
mod auth;
mod bench;
mod cli;
mod community;
mod contracts;
mod csr;
mod delta;
mod docs;
mod embed;
mod graph;
mod guard;
mod history;
mod http;
mod import_json;
mod import_scip;
mod imports;
mod ingest;
mod langcheck;
mod layout;
mod lsp;
mod mcp;
mod parallel;
mod parse_ast;
mod pem;
mod physics;
mod published;
mod resolve;
mod scip_wire;
mod search;
mod snapshot;
mod store;
#[cfg(test)]
mod test_checks;
mod view;
mod watcher;

use csr::{Confidence, CsrBuilder, Edge};

fn main() -> std::io::Result<()> {
    let args = cli::Args::parse(std::env::args());
    // Before the help branch, or `--version` prints the help and an operator
    // cannot answer "which version is running" — the question a CVE list is
    // checked against.
    if args.has("version") || args.has("V") {
        println!("glasir {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.has("help") || args.has("h") || args.command.is_empty() {
        print!("{}", cli::HELP);
        return Ok(());
    }
    if args.has("language-rules") || args.value("language-rules") == Some("") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "--language-rules requires a directory",
        ));
    }
    native_parsers::rules::configure(args.value("language-rules").map(std::path::Path::new))?;
    match args.command.as_str() {
        "install" => run_install(&args),
        "uninstall" => run_uninstall(&args),
        "status" => run_status(&args),
        "serve" => run_serve(&args),
        "view" => run_view(&args),
        "benchlayout" => bench_layout(&args.path),
        "token" => run_token(&args),
        "watch" => run_watch(&args.path),
        "impact-of" => run_impact_of(&args),
        "guard" => run_guard(&args),
        // Refreshes the stored snapshot and exits. What the git hooks run, and
        // useful by hand: it moves the cost of a cold analysis off the moment
        // an agent is waiting for an answer.
        "analyse" | "analyze" => {
            let root = canonical(std::path::Path::new(&args.path))?;
            let t = std::time::Instant::now();
            let a = analyse(&root)?;
            eprintln!(
                "glasir: {} nodes, {} symbols in {:?}{}",
                a.snap.width(),
                a.registry.len(),
                t.elapsed(),
                if a.from_snapshot {
                    " (incremental)"
                } else {
                    ""
                }
            );
            Ok(())
        }
        // Why one question scores what it does: seeds, results, and where the
        // expected answer actually ranked. Diagnostic, not part of any measure.
        "why" => run_why(&args),
        "import" => bench_import(&args.path),
        "benchmark" => run_benchmark(&args),
        "langcheck" => run_langcheck(&args),
        // No argument at all runs the self-checks, which is how the build is
        // verified; an unknown verb is a mistake worth naming.
        "selfcheck" => demo(),
        // Live tier 1 against a real language server: needs one installed, so
        // it is a separate command rather than part of the self-checks.
        "lspcheck" => probe_lsp(&args.path),
        "lspfile" => {
            let root = canonical(std::path::Path::new(&args.path))?;
            // Named, not panicked: a missing argument is a usage mistake, and
            // an unwrap here printed a backtrace where one line would do.
            let Some(f) = std::env::args().nth(3) else {
                eprintln!("usage: glasir lspfile <root> <file>");
                std::process::exit(2);
            };
            probe_lsp_file(&root, &canonical(std::path::Path::new(&f))?)
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print!("{}", cli::HELP);
            std::process::exit(2);
        }
    }
}

/// Exercises the live LSP client end to end against whatever server covers the
/// tree. Reports timings, because the interesting property is how long the
/// server answers wrongly before it answers correctly.
fn probe_lsp(root: &str) -> std::io::Result<()> {
    use std::time::{Duration, Instant};

    let root = canonical(std::path::Path::new(root))?;
    let file = walk(&root)
        .into_iter()
        // build.rs sits outside the crate graph, so a server answers about it
        // with nothing at all; probe something the crate actually contains.
        .filter(|p| p.components().any(|c| c.as_os_str() == "src"))
        .find(|p| p.extension().is_some_and(|e| e == "rs"))
        .ok_or_else(|| std::io::Error::other("no source file to probe"))?;
    let (command, language) = lsp::server_for(&file)
        .ok_or_else(|| std::io::Error::other("no server for this language"))?;
    println!("server: {command} for {}", file.display());

    let t = Instant::now();
    let Some(mut client) = lsp::LspClient::start(command, language, &root) else {
        println!("{command} is not installed — tier 2 covers this tree instead");
        return Ok(());
    };
    println!("handshake: {:?}", t.elapsed());

    let src = std::fs::read_to_string(&file)?;
    client.open(&file, &src);

    let t = Instant::now();
    let ready = client.wait_until_ready(Duration::from_secs(60));
    println!("indexed: {:?} (ready={ready})", t.elapsed());

    let symbols = client.document_symbols(&file).unwrap_or_default();
    println!("symbols: {}", symbols.len());

    // Ask about the first function-shaped symbol and see whether the answer is
    // real. Before priming the server returns an empty list, successfully.
    // Ask about every symbol until one answers: a reported line may point at a
    // declaration keyword rather than the identifier, and the server returns
    // null for a position that is not on a name.
    for (name, line) in symbols.iter().filter(|(n, _)| !n.starts_with("impl")) {
        let Some(col) = src
            .lines()
            .nth(*line as usize)
            .and_then(|l| l.find(name.as_str()))
            .map(|c| c as u32)
        else {
            continue;
        };
        let t = Instant::now();
        match client.references(&file, *line, col) {
            Ok(refs) => {
                println!("references to {name}: {} in {:?}", refs.len(), t.elapsed());
                for (f, l) in refs.iter().take(3) {
                    println!("   {}:{l}", f.rsplit('/').next().unwrap_or(f));
                }
                return Ok(());
            }
            Err(why) => println!("references to {name}: {why}"),
        }
    }
    Ok(())
}

/// Cost of a whole-file re-query, which is what a save would trigger.
fn probe_lsp_file(root: &std::path::Path, file: &std::path::Path) -> std::io::Result<()> {
    use std::time::{Duration, Instant};
    let (command, language) =
        lsp::server_for(file).ok_or_else(|| std::io::Error::other("no server"))?;
    let Some(mut client) = lsp::LspClient::start(command, language, root) else {
        return Ok(());
    };
    client.wait_until_ready(Duration::from_secs(60));
    let src = std::fs::read_to_string(file)?;
    let t = Instant::now();
    let facts = lsp::file_facts(&mut client, file, &src).unwrap_or_default();
    println!(
        "whole-file re-query: {} edges in {:?} for {}",
        facts.len(),
        t.elapsed(),
        file.display()
    );
    Ok(())
}

/// Where Glasir writes its optional local MCP registration.
struct Target {
    /// What `--platform` calls it.
    name: &'static str,
    /// Config file relative to the project root.
    project: Option<&'static str>,
    /// Config file relative to the user's home, if supported.
    user: Option<&'static str>,
    format: Format,
    /// Programs on `PATH` that show the client is installed.
    binaries: &'static [&'static str],
    /// Paths under the project that show the client is in use.
    markers: &'static [&'static str],
    /// Paths under the user's home that show the client is installed.
    home_markers: &'static [&'static str],
    /// What the user does next, printed after a write.
    next: &'static str,
}

/// How a client stores MCP servers. Each shape was taken from what the client
/// itself writes (`<cli> mcp add` under a scratch `$HOME`) or lists back, not
/// from memory: a wrong key is a registration the client silently ignores.
#[derive(Clone, Copy, PartialEq)]
enum Format {
    /// `{"mcpServers": {"glasir": {"type": "stdio", "command", "args"}}}`
    McpJson,
    /// The same without `type`, as Gemini CLI and Qwen Code write it.
    McpJsonPlain,
    /// VS Code's `{"servers": {...}}`.
    VsCode,
    /// OpenCode's `{"mcp": {"glasir": {"type": "local", "command": [...]}}}`.
    OpenCode,
    /// Codex's `[mcp_servers.glasir]` table, edited as text so the comments and
    /// layout of the rest of the file survive.
    CodexToml,
}

impl Format {
    /// The JSON object holding the servers; `None` for the TOML format.
    fn container(self) -> Option<&'static str> {
        match self {
            Format::McpJson | Format::McpJsonPlain => Some("mcpServers"),
            Format::VsCode => Some("servers"),
            Format::OpenCode => Some("mcp"),
            Format::CodexToml => None,
        }
    }

    fn entry(self, exe: &str, root: &str) -> serde_json::Value {
        match self {
            Format::OpenCode => serde_json::json!({
                "type": "local",
                "command": [exe, "serve", root],
                "enabled": true,
            }),
            Format::McpJsonPlain => serde_json::json!({"command": exe, "args": ["serve", root]}),
            _ => serde_json::json!({"type": "stdio", "command": exe, "args": ["serve", root]}),
        }
    }
}

const CODEX_HEADER: &str = "[mcp_servers.glasir]";

/// The file with our `[mcp_servers.glasir]` table and its subtables removed,
/// and whether one was there.
fn strip_codex_block(text: &str) -> (String, bool) {
    let mut out = String::new();
    let mut inside = false;
    let mut found = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == CODEX_HEADER || t.starts_with("[mcp_servers.glasir.");
            found |= inside;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, found)
}

fn codex_block(exe: &str, root: &str) -> String {
    // A JSON string literal is a valid TOML basic string.
    let q = |v: &str| serde_json::Value::from(v).to_string();
    format!(
        "{CODEX_HEADER}\ncommand = {}\nargs = [\"serve\", {}]\n",
        q(exe),
        q(root)
    )
}

/// The neutral registration is useful to deployment tooling without coupling
/// the Core to a particular editor, assistant, or vendor configuration, and it
/// is what `install` writes when no client below is detected.
///
/// The clients are written only where they are found. Without them `install`
/// ended in a file no client reads and a sentence telling the user to finish
/// the job by hand, which is where a first-time user stops. Every client has a
/// project-scoped file, so one tree's registration never overwrites another's.
const TARGETS: &[Target] = &[
    Target {
        name: "mcp",
        project: Some("glasir-mcp.json"),
        user: None,
        format: Format::McpJson,
        binaries: &[],
        markers: &[],
        home_markers: &[],
        next: "point your MCP client at the entry in glasir-mcp.json",
    },
    Target {
        name: "claude",
        project: Some(".mcp.json"),
        user: None,
        format: Format::McpJson,
        binaries: &["claude"],
        markers: &[".mcp.json", ".claude"],
        home_markers: &[".claude"],
        next: "Claude Code: restart it here and approve the glasir server",
    },
    Target {
        name: "cursor",
        project: Some(".cursor/mcp.json"),
        user: Some(".cursor/mcp.json"),
        format: Format::McpJson,
        binaries: &["cursor", "cursor-agent"],
        markers: &[".cursor"],
        home_markers: &[".cursor"],
        next: "Cursor: enable glasir under Settings > MCP",
    },
    Target {
        name: "codex",
        project: Some(".codex/config.toml"),
        user: Some(".codex/config.toml"),
        format: Format::CodexToml,
        binaries: &["codex"],
        markers: &[".codex"],
        home_markers: &[".codex"],
        // Measured: an untrusted project's .codex/config.toml is ignored.
        next: "Codex: start it here and trust the project when asked",
    },
    Target {
        name: "gemini",
        project: Some(".gemini/settings.json"),
        user: Some(".gemini/settings.json"),
        format: Format::McpJsonPlain,
        binaries: &["gemini"],
        markers: &[".gemini"],
        home_markers: &[".gemini"],
        next: "Gemini CLI: restart it here",
    },
    Target {
        name: "qwen",
        project: Some(".qwen/settings.json"),
        user: Some(".qwen/settings.json"),
        format: Format::McpJsonPlain,
        binaries: &["qwen"],
        markers: &[".qwen"],
        home_markers: &[".qwen"],
        next: "Qwen Code: restart it here",
    },
    Target {
        name: "vscode",
        project: Some(".vscode/mcp.json"),
        user: None,
        format: Format::VsCode,
        binaries: &["code", "code-insiders"],
        markers: &[".vscode"],
        home_markers: &[],
        next: "VS Code: open this folder and start glasir from .vscode/mcp.json",
    },
    Target {
        name: "opencode",
        project: Some("opencode.json"),
        user: None,
        format: Format::OpenCode,
        binaries: &["opencode"],
        markers: &["opencode.json", ".opencode"],
        home_markers: &[".config/opencode", ".opencode"],
        next: "OpenCode: restart it here",
    },
];

/// Whether a program of this name is on `PATH`.
fn on_path(bin: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        ["", ".exe", ".cmd"]
            .iter()
            .any(|ext| dir.join(format!("{bin}{ext}")).is_file())
    })
}

impl Target {
    /// Config path for the requested scope, if this target has one.
    fn path(&self, root: &std::path::Path, user_scope: bool) -> Option<std::path::PathBuf> {
        if user_scope {
            let home = home()?;
            self.user.map(|p| join_rel(&home, p))
        } else {
            self.project.map(|p| join_rel(root, p))
        }
    }

    /// Whether the client is in use here: its program on `PATH`, its settings
    /// in the home directory, or its files in the project. The neutral
    /// registration has no markers and is always available.
    fn detected(&self, root: &std::path::Path) -> bool {
        if !self.is_client() {
            return true;
        }
        let home = home();
        self.markers.iter().any(|m| root.join(m).exists())
            || home.is_some_and(|h| self.home_markers.iter().any(|m| h.join(m).exists()))
            || self.binaries.iter().any(|b| on_path(b))
    }

    fn is_client(&self) -> bool {
        !self.markers.is_empty()
    }
}

/// Builds the graph for a tree: tier 1 where an index covers it, tier 2 for the
/// rest, then tier 3 over the whole registry.
///
/// Shared by `serve`, `view` and the initial sweep of `watch` — three commands
/// that need the same graph and differ only in what they do with it.
fn build_graph(
    root: &std::path::Path,
) -> std::io::Result<(
    std::sync::Arc<graph::Graph>,
    ingest::SymbolRegistry,
    arena::SymbolArena,
)> {
    let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let mut arena = arena::SymbolArena::new();
    let mut reg = ingest::SymbolRegistry::new(0);

    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let scip = root.join("index.scip");
    if scip.exists()
        && let Ok(r) = import_scip::import(&scip, root, &mut reg)
    {
        covered.extend(r.fresh_files().cloned());
        let edges = r.edges.clone();
        g.update(|_, d| {
            for &(src, e) in &edges {
                d.add_edge(src, e);
            }
        });
    }

    // Read and parse every file in parallel, then fold the results in serially.
    //
    // With the former tree-sitter parser, parsing was 63% of a cold start — 3.2 s of
    // 5.0 s on a 153k-line tree. Parsing is stateless: each scanner reads a string and
    // returns facts. What follows it is not: the registry mints node ids, the
    // arena interns paths, the delta is one structure. So the split is exactly
    // there, and the fold keeps `walk()`'s order, which is what makes the
    // parallel result bit-identical to the serial one rather than merely
    // equivalent. Determinism is not negotiable here; every id downstream
    // depends on the order symbols are first seen.
    let now = now();
    // The source text is dropped as soon as it has been parsed, and kept only
    // for Markdown, which is turned into nodes during the fold rather than
    // before it. Holding every file's text until then cost 142 MB against 34 MB
    // on a 153k-line tree — the parallel version's one real regression, and
    // avoidable because nothing downstream of extraction needs the bytes.
    // Historical tree-sitter memory measurements: the facts for
    // a 153k-line tree are ~10 MB, while sixteen tree-sitter parses in flight
    // cost 130 MB against 33 MB for one. The syntax trees are what is large,
    // and they are transient — so the cap is on how many exist at once.
    //
    // Measured on that tree, resident set after parsing against wall time:
    //
    //   threads |  1     2     4     8    16
    //   MB      | 33    41    56    83   130
    //   seconds | 3.2   2.1   1.4   1.15  1.05
    //
    // Eight is where the curve turns: past it another 47 MB buys 0.1 s. A
    // server holding a monorepo resident is the case that matters, and there
    // the memory is the risk rather than a fifth of a second at startup.
    let paths = walk(root);
    let mut parsed: Vec<(
        std::path::PathBuf,
        Option<String>,
        Option<parse_ast::FileFacts>,
    )> = parallel::map_ordered(&paths, |path| {
        let src = read_source(path)?;
        let facts = parse_ast::Lang::from_path(path)
            .and_then(|lang| parse_ast::parse_file(path, &src, lang));
        let keep = docs::is_markdown(path).then_some(src);
        Some((path.clone(), keep, facts))
    })
    .into_iter()
    .flatten()
    .collect();

    // One delta for the whole tree rather than one published snapshot per
    // file. `update` clones the delta on every call so readers are never
    // disturbed — right while agents are querying, quadratic while a tree
    // is being built: measured over 1M lines, the per-file cost rose from
    // 24 µs to 9,482 µs as the delta grew past 280,000 edges. Compaction
    // cannot help, since the worker is speculative and loses every race
    // against a fold that writes faster than it rebuilds.
    //
    // Markdown is folded after the code, because a mention only becomes an
    // edge when the symbol already exists.
    let mut markdown: Vec<(std::path::PathBuf, String)> = Vec::new();
    g.update_batch(|snap, d| {
        for (path, src, facts) in parsed.drain(..) {
            // Tier 1 supersedes tier 2 for *edges* — that is what the
            // cascade is for — but it carries no documentation, and prose
            // is what a question is phrased in. So a covered file still
            // gets parsed for its docs, while its edges stay
            // compiler-resolved. Skipping the file entirely left every
            // symbol in an indexed tree undocumented, which is most of
            // them, and search fell back to matching identifiers alone.
            if let Some(src) = src {
                markdown.push((path, src));
                continue;
            }
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
            if covered.contains(&*rel) {
                drop(rel);
                ingest::ingest_docs_from(&mut reg, &path, root, facts);
                continue;
            }
            if let Some(facts) = facts {
                ingest::apply_facts_into(snap, d, &mut arena, &mut reg, &path, root, facts, now);
            }
        }
        ingest::ingest_markdown_into(snap, d, &mut arena, &mut reg, root, &markdown, now);
    });
    reg.prune_refs();
    link_placeholders_quiet(&g, &reg);
    Ok((g, reg, arena))
}

/// Largest source file read, in bytes.
///
/// A generated file — protobuf output, bindings, a migration — can run to tens
/// of megabytes, and eight of those parse in parallel. Measured: a 12 MB file
/// did not finish in two minutes before the parser's reference attribution was
/// made linear, and even now the memory of eight simultaneous syntax trees is
/// the risk.
///
/// **1 MiB, and the 4 MiB it replaces was never measured.** What costs is a
/// deeply nested generated table, not size as such: parsing `i386-tbl.h`
/// (5.1 MB of one initialiser list) costs 2.45 s and **486 MB of resident
/// memory for two symbols**, against 113 ms and 49 MB for this whole
/// repository's 1,384. The parse pool runs eight files at once, so the worst
/// case *under the old limit* is eight of those: measured at **3.0 GB RSS and
/// zero symbols**. Cost is linear in file size (0.34/0.65/1.28/2.60 s over
/// 0.7/1.4/2.7/5.3 MB), so the limit is the only thing bounding it.
///
/// 1 MiB is where the two populations separate, measured over 14,294 source
/// files in five trees: the largest hand-written file is `tc-arm.c` at 793 KB
/// (1,112 symbols), and everything above 1 MB is a generated table —
/// `arc-tbl.h` 1.2 MB/**0** symbols, `m32c-opc.h` 1.1 MB/11, `m32c-opc.c`
/// 4.0 MB/23. The cap drops 0.04% of files and costs no hand-written code.
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// Reads a source file, reporting the reasons it might not be readable.
///
/// `read_to_string(..).ok()` treats three different situations as one: a file
/// that is not valid UTF-8 (Latin-1 sources are common in older C and Java
/// trees), a file the process may not read, and a file that is simply absent.
/// All three vanished from the graph without a word — measured, a tree of three
/// files yielded one symbol and reported success. A tool that silently indexes
/// half a repository is worse than one that refuses, because nobody knows to
/// look.
pub(crate) fn read_source(path: &std::path::Path) -> Option<String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_SOURCE_BYTES => {
            eprintln!(
                "glasir: skipping {} — {:.1} MB exceeds the {} MB limit for one file",
                path.display(),
                m.len() as f64 / (1024.0 * 1024.0),
                MAX_SOURCE_BYTES / (1024 * 1024)
            );
            return None;
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("glasir: skipping {} — {e}", path.display());
            return None;
        }
    }
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            // Latin-1 fallback. Refusing the file loses all of it over bytes
            // that are almost never in the code: measured on the four
            // non-UTF-8 files in binutils plus zlib's C# bindings, every one
            // is broken by a single byte in a copyright header — `Kämpf`,
            // `Jean-François`, `©` — while the code below is plain ASCII.
            // Indexing them takes that fixture from **52 to 268 symbols**.
            //
            // Latin-1 is the right fallback because it cannot fail: every byte
            // maps to the code point of the same value, so this decodes any
            // input rather than refusing a second time. It may mis-read a
            // different 8-bit encoding's accents, but an identifier is ASCII
            // in every one of them, and a wrong accent in a comment costs a
            // term where refusing costs the file.
            Err(e) => {
                let text: String = e.as_bytes().iter().map(|&b| b as char).collect();
                eprintln!(
                    "glasir: {} is not valid UTF-8 — reading it as Latin-1",
                    path.display()
                );
                Some(text)
            }
        },
        Err(e) => {
            eprintln!("glasir: skipping {} — {e}", path.display());
            None
        }
    }
}

/// Everything a reader needs, from a stored snapshot when the tree has not
/// changed and from source otherwise.
///
/// This is what makes a second start cheap: parsing a tree costs ~80 ms here
/// and seconds on a large repository, while mapping a snapshot back costs
/// microseconds. The snapshot carries the names too, which a bare `.csr` does
/// not — that omission is why every command used to rebuild.
struct Analysed {
    snap: std::sync::Arc<graph::GraphSnapshot>,
    registry: ingest::SymbolRegistry,
    communities: community::Communities,
    defined: std::collections::HashSet<csr::NodeId>,
    from_snapshot: bool,
}

/// The user's home directory. Windows sets `USERPROFILE` and usually no
/// `HOME`, so reading `HOME` alone made `--user` refuse and every client's
/// home-directory marker invisible there. `HOME` first, which is what the
/// checks set.
fn home() -> Option<std::path::PathBuf> {
    home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

fn home_from(
    home: Option<std::ffi::OsString>,
    profile: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    home.or(profile).map(std::path::PathBuf::from)
}

/// A `/`-separated relative path joined component by component, so Windows
/// prints `tree\.vscode\mcp.json` rather than `tree\.vscode/mcp.json`.
fn join_rel(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |p, part| p.join(part))
}

/// `canonicalize`, without the `\\?\` prefix Windows puts on every result.
///
/// Measured on Windows: `install` wrote `\\?\C:\Users\…\tree` into a
/// client's configuration and `status` printed it. Both work — the prefix is
/// a valid path — but a client that passes the path on through a shell or a
/// URL does not expect it, and a person reading it should not have to. Only a
/// drive path short of `MAX_PATH` loses it: a longer one needs the prefix to
/// be opened at all, and `\\?\UNC\` is left as it is.
fn canonical(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let full = path.canonicalize()?;
    if let Some(plain) = full.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && plain.as_bytes().get(1) == Some(&b':')
        && plain.len() < 260
    {
        return Ok(std::path::PathBuf::from(plain));
    }
    Ok(full)
}

fn analyse(root: &std::path::Path) -> std::io::Result<Analysed> {
    let path = root.join(".glasir-graph");
    let files = walk(root);

    if let Some((stored, stale)) = snapshot::read_partial(&path, root) {
        // Files the snapshot never saw — added since it was written.
        let known: std::collections::HashSet<&str> =
            stored.sources.iter().map(|(f, _)| f.as_str()).collect();
        let added: Vec<std::path::PathBuf> = files
            .iter()
            .filter(|p| {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string();
                !known.contains(rel.as_str())
            })
            .cloned()
            .collect();

        // Above this, re-parsing the tree beats patching it: every changed file
        // costs a delta entry that every later read filters through, and the
        // partition has to be recomputed regardless. A fifth is the same
        // proportion `run_watch` uses to decide an index has drifted too far.
        let churn = stale.len() + added.len();
        let too_far = churn * 5 > stored.sources.len().max(1);

        if !too_far {
            let mut registry = ingest::SymbolRegistry::new(stored.csr.node_count());
            for (id, name) in stored.names.iter().enumerate() {
                if !name.is_empty() {
                    registry.insert(name.clone(), id as csr::NodeId);
                }
            }
            for (node, doc) in &stored.docs {
                registry.set_doc(*node, doc.clone());
            }
            for (node, span) in &stored.spans {
                registry.set_span(*node, *span);
            }
            for (file, refs) in stored.refs {
                registry.set_refs(&file, refs);
            }
            let g = std::sync::Arc::new(graph::Graph::new(stored.csr));

            let refreshed =
                churn > 0 && refresh_files(&g, &mut registry, root, &stale, &added).is_some();
            if refreshed || churn == 0 {
                let snap = g.load();
                let defined: std::collections::HashSet<csr::NodeId> = registry
                    .entries()
                    .filter(|(n, _)| n.contains('#'))
                    .map(|(_, &n)| n)
                    .collect();
                // The partition is recomputed when anything changed: a stored
                // one is indexed by node id and a re-parse mints new ids, so
                // reusing it would label the wrong symbols.
                let communities = if churn == 0 {
                    community::Communities {
                        of_node: stored.community,
                        hubs: stored.hubs,
                        pendants: Vec::new(),
                    }
                } else {
                    community::detect(
                        &snap,
                        &community::Params::default(),
                        &community::Context::from_names(
                            Some(&defined),
                            snap.width(),
                            registry.entries().map(|(s, &n)| (s.as_str(), n)),
                        ),
                    )
                };
                if churn > 0 {
                    store_snapshot(root, &path, &snap, &registry, &communities, &files)?;
                }
                return Ok(Analysed {
                    snap,
                    registry,
                    communities,
                    defined,
                    from_snapshot: true,
                });
            }
        }
    }

    let (g, reg, _arena) = build_graph(root)?;
    let snap = g.load();
    let defined: std::collections::HashSet<csr::NodeId> = reg
        .entries()
        .filter(|(n, _)| n.contains('#'))
        .map(|(_, &n)| n)
        .collect();
    let communities = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );

    store_snapshot(root, &path, &snap, &reg, &communities, &files)?;
    Ok(Analysed {
        snap,
        registry: reg,
        communities,
        defined,
        from_snapshot: false,
    })
}
/// Node-ordered, so a snapshot's bytes do not depend on HashMap iteration order.
fn sorted_by_node<T>(it: impl Iterator<Item = (csr::NodeId, T)>) -> Vec<(csr::NodeId, T)> {
    let mut v: Vec<(csr::NodeId, T)> = it.collect();
    v.sort_unstable_by_key(|(n, _)| *n);
    v
}

/// Writes the analysed graph beside the tree.
///
/// Shared by the full build and the incremental path, which must produce the
/// same stored form — an incremental start that wrote a subtly different
/// snapshot would be worse than no incremental start at all.
fn store_snapshot(
    root: &std::path::Path,
    path: &std::path::Path,
    snap: &graph::GraphSnapshot,
    reg: &ingest::SymbolRegistry,
    communities: &community::Communities,
    files: &[std::path::PathBuf],
) -> std::io::Result<()> {
    // Compacting first folds the delta into a base CSR, which is the only form
    // that can be stored — the delta is runtime state.
    let compacted = snap.compact();
    let mut names = vec![String::new(); compacted.base.node_count()];
    for (symbol, &node) in reg.entries() {
        if let Some(slot) = names.get_mut(node as usize) {
            *slot = symbol.clone();
        }
    }
    let mut stored = snapshot::Snapshot::new(
        std::sync::Arc::try_unwrap(compacted.base).unwrap_or_default(),
        names,
        communities.of_node.clone(),
        communities.hubs.clone(),
        // Sorted for the same reason `source_times` is: both come off a
        // HashMap, whose order varies per run, so an unsorted vector makes two
        // identical analyses write different bytes. The graph was already
        // deterministic — answers matched — but the file was not, which is
        // what a reader comparing snapshots across machines checks.
        sorted_by_node(reg.docs().iter().map(|(&n, d)| (n, d.clone()))),
        sorted_by_node(reg.spans().iter().map(|(&n, &s)| (n, s))),
        snapshot::source_times(root, files),
    );
    let mut refs: Vec<(String, Vec<(csr::NodeId, String)>)> = reg
        .refs()
        .iter()
        .map(|(f, r)| (f.clone(), r.clone()))
        .collect();
    refs.sort();
    stored.refs = refs;
    let contract_path = root.join(".glasir/contracts.json");
    if contract_path.exists() {
        stored.contracts =
            serde_json::to_value(contracts::load(&contract_path).map_err(std::io::Error::other)?)
                .map_err(std::io::Error::other)?;
    }
    // A failed write is not fatal — the graph in memory is correct and the
    // command still answers — but it must not be silent: every later start
    // then rebuilds the whole tree instead of mapping the snapshot, which on a
    // 1M-line tree is 11.9 s against 160 ms, on every start, with nothing
    // saying why. Measured in a read-only tree: 9 ms against 0.09 ms here.
    if let Err(e) = snapshot::write(&stored, path) {
        eprintln!(
            "glasir: could not store the graph at {} — {e}\n\
             every start will rebuild the tree until this is fixed",
            path.display()
        );
    }
    Ok(())
}

/// Re-parses the files a snapshot has fallen behind on, in place.
///
/// This is what makes a start incremental. Previously one edited file discarded
/// the whole snapshot, so a save in a large tree cost a full re-analysis —
/// measured on a 153k-line tree, roughly a second, and rising with the
/// repository.
///
/// The scoping already exists: `DeltaStore` tracks which nodes a file owns and
/// `replace_file_edges` invalidates exactly those. What the stored form lacks
/// is that mapping, because compaction folds the delta away — but it does not
/// need to carry it, since every symbol name is prefixed with its file and the
/// mapping can be rebuilt in one pass.
///
/// Returns `None` when a file cannot be read, leaving the caller to fall back
/// to a full build rather than serving a graph missing part of the tree.
fn refresh_files(
    g: &std::sync::Arc<graph::Graph>,
    reg: &mut ingest::SymbolRegistry,
    root: &std::path::Path,
    stale: &[String],
    added: &[std::path::PathBuf],
) -> Option<()> {
    let mut arena = arena::SymbolArena::new();
    // Rebuild file ownership from the names, then hand it back to the delta so
    // `replace_file_edges` can scope a re-parse the way it does at runtime.
    let mut owned: std::collections::HashMap<String, Vec<csr::NodeId>> = Default::default();
    for (symbol, &node) in reg.entries() {
        if let Some((file, _)) = symbol.split_once('#') {
            owned.entry(file.to_string()).or_default().push(node);
        }
    }

    let mut paths: Vec<std::path::PathBuf> = stale.iter().map(|f| root.join(f)).collect();
    paths.extend(added.iter().cloned());
    // Deterministic: the order files are folded in decides which ids new
    // symbols get, exactly as in a full build.
    paths.sort();

    // Extraction goes through the pool, the fold stays serial and sorted —
    // the same split the whole-tree path uses, and the fold is what assigns
    // ids. Doing the first half one at a time made this path cost 16.1 s
    // where the whole-tree path costs 11.9 s on the same 1M-line input.
    let code_paths: Vec<_> = paths
        .iter()
        .filter(|path| !docs::is_markdown(path))
        .cloned()
        .collect();
    let mut parsed: std::collections::HashMap<std::path::PathBuf, Option<parse_ast::FileFacts>> =
        parallel::map_ordered(&code_paths, |path| {
            let src = read_source(path)?;
            let facts = parse_ast::Lang::from_path(path)
                .and_then(|lang| parse_ast::parse_file(path, &src, lang));
            Some((path.clone(), facts))
        })
        .into_iter()
        .flatten()
        .collect();

    // One buffer for the whole run rather than one published snapshot per file.
    // `update` copies the buffer on every call, which is right while agents are
    // querying and quadratic here: measured on a 1M-line tree, the per-file
    // cost rose from 334 µs at file 400 to 1,231 µs at file 1,600. Nothing can
    // read this graph yet — it is not published until `analyse` returns — so
    // one copy covers the run, exactly as the whole-tree path does it.
    //
    // Markdown is folded afterwards, because a mention only becomes an edge
    // once the symbol it names exists.
    let mut markdown: Vec<(std::path::PathBuf, String)> = Vec::new();
    g.update_batch(|snap, d| {
        for path in &paths {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            let key = arena.intern(&rel);
            if let Some(nodes) = owned.get(&rel) {
                let mut nodes = nodes.clone();
                nodes.sort_unstable();
                d.set_file_nodes(key, nodes.clone());
            }

            // A file that is present but unreadable — no permission, mid-rename
            // by an editor, briefly locked — must keep what it had. Only a file
            // that is genuinely gone loses its edges. Treating the two alike
            // discarded a file's whole call graph because a save was caught
            // halfway.
            if path.exists() && read_source(path).is_none() {
                continue;
            }

            // A vanished file loses its edges *and* its names. The path is part
            // of the key, so nothing can legitimately reference
            // `src/old/x.rs#f` once that file is gone — keeping the names left
            // a corpse that the search index serves beside the real symbol.
            // See `forget_file`.
            let Ok(src) = std::fs::read_to_string(path) else {
                reg.forget_file(&rel);
                let owned: Vec<csr::NodeId> = d.file_nodes(key).to_vec();
                let targets: std::collections::HashMap<csr::NodeId, Vec<csr::NodeId>> =
                    owned.iter().map(|&n| (n, snap.base_targets(n))).collect();
                d.replace_file_edges(
                    key,
                    |n| targets.get(&n).cloned().unwrap_or_default(),
                    Vec::new(),
                    &owned,
                );
                continue;
            };

            if docs::is_markdown(path) {
                markdown.push((path.clone(), src));
                continue;
            }
            if let Some(Some(facts)) = parsed.remove(path) {
                ingest::apply_facts_into(snap, d, &mut arena, reg, path, root, facts, now());
            }
        }
        ingest::ingest_markdown_into(snap, d, &mut arena, reg, root, &markdown, now());
    });

    // Tier 3 runs over the whole registry, not per file: a call is routinely
    // parsed before the file defining it, and its links are re-applied as a set
    // rather than added.
    reg.prune_refs();
    link_placeholders_quiet(g, reg);

    // A deleted file's placeholders outlive it — see `forget_orphans`. Runs
    // after tier 3, so a placeholder it just linked counts as referenced.
    let snap = g.load();
    let mut referenced: std::collections::HashSet<csr::NodeId> = Default::default();
    for n in 0..snap.width() as csr::NodeId {
        let mut any = false;
        for e in snap.neighbors(n) {
            referenced.insert(e.target);
            any = true;
        }
        if any {
            referenced.insert(n);
        }
    }
    reg.forget_orphans(&referenced);
    Some(())
}

/// Layout cost against graph size, which decides whether it can run at startup.
fn bench_layout(root: &str) -> std::io::Result<()> {
    let root = &canonical(std::path::Path::new(root))?;
    let a = analyse(root)?;
    let t = std::time::Instant::now();
    let l = layout::compute(&a.snap, &a.communities.of_node, layout::Mode::Grouped);
    let elapsed = t.elapsed();
    let edges: usize = (0..a.snap.width() as csr::NodeId)
        .map(|n| a.snap.neighbors(n).count())
        .sum();
    println!(
        "{} nodes, {edges} edges -> layout in {elapsed:?}",
        a.snap.width()
    );

    let (x0, y0, x1, y1) = l.bounds;
    let (w, h) = ((x1 - x0) * 0.25, (y1 - y0) * 0.25);
    let t = std::time::Instant::now();
    let mut total = 0;
    for i in 0..100 {
        let ox = x0 + (x1 - x0) * (i as f32 / 100.0) * 0.7;
        total += l.in_view(ox, y0, ox + w, y0 + h).len();
    }
    println!("100 viewport queries in {:?} ({total} nodes)", t.elapsed());
    Ok(())
}

/// Serves the graph as a map in the browser.
///
/// A separate command from `serve`: that one speaks JSON-RPC to an agent, this
/// one answers a person's first question about an unfamiliar tree — what is in
/// here, and what talks to what.
fn run_view(args: &cli::Args) -> std::io::Result<()> {
    let root = &canonical(std::path::Path::new(&args.path))?;
    let addr = http::addr_for(args.value("port").unwrap_or("7878"));

    let t = std::time::Instant::now();
    let a = analyse(root)?;
    let load_time = t.elapsed();
    let (snap, reg, communities, defined) = (&a.snap, &a.registry, &a.communities, &a.defined);
    // Rebuilt rather than stored: both cost less to compute than to validate.
    let embeddings = embed::embed(snap, 4);
    let search = search::SearchIndex::build_with_docs(reg, reg.docs());
    let names = mcp::name_table(reg, snap.width());
    let served = mcp::Served {
        snap,
        names: &names,
        defined,
        search: &search,
        registry: reg,
        communities,
        embeddings: &embeddings,
        physics: physics::Physics::default(),
        now: now(),
        files: None,
        mentions: None,
        references: None,
        root: None,
    };
    // Layout is stored beside the graph: it costs ~100 ms here but minutes on a
    // monorepo, and it is two flat float arrays, so it maps back as cheaply as
    // the CSR does.
    // Both layouts are computed and stored: switching between them is a button
    // press, and recomputing on demand would stall the view for seconds on a
    // large graph.
    let mut layouts = std::collections::HashMap::new();
    for (mode, name) in [
        (layout::Mode::Grouped, "grouped"),
        (layout::Mode::Free, "free"),
    ] {
        let path = root.join(format!(".glasir-layout-{name}"));
        let l = match layout::read(&path, snap.width()) {
            Some(l) => l,
            None => {
                let t = std::time::Instant::now();
                let l = layout::compute(snap, &communities.of_node, mode);
                eprintln!(
                    "laid out {} nodes ({name}) in {:?}",
                    snap.width(),
                    t.elapsed()
                );
                if let Err(e) = layout::write(&l, &path) {
                    eprintln!(
                        "glasir: could not store the {name} layout at {} — {e}\n\
                         it will be recomputed on every start",
                        path.display()
                    );
                }
                l
            }
        };
        layouts.insert(name, l);
    }
    let layout = &layouts["grouped"];

    let page = include_str!("../assets/view.html")
        .replace(
            "__TITLE__",
            &root.file_name().unwrap_or_default().to_string_lossy(),
        )
        .replace(
            "__OVERVIEW__",
            &view::overview_json(&served, layout).to_string(),
        )
        .replace(
            "__BOUNDS__",
            &format!(
                "[{},{},{},{}]",
                layout.bounds.0, layout.bounds.1, layout.bounds.2, layout.bounds.3
            ),
        );

    println!("http://{addr}");
    println!(
        "  {} nodes, streamed by viewport ({} in {load_time:?})",
        snap.width(),
        if a.from_snapshot {
            "mapped"
        } else {
            "analysed"
        }
    );
    http::serve_view(&addr, &page, |path, query| {
        // Which layout the page is asking about; an unknown name falls back to
        // the default rather than erroring mid-pan.
        let chosen = layouts
            .get(http::text_param(query, "mode").unwrap_or("grouped"))
            .unwrap_or(&layouts["grouped"]);
        match path {
            "/viewport" => {
                // A malformed query yields an empty view rather than an error:
                // the page is mid-pan, and a 400 would leave it blank.
                let g = |k| http::param(query, k).unwrap_or(0.0);
                Some(
                    view::viewport_json(&served, chosen, g("x0"), g("y0"), g("x1"), g("y1"))
                        .to_string(),
                )
            }
            // Switching layout moves every clump, so the page needs the new
            // centres and bounds before it can draw or query anything.
            "/overview" => Some(
                json!({
                    "clumps": view::overview_json(&served, chosen),
                    "bounds": [chosen.bounds.0, chosen.bounds.1, chosen.bounds.2, chosen.bounds.3],
                })
                .to_string(),
            ),
            _ => None,
        }
    })
}

/// Registers this binary as an MCP server with the assistants in use.
fn run_install(args: &cli::Args) -> std::io::Result<()> {
    let root = std::path::Path::new(&args.path);
    let abs = canonical(root).unwrap_or_else(|_| root.into());
    let exe = std::env::current_exe()?;
    let user_scope = args.has("user");
    let dry = args.has("dry-run");
    let quiet = args.has("quiet");

    let selected: Vec<&Target> = match args.value("platform") {
        Some(name) => {
            let found: Vec<&Target> = TARGETS.iter().filter(|t| t.name == name).collect();
            if found.is_empty() {
                eprintln!(
                    "unknown platform: {name}\nknown: {}",
                    TARGETS
                        .iter()
                        .map(|t| t.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(2);
            }
            found
        }
        // Without --platform, every detected client, or the neutral file when
        // none is found.
        None => {
            let clients: Vec<&Target> = TARGETS
                .iter()
                .filter(|t| t.is_client() && t.detected(root))
                .collect();
            if clients.is_empty() {
                TARGETS.iter().filter(|t| !t.is_client()).collect()
            } else {
                clients
            }
        }
    };

    if selected.is_empty() && !quiet {
        println!("no MCP registration target in {}", abs.display());
        println!("pick one explicitly: glasir install --platform mcp");
        return Ok(());
    }

    let mut wrote = 0;
    let mut next = Vec::new();
    for target in selected {
        let Some(path) = target.path(root, user_scope) else {
            if !quiet {
                println!(
                    "{}: no {} scope, skipping",
                    target.name,
                    if user_scope { "user" } else { "project" }
                );
            }
            continue;
        };
        // A file created here holds absolute paths to this machine, so it must
        // not be committed; one that existed is the team's and stays tracked.
        let created = !path.exists();
        match write_registration(target, &path, &exe, &abs, dry) {
            Ok(true) => {
                if created
                    && !user_scope
                    && !dry
                    && let Some(rel) = target.project
                {
                    ignore_generated_files(&abs, &[rel]);
                }
                next.push(target.next);
                wrote += 1;
                if quiet {
                    continue;
                }
                println!(
                    "{} {}: {}",
                    if dry { "would write" } else { "wrote" },
                    target.name,
                    path.display()
                );
            }
            Ok(false) if quiet => {}
            Ok(false) => println!("{}: already registered", target.name),
            Err(e) => eprintln!("{}: {e}", target.name),
        }
    }

    // The stored graph is rewritten on every analysis, so an untracked one
    // leaves the tree dirty and git refuses the next branch switch.
    if !user_scope && !dry {
        ignore_generated_files(&abs, GENERATED);
    }

    // The hooks are per-tree, so they are skipped for a user-scope install:
    // there is no one tree to refresh.
    if !user_scope && !args.has("no-hook") {
        let notes = install_hooks(&abs, &exe, dry);
        if !notes.is_empty() && !quiet {
            println!("\ngit hooks (refresh the graph after pull, checkout, rebase):");
            for note in notes {
                println!("  {note}");
            }
        }
    }

    if wrote > 0 && !dry && !quiet {
        println!("\nnext:");
        for step in next {
            println!("  {step}");
        }
    }
    Ok(())
}

/// Writes one registration. Returns false if it was already there.
///
/// Existing files are merged, never replaced: they hold other servers and other
/// settings, and clobbering someone's configuration to save a parse is not a
/// trade worth making. A file that cannot be parsed is left alone entirely.
fn write_registration(
    target: &Target,
    path: &std::path::Path,
    exe: &std::path::Path,
    root: &std::path::Path,
    dry: bool,
) -> std::io::Result<bool> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let (exe, root) = (exe.to_string_lossy(), root.to_string_lossy());

    let updated = match target.format.container() {
        None => {
            let (rest, _) = strip_codex_block(&existing);
            let block = codex_block(&exe, &root);
            if existing.contains(&block) {
                return Ok(false);
            }
            let rest = rest.trim_end();
            if rest.is_empty() {
                block
            } else {
                format!("{rest}\n\n{block}")
            }
        }
        Some(key) => {
            let mut doc: serde_json::Value = if existing.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&existing).map_err(|e| {
                    std::io::Error::other(format!(
                        "{} is not valid JSON ({e}); left alone",
                        path.display()
                    ))
                })?
            };
            if !doc.is_object() {
                return Err(std::io::Error::other("config root is not an object"));
            }
            let entry = target.format.entry(&exe, &root);
            let already = doc[key]["glasir"] == entry;
            doc[key]["glasir"] = entry;

            if already {
                return Ok(false);
            }
            format!("{doc:#}\n")
        }
    };

    if !dry {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, updated)?;
    }
    Ok(true)
}

/// Removes registrations this tool wrote, leaving everything else in place.
fn run_uninstall(args: &cli::Args) -> std::io::Result<()> {
    let root = std::path::Path::new(&args.path);
    let user_scope = args.has("user");
    let dry = args.has("dry-run");
    let quiet = args.has("quiet");
    let mut removed = 0;

    for target in TARGETS {
        let Some(path) = target.path(root, user_scope) else {
            continue;
        };
        let Ok(existing) = std::fs::read_to_string(&path) else {
            continue;
        };

        let updated = match target.format.container() {
            None => {
                let (rest, found) = strip_codex_block(&existing);
                if !found {
                    continue;
                }
                rest
            }
            Some(key) => {
                let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&existing) else {
                    continue;
                };
                if doc[key]["glasir"].is_null() {
                    continue;
                }
                if let Some(servers) = doc[key].as_object_mut() {
                    servers.remove("glasir");
                }
                format!("{doc:#}\n")
            }
        };

        if !dry {
            std::fs::write(&path, updated)?;
        }
        if !quiet {
            println!(
                "{} {}: {}",
                if dry { "would clean" } else { "cleaned" },
                target.name,
                path.display()
            );
        }
        removed += 1;
    }

    if !user_scope {
        let abs = canonical(root).unwrap_or_else(|_| root.into());
        let notes = uninstall_hooks(&abs, dry);
        for note in &notes {
            if !quiet {
                println!("git hook {note}");
            }
        }
        removed += notes.len();
    }

    // What an analysis writes. It named a `.glasir` directory that nothing
    // creates, so `--purge` deleted nothing. Tokens and the audit log are
    // access records, not cache, and stay.
    if args.has("purge") {
        let generated = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == ".glasir-graph" || n.starts_with(".glasir-layout-"))
            });
        for path in generated {
            if !dry {
                std::fs::remove_file(&path)?;
            }
            removed += 1;
            if !quiet {
                println!(
                    "{} {}",
                    if dry { "would delete" } else { "deleted" },
                    path.display()
                );
            }
        }
    }
    if removed == 0 && !quiet {
        println!("nothing to remove");
    }
    Ok(())
}

/// True if this config actually registers us.
///
/// Structural, not a substring search: a config file routinely contains the
/// string "glasir" for unrelated reasons — a project path, a directory name —
/// and reporting that as an installation is worse than reporting nothing.
fn is_registered(target: &Target, path: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    match target.format.container() {
        None => strip_codex_block(&text).1,
        Some(key) => serde_json::from_str::<serde_json::Value>(&text)
            .is_ok_and(|d| d[key]["glasir"].is_object()),
    }
}

/// Reports what is registered and what the graph over this tree looks like.
fn run_status(args: &cli::Args) -> std::io::Result<()> {
    let root = std::path::Path::new(&args.path);
    let abs = canonical(root).unwrap_or_else(|_| root.into());
    println!("{}\n", abs.display());

    println!("assistants:");
    for target in TARGETS {
        let mut where_ = Vec::new();
        for (scope, user) in [("project", false), ("user", true)] {
            if let Some(path) = target.path(root, user)
                && is_registered(target, &path)
            {
                where_.push(scope);
            }
        }
        let state = if !where_.is_empty() {
            format!("registered ({})", where_.join(", "))
        } else if !target.is_client() {
            "available (--platform mcp)".into()
        } else if target.detected(root) {
            "detected, not registered".into()
        } else {
            "not found".into()
        };
        println!("  {:8} {state}", target.name);
    }

    // Who may reach a served tree is part of what the tree holds, and an
    // expired token that nobody noticed is exactly the state this reports for.
    let entries = auth::read(&auth::token_path(&abs));
    if !entries.is_empty() {
        let now = auth::now();
        let live = entries
            .iter()
            .filter(|e| e.expires == 0 || e.expires > now)
            .count();
        println!("\naccess: {} token(s), {live} valid", entries.len());
        for e in &entries {
            let state = match e.expires {
                0 => "no expiry".to_string(),
                t if t <= now => "EXPIRED".to_string(),
                // Rounded up: a token with twenty hours left reads as "0 days"
                // under integer division, which is what an expired one should
                // say and this one is not.
                t => format!("{} days left", (t - now).div_ceil(86_400)),
            };
            println!("  {:<16} {state}", e.name);
        }
    }

    if let Some((records, last)) = audit::summary(&audit::audit_path(&abs)) {
        let ago = auth::now().saturating_sub(last);
        println!(
            "\naudit: {records} records, last {}",
            if last == 0 {
                "never".to_string()
            } else if ago < 3600 {
                format!("{} min ago", ago / 60)
            } else {
                format!("{} h ago", ago / 3600)
            }
        );
    }

    let files = walk(root);
    println!("\nsources: {} files", files.len());
    let scip = root.join("index.scip");
    if scip.exists() {
        // "Present" is not the useful fact — how much of the tree it still
        // describes is. An index that covers nothing current is worse than
        // none, because its edges still claim to be compiler-verified.
        let mut reg = ingest::SymbolRegistry::new(0);
        match import_scip::import(&scip, root, &mut reg) {
            Ok(r) => {
                let total = r.file_nodes.len();
                let stale = r.stale_files.len();
                println!(
                    "index.scip: {} of {total} indexed files still current",
                    total - stale
                );
                if stale > 0 {
                    println!(
                        "  {stale} edited since it was built; `glasir watch` rebuilds automatically"
                    );
                }
            }
            Err(e) => println!("index.scip: unreadable ({e}) — tier 2 covers the tree"),
        }
    } else {
        println!("index.scip: absent — edges will be inferred, not compiler-resolved");
        if import_scip::indexer_for(root).is_some() {
            println!("  `glasir watch` builds one on the first drift");
        }
    }
    Ok(())
}

/// Indexes a tree and serves it over MCP on stdio.
///
/// Progress goes to stderr: stdout is the JSON-RPC channel and a stray line
/// there corrupts the protocol stream.
/// Measures whether the filtering actually helps: recall against a ground
/// truth first, token cost second.
///
/// Both numbers are needed and neither means much alone. A tool that returns
/// one node has a wonderful token count; a tool that returns the whole
/// repository has perfect recall. Only together do they say anything.
/// Scores one question set against a tree that is not this repository.
///
/// A separate analysis on purpose: `bench/deep` shares no vocabulary budget
/// with the sets above, and indexing it as part of this tree would move every
/// term's document frequency — the four floors would then measure the fixture
/// as much as the code.
fn run_deep(tree: &std::path::Path, set: &std::path::Path) -> std::io::Result<f32> {
    run_tree(tree, set, false)
}

/// One question set against one tree analysed on its own; `structural` scores
/// it by calling the tools, which also read the tree on disk.
fn run_tree(
    tree: &std::path::Path,
    set: &std::path::Path,
    structural: bool,
) -> std::io::Result<f32> {
    let tree = canonical(tree)?;
    let questions = bench::load_questions(set)?;
    // A benchmark measures this build, never a cache of an older one. The
    // snapshot expires on source mtimes, and the fixture's sources never
    // change — so a stored graph here survives every change to the code that
    // reads them and silently reinstates the behaviour it was written with.
    // Measured: a snapshot left by an earlier build scored 75% against this
    // set's own 78% floor, with the working tree at that floor, which reads as
    // a code regression and bisects to nothing.
    let _ = std::fs::remove_file(tree.join(".glasir-graph"));
    let a = analyse(&tree)?;
    let embeddings = embed::embed(&a.snap, 4);
    let search = search::SearchIndex::build_with_docs(&a.registry, a.registry.docs());
    let names = mcp::name_table(&a.registry, a.snap.width());
    let mentions = std::sync::OnceLock::new();
    let refs_cache = std::sync::OnceLock::new();
    let served = mcp::Served {
        snap: &a.snap,
        names: &names,
        defined: &a.defined,
        search: &search,
        registry: &a.registry,
        communities: &a.communities,
        embeddings: &embeddings,
        physics: physics::Physics::default(),
        now: now(),
        files: None,
        mentions: Some(&mentions),
        references: Some(&refs_cache),
        root: structural.then_some(tree.as_path()),
    };
    let mut files: Vec<(String, String)> = Vec::new();
    for path in walk(&tree) {
        if let Ok(body) = std::fs::read_to_string(&path)
            && let Ok(rel) = path.strip_prefix(&tree)
        {
            files.push((rel.to_string_lossy().replace('\\', "/"), body));
        }
    }
    files.sort();
    Ok(if structural {
        bench::run_structural(&served, &questions, &files)
    } else {
        bench::run(&served, &questions, &files)
    })
}

/// Scores `bench/foreign/<name>.txt` against a clone of each repository in
/// `bench/foreign/repos.txt`, pinned by commit, and gates it on
/// `bench/foreign/baseline.txt`. Every other floor reads this Rust tree; these
/// read Go, Java, Python and Kotlin written by other people.
fn run_foreign(
    root: &std::path::Path,
    clones: &std::path::Path,
    check: bool,
) -> std::io::Result<()> {
    let list = std::fs::read_to_string(root.join("bench/foreign/repos.txt"))?;
    let mut measured = Vec::new();
    for line in list
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let name = line.split_whitespace().next().unwrap_or_default();
        let tree = clones.join(name);
        if !tree.is_dir() {
            // A floor whose tree is missing measured nothing, which `--check`
            // must not read as a pass.
            println!("\n=== {name}: no clone at {}", tree.display());
            continue;
        }
        println!("\n=== {name}");
        let set = root.join(format!("bench/foreign/{name}.txt"));
        measured.push((name.to_string(), run_tree(&tree, &set, true)?));
    }
    if check {
        enforce_floors(&measured, &root.join("bench/foreign/baseline.txt"), true)?;
    }
    Ok(())
}

fn run_benchmark(args: &cli::Args) -> std::io::Result<()> {
    let root = &canonical(std::path::Path::new(&args.path))?;
    if let Some(clones) = args.value("foreign") {
        return run_foreign(root, std::path::Path::new(clones), args.has("check"));
    }
    let questions_path = args
        .value("questions")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("bench/questions.txt"));
    // Two ground truths, measured separately on purpose: one asks which code
    // does a thing, the other what was intended and why. A documentation node
    // is a miss in the first and the answer in the second, so a single score
    // would hide every trade between them.
    let also = [
        root.join("bench/questions-identifier.txt"),
        root.join("bench/questions-docs.txt"),
        root.join("bench/questions-docs-de.txt"),
    ];
    let questions = bench::load_questions(&questions_path)?;
    if questions.is_empty() {
        eprintln!("no questions in {}", questions_path.display());
        return Ok(());
    }

    // Same rule as `run_deep`: a floor guards this build, so it is measured
    // against a fresh reading of the tree. A stale snapshot once hid a 93% -> 90% drop
    // for five commits — a step the tool can take itself rather than ask for.
    let _ = std::fs::remove_file(root.join(".glasir-graph"));
    let a = analyse(root)?;
    let embeddings = embed::embed(&a.snap, 4);
    let search = search::SearchIndex::build_with_docs(&a.registry, a.registry.docs());
    let names = mcp::name_table(&a.registry, a.snap.width());
    let refs_cache = std::sync::OnceLock::new();
    let served = mcp::Served {
        snap: &a.snap,
        names: &names,
        defined: &a.defined,
        search: &search,
        registry: &a.registry,
        communities: &a.communities,
        embeddings: &embeddings,
        physics: physics::Physics::default(),
        now: now(),
        files: None,
        mentions: None,
        references: Some(&refs_cache),
        root: None,
    };

    // The baseline reads source files, so it needs them as an agent would see
    // them: repo-relative path plus full text.
    let mut files: Vec<(String, String)> = Vec::new();
    for path in walk(root) {
        if let Ok(body) = std::fs::read_to_string(&path)
            && let Ok(rel) = path.strip_prefix(root)
        {
            files.push((rel.to_string_lossy().replace('\\', "/"), body));
        }
    }
    files.sort();

    println!(
        "{} questions, {} expected symbols, {} source files\n",
        questions.len(),
        questions.iter().map(|q| q.expected.len()).sum::<usize>(),
        files.len()
    );
    // Name each set by its file stem, which is how the baseline keys them.
    let stem = |p: &std::path::Path| {
        p.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    // Kept for the partition scale below, which scores the same answer
    // symbols. Re-reading them from a fixed `bench/<name>.txt` under the
    // measured root was wrong twice over: it does not survive `--questions`,
    // and a tree that is not this repository has no `bench/` at all — the
    // deep-tree set reported "0 of 0" until this carried the questions instead
    // of looking for them again.
    let mut asked: Vec<bench::Question> = questions.clone();
    // The structural sets are scored by calling the tools; everything else by
    // expanding seeds. Keyed on the file name so `--questions` reaches the
    // right scorer too — without this a structural set passed explicitly was
    // silently measured through `query_graph` and read as a 37-point drop.
    let structural = stem(&questions_path).contains("structural");
    let mut measured = vec![(
        stem(&questions_path),
        if structural {
            bench::run_structural(&served, &questions, &files)
        } else {
            bench::run(&served, &questions, &files)
        },
    )];

    if args.value("questions").is_none() {
        for path in also.iter().filter(|p| p.exists()) {
            let set = bench::load_questions(path)?;
            if set.is_empty() {
                continue;
            }
            println!(
                "\n=== {} ({} questions)",
                path.file_name().unwrap_or_default().to_string_lossy(),
                set.len()
            );
            asked.extend(set.iter().cloned());
            measured.push((stem(path), bench::run(&served, &set, &files)));
        }
    }

    // The structural tools, scored by calling them rather than by expanding
    // seeds. This is the half the literature says a graph should win — who
    // calls this, how does A reach B — and until now nothing measured it.
    if args.value("questions").is_none() {
        let set = root.join("bench/questions-structural.txt");
        if set.exists() {
            let questions = bench::load_questions(&set)?;
            if !questions.is_empty() {
                println!(
                    "\n=== questions-structural.txt ({} questions, via `impact` and `shortest_path`)",
                    questions.len()
                );
                // Deliberately *not* added to `asked`: the partition scale
                // scores expected symbols by the subsystem they land in, and
                // four of these questions expect a phrase ("no path", "unknown
                // symbol") rather than a symbol. Folding them in counted those
                // as "in no subsystem" and moved the partition floor by four
                // points without anything about the partition changing.
                measured.push((
                    "structural".to_string(),
                    bench::run_structural(&served, &questions, &files),
                ));
            }
        }
    }

    let partition_score;
    // The partition gets its own scale, because none of the four recall sets
    // can see it: `query_graph` does not read communities, so every partition
    // parameter measures identically on all four. That blindness hid a lump —
    // 497 of 591 covered symbols in one community on a real tree — for five
    // sessions. See `bench::overview_scale`.
    {
        let (placed, wrong, absent) = bench::overview_scale(&served, &asked);
        let total = placed + wrong + absent;
        partition_score = 100.0 * placed as f32 / total.max(1) as f32;
        println!(
            "\n=== partition (what `overview` answers)\n  {placed} of {total} answer symbols reachable through a subsystem named after their own file, {wrong} mislabelled, {absent} in no subsystem"
        );
    }
    measured.push(("partition".to_string(), partition_score));

    // A second tree, measured in its own right rather than folded in with the
    // others.
    //
    // Every set above scores this repository: flat Rust, `src/` one level
    // deep, a file name usually a subsystem name. Three findings the audit
    // could not decide were blocked on having only that shape — the path in
    // tokenization, `B`, and the hub exclusion each win on one shape and lose
    // on the other. `bench/deep` is the second shape, and it has to be
    // *analysed separately*: folding it into this tree would change the
    // document frequency of every word here, which is the thing the four sets
    // above measure.
    if args.value("questions").is_none() {
        let deep_tree = root.join("bench/deep");
        let deep_set = root.join("bench/questions-deep.txt");
        if deep_tree.is_dir() && deep_set.exists() {
            println!("\n=== questions-deep.txt (a deep tree, analysed on its own)");
            measured.push(("deep".to_string(), run_deep(&deep_tree, &deep_set)?));
        }
    }

    // `--check` is what makes the benchmark a guard rather than a report: CI
    // runs it, and a set that falls below its floor breaks the build.
    if args.has("check") {
        enforce_floors(&measured, &root.join("bench/baseline.txt"), false)?;
    }
    Ok(())
}

/// Fails the process when a measured set is below its floor. With `strict`, a
/// floor nothing measured fails too.
fn enforce_floors(
    measured: &[(String, f32)],
    path: &std::path::Path,
    strict: bool,
) -> std::io::Result<()> {
    let baseline = bench::load_baseline(path)?;
    let mut failed = Vec::new();
    println!("\n=== against {}", path.display());
    if strict {
        for (name, _) in &baseline.floors {
            if !measured.iter().any(|(m, _)| m == name) {
                failed.push(format!("{name}: not measured"));
            }
        }
    }
    for (name, got) in measured {
        let Some((_, floor)) = baseline.floors.iter().find(|(k, _)| k == name) else {
            println!("  {name:<24} {got:>3.0}%   (no floor recorded)");
            continue;
        };
        // `<=`, not `<`: with `<` a drop of exactly the tolerance passes,
        // and that is not a corner case — it is what happened. The
        // identifier set fell 93% -> 90% and the guard reported "ok" for
        // five commits, because 90 + 3 is not less than 93.
        let low = got + baseline.tolerance <= *floor;
        println!(
            "  {name:<24} {got:>3.0}%   floor {floor:.0}%  {}",
            if low { "REGRESSION" } else { "ok" }
        );
        if low {
            failed.push(format!("{name}: {got:.0}% against a floor of {floor:.0}%"));
        }
    }
    if !failed.is_empty() {
        eprintln!("\nretrieval regressed:");
        for f in &failed {
            eprintln!("  {f}");
        }
        eprintln!(
            "\nIf the drop is intended, lower the floor in {} in the \n\
             same commit, with the reason in the message.",
            path.display()
        );
        std::process::exit(1);
    }
    Ok(())
}

/// Explains one question: what it seeded on, what came back, and where the
/// expected symbols ranked if they were reached at all.
fn run_why(args: &cli::Args) -> std::io::Result<()> {
    let root = canonical(std::path::Path::new("."))?;
    let question = args.rest.join(" ");
    let question = if question.is_empty() {
        args.path.clone()
    } else {
        format!("{} {question}", args.path)
    };

    let a = analyse(&root)?;
    let embeddings = embed::embed(&a.snap, 4);
    let search = search::SearchIndex::build_with_docs(&a.registry, a.registry.docs());
    let names = mcp::name_table(&a.registry, a.snap.width());
    let served = mcp::Served {
        snap: &a.snap,
        names: &names,
        defined: &a.defined,
        search: &search,
        registry: &a.registry,
        communities: &a.communities,
        embeddings: &embeddings,
        physics: physics::Physics::default(),
        now: now(),
        files: None,
        mentions: None,
        references: None,
        root: None,
    };

    println!("question: {question}\n");
    // The terms are half the answer on their own: a question carrying eight
    // filler words and two content words ranks on the filler, because a German
    // "wie" is rare in a mostly-English corpus and rare means high IDF.
    println!("terms: {:?}", search::tokenize(&question));
    let split: Vec<(String, Vec<String>)> = search::tokenize(&question)
        .into_iter()
        .map(|t| {
            let parts = search.split_for_test(&t);
            (t, parts)
        })
        .filter(|(_, p)| !p.is_empty())
        .collect();
    if !split.is_empty() {
        println!("compound splits: {split:?}");
    }
    println!();
    println!("raw BM25 (before normalisation):");
    for (node, score) in search.search(&question, 12) {
        println!("  {score:.3}  {}", served.name(node));
    }
    println!("\nseeds:");
    for (node, score) in mcp::seeds_for_test(&served, &question).iter().take(24) {
        println!("  {score:.3}  {}", served.name(*node));
    }

    let kept = physics::expand(
        &a.snap,
        &served.seeds(&question),
        served.now,
        &physics::Physics {
            max_nodes: mcp::DEFAULT_MAX_NODES,
            max_hops: mcp::DEFAULT_MAX_HOPS,
            ..served.physics
        },
        Some(&a.defined),
    );
    println!("\nreturned ({}):", kept.len());
    for (i, s) in kept.iter().enumerate() {
        println!("  {i:>2}. {:.3}  {}", s.score, served.name(s.node));
    }
    Ok(())
}

fn run_serve(args: &cli::Args) -> std::io::Result<()> {
    use std::sync::Arc;

    let root = &canonical(std::path::Path::new(&args.path))?;
    let state = Arc::new(published::Published::from_pointee(served_state(root)?));
    {
        let s = state.load();
        eprintln!(
            "glasir: {} nodes, {} symbols",
            s.snap.width(),
            s.registry.len()
        );
    }

    // Bumped after every republish, so an open SSE stream can tell its client
    // that the graph it cached has moved on. Created only under `--watch`,
    // because that is the only thing that republishes — see `HttpConfig`.
    let reindexed = args
        .has("watch")
        .then(|| Arc::new(std::sync::atomic::AtomicU64::new(0)));
    let reindex_error = Arc::new(std::sync::Mutex::new(None));
    let max_index_age = match args.value("max-index-age") {
        Some(value) => match value.parse::<u64>() {
            Ok(seconds) => Some(std::time::Duration::from_secs(seconds)),
            Err(_) => {
                eprintln!("--max-index-age must be a non-negative number of seconds.");
                std::process::exit(2);
            }
        },
        None => None,
    };

    // `--watch` keeps the served graph current: without it a long-running
    // server answers from the tree as it was at startup, and nobody can tell
    // from the outside that the answer is stale. The re-analysis is the
    // incremental path, so an edit costs what a warm start costs.
    if args.has("watch") {
        let (root, state, reindex_error) = (root.clone(), state.clone(), reindex_error.clone());
        let reindexed = reindexed.clone();
        std::thread::spawn(move || {
            let _ = watcher::watch(&root, |_paths| match served_state(&root) {
                // Published whole: a reader holds the old state until its
                // request ends and the next one sees the new one. No reader
                // ever observes a partly re-indexed graph.
                Ok(fresh) => {
                    state.store(Arc::new(fresh));
                    *reindex_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    // After the store, never before: a client woken by this
                    // must find the new graph already published, or it
                    // re-fetches the stale one and caches it a second time.
                    if let Some(c) = &reindexed {
                        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    eprintln!(
                        "glasir: re-indexed, {} symbols",
                        state.load().registry.len()
                    );
                }
                // Serving the previous graph beats serving none: a syntax
                // error mid-edit must not take the server down.
                Err(e) => {
                    *reindex_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
                    eprintln!("glasir: re-index failed, keeping previous graph ({e})");
                }
            });
        });
    }

    let tokens = Arc::new(auth::Tokens::new(auth::token_path(root)));
    let behind_control_plane = args.has("behind-control-plane");
    if behind_control_plane && args.value("http").is_none() {
        eprintln!("--behind-control-plane requires --http <port|addr>.");
        std::process::exit(2);
    }
    if behind_control_plane && args.value("token").is_some() {
        eprintln!(
            "--behind-control-plane uses a revocable token from .glasir-tokens; \\
             do not pass --token. Issue one with `glasir token add control`."
        );
        std::process::exit(2);
    }
    if behind_control_plane && !tokens.configured() {
        eprintln!(
            "--behind-control-plane requires a per-service token. \\
             Issue one with `glasir token add control`."
        );
        std::process::exit(2);
    }
    let control_plane_cidr = match args.value("control-plane-cidr") {
        Some(value) if behind_control_plane => match http::ControlPlaneCidr::parse(value) {
            Ok(cidr) => Some(cidr),
            Err(error) => {
                eprintln!("--control-plane-cidr {value:?}: {error}");
                std::process::exit(2);
            }
        },
        Some(_) => {
            eprintln!("--control-plane-cidr requires --behind-control-plane.");
            std::process::exit(2);
        }
        None => None,
    };
    // Both or neither: a certificate with no key cannot serve, and a key with
    // no certificate is a typo that would otherwise start in plaintext while
    // the operator believed otherwise.
    let tls = match (
        args.value("tls-cert"),
        args.value("tls-key"),
        args.value("control-plane-client-ca"),
    ) {
        (Some(cert), Some(key), Some(ca)) if behind_control_plane => {
            Some(Arc::new(http::mtls_config(
                std::path::Path::new(cert),
                std::path::Path::new(key),
                std::path::Path::new(ca),
            )?))
        }
        (Some(_), Some(_), Some(_)) => {
            eprintln!("--control-plane-client-ca requires --behind-control-plane.");
            std::process::exit(2);
        }
        (Some(cert), Some(key), None) => Some(Arc::new(http::tls_config(
            std::path::Path::new(cert),
            std::path::Path::new(key),
        )?)),
        (None, None, Some(_)) => {
            eprintln!("--control-plane-client-ca requires --tls-cert and --tls-key.");
            std::process::exit(2);
        }
        (None, None, None) => None,
        _ => {
            eprintln!("--tls-cert and --tls-key go together; pass both or neither.");
            std::process::exit(2);
        }
    };
    match args.value("http") {
        // Loopback by default: a graph of someone's source on 0.0.0.0 is a
        // disclosure, and the spec says to bind locally unless told otherwise.
        Some(addr) => http::serve(
            &state,
            &http::HttpConfig {
                metrics: Default::default(),
                addr: http::addr_for(addr),
                token: args.value("token").map(str::to_string),
                tokens: tokens.clone(),
                // Only where identities exist: without tokens every line would
                // read "anonymous", which answers no audit question and still
                // records what a developer asked on their own machine.
                audit: tokens.configured().then(|| {
                    Arc::new(audit::Audit::start(
                        audit::audit_path(root),
                        tree_name(root),
                    ))
                }),
                public_url: args.value("public-url").map(str::to_string),
                reindexed: reindexed.clone(),
                tls,
                behind_control_plane,
                control_plane_cidr,
                reindex_error: reindex_error.clone(),
                max_index_age,
            },
        ),
        None => {
            eprintln!("glasir: serving on stdio");
            // Loaded per request, exactly as the HTTP path does. Borrowing once
            // for the life of the server was wrong in a way that looked
            // reasonable — stdio *is* one client — but it made `--watch` a
            // no-op there: the watcher re-indexed and said so on stderr while
            // the client kept getting the tree as it looked at startup. That is
            // the default configuration `install` writes, so the silent staleness
            // this whole feature exists to prevent was shipped by it.
            mcp::serve_swappable(&state)
        }
    }
}

/// `glasir impact-of <rev>` — what a change reaches, from a git diff.
///
/// Three pieces that already existed and had never been joined: the hooks fire
/// after every git operation, a symbol carries its file as a name prefix, and
/// `impact` already groups dependents by distance. What was missing is the
/// step from "these files changed" to "these symbols, and what depends on
/// them".
///
/// A command rather than an MCP tool, and deliberately: it shells out to git
/// and needs the working tree, neither of which a served graph has — `mcp.rs`
/// has no `root` at all. This is what a hook or a CI job runs.
fn run_impact_of(args: &cli::Args) -> std::io::Result<()> {
    print!(
        "{}",
        impact_of_report(args, &canonical(std::path::Path::new("."))?)?
    );
    Ok(())
}

/// The report itself, as text.
///
/// Separate from printing so a check can assert what it says. Capturing stdout
/// would be the alternative and is worse: it needs a global redirect that
/// every other test in the process shares.
fn impact_of_report(args: &cli::Args, root: &std::path::Path) -> std::io::Result<String> {
    let mut report = String::new();
    // Default to the working tree against HEAD, which is what someone typing
    // this mid-change means. A revision compares that revision to HEAD.
    let rev = args.path.as_str();
    let (range, what): (Vec<&str>, String) = match rev {
        "." | "" => (
            vec!["diff", "--name-only", "HEAD"],
            "uncommitted changes".into(),
        ),
        r => (vec!["diff", "--name-only", r, "HEAD"], format!("{r}..HEAD")),
    };
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(&range)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "git could not diff {what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let changed: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|path| path.replace('\\', "/"))
        .collect();
    if changed.is_empty() {
        report.push_str(&format!("no files changed in {what}"));
        report.push('\n');
        return Ok(report);
    }

    let a = analyse(root)?;
    let names = mcp::name_table(&a.registry, a.snap.width());
    let reverse = a.snap.reverse();

    // Symbols defined in a changed file. The file is the name's prefix, so no
    // file-to-node map has to be stored — the same property incremental
    // indexing relies on.
    let mut touched: Vec<(String, csr::NodeId)> = Vec::new();
    for (symbol, &node) in a.registry.entries() {
        if let Some((file, _)) = symbol.split_once('#')
            && changed.iter().any(|c| c == file)
            && a.defined.contains(&node)
        {
            touched.push((symbol.clone(), node));
        }
    }
    touched.sort();

    report.push_str(&format!(
        "{} file(s) changed in {what}, {} symbol(s) in them\n",
        changed.len(),
        touched.len()
    ));
    report.push('\n');
    if touched.is_empty() {
        report.push_str(
            "none of the changed files define a symbol this graph knows.\n\
             A new file is not in the graph until the next analysis.",
        );
        report.push('\n');
        return Ok(report);
    }

    // One backwards sweep from every changed symbol at once, not one per
    // symbol: they overlap heavily in a real diff, and a caller reached from
    // two of them should be reported at its shortest distance, once.
    let depth: usize = args
        .value("depth")
        .and_then(|d| d.parse().ok())
        .unwrap_or(3)
        .clamp(1, 10);
    let seeds: std::collections::HashSet<csr::NodeId> = touched.iter().map(|&(_, n)| n).collect();
    let mut seen = seeds.clone();
    let mut frontier: Vec<csr::NodeId> = {
        let mut f: Vec<csr::NodeId> = seeds.iter().copied().collect();
        f.sort_unstable();
        f
    };
    let mut levels: Vec<Vec<csr::NodeId>> = Vec::new();
    for _ in 0..depth {
        let mut next: Vec<csr::NodeId> = Vec::new();
        for &n in &frontier {
            for (caller, _) in reverse.callers(n) {
                if seen.insert(caller) {
                    next.push(caller);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_unstable();
        frontier.clone_from(&next);
        levels.push(next);
    }

    let name = |n: csr::NodeId| -> String {
        names
            .get(n as usize)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("node:{n}"))
    };

    let total: usize = levels.iter().map(|l| l.len()).sum();
    if total == 0 {
        report.push_str("nothing in the graph depends on what changed.");
        report.push('\n');
        return Ok(report);
    }
    report.push_str(&format!("{total} symbol(s) depend on the change:\n"));
    report.push('\n');
    for (i, level) in levels.iter().enumerate() {
        // Grouped by file rather than listed flat: a reviewer decides what to
        // re-read by file, and a hop-3 list of bare symbols is unreadable.
        let mut by_file: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut unplaced = 0usize;
        for &n in level {
            let full = name(n);
            // Tier 2 mints an unqualified placeholder for every callee, so a
            // cross-file call routes through a node belonging to no file.
            // Those dominated the first hop — 51 of 51 here — and name no
            // place a reviewer could go. Counted, not listed: the path they
            // carry is real, the destination is not a location.
            let Some((file, sym)) = full.split_once('#') else {
                unplaced += 1;
                continue;
            };
            by_file
                .entry(file.to_string())
                .or_default()
                .push(sym.to_string());
        }
        // Distance is the signal: a direct caller almost certainly breaks, a
        // third-hop one probably does not. Same reasoning as `impact`.
        report.push_str(&format!(
            "  {} hop{} — {} symbol(s)",
            i + 1,
            if i == 0 { "" } else { "s" },
            level.len()
        ));
        report.push('\n');
        // Documents that mention a changed symbol are separated, not dropped:
        // "the architecture note describes this" is worth knowing and is a
        // different action from "this code calls it".
        let (docs, code): (Vec<_>, Vec<_>) = by_file
            .into_iter()
            .partition(|(f, _)| crate::docs::is_markdown(std::path::Path::new(f)));
        for (file, mut syms) in code {
            syms.sort();
            let shown: Vec<String> = syms.iter().take(6).cloned().collect();
            let more = syms.len().saturating_sub(shown.len());
            let tail = if more > 0 {
                format!(", +{more} more")
            } else {
                String::new()
            };
            report.push_str(&format!("    {file}: {}{tail}", shown.join(", ")));
            report.push('\n');
        }
        if !docs.is_empty() {
            let files: Vec<&str> = docs.iter().map(|(f, _)| f.as_str()).collect();
            report.push_str(&format!(
                "    documentation describing it: {}",
                files.join(", ")
            ));
            report.push('\n');
        }
        if unplaced > 0 {
            report.push_str(&format!(
                "    (+{unplaced} unresolved callees, which name no file)\n"
            ));
        }
        report.push('\n');
    }
    Ok(report)
}

/// `impact-of` turns a diff into the symbols a reviewer has to re-read.
///
/// Against a real git repository, because the step being tested is the one
/// from "git says these files" to "the graph says these symbols" — a fixture
/// that hands over the file list would test everything except that.
fn demo_impact_of() {
    let dir = std::env::temp_dir().join(format!("glasir-impactof-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .expect("git");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);

    std::fs::write(
        dir.join("src/core.rs"),
        "pub fn core() -> u32 { 1 }\npub fn untouched() -> u32 { 9 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/caller.rs"),
        "use crate::core;\npub fn calls_core() -> u32 { core::core() }\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "base"]);

    // Change only core.rs. caller.rs must show up as depending on it.
    std::fs::write(
        dir.join("src/core.rs"),
        "pub fn core() -> u32 { 2 }\npub fn untouched() -> u32 { 9 }\n",
    )
    .unwrap();

    // Called directly rather than as a subprocess: under `cargo test` the
    // current executable is the test harness, which runs tests instead of the
    // command — the first version asserted against an empty string and would
    // have passed a broken tool just as happily.
    let run = || -> String {
        let args = cli::Args::parse(["glasir", "impact-of", "."].iter().map(|s| s.to_string()));
        impact_of_report(&args, &dir).unwrap()
    };

    let text = run();
    // The point of the tool: a file nobody named is reported because the graph
    // knows it calls something that changed.
    assert!(
        text.contains("caller.rs"),
        "a dependent the diff never mentions must be found:\n{text}"
    );
    assert!(
        text.contains("1 file(s) changed"),
        "the changed file is counted:\n{text}"
    );

    // With nothing changed there is nothing to say.
    std::process::Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["checkout", "--", "."])
        .output()
        .unwrap();
    let clean = run();
    assert!(
        clean.contains("no files changed"),
        "a clean tree reports nothing changed:\n{clean}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase E.1 ok: a diff becomes the symbols that depend on it");
}

/// `glasir guard` — the architecture contract, checked in CI.
///
/// `benchmark --check` guards recall the same way and the shape is deliberately
/// identical: a versioned file of rules, a run that exits non-zero when one is
/// broken, and comments in the file carrying the reason each rule exists.
/// A report tells you what the architecture was on the day it ran; this tells
/// you it still holds, on every commit.
fn run_guard(args: &cli::Args) -> std::io::Result<()> {
    let root = canonical(std::path::Path::new(&args.path))?;
    let path = args
        .value("rules")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("glasir-rules.txt"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "no contract at {}\n\n\
             Write one, a rule per line:\n\n\
             \x20 deny src/a.rs -> src/b.rs   # this must not call that\n\
             \x20 deny src/a -> src/b.rs      # a prefix covers a directory\n\
             \x20 no-cycles [extracted]       # no file may depend on itself\n",
            path.display()
        );
        std::process::exit(2);
    };
    let rules = match guard::parse(&text) {
        Ok(r) => r,
        // A contract that cannot be read is a failure, never an empty contract:
        // passing here would mean a typo silently switches the gate off.
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            std::process::exit(2);
        }
    };
    if rules.is_empty() {
        eprintln!("{}: no rules", path.display());
        std::process::exit(2);
    }

    let a = analyse(&root)?;
    let names = mcp::name_table(&a.registry, a.snap.width());
    let known = guard::known_files(&names, &a.defined);

    // A path covering no file matches nothing, finds no edge and reports
    // success — the quietest way for a contract to stop being one. Checked
    // before any rule runs, and fatal: a renamed file should break the build
    // that day, not go unnoticed until someone reads the contract.
    let mut unknown: Vec<String> = Vec::new();
    for r in &rules {
        if let guard::Rule::Deny { from, to, .. } = r {
            for side in [from, to] {
                if !guard::matches_any(side, &known) && !unknown.contains(side) {
                    unknown.push(side.clone());
                }
            }
        }
    }
    if !unknown.is_empty() {
        eprintln!(
            "{}: no file matches {}\n\nthis tree has {} files, for example: {}",
            path.display(),
            unknown.join(", "),
            known.len(),
            known.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
        );
        std::process::exit(2);
    }

    let violations = guard::check(&a.snap, &a.registry, &names, &a.defined, &rules);
    if violations.is_empty() {
        println!("{} rule(s) hold", rules.len());
        return Ok(());
    }
    println!("{} of {} rule(s) broken\n", violations.len(), rules.len());
    for v in &violations {
        println!("  {}", v.rule);
        for e in &v.evidence {
            println!("      {e}");
        }
        println!();
    }
    println!(
        "If the architecture changed on purpose, change the rule in the same \n\
         commit and say why — a contract nobody may edit gets deleted instead."
    );
    std::process::exit(1);
}

/// `guard` is the architecture contract, so the property is that it *fails*.
///
/// A gate that passes is indistinguishable from a gate that does nothing —
/// which is exactly how the first version behaved, and only a fixture built to
/// break a rule showed it.
fn demo_guard() {
    let dir = std::env::temp_dir().join(format!("glasir-guard-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    // Two subsystems, with a call from the first into the second.
    std::fs::write(
        dir.join("src/gateway.rs"),
        "use crate::driver;\n\
         pub fn gateway_handle() -> u32 { gateway_route() }\n\
         pub fn gateway_route() -> u32 { driver::driver_start() }\n\
         pub fn gateway_reply() -> u32 { gateway_route() }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/driver.rs"),
        "pub fn driver_start() -> u32 { driver_spawn() }\n\
         pub fn driver_spawn() -> u32 { driver_wait() }\n\
         pub fn driver_wait() -> u32 { 1 }\n",
    )
    .unwrap();

    let a = analyse(&dir).unwrap();
    let names = mcp::name_table(&a.registry, a.snap.width());
    let run = |text: &str| -> Vec<guard::Violation> {
        let rules = guard::parse(text).expect("rules parse");
        guard::check(&a.snap, &a.registry, &names, &a.defined, &rules)
    };

    // **The one that was wrong.** A cross-subsystem call routes through the
    // unqualified placeholder tier 2 mints for the callee, so that middle hop
    // belongs to no community and carries no label — a rule matching on labels
    // alone found nothing and the gate reported success on a tree built to
    // break it.
    let broken = run("deny src/gateway.rs -> src/driver.rs\n");
    assert_eq!(
        broken.len(),
        1,
        "the call from gateway into driver is a violation"
    );
    assert!(
        broken[0]
            .evidence
            .iter()
            .any(|e| e.contains("driver.rs#driver_start")),
        "the evidence names the definition, not the placeholder it routes through: {:?}",
        broken[0].evidence
    );

    // The same rule the other way round holds — driver calls nothing here.
    assert!(
        run("deny src/driver.rs -> src/gateway.rs\n").is_empty(),
        "a rule the code keeps must pass, or the gate is noise"
    );

    // No cycle in this shape.
    assert!(
        run("no-cycles inferred\n").is_empty(),
        "gateway -> driver runs one way"
    );

    // Add one, and it is found. Written as a second file rather than a second
    // fixture so the only difference is the edge under test.
    std::fs::write(
        dir.join("src/driver.rs"),
        "use crate::gateway;\n\
         pub fn driver_start() -> u32 { driver_spawn() }\n\
         pub fn driver_spawn() -> u32 { gateway::gateway_reply() }\n\
         pub fn driver_wait() -> u32 { 1 }\n",
    )
    .unwrap();
    let _ = std::fs::remove_file(dir.join(".glasir-graph"));
    let b = analyse(&dir).unwrap();
    let names_b = mcp::name_table(&b.registry, b.snap.width());
    let cyc = guard::check(
        &b.snap,
        &b.registry,
        &names_b,
        &b.defined,
        &guard::parse("no-cycles inferred\n").unwrap(),
    );
    assert_eq!(cyc.len(), 1, "a mutual dependency is a broken contract");

    // A rule that cannot be read is an error, never an empty contract: the
    // quietest way for a gate to stop being one is to silently hold no rules.
    assert!(guard::parse("deny gateway must not call driver here\n").is_err());
    assert!(guard::parse("no-cycles guessed\n").is_err());
    // A comment and a blank line are not errors.
    assert_eq!(
        guard::parse("# why this rule exists\n\ndeny a -> b\n")
            .unwrap()
            .len(),
        1
    );
    // Written without spaces around the arrow, because someone will.
    assert_eq!(
        guard::parse("deny a->b\n").unwrap(),
        vec![guard::Rule::Deny {
            from: "a".into(),
            to: "b".into(),
            floor: csr::Confidence::Inferred,
        }]
    );

    // A path covering no file matches nothing and would pass silently, so the
    // command checks every path against the tree before running a rule.
    let known = guard::known_files(&names, &a.defined);
    assert!(guard::matches_any("src/gateway.rs", &known));
    assert!(
        guard::matches_any("src", &known),
        "a prefix covers a directory"
    );
    assert!(
        !guard::matches_any("src/gatway.rs", &known),
        "so a typo is reported rather than passing"
    );
    // The boundary is why the prefix is not a bare `starts_with`: `src/gate`
    // must not quietly cover `src/gateway.rs`, or a rule means more than it
    // says.
    assert!(!guard::matches_any("src/gate", &known));

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase E.3 ok: the contract catches what breaks it, and says what did");
}

/// `token add|revoke|list` — the operator's side of per-user access.
///
/// The tree is the working directory, not a positional argument: the first one
/// is the verb, because `glasir token add anna` is what an operator expects to
/// type. `--path` is not offered for the same reason — a token file belongs to
/// the tree you are standing in.
fn run_token(args: &cli::Args) -> std::io::Result<()> {
    let root = canonical(std::path::Path::new("."))?;
    let path = auth::token_path(&root);
    let verb = args.path.as_str();
    let name = args.rest.first().map(String::as_str);

    match (verb, name) {
        ("add", Some(name)) => {
            let days: u64 = args.value("days").and_then(|d| d.parse().ok()).unwrap_or(0);
            let expires = if days == 0 {
                0
            } else {
                auth::now() + days * 86_400
            };
            let secret = auth::mint()?;
            auth::append(
                &path,
                &auth::Entry {
                    hash: auth::sha256_hex(secret.as_bytes()),
                    name: name.to_string(),
                    expires,
                },
            )?;
            // Printed once and never stored: only its hash reaches the file, so
            // there is no second chance to read it. Say so plainly rather than
            // letting someone discover it when they need it.
            println!("{secret}");
            eprintln!(
                "token for {name}, {}. It is shown once — the file keeps only its hash.",
                if days == 0 {
                    "no expiry".to_string()
                } else {
                    format!("expires in {days} days")
                }
            );
            eprintln!("  Authorization: Bearer {secret}");
            ignore_generated_files(&root, GENERATED);
            Ok(())
        }
        ("rotate", Some(name)) => {
            let days: u64 = args.value("days").and_then(|d| d.parse().ok()).unwrap_or(0);
            let expires = if days == 0 {
                0
            } else {
                auth::now() + days * 86_400
            };
            let secret = auth::rotate(&path, name, expires)?;
            println!("{secret}");
            eprintln!(
                "rotated token for {name}, {}. Update the control-plane backend credential before using it.",
                if days == 0 {
                    "no expiry".to_string()
                } else {
                    format!("expires in {days} days")
                }
            );
            ignore_generated_files(&root, GENERATED);
            Ok(())
        }
        ("revoke", Some(name)) => {
            let gone = auth::revoke(&path, name)?;
            match gone {
                0 => println!("no token for {name}"),
                n => println!("revoked {n} token(s) for {name} — effective immediately"),
            }
            Ok(())
        }
        ("list", _) => {
            let now = auth::now();
            let entries = auth::read(&path);
            if entries.is_empty() {
                println!("no tokens in {}", path.display());
            }
            for e in entries {
                let state = match e.expires {
                    0 => "no expiry".to_string(),
                    t if t <= now => "EXPIRED".to_string(),
                    // Rounded up: a token with twenty hours left reads as "0 days"
                    // under integer division, which is what an expired one should
                    // say and this one is not.
                    t => format!("{} days left", (t - now).div_ceil(86_400)),
                };
                println!("{:<20} {state}", e.name);
            }
            Ok(())
        }
        _ => {
            eprintln!("usage: glasir token add|rotate <name> [--days n] | revoke <name> | list");
            Ok(())
        }
    }
}

/// Shell fenced by these, so an uninstall removes exactly its own lines.
const HOOK_BEGIN: &str = "# glasir:begin";
const HOOK_END: &str = "# glasir:end";

/// The hooks that see a tree change under a running server.
///
/// `post-merge` alone is not enough, which is the mistake worth naming: a
/// `git pull --rebase` never merges, so it fires `post-rewrite` instead, and a
/// branch switch fires neither. All three, or the stale graph this is meant to
/// prevent simply moves to whichever verb was left out.
const HOOKS: &[&str] = &["post-merge", "post-checkout", "post-rewrite"];

/// The body of one hook.
///
/// Detached, because git must return immediately: a cold analysis is 13.6 s on
/// a million-line tree and a hook that makes people wait is a hook they delete.
/// Failures go to a log rather than to the terminal — a hook that prints during
/// a pull is noise, but one that fails silently forever is the bug this whole
/// item exists to fix.
fn hook_script(hook: &str, exe: &std::path::Path) -> String {
    // A checkout that is not a branch switch touches nothing, and git says
    // which it was in $3. Each guard is tied to a documented Git transition.
    let guard = if hook == "post-checkout" {
        "\n# $3 is 1 for a branch switch, 0 for a file checkout, which changes nothing.\n\
         [ \"$3\" = \"1\" ] || exit 0\n\
         # `git checkout -b` with no start point reports a switch but moves nothing.\n\
         [ \"$1\" = \"$2\" ] && exit 0\n"
    } else {
        ""
    };
    format!(
        "{HOOK_BEGIN}\n\
         # Keeps the code graph current after the tree changes under it.\n\
         # Installed by: glasir install — remove with: glasir uninstall\n\
         [ \"${{GLASIR_SKIP_HOOK:-0}}\" = \"1\" ] && exit 0\n\
         {guard}\n\
         # Mid-operation states are not worth indexing: a rebase or merge in\n\
         # progress will change the tree again before it is done.\n\
         GIT_DIR=${{GIT_DIR:-$(git rev-parse --git-dir 2>/dev/null)}}\n\
         [ -d \"$GIT_DIR/rebase-merge\" ] && exit 0\n\
         [ -d \"$GIT_DIR/rebase-apply\" ] && exit 0\n\
         [ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0\n\
         \n\
         # Nothing to refresh if this tree was never analysed.\n\
         [ -f .glasir-graph ] || exit 0\n\
         \n\
         # Detached: git returns now, the graph catches up. A cold analysis is\n\
         # seconds on a large tree and nobody should wait for it during a pull.\n\
         _log=\"${{TMPDIR:-/tmp}}/glasir-refresh.log\"\n\
         ({exe} analyse . >>\"$_log\" 2>&1 || \\\n\
         \techo \"$(date): glasir analyse failed, graph may be stale\" >>\"$_log\") &\n\
         {HOOK_END}\n",
        // Single-quoted, with any embedded quote escaped. `current_exe` is not
        // under our control: a path with a space silently breaks the hook, and
        // one containing `;` or `$(...)` — a CI checkout, a package manager's
        // directory — would run as shell on every pull.
        exe = shell_quote(&exe.to_string_lossy())
    )
}

/// Wraps a value in single quotes for POSIX sh, escaping any it contains.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Where this repository keeps its hooks.
///
/// Asked of git rather than assumed to be `.git/hooks`: `core.hooksPath`
/// (husky and friends), a linked worktree where `.git` is a file, and
/// `includeIf` all change the answer, and git resolves every one of them.
/// `--path-format=absolute` is deliberately not passed — it arrived in git 2.31
/// and older versions echo it back as a literal argument.
fn hooks_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let rel = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if rel.is_empty() {
        return None;
    }
    let path = std::path::Path::new(&rel);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
}

/// Installs the refresh hooks, merging into whatever is already there.
///
/// An existing hook is another tool's file — husky, a pre-commit framework,
/// someone's own script — so the block is appended and fenced, never written
/// over. A file that does not start with a shebang is left alone entirely and
/// reported: appending shell to something that is not shell would break it.
fn install_hooks(root: &std::path::Path, exe: &std::path::Path, dry: bool) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(dir) = hooks_dir(root) else {
        return notes;
    };
    if !dir.exists() && !dry && std::fs::create_dir_all(&dir).is_err() {
        return notes;
    }

    for hook in HOOKS {
        let path = dir.join(hook);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.contains(HOOK_BEGIN) {
            notes.push(format!("{hook}: already installed"));
            continue;
        }
        if !existing.trim().is_empty() && !existing.starts_with("#!") {
            notes.push(format!(
                "{hook}: exists but is not a shell script, leaving it alone"
            ));
            continue;
        }

        let body = if existing.trim().is_empty() {
            format!("#!/bin/sh\n{}", hook_script(hook, exe))
        } else {
            format!("{}\n{}", existing.trim_end(), hook_script(hook, exe))
        };
        if dry {
            notes.push(format!("{hook}: would write {}", path.display()));
            continue;
        }
        if std::fs::write(&path, body).is_err() {
            notes.push(format!("{hook}: could not write {}", path.display()));
            continue;
        }
        // Git ignores a hook that is not executable, and says nothing about it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        notes.push(format!("{hook}: installed"));
    }
    notes
}

/// Removes exactly the fenced block, leaving anyone else's hook intact.
///
/// A hook file that holds nothing but our block is deleted rather than left as
/// a bare shebang, which git would still run on every operation.
fn uninstall_hooks(root: &std::path::Path, dry: bool) -> Vec<String> {
    let mut notes = Vec::new();
    let Some(dir) = hooks_dir(root) else {
        return notes;
    };
    for hook in HOOKS {
        let path = dir.join(hook);
        let Ok(existing) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (Some(a), Some(b)) = (existing.find(HOOK_BEGIN), existing.find(HOOK_END)) else {
            continue;
        };
        let mut rest = existing[..a].trim_end().to_string();
        rest.push_str(&existing[b + HOOK_END.len()..]);
        let bare = rest.trim() == "#!/bin/sh" || rest.trim().is_empty();
        if dry {
            notes.push(format!("{hook}: would remove"));
            continue;
        }
        let done = if bare {
            std::fs::remove_file(&path)
        } else {
            std::fs::write(&path, rest)
        };
        notes.push(match done {
            Ok(()) => format!("{hook}: removed"),
            Err(e) => format!("{hook}: {e}"),
        });
    }
    notes
}

/// Keeps the files glasir writes out of version control.
///
/// Not tidiness — the stored graph is rewritten on every analysis, so leaving
/// it untracked makes the tree permanently dirty and git then **refuses a
/// branch switch**: "Please commit your changes or stash them". Found exactly
/// that way, by a hook that had just refreshed the graph. The token file holds
/// no usable secret, only hashes, but committing who has access to what is an
/// operational leak, and the audit log holds the questions people asked.
fn ignore_generated_files(root: &std::path::Path, names: &[&str]) {
    let path = root.join(".gitignore");
    let mut current = std::fs::read_to_string(&path).unwrap_or_default();
    for name in names {
        if current.lines().any(|l| l.trim() == *name) {
            continue;
        }
        let sep = if current.is_empty() || current.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        current = format!("{current}{sep}{name}\n");
    }
    let _ = std::fs::write(&path, current);
}

/// Everything an analysis or a served tree writes beside the sources.
const GENERATED: &[&str] = &[
    ".glasir-graph",
    // A pattern, not two names: layouts are `.glasir-layout-<mode>` and a
    // third mode would otherwise be missed silently.
    ".glasir-layout-*",
    auth::TOKEN_FILE,
    audit::AUDIT_FILE,
];

/// What a tree is called in the audit log: its directory name, not its path.
///
/// The same reasoning that keeps symbol names relative to the root — an
/// absolute path bloats every line and leaks where the tree lives on the
/// server. A control plane reading several logs together needs to tell them
/// apart, and the directory name does that; it maps the name it registered
/// onto the records itself.
fn tree_name(root: &std::path::Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

/// The analysed tree in the form a server holds it: owning its parts, so it can
/// be swapped under readers when the tree changes.
fn served_state(root: &std::path::Path) -> std::io::Result<mcp::ServedState> {
    let a = analyse(root)?;
    let embeddings = embed::embed(&a.snap, 4);
    let search = search::SearchIndex::build_with_docs(&a.registry, a.registry.docs());
    let names = mcp::name_table(&a.registry, a.snap.width());
    Ok(mcp::ServedState {
        snap: a.snap,
        names,
        defined: a.defined,
        registry: a.registry,
        communities: a.communities,
        embeddings,
        search,
        physics: physics::Physics::default(),
        now: now(),
        files: std::sync::OnceLock::new(),
        mentions: std::sync::OnceLock::new(),
        references: std::sync::OnceLock::new(),
        root: root.to_path_buf(),
    })
}

/// Replaces tier 2's guesses for one file with the server's answers.
///
/// LSP reports who references a definition, so each fact names the *caller's*
/// file and the definition it reaches. Those edges are applied as their own
/// kind, which lets a re-query replace the previous set wholesale rather than
/// piling duplicates on every save.
fn upgrade_edges(
    g: &std::sync::Arc<graph::Graph>,
    reg: &mut ingest::SymbolRegistry,
    root: &std::path::Path,
    facts: Vec<(String, u32, String)>,
    now: u64,
) -> usize {
    let mut edges = Vec::new();
    for (caller_file, _line, definition) in facts {
        let caller_rel = std::path::Path::new(&caller_file)
            .strip_prefix(root)
            .unwrap_or(std::path::Path::new(&caller_file))
            .to_string_lossy()
            .to_string();
        // The caller is named by file only: LSP gives a position, not the
        // enclosing function, and resolving that would need a second query per
        // reference. File-level is enough to say "this file calls that".
        let source = reg.get_or_mint(&caller_rel);
        let target = reg.get_or_mint(&definition);
        edges.push((
            source,
            csr::Edge {
                target,
                timestamp: now,
                authority: physics::SOURCE_CODE,
                edge_kind: LSP_EDGE,
                confidence: csr::Confidence::Extracted,
            },
        ));
    }
    let count = edges.len();
    g.update(|_, d| d.replace_edges_of_kind(LSP_EDGE, edges.clone()));
    count
}

/// Edge kind for a live LSP link, so a re-query can replace exactly its own
/// edges without touching tier 2's or tier 3's.
const LSP_EDGE: u16 = 2;

/// Tier 3 without the progress line, for the stdio server.
fn link_placeholders_quiet(g: &std::sync::Arc<graph::Graph>, reg: &ingest::SymbolRegistry) {
    let links = resolve::resolve(reg);
    let edges = resolve::link_edges(&links, now());
    if !edges.is_empty() {
        g.update(|_, d| d.replace_edges_of_kind(resolve::RESOLVES_TO, edges.clone()));
    }
}

/// Watches a directory and keeps the graph in sync with what is on disk.
fn run_watch(root: &str) -> std::io::Result<()> {
    use std::sync::Arc;

    // Canonical, or strip_prefix fails against the absolute paths the watcher
    // reports and every symbol keeps a "./" prefix.
    let root = &canonical(std::path::Path::new(root))?;
    let g = Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let mut arena = arena::SymbolArena::new();
    let mut reg = ingest::SymbolRegistry::new(0);

    // Tier 1 first: an index.scip next to the tree means compiler-resolved
    // edges for the files it covers, and tier 2 is only the fallback for the
    // rest. Its files are recorded so the sweep can skip them.
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let scip = root.join("index.scip");
    if scip.exists() {
        match import_scip::import(&scip, root, &mut reg) {
            Ok(r) => {
                covered.extend(r.fresh_files().cloned());
                let edges = r.edges.clone();
                g.update(|_, d| {
                    for &(src, e) in &edges {
                        d.add_edge(src, e);
                    }
                });
                println!(
                    "tier 1: {} definitions, {} references, {} external, {} files",
                    r.definitions,
                    r.references,
                    r.external,
                    r.file_nodes.len()
                );
                // Say it plainly: a silently ageing index is the failure mode
                // this check exists to prevent.
                if !r.stale_files.is_empty() {
                    println!(
                        "  {} of them edited since the index was built — those fall back to tier 2",
                        r.stale_files.len()
                    );
                    println!("  rebuild with: rust-analyzer scip .");
                }
            }
            // A malformed or stale index must not stop the fallback tiers —
            // that is the whole point of a cascade.
            Err(e) => println!("tier 1 unavailable ({e}), falling back to tier 2"),
        }
    }

    // Initial sweep, so the graph reflects the tree before the first edit.
    let mut files = 0usize;
    let mut skipped = 0usize;
    for path in walk(root) {
        // Tier 1 already covered this file with better data.
        if covered.contains(&*path.strip_prefix(root).unwrap_or(&path).to_string_lossy()) {
            skipped += 1;
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&path)
            && ingest::ingest_file(&g, &mut arena, &mut reg, &path, root, &src, now()).is_some()
        {
            files += 1;
        }
    }
    println!(
        "indexed {files} files ({skipped} covered by tier 1), {} symbols",
        reg.len()
    );
    let (bare, qualified) = reg.split_counts();
    println!("  {qualified} defined, {bare} unresolved references");

    // Tier 3 runs after the sweep, not per file: a call is routinely parsed
    // before the file that defines it, so linking earlier would miss most of it.
    link_placeholders(&g, &reg);

    // Phase 4 over the finished graph.
    let snap = g.load();
    let t = std::time::Instant::now();
    // Only symbols defined in this tree can form a community; the registry
    // knows which those are, the graph does not.
    let defined: std::collections::HashSet<csr::NodeId> = reg
        .entries()
        .filter(|(name, _)| name.contains('#'))
        .map(|(_, &n)| n)
        .collect();
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let t_comm = t.elapsed();
    let t = std::time::Instant::now();
    let emb = embed::embed(&snap, 4);
    let t_emb = t.elapsed();
    let mut sizes: Vec<usize> = (0..comms.count() as u32)
        .map(|c| comms.members(c).len())
        .collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    println!(
        "  community sizes (top 8): {:?}",
        &sizes[..sizes.len().min(8)]
    );
    let broken = (0..comms.count() as u32)
        .filter(|&c| !community::is_connected(&snap, &comms.members(c)))
        .count();
    println!("  internally disconnected communities: {broken}");
    let leaves = (0..snap.width() as csr::NodeId)
        .filter(|&n| snap.neighbors(n).next().is_none())
        .count();
    println!(
        "  singletons: {} (of which {leaves} are leaf nodes with no outgoing edges)",
        sizes.iter().filter(|&&s| s == 1).count()
    );
    let real: Vec<usize> = (0..comms.count() as u32)
        .map(|c| comms.members(c).len())
        .filter(|&s| s > 1)
        .collect();
    // The number that matters: communities holding at least two symbols this
    // codebase actually defines. A group of foreign names clustering together
    // inflates the count while saying nothing about the code.
    let substantive: Vec<usize> = (0..comms.count() as u32)
        .map(|c| {
            comms
                .members(c)
                .iter()
                .filter(|n| defined.contains(n))
                .count()
        })
        .filter(|&d| d >= 2)
        .collect();
    println!(
        "  meaningful communities: {} covering {} nodes ({} with >=2 defined symbols, {} defined nodes)",
        real.len(),
        real.iter().sum::<usize>(),
        substantive.len(),
        substantive.iter().sum::<usize>()
    );
    println!(
        "communities: {} ({} hubs, {} pendants) in {t_comm:?} | embeddings: {}d in {t_emb:?}",
        comms.count(),
        comms.hubs.len(),
        comms.pendants.len(),
        embed::DIM
    );

    // What a query actually costs: expand from a seed and report the cut.
    // Pick a well-connected symbol rather than a fixed name: tier 1 qualifies
    // methods as `[Graph]update`, so a hard-coded one silently misses and the
    // fallback reports a leaf's subgraph as if it were the example.
    let seed = (0..snap.width() as csr::NodeId)
        .filter(|n| defined.contains(n))
        .max_by_key(|&n| snap.neighbors(n).count());
    if let Some(seed) = seed {
        let cfg = physics::Physics::default();
        let kept = physics::expand(&snap, &[(seed, 1.0)], now(), &cfg, Some(&defined));
        let name = reg
            .entries()
            .find(|&(_, &n)| n == seed)
            .map(|(s, _)| s.as_str())
            .unwrap_or("?");
        println!(
            "query from {name}: {} of {} nodes ({:.1}% cut), nearest by embedding: {:?}",
            kept.len(),
            snap.width(),
            physics::reduction(kept.len(), snap.width()),
            emb.nearest(seed, 3)
                .iter()
                .map(|(n, s)| (*n, (s * 100.0).round() / 100.0))
                .collect::<Vec<_>>()
        );
    }

    // Tier 1 live: one server per language, started once and kept warm. A
    // save then costs ~1 s of compiler-grade re-query instead of degrading to
    // tier 2's syntactic guesses until the next index build.
    // `None` records a server that could not be started, so a missing binary
    // is not retried on every save.
    let mut servers: std::collections::HashMap<&str, Option<lsp::LspClient>> =
        std::collections::HashMap::new();

    // Files edited since the index was built. Once enough of the tree has moved
    // on, the index is rebuilt in the background rather than left to rot — the
    // live server covers a saved file, but not a file someone else changed, and
    // not the next cold start.
    let mut drifted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rebuilding = false;

    println!("watching {} (ctrl-c to stop)", root.display());
    watcher::watch(root, |batch| {
        let mut touched = 0;
        for path in batch {
            // A deleted file has no source to parse; its edges are dropped by
            // re-parsing it as empty so the graph does not keep stale calls.
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            if ingest::ingest_file(&g, &mut arena, &mut reg, &path, root, &src, now()).is_none() {
                continue;
            }
            touched += 1;

            // Then upgrade what tier 2 just guessed, where a server can do
            // better. Starting one costs seconds, so it is kept for the life of
            // the watch; a tree with no server installed simply keeps tier 2.
            let upgraded = (!src.is_empty())
                .then(|| lsp::server_for(&path))
                .flatten()
                .and_then(|(command, language)| {
                    let slot = servers.entry(command).or_insert_with(|| {
                        eprintln!("  starting {command}…");
                        let mut client = lsp::LspClient::start(command, language, root)?;
                        // The first query would otherwise come back empty and
                        // successful, which reads as "nothing calls this".
                        client.wait_until_ready(std::time::Duration::from_secs(60));
                        Some(client)
                    });
                    lsp::file_facts(slot.as_mut()?, &path, &src)
                })
                .map(|facts| upgrade_edges(&g, &mut reg, root, facts, now()))
                .unwrap_or(0);

            // Track drift against the index and heal it before it matters.
            if scip.exists()
                && let Ok(rel) = path.strip_prefix(root)
            {
                drifted.insert(rel.to_string_lossy().to_string());
                // A handful of saved files is normal; a fifth of the indexed
                // tree means the index describes something else by now.
                let threshold = (covered.len() / 5).max(3);
                if drifted.len() >= threshold && !rebuilding {
                    match import_scip::rebuild_in_background(root) {
                        Some(name) => {
                            println!(
                                "  {} files drifted from the index — rebuilding with {name} in the background",
                                drifted.len()
                            );
                            rebuilding = true;
                        }
                        // No indexer for this tree: say so once rather than
                        // checking again on every save.
                        None => rebuilding = true,
                    }
                    drifted.clear();
                }
            }

            if upgraded > 0 {
                println!(
                    "  {} -> {} symbols, {upgraded} compiler-verified",
                    path.display(),
                    reg.len()
                );
            } else {
                println!("  {} -> {} symbols", path.display(), reg.len());
            }
        }
        if touched > 0 {
            // A new file can define what an older call referenced, so the
            // links are recomputed after each batch.
            link_placeholders(&g, &reg);
            let snap = g.load();
            println!(
                "graph: {} base nodes, {} delta mutations",
                snap.base.node_count(),
                snap.delta.len()
            );
        }
    })
    .map_err(std::io::Error::other)
}

/// Applies tier 3 to the current registry and reports what it could link.
fn link_placeholders(g: &std::sync::Arc<graph::Graph>, reg: &ingest::SymbolRegistry) {
    let links = resolve::resolve(reg);
    let edges = resolve::link_edges(&links, now());
    let ambiguous = links.len() - edges.len();
    if edges.is_empty() && ambiguous == 0 {
        return;
    }
    g.update(|_, d| d.replace_edges_of_kind(resolve::RESOLVES_TO, edges.clone()));
    println!("  tier 3: linked {}, {ambiguous} ambiguous", edges.len());
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Source files under `root`, skipping the directories that would otherwise
/// dominate the index with build output and dependencies.
pub(crate) fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if watcher::ignored_under(&p, Some(root)) {
                continue;
            }
            // `file_type()` reports the entry itself; `is_dir()` follows the
            // link. A `ln -s . loop` anywhere in the tree then descends until
            // the path hits the system limit — measured on a one-file tree, it
            // produced 41 symbols from one function. A symlinked directory is
            // skipped rather than resolved: following it also indexes the same
            // sources twice under two names, which is the same problem without
            // the loop.
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(p);
            } else if parse_ast::Lang::from_path(&p).is_some() || docs::is_markdown(&p) {
                out.push(p);
            }
        }
    }
    // `read_dir` deliberately makes no ordering promise. The registry assigns
    // monotonic node IDs while files are folded, so accepting that order would
    // make an otherwise identical checkout produce a different graph and move
    // retrieval tie-breaks. Sort once at the boundary rather than asking every
    // graph consumer to remember this invariant.
    out.sort();
    out
}

/// Loads a graph the way the real entry point does — from the CSR cache when it
/// is fresh, falling back to JSON — and reports the cost of each path.
fn bench_import(path: &str) -> std::io::Result<()> {
    use std::time::Instant;

    let source = std::path::Path::new(path);
    let cache = source.with_extension("csr");

    if store::is_fresh(&cache, source) {
        let t = Instant::now();
        let mapped = store::MappedCsr::open(&cache)?;
        let g = mapped.graph()?;
        let (n, e) = (g.node_count(), g.edge_count());
        let warm = t.elapsed();

        let t = Instant::now();
        let mut seen = 0usize;
        for node in 0..n as csr::NodeId {
            seen += g.neighbors(node).count();
        }
        println!("warm start {warm:?}  ({n} nodes, {e} edges, no JSON)");
        println!("traverse   {:?} over {seen} edges", t.elapsed());
        return Ok(());
    }

    let mut arena = arena::SymbolArena::new();
    let t = Instant::now();
    let g = import_json::import(source, &mut arena, 1_756_600_000)?;
    let parse = t.elapsed();

    let t = Instant::now();
    store::write(&g.csr, &cache)?;
    let write = t.elapsed();
    let on_disk = std::fs::metadata(&cache)?.len();

    println!("nodes      {}", g.csr.node_count());
    println!(
        "edges      {} ({} skipped)",
        g.csr.edge_count(),
        g.skipped_edges
    );
    println!("relations  {}", g.relations.len());
    println!("files      {}", g.file_nodes.len());
    println!("cold start {parse:?} (JSON)");
    println!("csr write  {write:?}  -> {} KiB on disk", on_disk / 1024);
    println!("           re-run to measure the warm start");
    Ok(())
}

/// Round-trips a small graph through build -> mmap -> traversal.
/// A name no other concurrently running check will pick.
///
/// Under `cargo test` every test runs in the same process on its own thread, so
/// a process id is not enough — two checks sharing a fixture path would delete
/// each other's files and fail for a reason that has nothing to do with what
/// they test. The thread name is the test's own name there, and "main" in the
/// single-threaded `selfcheck` run.
fn fixture_id() -> String {
    let current_thread = std::thread::current();
    let thread = current_thread.name().unwrap_or("main");
    let safe_thread: String = thread
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{}-{safe_thread}", std::process::id())
}

fn demo() -> std::io::Result<()> {
    // First: every check below builds fixtures under the system temp dir, so
    // an over-broad ignore rule makes eleven of them fail for a reason that
    // has nothing to do with what they test.
    demo_ignored_paths();

    let mut arena = arena::SymbolArena::new();
    let mut b = CsrBuilder::new();

    let checkout = b.add_node(arena.intern("src/checkout.rs#pay"));
    let payment = b.add_node(arena.intern("src/payment.rs#charge"));
    let logger = b.add_node(arena.intern("src/log.rs#info"));

    let call = |target, confidence| Edge {
        target,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence,
    };
    b.add_edge(checkout, call(payment, Confidence::Extracted));
    b.add_edge(checkout, call(logger, Confidence::Inferred));
    b.add_edge(payment, call(logger, Confidence::Extracted));

    let csr = b.build();
    assert_eq!(csr.node_count(), 3);
    assert_eq!(csr.edge_count(), 3);
    assert_eq!(csr.neighbors(checkout).count(), 2);
    assert_eq!(csr.neighbors(logger).count(), 0);
    // Out-of-range nodes yield an empty range instead of panicking.
    assert_eq!(csr.neighbors(99).count(), 0);
    // Insertion order survives the sort.
    let first = csr.neighbors(checkout).next().unwrap();
    assert_eq!(first.target, payment);
    assert_eq!(first.confidence, Confidence::Extracted);
    // Confidence ordering drives filtering later on.
    assert!(Confidence::Extracted > Confidence::Inferred);

    let path = std::env::temp_dir().join(format!("glasir-demo-{}.csr", fixture_id()));
    store::write(&csr, &path)?;
    let mapped = store::MappedCsr::open(&path)?;
    let archived = mapped.graph()?;
    assert_eq!(archived.node_count(), 3);
    assert_eq!(archived.neighbors(checkout).count(), 2);
    assert_eq!(
        arena.resolve(archived.symbol(payment).unwrap()),
        Some("src/payment.rs#charge")
    );
    std::fs::remove_file(&path)?;

    println!(
        "phase 1 ok: {} nodes, {} edges",
        csr.node_count(),
        csr.edge_count()
    );

    demo_delta(arena.intern("src/checkout.rs"), checkout, payment, logger);
    Ok(())
}

/// Round-trips the union view: tombstoning, re-parse scoping and compaction.
fn demo_delta(file: u32, checkout: csr::NodeId, payment: csr::NodeId, logger: csr::NodeId) {
    use graph::Graph;

    let mut b = CsrBuilder::new();
    for name in ["checkout", "payment", "logger"] {
        b.add_node(name.len() as u32);
    }
    let call = |target| Edge {
        target,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: Confidence::Extracted,
    };
    b.add_edge(checkout, call(payment));
    b.add_edge(checkout, call(logger));
    let g = std::sync::Arc::new(Graph::new(b.build()));

    // The file owns checkout; its base edges go to payment and logger.
    g.update(|_, d| d.set_file_nodes(file, vec![checkout]));
    assert_eq!(g.load().neighbors(checkout).count(), 2);

    // Re-parse drops the logger call and keeps the payment call. The kept edge
    // must survive as exactly one edge, not vanish with its tombstone.
    g.update(|snap, d| {
        let targets = snap.base_targets(checkout);
        d.replace_file_edges(
            file,
            |_| targets.clone(),
            vec![(checkout, call(payment))],
            &[checkout],
        );
    });
    let snap = g.load();
    let after: Vec<_> = snap.neighbors(checkout).map(|e| e.target).collect();
    assert_eq!(after, vec![payment], "re-parse must scope to the file");
    assert_eq!(snap.neighbors(payment).count(), 0);
    assert!(!snap.delta.is_empty());

    // The reverse index is cached per snapshot, so it must describe the
    // snapshot that owns it and never a later one. `checkout -> logger` was
    // just dropped: the *old* snapshot still reports it, the new one must not.
    // Broken on purpose by caching on `Graph` instead of `GraphSnapshot` — the
    // second assert then still sees the retracted caller.
    let callers_of_logger = |s: &graph::GraphSnapshot| s.reverse().callers(logger).count();
    assert_eq!(
        callers_of_logger(&snap),
        0,
        "the re-parse dropped that call"
    );
    // Asking twice must give the same answer: the second read comes from the
    // cache, and a cache that disagrees with its first answer is worse than
    // none.
    assert_eq!(callers_of_logger(&snap), 0);
    assert_eq!(
        snap.reverse().callers(payment).count(),
        1,
        "payment keeps exactly one caller through the re-parse"
    );

    // Compaction folds the delta back into the base and preserves the view.
    let compacted = g.load().compact();
    assert!(compacted.delta.is_empty());
    assert_eq!(
        compacted
            .neighbors(checkout)
            .map(|e| e.target)
            .collect::<Vec<_>>(),
        vec![payment]
    );
    assert_eq!(compacted.base.node_count(), 3);

    // Crossing the threshold compacts on write, without a manual call.
    // Counted outside the closure: update() may retry it, so a counter
    // incremented inside would double-count.
    let to_payment = 1
        + (0..graph::COMPACTION_THRESHOLD)
            .filter(|i| (i % 3) as csr::NodeId == payment)
            .count();
    g.update(|_, d| {
        for i in 0..graph::COMPACTION_THRESHOLD {
            d.add_edge(checkout, call((i % 3) as csr::NodeId));
        }
    });
    // The write returns before the worker is done, so the delta is still
    // there right after it — compaction lands asynchronously.
    g.wait_for_compaction();
    assert!(
        g.load().delta.is_empty(),
        "threshold must trigger compaction"
    );
    assert_eq!(
        g.load()
            .neighbors(checkout)
            .filter(|e| e.target == payment)
            .count(),
        to_payment,
        "compaction must preserve the union view"
    );

    demo_import();
    demo_native_rust();
    demo_parse();
    demo_ingest();
    demo_resolve();
    demo_scip();
    demo_physics();
    demo_embed();
    demo_community();
    demo_mcp();
    demo_change_tools();
    demo_imports();
    demo_references();
    demo_install();
    demo_search();
    demo_snapshot();
    demo_http();
    demo_http_concurrent();
    demo_http_events();
    demo_snippet();
    demo_deterministic();
    demo_tls();
    demo_auth();
    demo_audit();
    demo_hooks();
    demo_baseline();
    demo_langcheck();
    demo_extension_collision();
    demo_edges();
    demo_view();
    demo_cycles();
    demo_bench();
    demo_docs();
    demo_doc_coverage();
    demo_doc_ranking();
    demo_markdown();
    demo_walk_order();
    demo_parallel_ingest();
    demo_incremental();
    demo_index_scale();
    demo_cohesion_scale();
    demo_subsystem_scale();
    demo_batch_build();
    demo_concurrent_compaction();
    println!("phase 2 ok: union view, file scoping, compaction");
}

/// File discovery is a deterministic input to graph construction.
///
/// Directory enumeration order is filesystem-specific. A sorted walk is what
/// keeps node IDs, snapshots, search tie-breaks, and benchmark recall stable
/// across an engineer's machine and a clean CI checkout.
fn demo_walk_order() {
    let dir = std::env::temp_dir().join(format!("glasir-walk-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("z/nested")).unwrap();
    std::fs::create_dir_all(dir.join("a")).unwrap();
    // Create in deliberately non-lexical order. A filesystem is free to
    // return this order, creation order, hash order, or another order entirely.
    for path in ["z/nested/last.rs", "a/first.rs", "root.rs", "z/middle.rs"] {
        std::fs::write(dir.join(path), "fn stable() {}\n").unwrap();
    }

    let files = walk(&dir);
    let mut expected = files.clone();
    expected.sort();
    assert_eq!(
        files, expected,
        "walk must canonicalize filesystem enumeration before graph construction"
    );
    assert_eq!(files.len(), 4, "the fixture must exercise nested paths");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Hammers the graph with writes while compaction runs in the background: no
/// mutation may be lost, and readers must never observe a torn view.
fn demo_concurrent_compaction() {
    use graph::Graph;
    use std::sync::Arc;

    let mut b = CsrBuilder::new();
    for _ in 0..4 {
        b.add_node(0);
    }
    let g = Arc::new(Graph::new(b.build()));

    let call = |target| Edge {
        target,
        timestamp: 0,
        authority: 1.0,
        edge_kind: 0,
        confidence: Confidence::Inferred,
    };

    // A reader spinning on the union view the whole time. If a compaction ever
    // published a half-built base, the count would dip below what was written.
    let reader = {
        let g = Arc::clone(&g);
        std::thread::spawn(move || {
            let mut lowest = usize::MAX;
            for _ in 0..2_000 {
                lowest = lowest.min(g.load().neighbors(0).count());
            }
            lowest
        })
    };

    let writes = graph::COMPACTION_THRESHOLD * 3;
    // Two writers contend on the same read-modify-write path.
    for _ in 0..writes {
        g.update(|_, d| d.add_edge(0, call(1)));
    }
    let lowest = reader.join().unwrap();
    g.wait_for_compaction();

    // Every write is accounted for, whether it ended up in a compacted base or
    // in the delta on top of it.
    let snap = g.load();
    assert_eq!(
        snap.neighbors(0).count(),
        writes,
        "concurrent compaction must not drop writes"
    );
    assert!(lowest <= writes, "reader saw more edges than were written");
    println!("phase 2 ok: {writes} writes survived background compaction");
}

/// Imports a node-link fixture covering the cases that actually bite: both edge
/// spellings, all three confidence tiers, a missing weight, parallel edges and
/// an edge pointing at an undeclared node.
fn demo_import() {
    let fixture = r#"{
      "nodes": [
        {"id": "a", "label": "Transformer", "file_type": "code", "source_file": "model.py"},
        {"id": "b", "label": "Attention", "file_type": "code", "source_file": "model.py"},
        {"id": "c", "label": "attention mechanism", "file_type": "document", "source_file": "paper.md"}
      ],
      "links": [
        {"source": "a", "target": "b", "relation": "contains", "confidence": "EXTRACTED", "weight": 1.0},
        {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED"},
        {"source": "b", "target": "c", "relation": "implements", "confidence": "INFERRED", "weight": 0.8},
        {"source": "c", "target": "a", "relation": "referenced", "confidence": "WEIRD", "weight": 0.5},
        {"source": "a", "target": "ghost", "relation": "calls", "confidence": "EXTRACTED"}
      ]
    }"#;

    let path = std::env::temp_dir().join(format!("glasir-import-{}.json", fixture_id()));
    std::fs::write(&path, fixture).unwrap();
    let mut arena = arena::SymbolArena::new();
    let g = import_json::import(&path, &mut arena, 1_756_600_000).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(g.csr.node_count(), 3);
    assert_eq!(g.skipped_edges, 1, "the dangling edge must be reported");
    assert_eq!(g.csr.edge_count(), 4);

    let a = g.by_external_id["a"];
    let b = g.by_external_id["b"];
    // Parallel edges survive: the source graph is a multigraph.
    assert_eq!(g.csr.neighbors(a).filter(|e| e.target == b).count(), 2);
    // Distinct relations get distinct edge_kind ids.
    let kinds: Vec<u16> = g.csr.neighbors(a).map(|e| e.edge_kind).collect();
    assert_eq!(kinds[0], 0);
    assert_ne!(kinds[0], kinds[1]);
    // A missing weight falls back to the tier's authority, never to zero.
    let implicit = g.csr.neighbors(a).nth(1).unwrap();
    assert_eq!(implicit.authority, 1.0);
    // An unknown confidence tag degrades to the weakest tier instead of dropping.
    let c = g.by_external_id["c"];
    assert_eq!(
        g.csr.neighbors(c).next().unwrap().confidence,
        Confidence::Ambiguous
    );
    // File scoping needs the file -> nodes map the delta store consumes.
    assert_eq!(g.file_nodes[&arena.intern("model.py")], vec![a, b]);

    println!(
        "tier 0 ok: {} nodes, {} edges, {} relations, {} skipped",
        g.csr.node_count(),
        g.csr.edge_count(),
        g.relations.len(),
        g.skipped_edges
    );
}

/// Parses one file per language plus a syntactically broken one: tier 2 exists
/// precisely for code that does not compile, so a broken file must still yield
/// the facts it can.
/// `(caller, callee)` regardless of whether the call had a receiver. The
/// receiver flag is asserted on its own where it matters, so the shape checks
/// stay readable.
fn calls_contain(calls: &[(String, String, bool)], caller: &str, callee: &str) -> bool {
    calls.iter().any(|(a, b, _)| a == caller && b == callee)
}

/// The Rust scanner, on the constructs that broke it.
///
/// Each case here is a real failure found by measuring against the grammar
/// path, not a guess: a scanner that gets any of them wrong loses definitions
/// silently, which is the failure mode this project keeps finding.
fn demo_native_rust() {
    use parse_ast::{Lang, parse};

    // A raw string ends at a quote followed by as many hashes as opened it.
    // Stopping at the first quote swallowed the rest of the file.
    // An *odd* number of quotes inside is what separates the two paths: with an
    // even count a naive scanner happens to land on the same byte. Checked by
    // deleting the raw-string branch — the even case still passed, this one
    // drops from 3 definitions to 1.
    let f = parse(
        "const Q: &str = r#\"unmatched \" quote\"#;\nfn after() {}\nfn third() {}",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        f.defines.contains(&"after".to_string()) && f.defines.contains(&"third".to_string()),
        "code after a raw string is still parsed: {:?}",
        f.defines
    );
    let f = parse(
        "const Q: &str = r#\"fn not_real() {}\"#;\nfn after() {}",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        !f.defines.contains(&"not_real".to_string()),
        "a definition inside a raw string is text, not code: {:?}",
        f.defines
    );

    // `&'static str` — a lifetime is not a char literal, and `static` after it
    // is not the keyword. Seven type aliases invented a definition called `str`.
    let f = parse("type Name = &'static str;", Lang::Rust).unwrap();
    assert_eq!(f.defines, vec!["Name".to_string()], "{:?}", f.defines);

    // A modifier stands between a doc comment and what it documents.
    let f = parse("/// Charges the cart.\npub fn pay() {}", Lang::Rust).unwrap();
    assert!(
        f.docs
            .iter()
            .any(|(n, d)| n == "pay" && d.contains("Charges")),
        "a doc comment survives `pub`: {:?}",
        f.docs
    );

    // `self.f()` reaches the local definition; `Other::f()` does not. The
    // same-file rule keys on this, and without the exemption `compact` called a
    // bare `neighbors` instead of `src/graph.rs#neighbors`.
    let f = parse(
        "fn a() { self.helper(); }\nfn b() { Other::helper(); }",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        f.calls
            .iter()
            .any(|(c, t, r)| c == "a" && t == "helper" && !r),
        "self is not a receiver: {:?}",
        f.calls
    );
    assert!(
        f.calls
            .iter()
            .any(|(c, t, r)| c == "b" && t == "helper" && *r),
        "a path call has a receiver: {:?}",
        f.calls
    );

    // `extern "C"` declares what another object file defines. Counting those
    // gives tier 3 a local target for a name this tree does not implement.
    let f = parse(
        "unsafe extern \"C\" { fn imported(); }\nfn ours() {}",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        !f.defines.contains(&"imported".to_string()),
        "an extern declaration is not a definition here: {:?}",
        f.defines
    );

    // A macro body holds real definitions, which the grammar path cannot see at
    // all — `csr.rs#edge_range` was absent from the graph entirely.
    let f = parse(
        "macro_rules! m { ($t:ty) => { impl $t { pub fn accessor(&self) {} } } }",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        f.defines.contains(&"accessor".to_string()),
        "a method defined in a macro is a definition: {:?}",
        f.defines
    );

    // One doc entry per symbol, never two: `set_doc` replaces rather than
    // appends, so a second entry silently discards the first — measured at 14
    // points of `questions` recall when the body block was pushed separately.
    let f = parse(
        "/// Head.\nfn f() {\n    // Body.\n    helper();\n}",
        Lang::Rust,
    )
    .unwrap();
    let entries: Vec<&(String, String)> = f.docs.iter().filter(|(n, _)| n == "f").collect();
    assert_eq!(entries.len(), 1, "one entry per symbol: {:?}", f.docs);
    let text = &entries[0].1;
    assert!(
        text.contains("Head") && text.contains("Body"),
        "the entry carries both halves: {text:?}"
    );

    // A byte offset landing inside a multi-byte character must not panic. The
    // scanner walks bytes, so an em dash in a comment can put a boundary
    // there — measured on real code, that took down 10 of 48 Java files and
    // 1 of 382 C++ files before every slice went through a checked helper.
    // The real files put it in a Javadoc block, which is the form that broke:
    // `/** … — … */` over several lines.
    for (lang, src) in [
        (
            Lang::Java,
            "/**\n * A check never acts on a single observation — it escalates\n * only as evidence crosses its thresholds.\n */\nclass A { void f() {} }",
        ),
        (
            Lang::Cpp,
            "/**\n * Draws the frame — one line per scanline.\n */\nint f() { return 1; }",
        ),
        (
            Lang::Python,
            "# Umlaute: ä ö ü — und ein Strich\ndef f(): pass",
        ),
    ] {
        let f = parse(src, lang).expect("a multi-byte character must not panic");
        assert!(!f.defines.is_empty(), "{lang:?} still yields facts: {f:?}");
    }

    // `if (x) { … }` is lexically a shorthand method, and four parsers carried
    // a copy of that check with only one excluding keywords. Measured on real
    // code: C# put `if` in the graph 786 times, Java 72 — and a bare `if` node
    // collides with every other file that has one, which is how `cycles` came
    // to report a JavaScript file calling Rust.
    for (lang, src) in [
        (
            Lang::Java,
            "class C {\n  void handle(int x) {\n    if (x > 0) { log(x); }\n    for (int i=0;i<3;i++) { step(i); }\n  }\n}",
        ),
        (
            Lang::CSharp,
            "class C {\n  void Handle(int x) {\n    if (x > 0) { Log(x); }\n  }\n}",
        ),
        (
            Lang::JavaScript,
            "function run() {\n  if (a) { log(a); }\n  while (b) { step(b); }\n}",
        ),
    ] {
        let f = parse(src, lang).unwrap();
        for kw in ["if", "for", "while", "catch"] {
            assert!(
                !f.defines.contains(&kw.to_string()),
                "{lang:?}: `{kw}` is control flow, not a definition: {:?}",
                f.defines
            );
        }
        assert!(
            f.calls.iter().any(|(_, t, _)| t == "log" || t == "Log"),
            "{lang:?}: the call inside it still lands: {:?}",
            f.calls
        );
    }

    // `def this(…)` is Scala's auxiliary constructor. Named after its class the
    // way Java names one: a symbol called `this` collides with every other file
    // that has one, and there were 113 across Scala's own library.
    let f = parse(
        "class Foo(x: Int) {\n  def this(s: String) = this(s.length)\n  def run(): Int = x\n}",
        Lang::Scala,
    )
    .unwrap();
    assert!(
        !f.defines.contains(&"this".to_string()),
        "a constructor is named after its class: {:?}",
        f.defines
    );
    assert!(f.defines.contains(&"run".to_string()), "{:?}", f.defines);

    // Julia's short form is as common as `function … end` — 3,939 against
    // 4,037 on 200 files of its own base library — and without it half the
    // definitions are missing and their calls land on `<module>`.
    let f = parse(
        "pay(cart) = charge(cart.total)\nfunction charge(t)\n    t\nend",
        Lang::Julia,
    )
    .unwrap();
    assert!(
        f.defines.contains(&"pay".to_string()) && f.defines.contains(&"charge".to_string()),
        "both forms define: {:?}",
        f.defines
    );
    assert!(
        calls_contain(&f.calls, "pay", "charge"),
        "the short form owns its body: {:?}",
        f.calls
    );

    // `class << self` reopens the singleton class: it groups what follows and
    // defines no name. Found on real code — every Ruby file using the form had
    // a definition called `self`.
    let f = parse(
        "class C\n  class << self\n    def helper; end\n  end\nend",
        Lang::Ruby,
    )
    .unwrap();
    assert!(
        !f.defines.contains(&"self".to_string()),
        "`class << self` defines no name: {:?}",
        f.defines
    );
    assert!(f.defines.contains(&"helper".to_string()), "{:?}", f.defines);

    // Rust identifiers are Unicode.
    let f = parse("fn grüßen() {}", Lang::Rust).unwrap();
    assert!(f.defines.contains(&"grüßen".to_string()), "{:?}", f.defines);

    println!(
        "tier 2 ok: the Rust scanner holds on raw strings, lifetimes, macros \
         and Unicode names"
    );
}

fn demo_parse() {
    use parse_ast::{Lang, parse};

    let rust = r#"
        use std::collections::HashMap;
        struct Checkout { total: u32 }
        impl Checkout {
            fn pay(&self) -> bool { self.charge() && log_info("paid") }
            fn charge(&self) -> bool { payment::process(self.total) }
        }
    "#;
    let f = parse(rust, Lang::Rust).unwrap();
    assert!(!f.had_errors);
    assert!(f.defines.contains(&"pay".to_string()));
    assert!(f.defines.contains(&"Checkout".to_string()));
    // A call is attributed to the function that encloses it, not to the file.
    assert!(calls_contain(&f.calls, "pay", "charge"));
    assert!(calls_contain(&f.calls, "pay", "log_info"));
    // `payment::process(..)` resolves to the tail of the path.
    assert!(calls_contain(&f.calls, "charge", "process"));

    let py = "\
import os
from payment import charge

class Checkout:
    def pay(self):
        return charge(self.total)

def main():
    Checkout().pay()
";
    let f = parse(py, Lang::Python).unwrap();
    assert!(f.defines.contains(&"pay".to_string()));
    assert!(f.defines.contains(&"Checkout".to_string()));
    assert!(calls_contain(&f.calls, "pay", "charge"));
    // `Checkout().pay()` is a method call on a fresh instance.
    assert!(calls_contain(&f.calls, "main", "pay"));

    let js = "\
import { charge } from './payment.js';
class Checkout {
  pay() { return charge(this.total); }
}
function main() { new Checkout().pay(); }
";
    let f = parse(js, Lang::JavaScript).unwrap();
    assert!(f.defines.contains(&"pay".to_string()));
    assert!(calls_contain(&f.calls, "pay", "charge"));

    // The whole point of this tier: a file that does not compile still parses.
    let broken = r#"
        fn good() { helper(); }
        fn broken( { this is not rust
    "#;
    let f = parse(broken, Lang::Rust).unwrap();
    assert!(f.had_errors, "a broken file must be flagged");
    assert!(
        calls_contain(&f.calls, "good", "helper"),
        "the intact part must still yield facts"
    );

    // A call outside any definition belongs to the module.
    let top = parse("print('hi')", Lang::Python).unwrap();
    assert_eq!(
        top.calls,
        vec![("<module>".to_string(), "print".to_string(), false)]
    );

    assert_eq!(
        Lang::from_path(std::path::Path::new("a/b.py")),
        Some(Lang::Python)
    );
    assert_eq!(Lang::from_path(std::path::Path::new("a/b.txt")), None);

    // Every registered language needs a sample here: a grammar whose tags
    // query does not match yields empty facts silently, not an error.
    let cases: &[(Lang, &str, &str, &str)] = &[
        (
            Lang::TypeScript,
            "class Checkout { pay(): boolean { return charge(this.total); } }",
            "pay",
            "charge",
        ),
        (
            Lang::Go,
            "func Pay(c *Checkout) bool {\n\treturn Charge(c.Total)\n}",
            "Pay",
            "Charge",
        ),
        (
            Lang::Java,
            "class Checkout { boolean pay() { return charge(total); } }",
            "pay",
            "charge",
        ),
        (
            Lang::C,
            "int pay(struct Checkout *c) {\n  return charge(c->total);\n}",
            "pay",
            "charge",
        ),
        (
            Lang::CSharp,
            "class Checkout { bool Pay() { return Charge(total); } }",
            "Pay",
            "Charge",
        ),
        (
            Lang::Cpp,
            "bool pay(Checkout *c) { return charge(c->total); }",
            "pay",
            "charge",
        ),
        (
            Lang::Ruby,
            "class Checkout\n  def pay\n    charge(total)\n  end\nend",
            "pay",
            "charge",
        ),
        (
            Lang::Php,
            "<?php\nfunction pay($c) { return charge($c->total); }",
            "pay",
            "charge",
        ),
        (
            Lang::Swift,
            "func pay(_ c: Checkout) -> Bool { return charge(c.total) }",
            "pay",
            "charge",
        ),
        (
            Lang::Scala,
            "object Checkout {\n  def pay(c: Cart): Boolean = charge(c.total)\n}",
            "pay",
            "charge",
        ),
        (
            Lang::Kotlin,
            "fun pay(c: Checkout): Boolean { return charge(c.total) }",
            "pay",
            "charge",
        ),
        (
            Lang::Bash,
            "charge() {\n  echo billing\n}\npay() {\n  charge\n}\n",
            "pay",
            "charge",
        ),
        (
            Lang::Lua,
            "function pay(cart)\n  return charge(cart.total)\nend\nfunction charge(t) return t end\n",
            "pay",
            "charge",
        ),
        (
            Lang::Elixir,
            "defmodule Checkout do\n  def pay(c) do\n    charge(c.total)\n  end\n  def charge(t), do: t\nend\n",
            "pay",
            "charge",
        ),
        (
            Lang::Dart,
            "class Checkout {\n  bool pay(Cart c) { return charge(c.total); }\n  bool charge(int t) => t > 0;\n}\n",
            "pay",
            "charge",
        ),
        (
            Lang::Haskell,
            "pay :: Cart -> Int\npay c = charge (total c)\n\ncharge :: Int -> Int\ncharge t = t\n",
            "pay",
            "charge",
        ),
        (
            Lang::Zig,
            "pub fn pay(c: Cart) u32 {\n    return charge(c.total);\n}\nfn charge(t: u32) u32 { return t; }\n",
            "pay",
            "charge",
        ),
    ];
    // Every language must have a sample here. A grammar whose query does not
    // match yields empty facts rather than an error, so a language added
    // without one is silent — measured, a broken `extra_query` for Rust alone
    // took this tree from 1,279 nodes to 154 with no message. `Lang::count()`
    // was printed below and never compared, which is what let that be silent.
    // Rust, Python and JavaScript are checked above in more detail, so the
    // table carries the rest.
    // Twenty of the seventy-two carry a sample here, and those twenty are the
    // ones a question in this tree can reach. The rest are measured against
    // real code instead. That is the only way to tell "the scanner works" from
    // "the scanner returns nothing",
    // since a scanner that matches nothing yields empty facts rather than an
    // error. What this assert protects is that the twenty do not shrink.
    assert!(
        cases.len() + 3 >= 20,
        "the sampled languages must not shrink: {} covered of {} registered",
        cases.len() + 3,
        parse_ast::Lang::count()
    );
    for &(lang, src, def, call) in cases {
        let f = parse(src, lang).unwrap();
        assert!(
            f.defines.contains(&def.to_string()),
            "{lang:?} must define {def}, got {:?}",
            f.defines
        );
        assert!(
            calls_contain(&f.calls, def, call),
            "{lang:?} must attribute {call} to {def}, got {:?}",
            f.calls
        );
    }

    // A constant's documentation is where the reason for its value is written,
    // which is what a question about a tunable is phrased from — leaving them
    // untagged cost measurable recall when this was first found for Rust. Every
    // language added here needed its own node type, and three of the five
    // shipped no `@definition.constant` at all.
    let constants: &[(Lang, &str, &str)] = &[
        (Lang::Rust, "const MAX_RETRIES: u32 = 3;", "MAX_RETRIES"),
        (Lang::Go, "const MaxRetries = 3", "MaxRetries"),
        (Lang::Java, "class C { static final int MAX = 3; }", "MAX"),
        (Lang::C, "#define MAX_RETRIES 3", "MAX_RETRIES"),
        (
            Lang::CSharp,
            "class C { const int MaxRetries = 3; }",
            "MaxRetries",
        ),
        (Lang::Cpp, "const int kMaxRetries = 3;", "kMaxRetries"),
        (Lang::Ruby, "MAX_RETRIES = 3", "MAX_RETRIES"),
        (Lang::Php, "<?php\nconst MAX_RETRIES = 3;", "MAX_RETRIES"),
        (Lang::Swift, "let maxRetries = 3", "maxRetries"),
        (
            Lang::Scala,
            "object C {\n  val MaxRetries = 3\n}",
            "MaxRetries",
        ),
        (Lang::Kotlin, "const val MAX_RETRIES = 3", "MAX_RETRIES"),
        (Lang::Bash, "MAX_RETRIES=3", "MAX_RETRIES"),
        // Lua and Elixir ship no `@definition.constant` either, and each needed
        // a different node: Lua wraps a `local` assignment in a
        // `variable_declaration`, while Elixir's `@attr 3` parses as a unary
        // operator over the same `call` node an ordinary invocation produces —
        // matching the inner call alone would tag every function call in the
        // file. Both read off the parse tree, not guessed.
        (Lang::Lua, "local MAX_RETRIES = 3", "MAX_RETRIES"),
        (
            Lang::Elixir,
            "defmodule M do\n  @max_retries 3\nend",
            "max_retries",
        ),
        (Lang::Dart, "const int maxRetries = 3;", "maxRetries"),
        (
            Lang::Haskell,
            "maxRetries :: Int\nmaxRetries = 3",
            "maxRetries",
        ),
        (Lang::Zig, "const MAX_RETRIES: u32 = 3;", "MAX_RETRIES"),
        // JavaScript and TypeScript were absent from this table, and that is
        // how the gap below went unnoticed for both of them at once.
        (Lang::JavaScript, "const MAX_RETRIES = 3;", "MAX_RETRIES"),
        (Lang::TypeScript, "const MAX_RETRIES = 3;", "MAX_RETRIES"),
    ];
    for &(lang, src, name) in constants {
        let f = parse(src, lang).unwrap();
        assert!(
            f.defines.contains(&name.to_string()),
            "{lang:?} must tag the constant {name}, got {:?} — a language whose \
             query does not match yields empty facts, never an error",
            f.defines
        );
    }

    // TypeScript extends JavaScript rather than replacing it, so anything
    // appended to one and not the other silently halves the language that
    // misses it. Measured: 40 identical files parsed as `.ts` yielded 621
    // definitions against `.js`'s 1,921 while the constant pattern was
    // appended to JavaScript alone. The two must agree on identical input.
    let shared = "const LIMIT = 250;\nfunction f() { return g(LIMIT); }\n";
    let (js_facts, ts_facts) = (
        parse(shared, Lang::JavaScript).unwrap(),
        parse(shared, Lang::TypeScript).unwrap(),
    );
    assert_eq!(
        js_facts.defines, ts_facts.defines,
        "TypeScript must recognise what JavaScript does — it appends that query"
    );
    assert!(js_facts.defines.contains(&"LIMIT".to_string()));

    // A function-local `const` is not a definition anyone searches for. The
    // pattern is anchored at `program` for that reason: measured on 80 real
    // files, 2,608 of the 2,823 constants it matched were function-local, and
    // every one became a graph node.
    let local = parse(
        "function outer() { const step = 1; return helper(step); }",
        Lang::JavaScript,
    )
    .unwrap();
    assert!(
        !local.defines.contains(&"step".to_string()),
        "a function-local const must not become a definition, got {:?}",
        local.defines
    );

    // Swift's shipped query tags `property_declaration` twice already, so a
    // constant pattern of our own duplicated every one of them: measured, 80
    // real files went from 9,002 definitions to 6,427 once it was removed.
    let swift = parse(
        "let GLOBAL = 5\nfunc work() -> Int { return helper() }",
        Lang::Swift,
    )
    .unwrap();
    assert_eq!(
        swift.defines.iter().filter(|d| *d == "GLOBAL").count(),
        1,
        "Swift must tag a constant once, got {:?}",
        swift.defines
    );

    println!(
        "tier 2 ok: {} languages via their own tags queries, {} with tagged constants, \
         plus a file that does not compile",
        parse_ast::Lang::count(),
        constants.len()
    );
}

/// The live-sync path end to end: parse a file into the graph, edit it, and
/// re-ingest. This is what the delta store was built for, exercised for the
/// first time with a real parser instead of hand-made edges.
fn demo_ingest() {
    use ingest::{SymbolRegistry, ingest_file};
    use std::path::Path;
    use std::sync::Arc;

    let g = Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let mut arena = arena::SymbolArena::new();
    let mut reg = SymbolRegistry::new(0);
    let path = Path::new("src/checkout.py");

    let v1 = "\
def pay(self):
    charge(self.total)
    log(self.id)
";
    let n = ingest_file(&g, &mut arena, &mut reg, path, Path::new(""), v1, 1).unwrap();
    assert_eq!(n, 2);
    let pay = reg.get_or_mint("src/checkout.py#pay");
    let snap = g.load();
    assert_eq!(snap.neighbors(pay).count(), 2);
    assert!(
        snap.neighbors(pay)
            .all(|e| e.confidence == csr::Confidence::Inferred)
    );

    // Edit: the log call is gone, a new one appears. The file's old edges must
    // disappear rather than accumulate.
    let v2 = "\
def pay(self):
    charge(self.total)
    audit(self.id)
";
    ingest_file(&g, &mut arena, &mut reg, path, Path::new(""), v2, 2).unwrap();
    let snap = g.load();
    let targets: Vec<NodeIdName> = snap
        .neighbors(pay)
        .map(|e| e.target)
        .map(|t| {
            if t == reg.get_or_mint("charge") {
                "charge"
            } else if t == reg.get_or_mint("audit") {
                "audit"
            } else if t == reg.get_or_mint("log") {
                "log"
            } else {
                "?"
            }
        })
        .collect();
    assert_eq!(
        snap.neighbors(pay).count(),
        2,
        "stale edges must not accumulate"
    );
    assert!(targets.contains(&"charge") && targets.contains(&"audit"));
    assert!(!targets.contains(&"log"), "the removed call must be gone");
    // The re-parse carries the newer timestamp, which temporal decay needs.
    assert!(snap.neighbors(pay).all(|e| e.timestamp == 2));

    // A second file defining `charge` resolves onto the placeholder the first
    // file minted, so the call actually connects.
    let before = reg.get_or_mint("charge");
    ingest_file(
        &g,
        &mut arena,
        &mut reg,
        Path::new("src/payment.py"),
        Path::new(""),
        "def charge(amount):\n    return True\n",
        3,
    )
    .unwrap();
    assert_eq!(reg.get_or_mint("charge"), before);

    // Compaction must keep edges whose nodes exist only in the delta. The base
    // here started empty, so every node was minted at runtime — the case a
    // base-sized rebuild silently drops.
    let before: usize = (0..g.load().width() as csr::NodeId)
        .map(|n| g.load().neighbors(n).count())
        .sum();
    assert!(before > 0);
    let compacted = g.load().compact();
    let after: usize = (0..compacted.width() as csr::NodeId)
        .map(|n| compacted.neighbors(n).count())
        .sum();
    assert_eq!(after, before, "compaction dropped runtime-minted nodes");
    assert!(compacted.base.node_count() >= reg.len());

    // **A receiver-qualified call must not bind to the local definition of the
    // same name.** `Instant::now()` is not a call to this file's `now`, and
    // binding it built a false hub: measured, 205 of 531 same-file bindings on
    // this tree were receiver-qualified, 55 of 116 on epoch-engine and 86 of
    // 827 on office4u. `src/main.rs#now` collected 31 such incoming edges
    // against 16 real ones, and `src/main.rs#docs` 12 against zero.
    let g2 = Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let mut a2 = arena::SymbolArena::new();
    let mut r2 = SymbolRegistry::new(0);
    ingest_file(
        &g2,
        &mut a2,
        &mut r2,
        Path::new("src/t.rs"),
        Path::new(""),
        "fn now() -> u64 { 0 }\nfn f() { let a = Instant::now(); let b = now(); }\n",
        1,
    )
    .unwrap();
    let local_now = r2.get_or_mint("src/t.rs#now");
    let f = r2.get_or_mint("src/t.rs#f");
    let snap = g2.load();
    let to_local = snap.neighbors(f).filter(|e| e.target == local_now).count();
    assert_eq!(
        to_local, 1,
        "only the bare `now()` may bind locally; `Instant::now()` must not"
    );
    assert!(
        r2.contains("now"),
        "`Instant::now()` must land on the bare placeholder instead"
    );

    println!(
        "tier 2 wired: {} symbols, re-parse replaces edges in place",
        reg.len()
    );
}

type NodeIdName = &'static str;

/// Tier 3: what the heuristic links, and — more importantly — what it refuses
/// to link. A wrong edge is worse than a missing one here, since retrieval
/// traverses these paths.
fn demo_resolve() {
    use ingest::SymbolRegistry;

    let mut reg = SymbolRegistry::new(0);
    // A call to `charge` seen before payment.py was parsed.
    let placeholder = reg.get_or_mint("charge");
    let definition = reg.get_or_mint("src/payment.py#charge");
    // A name defined in two places is ambiguous.
    let ambiguous = reg.get_or_mint("run");
    reg.get_or_mint("src/a.py#run");
    reg.get_or_mint("src/b.py#run");
    // A placeholder reached from JavaScript must not land on a Rust definition
    // of the same name. `console.error` and `fn error` are the same string and
    // nothing in a bare name says otherwise — measured, that one pairing put an
    // edge from `assets/check_view.js` into `src/mcp.rs` and 18 of the 20
    // cycles `cycles` reported ran through it.
    let cross = reg.get_or_mint("error");
    reg.note_placeholder_lang(cross, parse_ast::Lang::JavaScript);
    reg.get_or_mint("src/mcp.rs#error");
    // A call into a foreign crate has no definition anywhere.
    let foreign = reg.get_or_mint("unwrap");
    // A test file must not capture a production name.
    let shadowed = reg.get_or_mint("setup");
    reg.get_or_mint("tests/test_db.py#setup");

    let links = resolve::resolve(&reg);
    let linked: Vec<_> = links.iter().map(|l| l.placeholder).collect();

    assert!(
        !linked.contains(&cross),
        "a JavaScript placeholder must not link to a Rust definition"
    );
    assert!(
        linked.contains(&placeholder),
        "an exact unique match must link"
    );
    assert_eq!(
        links
            .iter()
            .find(|l| l.placeholder == placeholder)
            .unwrap()
            .definition,
        definition
    );
    assert!(
        !linked.contains(&foreign),
        "a name with no definition must not link"
    );
    assert!(
        !linked.contains(&shadowed),
        "a test definition must not capture a production name"
    );
    // Ambiguity is reported, not silently resolved.
    let amb = links.iter().find(|l| l.placeholder == ambiguous);
    assert_eq!(amb.map(|l| l.candidates), Some(2));

    // Only unambiguous links become edges.
    let edges = resolve::link_edges(&links, 42);
    assert_eq!(
        edges.len(),
        1,
        "the ambiguous name must not produce an edge"
    );
    assert_eq!(edges[0].0, placeholder);
    assert_eq!(edges[0].1.target, definition);
    assert_eq!(edges[0].1.confidence, csr::Confidence::Ambiguous);
    assert_eq!(edges[0].1.edge_kind, resolve::RESOLVES_TO);
    // Authority is the *source* axis: tier 3 reads the same code tier 1 does,
    // so it weighs the same there and loses on `confidence` instead. Ranking it
    // down on both was double-counting — the provenance ordering already lives
    // in `Confidence`, and `expand` multiplies the two.
    assert_eq!(edges[0].1.authority, physics::SOURCE_CODE);
    assert!(
        physics::authority(edges[0].1.confidence) < physics::authority(csr::Confidence::Inferred),
        "tier 3 still ranks below tier 2, by provenance rather than by source"
    );

    // Resolution is deterministic despite the registry being a HashMap.
    let again = resolve::resolve(&reg);
    assert_eq!(links, again, "resolution must be stable across runs");

    // Links are recomputed after every batch, so re-applying them must replace
    // the previous set rather than pile another copy on top.
    let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let apply = |g: &std::sync::Arc<graph::Graph>| {
        g.update(|_, d| d.replace_edges_of_kind(resolve::RESOLVES_TO, edges.clone()))
    };
    apply(&g);
    let once = g.load().delta.len();
    apply(&g);
    apply(&g);
    assert_eq!(
        g.load().delta.len(),
        once,
        "tier 3 links must not accumulate"
    );

    println!("tier 3 ok: exact links only, ambiguity and test shadowing refused");
}

/// Tier 1: decode a SCIP index and resolve its occurrences. The fixture is
/// encoded here rather than checked in, so the test cannot drift from the
/// schema the importer decodes.
fn demo_scip() {
    use import_scip::symbol_name_for_test as name_of;

    // The symbol grammar is the part most likely to be wrong, so pin it.
    assert_eq!(
        name_of("rust-analyzer cargo glasir 0.1.0 csr/BaseCsr#"),
        Some("BaseCsr".into())
    );
    assert_eq!(
        name_of("rust-analyzer cargo glasir 0.1.0 csr/CsrBuilder#build()."),
        Some("build".into())
    );
    // **A descriptor chain ending in `/` is a namespace, not a symbol.**
    // Importing them minted nodes named after modules: `src/main.rs#crate`
    // collected 51 incoming edges, one per `crate::` prefix in the tree, and
    // the index carried 2,339 such occurrences over 82 module names. It cost
    // more than noise — `crate::auth::now()` in `audit.rs` became an edge into
    // `main.rs`, and `no-cycles` reported `audit.rs -> main.rs -> audit.rs` on
    // the strength of it. Dropping them is what closed the trade a compiler
    // index used to cost: `questions` 42% -> 58% with a fresh index, and
    // `nodes/question` back from 27 to the full budget of 40.
    assert_eq!(name_of("rust-analyzer cargo glasir 0.1.0 crate/"), None);
    assert_eq!(
        name_of("rust-analyzer cargo std https://github.com/rust-lang/rust/library/std io/"),
        None
    );
    // But a *type* in a namespace still resolves — the `/` is a separator
    // there, not the ending.
    assert_eq!(
        name_of("rust-analyzer cargo glasir 0.1.0 io/Reader#"),
        Some("Reader".into())
    );
    assert_eq!(
        name_of("scip-python python mypkg 1.0 payment/charge()."),
        Some("charge".into())
    );
    // File-local symbols are not graph nodes.
    assert_eq!(name_of("local 4"), None);

    // A two-document index: pay() in checkout.rs calls charge() in payment.rs,
    // and also calls something from another crate.
    let pay = "cargo glasir 0.1.0 checkout/pay().";
    let charge = "cargo glasir 0.1.0 payment/charge().";
    let foreign = "cargo std 1.0 vec/Vec#push().";
    // A local term declared inside pay() must not capture the calls that
    // follow it: only a callable can enclose a call.
    let local = "cargo glasir 0.1.0 checkout/pay().total.";
    let index = import_scip::encode_index_for_test(&[
        (
            "src/checkout.rs",
            &[
                (pay, true, 10),
                (local, true, 11),
                (charge, false, 12),
                (foreign, false, 13),
            ],
        ),
        ("src/payment.rs", &[(charge, true, 5)]),
    ]);

    // A real tree, because freshness is now what decides whether an edge is
    // served: the importer trusts a document only while the file it describes
    // is on disk and older than the index. Passing a root the files are not in
    // is exactly the foreign-index case, and it must yield nothing.
    let root = std::env::temp_dir().join(format!("glasir-scip-{}", fixture_id()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/checkout.rs"), "fn pay() {}").unwrap();
    std::fs::write(root.join("src/payment.rs"), "fn charge() {}").unwrap();
    let path = root.join("index.scip");
    std::fs::write(&path, &index).unwrap();
    let mut reg = ingest::SymbolRegistry::new(0);
    let r = import_scip::import(&path, &root, &mut reg).unwrap();

    // Same index, a root that does not hold those files: an index built for
    // another tree. Every document is stale, so no edge is served — before
    // this, a missing file read as "not newer than the index" and therefore
    // fresh, and 2,923 edges about a different repository entered the graph.
    let mut foreign_reg = ingest::SymbolRegistry::new(0);
    let foreign = import_scip::import(&path, &root.join("elsewhere"), &mut foreign_reg).unwrap();
    assert_eq!(
        foreign.stale_files.len(),
        2,
        "a file that is not in this tree cannot be fresh"
    );
    assert!(
        foreign.edges.is_empty(),
        "an index of another tree must contribute no edges"
    );

    // And the same index once a covered file has been edited: that file's
    // edges go, the untouched file's stay. All-or-nothing would mean either
    // trusting stale facts or discarding good ones.
    std::fs::write(root.join("src/checkout.rs"), "fn pay() { charge(); }").unwrap();
    // Push the mtime a clear second past the index rather than trusting the
    // clock: `stale_since` compares whole seconds, so a write landing inside
    // the same second as the index reads as fresh and this check flakes.
    let past_index = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t + std::time::Duration::from_secs(2))
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(root.join("src/checkout.rs"))
        .and_then(|f| f.set_modified(past_index))
        .unwrap();
    let mut edited_reg = ingest::SymbolRegistry::new(0);
    let edited = import_scip::import(&path, &root, &mut edited_reg).unwrap();
    assert_eq!(edited.stale_files, vec!["src/checkout.rs".to_string()]);
    assert!(
        edited.edges.is_empty(),
        "the edge is attributed to the edited file, so it must not be served"
    );

    std::fs::remove_dir_all(&root).unwrap();

    assert_eq!(r.definitions, 3);
    assert_eq!(r.references, 2);
    // The foreign symbol is reported, not guessed at.
    assert_eq!(
        r.external, 1,
        "a symbol defined elsewhere must not be linked"
    );
    assert_eq!(r.edges.len(), 1);

    // A reference resolves across documents: charge is defined in the *second*
    // document but referenced in the first. That is what tier 3 cannot do.
    let src = reg.get_or_mint("src/checkout.rs#pay");
    let dst = reg.get_or_mint("src/payment.rs#charge");
    assert_eq!(r.edges[0].0, src);
    assert_eq!(r.edges[0].1.target, dst);
    assert_eq!(r.edges[0].1.confidence, csr::Confidence::Extracted);
    assert_eq!(r.edges[0].1.authority, 1.0);
    assert_eq!(r.file_nodes["src/payment.rs"], vec![dst]);

    // Tier 1 and tier 2 must name the same definition identically, or the
    // cascade would produce two disconnected nodes for one function instead of
    // tier 1 superseding tier 2.
    let facts = parse_ast::parse("fn charge() {}", parse_ast::Lang::Rust).unwrap();
    assert_eq!(facts.defines, vec!["charge"]);
    let tier2_node = reg.get_or_mint("src/payment.rs#charge");
    assert_eq!(tier2_node, dst, "tier 1 and tier 2 must agree on the node");

    // Freshness is per file: an index must not be trusted about a file edited
    // since it was written, and must not be discarded for the others.
    let dir = std::env::temp_dir().join(format!("glasir-stale-{}", fixture_id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/old.rs"), "fn old() {}").unwrap();
    std::fs::write(dir.join("src/new.rs"), "fn new() {}").unwrap();
    let idx = dir.join("index.scip");
    std::fs::write(
        &idx,
        import_scip::encode_index_for_test(&[
            ("src/old.rs", &[("cargo x 1.0 old/old().", true, 0)]),
            ("src/new.rs", &[("cargo x 1.0 new/new().", true, 0)]),
        ]),
    )
    .unwrap();
    // Touch one source file so it is newer than the index.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.join("src/new.rs"), "fn new() { changed(); }").unwrap();

    let mut reg2 = ingest::SymbolRegistry::new(0);
    let r2 = import_scip::import(&idx, &dir, &mut reg2).unwrap();
    assert_eq!(
        r2.stale_files,
        vec!["src/new.rs".to_string()],
        "an edited file must be reported stale"
    );
    let fresh: Vec<&String> = r2.fresh_files().collect();
    assert_eq!(
        fresh,
        vec!["src/old.rs"],
        "an untouched file must stay covered — staleness is per file, not per index"
    );
    std::fs::remove_dir_all(&dir).unwrap();

    println!(
        "tier 1 ok: {} definitions, {} references, {} external, staleness per file",
        r.definitions, r.references, r.external
    );
}

/// Semantic physics: the ordering guarantees the retrieval layer depends on.
fn demo_physics() {
    use physics::{Physics, authority, expand, temporal_weight};

    // The decay curve must be exact at the half-life, or the parameter is a lie.
    let hl = 14.0 * 86_400.0;
    assert!((temporal_weight(hl, hl) - 0.5).abs() < 1e-6);
    assert!((temporal_weight(0.0, hl) - 1.0).abs() < 1e-6);
    assert!(temporal_weight(4.0 * hl, hl) < 0.07);
    // A future timestamp (clock skew) must not score above a current one.
    assert_eq!(temporal_weight(-999.0, hl), 1.0);

    // Authority ordering is the guarantee that a guess cannot outrank a fact.
    assert!(authority(csr::Confidence::Extracted) > authority(csr::Confidence::Inferred));
    assert!(authority(csr::Confidence::Inferred) > authority(csr::Confidence::Ambiguous));

    let now = 1_756_600_000u64; // a plausible wall-clock, so ages do not underflow
    let edge = |target, ts, conf| csr::Edge {
        target,
        timestamp: ts,
        authority: 1.0,
        edge_kind: 0,
        confidence: conf,
    };

    // seed -> fresh(1), stale(2), guessed(3); fresh -> deep(4).
    let mut b = csr::CsrBuilder::new();
    for _ in 0..5 {
        b.add_node(0);
    }
    b.add_edge(0, edge(1, now, csr::Confidence::Extracted));
    b.add_edge(
        0,
        edge(2, now - (hl as u64) * 4, csr::Confidence::Extracted),
    );
    b.add_edge(0, edge(3, now, csr::Confidence::Ambiguous));
    b.add_edge(1, edge(4, now, csr::Confidence::Extracted));
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let cfg = Physics::default();
    let out = expand(&snap, &[(0, 1.0)], now, &cfg, None);
    let rank: Vec<csr::NodeId> = out.iter().map(|s| s.node).collect();
    assert_eq!(rank[0], 0, "the seed ranks first");
    // Fresh and compiler-resolved beats both stale and guessed.
    let pos = |n| rank.iter().position(|&x| x == n).unwrap();
    assert!(pos(1) < pos(2), "fresh must outrank stale");
    assert!(pos(1) < pos(3), "extracted must outrank ambiguous");
    // Distance decay: a node two hops out scores below its one-hop parent.
    assert!(out[pos(4)].score < out[pos(1)].score);
    assert_eq!(out[pos(4)].hops, 2);

    // A foreign reference must rank below a local symbol at the same distance,
    // or a result is standard-library names instead of code worth reading —
    // there are simply more foreign names than local ones.
    let local_only = std::collections::HashSet::from([0u32, 1, 4]);
    let ranked = expand(&snap, &[(0, 1.0)], now, &cfg, Some(&local_only));
    let place = |n| ranked.iter().position(|s| s.node == n).unwrap();
    assert!(
        place(1) < place(3),
        "a defined symbol must outrank a foreign one at the same hop"
    );
    // The path through a foreign node still reaches what lies beyond it.
    assert!(
        ranked.iter().any(|s| s.node == 4),
        "a penalty must not sever paths running through a foreign node"
    );

    // The same rule holds for a seed. It did not: the penalty was applied only
    // to nodes the expansion reached, so a lexical hit on a foreign name
    // entered at full confidence and outranked every local symbol — a question
    // containing "split" seeded on `split_once` and `split_whitespace`.
    let mixed = expand(&snap, &[(3, 1.0), (1, 1.0)], now, &cfg, Some(&local_only));
    let score_of = |n| mixed.iter().find(|s| s.node == n).unwrap().score;
    // On score, not position: at equal scores the order is a tie the sort
    // breaks arbitrarily, so a positional check passes whether or not the
    // penalty is applied.
    assert!(
        score_of(1) > score_of(3),
        "a local seed must outrank a foreign one entering at the same confidence, got {} vs {}",
        score_of(1),
        score_of(3)
    );
    // And the discount is to its rank only: what lies beyond a foreign seed is
    // still reachable, exactly as for a foreign node reached mid-expansion.
    let from_foreign = expand(&snap, &[(0, 1.0)], now, &cfg, Some(&local_only));
    assert!(
        from_foreign.iter().any(|s| s.node == 4),
        "a foreign seed's outgoing path keeps its undiscounted weight"
    );

    // Scale by subtraction: raising the threshold must cut the weak branches.
    let strict = Physics {
        threshold: 0.4,
        ..cfg
    };
    let few = expand(&snap, &[(0, 1.0)], now, &strict, None);
    assert!(few.len() < out.len(), "a higher threshold must drop nodes");
    assert!(
        !few.iter().any(|s| s.node == 2),
        "the stale node must be cut first"
    );
    assert!(physics::reduction(few.len(), snap.width()) > 0.0);

    // The cap bounds the result even when everything scores above threshold.
    let capped = expand(
        &snap,
        &[(0, 1.0)],
        now,
        &Physics {
            max_nodes: 2,
            ..cfg
        },
        None,
    );
    assert_eq!(capped.len(), 2);

    // Determinism: same input, same order, every time.
    assert_eq!(expand(&snap, &[(0, 1.0)], now, &cfg, None), out);

    println!(
        "phase 4.2 ok: {} of {} nodes kept, {:.0}% cut",
        few.len(),
        snap.width(),
        physics::reduction(few.len(), snap.width())
    );
}

/// Embeddings: the property that matters is that structurally similar nodes
/// converge, and that the whole thing is bit-for-bit reproducible.
fn demo_embed() {
    // Two callers of the same pair of helpers, plus one unrelated node.
    // a and b share a neighbourhood; c shares nothing with them.
    let mut b_ = csr::CsrBuilder::new();
    for i in 0..6u32 {
        b_.add_node(i + 1);
    }
    let e = |target| csr::Edge {
        target,
        timestamp: 0,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    // a(0) -> helper1(2), helper2(3)
    b_.add_edge(0, e(2));
    b_.add_edge(0, e(3));
    // b(1) -> helper1(2), helper2(3)   same neighbourhood as a
    b_.add_edge(1, e(2));
    b_.add_edge(1, e(3));
    // c(4) -> other(5)                 unrelated
    b_.add_edge(4, e(5));
    let g = graph::Graph::new(b_.build());
    let snap = g.load();

    let emb = embed::embed(&snap, 4);
    assert_eq!(emb.node_count(), 6);
    assert_eq!(emb.get(0).unwrap().len(), embed::DIM);

    // Nodes with identical neighbourhoods must be more similar to each other
    // than to an unrelated node. This is the whole claim of the method.
    let ab = emb.similarity(0, 1);
    let ac = emb.similarity(0, 4);
    assert!(ab > ac, "shared neighbourhood must beat none: {ab} vs {ac}");
    // Floating-point reductions can differ slightly across supported CPU
    // architectures. This lower bound still proves that shared structure has
    // a substantial similarity signal while avoiding a platform-specific
    // assertion about its exact magnitude.
    assert!(
        ab > 0.65,
        "identical neighbourhoods should come out close: {ab}"
    );
    // But *not* the same vector. Each node keeps a share of its own signature
    // (`SELF_WEIGHT`), so two distinct functions that happen to call the same
    // things stay distinguishable — an embedding that cannot tell them apart is
    // what `nearest` had for months, reporting unrelated symbols at 1.00.
    assert!(
        ab < 0.999,
        "two distinct nodes must not collapse onto one vector: {ab}"
    );

    // Vectors are L2-normalised, so self-similarity is 1.
    assert!((emb.similarity(0, 0) - 1.0).abs() < 1e-4);
    // Similarity is symmetric.
    assert!((emb.similarity(0, 1) - emb.similarity(1, 0)).abs() < 1e-6);

    // Nearest neighbours are ordered and exclude the query node.
    let near = emb.nearest(0, 2);
    assert!(
        near.iter().any(|(node, _)| *node == 1),
        "a node with the same neighbourhood is among the nearest results"
    );
    assert!(near[0].1 >= near[1].1);
    assert!(!near.iter().any(|(n, _)| *n == 0));

    // Determinism: no sampling anywhere, so a rerun is bit-identical.
    let again = embed::embed(&snap, 4);
    assert_eq!(
        emb.vectors, again.vectors,
        "embeddings must be reproducible"
    );

    // Deeper propagation stays finite and normalised — no blow-up, no collapse.
    let deep = embed::embed(&snap, 8);
    assert!(deep.vectors.iter().all(|v| v.is_finite()));
    assert!((deep.similarity(0, 0) - 1.0).abs() < 1e-4);

    // An empty graph must not panic.
    let empty = graph::Graph::new(csr::CsrBuilder::new().build());
    assert_eq!(embed::embed(&empty.load(), 3).node_count(), 0);

    // **The shape every real graph has, which the fixture above does not.**
    // `add_node(i + 1)` gives each node its own interned key; nothing on the
    // live path does. Tier 2 mints ids through `SymbolRegistry` and writes no
    // key at all, and `compact` copies what the base holds — zero for every
    // node. Seeding the signature from that key gave 667 defined symbols
    // **four distinct vectors**, so `nearest` answered with five unrelated
    // names at exactly 1.00 for months. The check could not see it because its
    // fixture was the one graph where the key is populated.
    let mut flat = csr::CsrBuilder::new();
    for _ in 0..6u32 {
        flat.add_node(0); // no interned key, as the real path leaves it
    }
    flat.add_edge(0, e(2));
    flat.add_edge(0, e(3));
    flat.add_edge(1, e(2));
    flat.add_edge(1, e(3));
    flat.add_edge(4, e(5));
    let fg = graph::Graph::new(flat.build());
    let fsnap = fg.load();
    let femb = embed::embed(&fsnap, 4);
    assert!(
        femb.similarity(0, 4) < 0.9,
        "nodes must be distinguishable without an interned key: got {}",
        femb.similarity(0, 4)
    );

    // A node with exactly one target must not simply *become* that target.
    // The mean over one neighbour is that neighbour, so without the restart
    // term two functions that both call only `readFileSync` come out identical
    // and `nearest` names them as each other's closest match: 74 of 118
    // out-degree-1 nodes here, 444 of 600 on a real monorepo.
    assert!(
        femb.similarity(4, 5) < 0.999,
        "an out-degree-1 node must keep something of its own: got {}",
        femb.similarity(4, 5)
    );

    println!(
        "phase 4.1 ok: {}d embeddings, deterministic, sim(a,b)={ab:.3}",
        embed::DIM
    );
}

/// Communities: two dense clusters joined only by a logger everything calls.
/// Without hub exclusion that logger merges them into one community, which is
/// the exact failure this parameter prevents.
fn demo_community() {
    let mut b = csr::CsrBuilder::new();
    // 0..3 checkout cluster, 4..7 payment cluster, 8 the logger.
    for _ in 0..9u32 {
        b.add_node(0);
    }
    let e = |t| csr::Edge {
        target: t,
        timestamp: 0,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    // Dense inside each cluster.
    for (a, c) in [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)] {
        b.add_edge(a, e(c));
    }
    for (a, c) in [(4, 5), (5, 6), (6, 7), (7, 4), (4, 6)] {
        b.add_edge(a, e(c));
    }
    // Everything calls the logger, and only that links the clusters.
    for n in 0..8u32 {
        b.add_edge(n, e(8));
    }
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let c = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::default(),
    );
    assert!(
        c.hubs.contains(&8),
        "the logger must be excluded as a hub, got {:?}",
        c.hubs
    );
    // The two clusters must land in different communities.
    assert_eq!(c.of_node[0], c.of_node[1], "checkout must stay together");
    assert_eq!(c.of_node[4], c.of_node[5], "payment must stay together");
    assert_ne!(
        c.of_node[0], c.of_node[4],
        "a hub must not merge unrelated modules"
    );
    assert!(c.members(c.of_node[0]).contains(&1));

    // **The same graph, but every node in one file: nothing is a hub.**
    //
    // Cohesion cannot tell a shared logger from a long function — both have
    // neighbours that do not know each other. What separates them is whether
    // those neighbours are spread across *modules*, and measured before this
    // test existed, half of what was excluded joined nothing at all: 11 of 18
    // on this tree, 211 of 211 on a real monorepo. `run_install`, `now`,
    // `walk` are not hubs, they are merely popular.
    let names_one_file: Vec<(String, csr::NodeId)> =
        (0..9u32).map(|n| (format!("a.rs#f{n}"), n)).collect();
    let one = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            None,
            9,
            names_one_file.iter().map(|(s, n)| (s.as_str(), *n)),
        ),
    );
    assert!(
        one.hubs.is_empty(),
        "a node whose neighbours all live in its own file joins nothing: {:?}",
        one.hubs
    );
    // And with the neighbours spread over two files it is a hub again, so the
    // test keys on the spread and not on having file information at all.
    let names_two_files: Vec<(String, csr::NodeId)> = (0..9u32)
        .map(|n| {
            let file = if n < 4 { "a.rs" } else { "b.rs" };
            (format!("{file}#f{n}"), n)
        })
        .collect();
    let two = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            None,
            9,
            names_two_files.iter().map(|(s, n)| (s.as_str(), *n)),
        ),
    );
    assert!(
        two.hubs.contains(&8),
        "a node reached from two files is a hub again, got {:?}",
        two.hubs
    );

    // Community ids are dense, so they are usable as indices.
    assert!(c.of_node.iter().all(|&x| (x as usize) < c.of_node.len()));

    // Deterministic across runs despite the HashMaps inside.
    let again = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::default(),
    );
    assert_eq!(c.of_node, again.of_node, "partition must be stable");

    // A graph with no edges must not panic and gives every node its own id.
    let mut lone = csr::CsrBuilder::new();
    lone.add_node(0);
    lone.add_node(0);
    let lg = graph::Graph::new(lone.build());
    assert_eq!(
        community::detect(
            &lg.load(),
            &community::Params::default(),
            &community::Context::default()
        )
        .count(),
        2
    );

    // Every community must be internally connected. This is the guarantee
    // Leiden's refinement phase adds over Louvain, and the reason to have it:
    // a community whose members cannot reach each other is not a subsystem.
    for cid in 0..c.count() as u32 {
        let members = c.members(cid);
        assert!(
            community::is_connected(&snap, &members),
            "community {cid} is internally disconnected: {members:?}"
        );
    }

    // Aggregation must find structure a single pass of local movement cannot:
    // four triangles chained into two pairs. Without a second level the best
    // Louvain can do is the triangles themselves.
    let mut chain = csr::CsrBuilder::new();
    for _ in 0..12u32 {
        chain.add_node(0);
    }
    let ce = |t| csr::Edge {
        target: t,
        timestamp: 0,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    // Triangles (0,1,2) (3,4,5) (6,7,8) (9,10,11), densely tied in pairs.
    for base in [0u32, 3, 6, 9] {
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            chain.add_edge(base + a, ce(base + b));
        }
    }
    for (a, b) in [(0, 3), (1, 4), (2, 5), (6, 9), (7, 10), (8, 11)] {
        chain.add_edge(a, ce(b));
        chain.add_edge(b, ce(a));
    }
    let cg = graph::Graph::new(chain.build());
    let csnap = cg.load();
    let cc = community::detect(
        &csnap,
        &community::Params::default(),
        &community::Context::default(),
    );
    let largest = (0..cc.count() as u32)
        .map(|x| cc.members(x).len())
        .max()
        .unwrap_or(0);
    assert!(
        largest >= 6,
        "aggregation must merge tied triangles, largest was {largest}"
    );
    for cid in 0..cc.count() as u32 {
        assert!(
            community::is_connected(&csnap, &cc.members(cid)),
            "refined community {cid} must stay connected"
        );
    }

    // The undefined filter keys on being undefined, and on nothing else. A
    // graph with two real clusters, a foreign leaf on each, and one foreign
    // name both clusters call — which is the shape that matters: `new` has
    // degree 334 across 25 files on this tree, and keeping it is what welds
    // unrelated files into one lump.
    let mut mix = csr::CsrBuilder::new();
    for _ in 0..10u32 {
        mix.add_node(0);
    }
    // 0,1,2 a cluster; 3,4,5 another; 6 a type nobody calls out of; 7,8
    // foreign leaves; 9 a foreign name *both* clusters reach.
    for (a, b) in [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)] {
        mix.add_edge(a, ce(b));
        mix.add_edge(b, ce(a));
    }
    mix.add_edge(0, ce(6)); // a type: referenced, calls nothing
    mix.add_edge(0, ce(7)); // foreign, weakly attached
    mix.add_edge(3, ce(8));
    for n in [0, 1, 2, 3, 4, 5] {
        mix.add_edge(n, ce(9)); // the shared foreign name
    }
    let mg = graph::Graph::new(mix.build());
    let msnap = mg.load();
    let defined: std::collections::HashSet<csr::NodeId> = (0..7).collect();
    let mc = community::detect(
        &msnap,
        &community::Params::default(),
        &community::Context {
            defined: Some(&defined),
            file_of: Vec::new(),
        },
    );
    assert!(
        mc.pendants.contains(&7) && mc.pendants.contains(&8),
        "undefined nodes must be detached, got {:?}",
        mc.pendants
    );
    // The one the old rule kept, on the reasoning that two callers of
    // `HashMap::new` are related. Measured over three trees, it is the reverse:
    // purity 36% -> 89% here and 32% -> 100% on a real monorepo once these go.
    assert!(
        mc.pendants.contains(&9),
        "a heavily shared foreign name must be detached too, not kept as evidence"
    );
    assert!(
        !mc.pendants.contains(&6),
        "a type definition makes no calls and must survive the filter"
    );
    assert_eq!(mc.of_node[0], mc.of_node[1], "clusters must stay intact");
    assert_ne!(
        mc.of_node[0], mc.of_node[3],
        "and the shared foreign name must not merge them"
    );

    // Refinement, directly: hand a partition that is internally disconnected
    // and check it gets split. Local movement alone can produce exactly this —
    // a node follows its neighbours out and leaves two halves behind that only
    // shared the node that left.
    let mut split = csr::CsrBuilder::new();
    for _ in 0..6u32 {
        split.add_node(0);
    }
    // Two triangles with no edge between them at all.
    for (a, b) in [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)] {
        split.add_edge(a, ce(b));
        split.add_edge(b, ce(a));
    }
    let sg = graph::Graph::new(split.build());
    let ssnap = sg.load();
    let refined = community::refine_for_test(&ssnap, &[0, 0, 0, 0, 0, 0]);
    assert_ne!(
        refined[0], refined[3],
        "refinement must split a community whose halves are unconnected"
    );
    assert_eq!(
        refined[0], refined[1],
        "a connected half must stay together"
    );
    assert_eq!(refined[3], refined[4]);

    println!(
        "phase 4.3 ok: {} communities, {} hub(s) excluded",
        c.count(),
        c.hubs.len()
    );
}

/// MCP wire format: the shapes a client actually parses. Getting one field name
/// wrong breaks the connection with no useful error, so they are pinned here.
fn demo_mcp() {
    use serde_json::json;

    let mut b = csr::CsrBuilder::new();
    for _ in 0..6u32 {
        b.add_node(0);
    }
    let e = |t| csr::Edge {
        target: t,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    b.add_edge(0, e(1));
    b.add_edge(1, e(2));
    // The placeholder resolves onto the definition, exactly as tier 3 links it.
    b.add_edge(4, e(2));
    // A call written as the bare name, reaching `log` through the placeholder.
    b.add_edge(5, e(4));
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let mut reg = ingest::SymbolRegistry::new(0);
    reg.insert("src/a.rs#pay".into(), 0);
    reg.insert("src/b.rs#charge".into(), 1);
    reg.insert("src/c.rs#log".into(), 2);
    // A Markdown heading carrying a `#` of its own — how issues and pull
    // requests are written, and 14 of 1,869 headings across 35 real
    // repositories have one. The name is everything after the *first* `#`, so
    // this symbol is called `Fixes for #charge`, not `charge`.
    reg.insert("notes.md#Fixes for #charge".into(), 3);
    // The bare placeholder tier 2 mints for a callee it cannot see a
    // definition for. It stands for *every* `log` in the tree, so a query for
    // `log` has two defensible answers and the caller must be told, not handed
    // whichever the registry found first.
    reg.insert("log".into(), 4);
    reg.insert("src/d.rs#audit".into(), 5);
    let defined = std::collections::HashSet::from([0, 1, 2, 3, 5]);
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let emb = embed::embed(&snap, 3);
    let search = search::SearchIndex::build_with_docs(&reg, reg.docs());
    let names = mcp::name_table(&reg, snap.width());
    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        search: &search,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        physics: physics::Physics::default(),
        now: 1_756_600_000,
        files: None,
        mentions: None,
        references: None,
        root: None,
    };
    let call = |m: serde_json::Value| mcp::handle_for_test(&served, &m);

    // Legacy handshake: the client echoes back its own version when we support it.
    let init = call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "1"}}
    }))
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert_eq!(init["result"]["serverInfo"]["name"], "glasir");

    // Structured content: every tool answers as data as well as prose, and the
    // two are the same facts because the text is rendered from the value. What
    // this pins is the property that made returning both worth doing — a tool
    // gaining a field in one half and not the other. Broken on purpose by
    // having `render` ignore a field: the count assert below fails.
    for (tool, args) in [
        ("overview", json!({})),
        ("query_graph", json!({"query": "pay"})),
        ("explain_node", json!({"symbol": "src/a.rs#pay"})),
        ("impact", json!({"symbol": "src/c.rs#log"})),
        ("cycles", json!({})),
        (
            "shortest_path",
            json!({"from": "src/a.rs#pay", "to": "src/c.rs#log"}),
        ),
        ("find_callers", json!({"symbol": "src/c.rs#log"})),
    ] {
        let r = call(json!({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        }))
        .unwrap();
        let result = &r["result"];
        assert_eq!(result["isError"], false, "{tool} failed on the fixture");
        let sc = &result["structuredContent"];
        assert!(sc.is_object(), "{tool} returned no structuredContent");
        // The prose is still there: a client that cannot read the structured
        // half must not be handed an empty answer.
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(!text.is_empty(), "{tool} returned no text");

        // Every symbol the JSON names appears in the prose. This is the drift
        // test: a field added to one half and not the other breaks it.
        let mut named = 0;
        let mut stack = vec![sc.clone()];
        while let Some(v) = stack.pop() {
            match v {
                serde_json::Value::Object(m) => {
                    for (k, val) in m {
                        if k == "symbol" || k == "name" {
                            if let Some(sym) = val.as_str() {
                                assert!(
                                    text.contains(sym),
                                    "{tool}: {sym} is in the JSON and not in the text"
                                );
                                named += 1;
                            }
                        } else {
                            stack.push(val);
                        }
                    }
                }
                serde_json::Value::Array(a) => stack.extend(a),
                _ => {}
            }
        }
        // `cycles` names files rather than symbols, and finds none on this
        // fixture — an empty list is its correct answer, so it is the one tool
        // with nothing to cross-check here.
        assert!(
            named > 0 || tool == "cycles",
            "{tool}: nothing in the structured answer was named"
        );
    }

    // A caller may widen the answer, but not without bound. `max_nodes` and
    // `max_hops` arrive in the request, so an authenticated client could ask
    // for a hundred million of either — measured on a 300,000-line tree that
    // costs nothing today, because `physics::expand` cuts a branch below its
    // scoring threshold and the answer is the same 6 KB either way. The
    // ceiling is here so the promise does not depend on a constant that exists
    // for a different reason: the next tuning of that threshold must not
    // quietly become a denial-of-service surface.
    let huge = call(json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "query_graph", "arguments": {
            "query": "pay", "max_nodes": 100_000_000u64, "max_hops": 1_000u64
        }}
    }))
    .unwrap();
    let returned = huge["result"]["structuredContent"]["nodes"]
        .as_array()
        .map_or(0, Vec::len);
    assert!(
        returned <= mcp::MAX_NODES_CEILING,
        "a caller asked for 100M nodes and got {returned}, above the ceiling of {}",
        mcp::MAX_NODES_CEILING
    );
    // **This assertion cannot fail on this fixture and is kept knowingly.**
    // Twelve symbols cannot reach four hundred, so removing the `.min()`
    // changes nothing here — verified. What it pins is the *shape*: the
    // request is accepted rather than refused, and the ceiling is a public
    // constant a later reader can find. Catching the regression would need a
    // fixture large enough to exceed 400 nodes, which would cost more analysis
    // time in every self-check run than the risk warrants.

    // A refusal carries no structured content — the spec ties it to a result
    // that conforms to the output schema, and an error message does not.
    let refused = call(json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "impact", "arguments": {"symbol": "log"}}
    }))
    .unwrap();
    assert_eq!(
        refused["result"]["isError"], true,
        "a bare name is ambiguous"
    );
    assert!(
        refused["result"].get("structuredContent").is_none(),
        "an error result must not claim to satisfy the output schema"
    );

    // Every tool advertises the schema its structured answer conforms to, or a
    // client has no way to know the shape it is being handed.
    let listed = call(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 13);
    for t in tools {
        assert!(
            t["outputSchema"]["type"] == "object",
            "{} has no outputSchema",
            t["name"]
        );
        assert!(
            t["annotations"]["readOnlyHint"] == true && t["annotations"]["openWorldHint"] == false,
            "{} must declare itself read-only and closed-world",
            t["name"]
        );
        assert!(
            t["outputSchema"]["properties"].is_object(),
            "{} declares a schema with no properties",
            t["name"]
        );
    }

    // `find_callers` answers the direct question `impact` over-answers, so what
    // has to hold is that it stops at hop 1. The fixture chain is
    // `pay -> charge -> log`, so `charge` is a caller of `log` and `pay` is
    // not — `impact` reports both, and reporting `pay` here would mean the
    // tool is just `impact` with a worse layout. Asserting on a symbol the
    // fixture does not contain would pass whatever the code did, which is how
    // a first version of this check passed a deliberate two-hop sabotage.
    let direct = call(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "find_callers", "arguments": {"symbol": "src/c.rs#log"}}
    }))
    .unwrap();
    let callers = direct["result"]["structuredContent"]["callers"]
        .as_array()
        .unwrap();
    let named: Vec<&str> = callers
        .iter()
        .filter_map(|c| c["symbol"].as_str())
        .collect();
    assert!(
        named.contains(&"src/b.rs#charge"),
        "charge calls log: {named:?}"
    );
    assert!(
        !named.contains(&"src/a.rs#pay"),
        "find_callers must report direct callers only, and pay is two hops out: {named:?}"
    );
    assert!(
        named.contains(&"src/d.rs#audit") && !named.contains(&"log"),
        "a caller through the placeholder, not the placeholder: {named:?}"
    );

    // Ambiguity is refused here exactly as in `impact` and `explain_node`: a
    // bare name that also names a definition stands for every one of them, and
    // answering about whichever sorted first is the plausibly-wrong answer this
    // whole class of tool must not give.
    let ambiguous = call(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "find_callers", "arguments": {"symbol": "log"}}
    }))
    .unwrap();
    assert_eq!(
        ambiguous["result"]["isError"], true,
        "find_callers must refuse a bare name that is also a definition"
    );

    // `detect_changes` reads the working tree, and this fixture has none. It
    // must say so rather than shelling out against an unknown directory — the
    // one tool here whose answer depends on something outside the graph.
    let no_tree = call(json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "detect_changes", "arguments": {}}
    }))
    .unwrap();
    assert_eq!(
        no_tree["result"]["isError"], true,
        "detect_changes must refuse without a tree on disk"
    );

    // An older client must get a version it named, not ours.
    let old = call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05"}
    }))
    .unwrap();
    assert_eq!(old["result"]["protocolVersion"], "2024-11-05");

    // Modern era: discovery instead of a handshake.
    let disc = call(json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover"})).unwrap();
    assert_eq!(disc["result"]["resultType"], "complete");
    assert!(
        disc["result"]["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28"))
    );
    assert_eq!(
        disc["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "glasir"
    );

    // A notification carries no id and must never be answered.
    assert!(call(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());

    // Tool list: every tool needs a name and an object inputSchema.
    let tools = call(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    let list = tools["result"]["tools"].as_array().unwrap();
    // The count is asserted so adding a tool is a deliberate act: a client's
    // whole picture of this server is this list.
    assert_eq!(list.len(), 13, "query_graph, overview and the other eleven");
    for t in list {
        assert!(t["name"].is_string());
        assert_eq!(t["inputSchema"]["type"], "object");
        // `required` only where something is: `detect_changes` takes a
        // revision and a depth, both optional, and an empty `required` array
        // would say the same thing more loudly.
        if let Some(req) = t["inputSchema"].get("required") {
            assert!(req.is_array(), "{}: required must be an array", t["name"]);
        }
    }

    let tool_call = |name: &str, args: serde_json::Value| {
        call(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": name, "arguments": args}}))
        .unwrap()
    };

    let q = tool_call("query_graph", json!({"query": "pay"}));
    assert_eq!(q["result"]["isError"], false);
    assert_eq!(q["result"]["content"][0]["type"], "text");
    let text = q["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("src/a.rs#pay"), "seed must appear: {text}");
    assert!(text.contains("src/b.rs#charge"), "neighbour must appear");

    let p = tool_call(
        "shortest_path",
        json!({"from": "pay", "to": "src/c.rs#log"}),
    );
    let text = p["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("2 hops"), "expected 2 hops, got: {text}");
    assert!(text.contains("extracted"), "edges must carry provenance");

    // Vocabulary mismatch is the ceiling on lexical retrieval, and this is the
    // answer to it: a question asked in the wrong words gets the right ones
    // back. The client is a language model that knows "subsystem" and
    // "community" are one idea — it just has no way to know which this tree
    // uses, so the result says.
    // Asked with a word this graph does not use: the answer has to come back
    // in the tree's own terms rather than as a bare miss.
    let v = tool_call("query_graph", json!({"query": "charge settlement"}));
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("this code's words for it:"),
        "a result must carry the vocabulary it is written in: {text}"
    );
    // The words come from the result, and exclude what was already asked.
    let line = text
        .lines()
        .find(|l| l.starts_with("this code's words for it:"))
        .unwrap();
    assert!(
        !line.contains("settlement") && !line.contains("charge"),
        "repeating the question's own words says nothing: {line}"
    );
    // What is left is what the query did not name but the graph reached — the
    // whole point: the caller learns a term it had no way to guess.
    assert!(
        line.contains("log"),
        "suggestions must name what was found but not asked for: {line}"
    );

    // **A question reaching one of its own words has failed, and the words of
    // what it incidentally reached are the worst thing to offer.** The
    // empty-seed path already answers with the tree's subsystems; a *partial*
    // miss never reached it, so the suggestion was drawn from whatever the one
    // matching term dragged in.
    //
    // Measured on a German question against an English tree: the result was
    // two sections of a game-design document and the words offered were
    // `gassen`, `cinematic`, `runenbrück` — further from the code than the
    // question began. Coverage is what separates it from a working question,
    // not the score: the miss reaches 1 of 2 terms while questions that work
    // reach 2 of 4, 3 of 5, or all.
    //
    // `settle` alone matches nothing here, so pairing it with a word the
    // fixture does use makes exactly one term land.
    let v = tool_call("query_graph", json!({"query": "settlement charge"}));
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    if let Some(line) = text
        .lines()
        .find(|l| l.starts_with("this code's words for it:"))
    {
        assert!(
            !line.contains("settlement"),
            "a half-missed query must not be answered with its own miss: {line}"
        );
    }

    let x = tool_call("explain_node", json!({"symbol": "charge"}));
    let text = x["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("called by"), "callers need a reverse scan");
    assert!(text.contains("src/a.rs#pay"), "pay calls charge: {text}");

    // Impact walks backwards: log is called by charge, which pay calls, so
    // changing log reaches both — and each hop is reported separately, since
    // distance is what tells a certain break from a possible one.
    // **A bare name that also names a definition is refused, not silently
    // resolved to the placeholder.** `registry.node_of` found the placeholder
    // first and returned it, so `impact compact` answered about every
    // `compact` in the tree (22 dependents) while `impact src/graph.rs#compact`
    // answered about one (10) — and nothing said which question was answered.
    // 37% of this tree's placeholders also name a definition (38% in office4u,
    // 16% in epoch-engine), so the case is common.
    let shadowed = tool_call("impact", json!({"symbol": "log"}));
    assert_eq!(
        shadowed["result"]["isError"], true,
        "a placeholder must not silently answer for the definition: {}",
        shadowed["result"]["content"][0]["text"]
    );
    let text = shadowed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("matches 2 symbols") && text.contains("src/c.rs#log"),
        "{text}"
    );

    // The other two tools share the contract, and each was found taking
    // `seeds().first()` in its own session. `shortest_path` is the worse case:
    // it also reports a *distance*, and a placeholder routes straight into its
    // own definition, so entering through one counted a hop that is not
    // structure — measured, 165 of this tree's 190 such placeholders do
    // exactly that.
    for (tool, args) in [
        (
            "shortest_path",
            json!({"from": "log", "to": "src/a.rs#alpha"}),
        ),
        ("explain_node", json!({"symbol": "log"})),
    ] {
        let r = tool_call(tool, args);
        assert_eq!(
            r["result"]["isError"], true,
            "{tool} must refuse an ambiguous bare name too: {}",
            r["result"]["content"][0]["text"]
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("matches 2 symbols"), "{tool}: {text}");
    }

    let i = tool_call("impact", json!({"symbol": "src/c.rs#log"}));
    let text = i["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("3 symbols depend on this"),
        "transitive, not just direct: {text}"
    );
    // Two at one hop: `charge`, and `audit` through the placeholder, which is
    // a name and not a caller — counting it put `audit` a hop too far out.
    assert!(
        text.contains("1 hop (2)") && text.contains("2 hop (1)"),
        "{text}"
    );
    let (first, second) = text.split_once("\n2 hop (").unwrap();
    assert!(
        first.contains("src/b.rs#charge") && first.contains("src/d.rs#audit"),
        "direct callers first: {text}"
    );
    assert!(
        !first.contains("]  log\n"),
        "the placeholder is not a dependent: {text}"
    );
    assert!(
        second.contains("src/a.rs#pay"),
        "indirect caller second: {text}"
    );
    assert!(
        text.contains("[extracted]"),
        "provenance per dependent: {text}"
    );

    // Depth is a real bound, not decoration.
    let shallow = tool_call("impact", json!({"symbol": "src/c.rs#log", "depth": 1}));
    let text = shallow["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("2 symbols depend"),
        "depth 1 stops at charge and audit: {text}"
    );

    // Pages keep the nearest-first order and add up to the whole answer.
    let symbols = |v: &serde_json::Value| -> Vec<String> {
        v["result"]["structuredContent"]["hops"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|h| h["symbols"].as_array().unwrap().clone())
            .map(|x| x["symbol"].as_str().unwrap().to_string())
            .collect()
    };
    let one = tool_call("impact", json!({"symbol": "src/c.rs#log", "limit": 2}));
    let two = tool_call(
        "impact",
        json!({"symbol": "src/c.rs#log", "limit": 2, "offset": 2}),
    );
    assert_eq!(one["result"]["structuredContent"]["next_offset"], 2);
    assert!(
        two["result"]["structuredContent"]
            .get("next_offset")
            .is_none()
    );
    let mut paged = symbols(&one);
    paged.extend(symbols(&two));
    assert_eq!(
        paged,
        symbols(&i),
        "pages must add up to the unpaged answer"
    );
    let text = one["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("3 symbols depend on this, within 2 hops") && text.contains("offset: 2"),
        "a page still states the whole count and how to continue: {text}"
    );

    // A leaf nobody calls must say so — and say it differently from a name
    // that is not in the graph at all. Answering "nothing depends on this" to
    // a typo is the one failure this tool must not have.
    let none = tool_call("impact", json!({"symbol": "pay"}));
    let text = none["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(none["result"]["isError"], false);
    assert!(text.contains("breaks no caller"), "{text}");
    let typo = tool_call("impact", json!({"symbol": "chage"}));
    assert_eq!(
        typo["result"]["isError"], true,
        "a typo must not read as zero impact"
    );
    let text = typo["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("unknown symbol"), "{text}");

    // An ambiguous name is refused with its candidates rather than resolved to
    // whichever happened to sort first.
    let ambiguous = tool_call("impact", json!({"symbol": "src/"}));
    assert_eq!(ambiguous["result"]["isError"], true);
    let text = ambiguous["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("matches 4 symbols"), "{text}");

    // The name is what follows the *first* `#`, not the last. Splitting at the
    // last one made `notes.md#Fixes #12: charge` an exact match for `charge`,
    // so `impact` refused a question it could answer and named a paragraph of
    // prose as the rival candidate.
    let one = tool_call("impact", json!({"symbol": "charge"}));
    assert_eq!(
        one["result"]["isError"], false,
        "a heading's trailing word must not shadow a function: {}",
        one["result"]["content"][0]["text"]
    );

    // a -> b -> c runs one way, so there is nothing to report yet.
    let c = tool_call("cycles", json!({}));
    let text = c["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("no cycles"), "a->b->c is not a cycle: {text}");

    // The file graph is cached per served state, and the floor is a per-call
    // argument — so what is cached must be the pairs *with* their strongest
    // confidence, not a set already filtered by one caller's floor. Asking at
    // two floors in a row is what catches that: a cache built for the first
    // answer would hand the same edges to the second.
    let strict = tool_call("cycles", json!({"min_confidence": "extracted"}));
    let loose = tool_call("cycles", json!({"min_confidence": "ambiguous"}));
    for r in [&strict, &loose] {
        assert_eq!(r["result"]["isError"], false);
    }
    assert_eq!(
        strict["result"]["structuredContent"]["min_confidence"], "extracted",
        "a cached graph must not answer with the previous call's floor"
    );
    assert_eq!(
        loose["result"]["structuredContent"]["min_confidence"],
        "ambiguous"
    );
    // And asking twice gives the same answer: a cache whose second reply
    // differs from its first is worse than no cache.
    let again = tool_call("cycles", json!({"min_confidence": "ambiguous"}));
    assert_eq!(
        again["result"]["structuredContent"], loose["result"]["structuredContent"],
        "the cached file graph must be stable across calls"
    );

    // A bad argument is a tool error the model can see and fix, not a protocol
    // error that kills the call.
    let bad = tool_call("query_graph", json!({"query": "nope_does_not_exist"}));
    assert_eq!(bad["result"]["isError"], true);
    // Even a total miss orients the caller: a bare "no match" leaves a model
    // with nowhere to go but reading files, and a question in the wrong
    // vocabulary fails here rather than returning something weak.
    let miss = bad["result"]["content"][0]["text"].as_str().unwrap();
    assert!(miss.contains("no symbol matches"), "{miss}");
    assert!(
        bad["error"].is_null(),
        "input errors must not be protocol errors"
    );

    // An unknown method is a protocol error.
    let unknown = call(json!({"jsonrpc": "2.0", "id": 9, "method": "nonsense"})).unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    println!("phase 5 ok: dual-era handshake, 7 tools, wire shapes pinned");
}

/// install/uninstall touch the user's editor configuration, so the property
/// that matters is that a round trip leaves other people's settings exactly as
/// they were.
/// A call through an import reaches the definition the import names, in each
/// language whose imports are read — and in Erlang, where the module is the
/// file: every name below is defined twice, so without that the call is
/// ambiguous and links nowhere.
fn demo_imports() {
    let dir = std::env::temp_dir().join(format!("glasir-imports-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (file, text) in [
        ("pkg/__init__.py", ""),
        ("pkg/a.py", "def helper():\n    return 1\n"),
        ("pkg/b.py", "def helper():\n    return 2\n"),
        (
            "app.py",
            "from pkg import a as mod\nfrom pkg.b import (\n    helper as h,\n)\n\ndef run():\n    mod.helper()\n    h()\n",
        ),
        ("go.mod", "module example.com/m\n"),
        ("util/u.go", "package util\n\nfunc Do() {}\n"),
        ("other/o.go", "package other\n\nfunc Do() {}\n"),
        (
            "main.go",
            "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/m/util\"\n)\n\nfunc main() {\n\tutil.Do()\n\tfmt.Println()\n}\n",
        ),
        ("lib/x.ts", "export function run() {}\n"),
        ("lib/y.ts", "export function run() {}\n"),
        (
            "app.ts",
            "import * as x from './lib/x';\nimport {\n  run as go,\n} from './lib/y';\n\nexport function start() {\n  x.run();\n  go();\n}\n",
        ),
        // Erlang names no import: a module is its file, wherever it lives.
        // `twin` is two files of one name, so a call into it stays refused.
        ("erl/lists.erl", "-module(lists).\nmap(F, L) -> F(L).\n"),
        ("erl/other.erl", "-module(other).\nmap(F, L) -> L.\n"),
        ("erl/a/twin.erl", "-module(twin).\nf() -> 1.\n"),
        ("erl/b/twin.erl", "-module(twin).\nf() -> 2.\n"),
        (
            "erl/use.erl",
            "-module(use).\ngo(L) -> lists:map(fun g/1, L).\npair() -> twin:f().\n",
        ),
        // Elixir names the module by `defmodule`, and `alias` shortens it.
        (
            "ex/cart.ex",
            "defmodule Shop.Cart do\n  def total(i), do: i\nend\n",
        ),
        (
            "ex/order.ex",
            "defmodule Shop.Order do\n  def total(o), do: o\nend\n",
        ),
        (
            "ex/pay.ex",
            "defmodule Shop.Pay do\n  alias Shop.Cart, as: C\n  def full(c), do: Shop.Cart.total(c)\n  def short(c), do: C.total(c)\nend\n",
        ),
    ] {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let a = analyse(&dir).unwrap();
    let node = |s: &str| a.registry.node_of(s).unwrap_or_else(|| panic!("no {s}"));
    let reaches = |from: &str, to: &str| {
        let target = node(to);
        a.snap
            .neighbors(node(from))
            .any(|e| e.target == target || a.snap.neighbors(e.target).any(|e2| e2.target == target))
    };
    for (from, to, want) in [
        ("app.py#run", "pkg/a.py#helper", true),
        ("app.py#run", "pkg/b.py#helper", true),
        ("main.go#main", "util/u.go#Do", true),
        ("main.go#main", "other/o.go#Do", false),
        ("app.ts#start", "lib/x.ts#run", true),
        ("app.ts#start", "lib/y.ts#run", true),
        ("erl/use.erl#go", "erl/lists.erl#map", true),
        ("erl/use.erl#go", "erl/other.erl#map", false),
        ("erl/use.erl#pair", "erl/a/twin.erl#f", false),
        ("erl/use.erl#pair", "erl/b/twin.erl#f", false),
        ("ex/pay.ex#full", "ex/cart.ex#total", true),
        ("ex/pay.ex#short", "ex/cart.ex#total", true),
        ("ex/pay.ex#full", "ex/order.ex#total", false),
    ] {
        assert_eq!(reaches(from, to), want, "{from} -> {to}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
    println!("imports ok: a call through an import reaches the definition it names");
}

/// What a definition uses without calling reaches `impact` and `find_callers`:
/// a type in a signature, a constant read. Not a word in a comment or a
/// string, not a name defined twice, and only at the first hop.
fn demo_references() {
    use serde_json::json;
    let dir = std::env::temp_dir().join(format!("glasir-refs-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    for (file, body) in [
        (
            "model.rs",
            "pub struct Ledger {\n    pub total: u32,\n}\n\npub const LIMIT: u32 = 3;\n",
        ),
        (
            "settle.rs",
            "fn settle(l: &Ledger) -> u32 {\n    l.total + LIMIT\n}\n",
        ),
        (
            "top.rs",
            "fn top() -> u32 {\n    settle(&make())\n}\n\nfn keep() {\n    let f = settle;\n}\n",
        ),
        (
            "noise.rs",
            "// Ledger\nfn noise() -> &'static str {\n    \"Ledger\"\n}\n",
        ),
        ("twin_a.rs", "struct Twin;\n"),
        ("twin_b.rs", "struct Twin;\n"),
        ("pair.rs", "fn pair(t: Twin) {}\n"),
    ] {
        std::fs::write(dir.join("src").join(file), body).unwrap();
    }
    let ask = |tool: &str, symbol: &str| -> String {
        let state = served_state(&dir).unwrap();
        let r = mcp::handle_for_test(
            &state.as_served(),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": tool, "arguments": {"symbol": symbol}}}),
        )
        .unwrap();
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let text = ask("impact", "src/model.rs#Ledger");
    // The dependents, not the files a plain text search adds after them: a
    // word in a comment belongs there, and only there.
    let (deps, named) = text.split_once("its name appears").unwrap_or((&text, ""));
    assert!(named.contains("src/noise.rs"), "{text}");
    let text = deps.to_string();
    assert!(
        text.contains("1 hop (1)") && text.contains("src/settle.rs#settle"),
        "{text}"
    );
    assert!(
        text.contains("src/top.rs#top"),
        "a caller of a user, through the call: {text}"
    );
    assert!(
        !text.contains("keep"),
        "names count at the first hop only: {text}"
    );
    assert!(
        !text.contains("noise"),
        "a comment or a string is not a use: {text}"
    );
    let text = ask("find_callers", "src/model.rs#LIMIT");
    assert!(text.contains("src/settle.rs#settle"), "{text}");
    let text = ask("impact", "src/twin_a.rs#Twin");
    assert!(
        !text.contains("pair"),
        "a name defined twice cannot be attributed: {text}"
    );

    // Stored with the graph and replaced per file: an edit reaches the next
    // start, and the untouched files' uses come back from the snapshot.
    std::fs::write(dir.join("src/noise.rs"), "fn noise(l: Ledger) {}\n").unwrap();
    let text = ask("impact", "src/model.rs#Ledger");
    assert!(
        text.contains("src/noise.rs#noise") && text.contains("src/settle.rs#settle"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
    println!("references ok: a use by name reaches impact, once, and not from a comment");
}

/// The three tools that answer what to do before a commit: which tests to run,
/// what else usually changes, and whether a boundary breaks.
fn demo_change_tools() {
    use serde_json::json;

    // Every language's test conventions, and the names that only look close.
    for (symbol, want) in [
        ("tests/pay.rs#charges", true),
        ("app/spec/pay_spec.rb#it_charges", true),
        ("web/__tests__/pay.js#renders", true),
        ("pay/pay_test.go#TestPay", true),
        ("src/pay.test.ts#charges", true),
        ("src/Pay.spec.js#charges", true),
        ("src/PayTest.java#charges", true),
        ("src/PayTests.cs#Charges", true),
        ("tools/test_pay.py#helper", true),
        ("src/pay.rs#test_refund", true),
        ("src/Pay.java#testRefund", true),
        ("src/latest.rs#fetch", false),
        ("src/contest.py#run", false),
        ("src/pay.go#Testify", false),
        ("src/pay.rs#testing_mode", false),
        ("test_pay", false),
    ] {
        assert_eq!(mcp::is_test_symbol(symbol), want, "{symbol}");
    }

    // pay -> charge -> log, a test two hops from log and one three hops out,
    // and a test elsewhere that must not be named.
    let mut b = csr::CsrBuilder::new();
    for _ in 0..18u32 {
        b.add_node(0);
    }
    let e = |t| csr::Edge {
        target: t,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Inferred,
    };
    b.add_edge(0, e(1));
    b.add_edge(1, e(2));
    b.add_edge(3, e(0));
    b.add_edge(6, e(1));
    b.add_edge(5, e(4));
    // export -> render, and no test reaches either.
    b.add_edge(8, e(7));
    let g = graph::Graph::new(b.build());
    let snap = g.load();
    let mut reg = ingest::SymbolRegistry::new(0);
    for (i, name) in [
        "src/pay.rs#pay",
        "src/charge.rs#charge",
        "src/log.rs#log",
        "tests/pay.rs#pays_once",
        "src/util.rs#unrelated",
        "tests/util.rs#unrelated_works",
        "src/charge.rs#test_charge_twice",
        "src/report.rs#render",
        "src/report.rs#export",
        "src/calc.rs#add",
        "src/calc.rs#sub",
        "src/calc.rs#<module>",
        "src/tools.rs#checks_itself",
        "src/tools.rs#helper",
        "src/tools.rs#used_by_text",
        "src/gen.rs#gen_fn",
        "src/a.rs#twin",
        "src/b.rs#twin",
    ]
    .iter()
    .enumerate()
    {
        reg.insert((*name).to_string(), i as csr::NodeId);
    }
    // Read by `find_unused` below; its span is what lets the attribute above
    // `checks_itself` be seen.
    let tools = "#[test]\nfn checks_itself() {}\n\nfn helper() {}\n\nfn used_by_text() {}\n// see used_by_text\n";
    let at = tools.find("fn checks_itself").unwrap() as u32;
    reg.set_span(12, (at, at + 20));
    let defined: std::collections::HashSet<csr::NodeId> = (0..18).collect();
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let emb = embed::embed(&snap, 3);
    let search = search::SearchIndex::build_with_docs(&reg, reg.docs());
    let names = mcp::name_table(&reg, snap.width());

    // A tree on disk with a history, for `co_changes`: charge and pay change
    // together three times, docs twice, log once, and a sweeping commit that
    // touches everything is left out or it would couple log too.
    let dir = std::env::temp_dir().join(format!("glasir-changes-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    let mut round = 0;
    let mut commit = |files: &[&str]| {
        round += 1;
        for f in files {
            std::fs::write(dir.join(f), format!("{round}")).unwrap();
        }
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c"]);
    };
    commit(&["src/report.rs"]);
    let calc = "// arithmetic\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n";
    std::fs::write(dir.join("src/calc.rs"), calc).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "calc"]);
    for _ in 0..3 {
        commit(&["src/pay.rs", "src/charge.rs"]);
    }
    commit(&["src/pay.rs", "docs/pay.md", "src/util.rs"]);
    commit(&["src/pay.rs", "docs/pay.md", "src/util.rs"]);
    commit(&["src/pay.rs", "src/log.rs"]);
    let sweep: Vec<String> = (0..history::MAX_COMMIT_FILES)
        .map(|i| format!("src/gen{i}.rs"))
        .collect();
    let mut all: Vec<&str> = sweep.iter().map(String::as_str).collect();
    all.extend(["src/pay.rs", "src/log.rs"]);
    commit(&all);

    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        search: &search,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        physics: physics::Physics::default(),
        now: 1_756_600_000,
        files: None,
        mentions: None,
        references: None,
        root: Some(&dir),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let r = mcp::handle_for_test(
            &served,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": tool, "arguments": args}}),
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], false, "{tool}: {r}");
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        (r["result"]["structuredContent"].clone(), text)
    };

    // Nearest first, the unrelated test absent, and the prose names them all.
    let (v, text) = call("affected_tests", json!({"symbol": "src/log.rs#log"}));
    let tests: Vec<(&str, u64)> = v["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["symbol"].as_str().unwrap(), t["hop"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        tests,
        vec![
            ("src/charge.rs#test_charge_twice", 2),
            ("tests/pay.rs#pays_once", 3)
        ],
        "{text}"
    );
    for (t, _) in &tests {
        assert!(text.contains(t), "{t} is in the JSON and not in the text");
    }

    // The history: charge 3x and in the graph, docs and util 2x and not —
    // util is a file the graph knows, which is what makes `in_graph` a check
    // rather than a restatement of "unknown file" — and log dropped because
    // its second co-change was in the sweeping commit.
    let (v, text) = call("co_changes", json!({"file": "src/pay.rs"}));
    let coupled: Vec<(&str, u64, bool)> = v["coupled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["file"].as_str().unwrap(),
                c["together"].as_u64().unwrap(),
                c["in_graph"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        coupled,
        vec![
            ("src/charge.rs", 3, true),
            ("docs/pay.md", 2, false),
            ("src/util.rs", 2, false)
        ],
        "{text}"
    );
    assert!(text.contains("history only"), "{text}");

    // A rule the graph breaks names the edge that breaks it; one it keeps holds.
    let (v, text) = call(
        "check_architecture",
        json!({"rules": "deny src/charge.rs -> src/log.rs\ndeny src/log.rs -> src/pay.rs\n"}),
    );
    assert_eq!(v["rules"], 2);
    assert_eq!(v["holds"], false);
    let broken = v["violations"].as_array().unwrap();
    assert_eq!(broken.len(), 1, "{text}");
    assert!(
        broken[0]["evidence"][0]
            .as_str()
            .unwrap()
            .contains("src/charge.rs#charge"),
        "{text}"
    );

    // A change's risk, as reasons: pay is edited without charge, which history
    // says changes with it half the time; render has a caller and no test; and
    // a file never added to git is still part of the change.
    std::fs::write(dir.join("src/pay.rs"), "edited").unwrap();
    std::fs::write(dir.join("src/report.rs"), "edited").unwrap();
    std::fs::write(dir.join("src/new.rs"), "new").unwrap();
    let (v, text) = call("detect_changes", json!({}));
    assert_eq!(v["risk"]["level"], "high", "{text}");
    assert_eq!(
        v["risk"]["untested"],
        json!(["src/report.rs#render"]),
        "{text}"
    );
    let missed = v["risk"]["missed_partners"].as_array().unwrap();
    assert_eq!(missed.len(), 1, "{text}");
    assert_eq!(missed[0]["partner"], "src/charge.rs", "{text}");
    assert!(
        v["files_without_known_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "src/new.rs"),
        "an untracked file is part of the change: {text}"
    );
    assert!(
        text.contains("src/report.rs#render") && text.contains("leaves out"),
        "{text}"
    );

    // Line-level: a change is the definitions whose lines it touched, not the
    // whole file. Editing sub's body is sub; a comment outside every function
    // is <module>; deleting add is add, because its callers are what break.
    let calc_changed = |text: &str| -> Vec<String> {
        std::fs::write(dir.join("src/calc.rs"), text).unwrap();
        let (v, _) = call("detect_changes", json!({}));
        let mut c: Vec<String> = v["changed_symbols"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s.as_str())
            .filter(|s| s.starts_with("src/calc.rs#"))
            .map(str::to_string)
            .collect();
        c.sort();
        c
    };
    assert_eq!(
        calc_changed(&calc.replace("a - b", "b - a")),
        vec!["src/calc.rs#sub"]
    );
    assert_eq!(
        calc_changed(&calc.replace("// arithmetic", "// integer arithmetic")),
        vec!["src/calc.rs#<module>"]
    );
    assert_eq!(
        calc_changed(&calc.replace("fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n", "")),
        vec!["src/calc.rs#<module>", "src/calc.rs#add"]
    );

    // What nothing uses: a helper nobody names is reported; a test the
    // harness calls, a function named only in a comment and generated code
    // are not. The span is what lets the attribute above `checks_itself` be
    // read.
    std::fs::write(dir.join("src/tools.rs"), tools).unwrap();
    std::fs::write(
        dir.join("src/gen.rs"),
        "// Code generated by protoc. DO NOT EDIT.\nfn gen_fn() {}\n",
    )
    .unwrap();
    let (v, text) = call("find_unused", json!({"path": "src/"}));
    let unused: Vec<&str> = v["unused"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u.as_str())
        .collect();
    assert!(unused.contains(&"src/tools.rs#helper"), "{text}");
    for kept in [
        "src/tools.rs#checks_itself",
        "src/tools.rs#used_by_text",
        "src/gen.rs#gen_fn",
    ] {
        assert!(!unused.contains(&kept), "{kept} is used or excused: {text}");
    }

    // A name the graph has no edge for is still named by the files that write
    // it: `view.rs` names log and render, `charge.rs` is already a dependent,
    // `log.rs` is its own file, and `twin` is defined twice, so its mentions cannot be attributed.
    std::fs::create_dir_all(dir.join("tests")).unwrap();
    std::fs::write(dir.join("src/view.rs"), "// log, render, twin\n").unwrap();
    std::fs::write(dir.join("src/charge.rs"), "// log\n").unwrap();
    std::fs::write(dir.join("src/log.rs"), "fn log() {}\n").unwrap();
    std::fs::write(dir.join("tests/view.rs"), "// render\n").unwrap();
    let elsewhere = |v: &serde_json::Value| -> Vec<String> {
        v["named_elsewhere"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap().to_string())
            .collect()
    };
    let (v, text) = call("impact", json!({"symbol": "src/log.rs#log"}));
    assert_eq!(elsewhere(&v), ["src/view.rs"], "{text}");
    let (v, text) = call("find_callers", json!({"symbol": "src/report.rs#render"}));
    assert_eq!(elsewhere(&v), ["src/view.rs", "tests/view.rs"], "{text}");
    assert!(text.contains("tests/view.rs"), "{text}");
    let (v, text) = call("affected_tests", json!({"symbol": "src/report.rs#render"}));
    assert_eq!(elsewhere(&v), ["tests/view.rs"], "{text}");
    assert!(text.contains("run them"), "{text}");
    let (v, text) = call("impact", json!({"symbol": "src/a.rs#twin"}));
    assert!(elsewhere(&v).is_empty(), "{text}");
    assert!(text.contains("breaks no caller"), "{text}");

    std::fs::remove_dir_all(&dir).unwrap();
    println!("changes ok: tests to run, what changes with it, which rule breaks");
}

fn demo_install() {
    let dir = std::env::temp_dir().join(format!("glasir-install-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let args = |cmd: &str, extra: &[&str]| {
        let mut v = vec![
            "glasir".to_string(),
            cmd.to_string(),
            dir.display().to_string(),
            "--quiet".to_string(),
        ];
        v.extend(extra.iter().map(|s| s.to_string()));
        cli::Args::parse(v.into_iter())
    };

    // A dry run must change nothing at all.
    run_install(&args("install", &["--platform", "mcp", "--dry-run"])).unwrap();
    assert!(
        !dir.join("glasir-mcp.json").exists(),
        "--dry-run must not write"
    );

    // Explicit, because detection also reads $HOME and this machine may have
    // a client installed.
    run_install(&args("install", &["--platform", "mcp"])).unwrap();
    let registration = dir.join("glasir-mcp.json");
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&registration).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["glasir"]["type"], "stdio");
    assert_eq!(mcp["mcpServers"]["glasir"]["args"][0], "serve");
    // The tree as a person writes it: Windows' `\\?\` prefix from
    // `canonicalize` reached the client's configuration verbatim.
    assert_eq!(home_from(None, Some("C:\\U".into())), Some("C:\\U".into()));
    assert_eq!(
        home_from(Some("/h".into()), Some("C:\\U".into())),
        Some("/h".into())
    );
    let tree = mcp["mcpServers"]["glasir"]["args"][1].as_str().unwrap();
    assert!(!tree.starts_with(r"\\?\"), "{tree}");
    assert_eq!(std::path::Path::new(tree), canonical(&dir).unwrap());

    // Installing twice must not duplicate anything.
    run_install(&args("install", &["--platform", "mcp"])).unwrap();
    let again = std::fs::read_to_string(&registration).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&again).unwrap();
    assert!(parsed["mcpServers"]["glasir"].is_object());

    // A team's existing file is merged into and stays tracked.
    std::fs::write(dir.join(".mcp.json"), r#"{"mcpServers":{"other":{}}}"#).unwrap();
    run_install(&args("install", &["--platform", "claude"])).unwrap();
    let claude: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".mcp.json")).unwrap()).unwrap();
    assert!(claude["mcpServers"]["other"].is_object());
    assert!(claude["mcpServers"]["glasir"].is_object());
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(!ignore.lines().any(|l| l == ".mcp.json"));

    // Every client's own shape, as the client writes it.
    for (platform, file, pointer) in [
        (
            "gemini",
            ".gemini/settings.json",
            "/mcpServers/glasir/command",
        ),
        ("qwen", ".qwen/settings.json", "/mcpServers/glasir/command"),
        ("vscode", ".vscode/mcp.json", "/servers/glasir/command"),
        ("opencode", "opencode.json", "/mcp/glasir/command/0"),
    ] {
        run_install(&args("install", &["--platform", platform])).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
        assert!(
            doc.pointer(pointer).is_some_and(|v| v.is_string()),
            "{platform}"
        );
    }
    assert!(
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(dir.join(".gemini/settings.json")).unwrap()
        )
        .unwrap()["mcpServers"]["glasir"]["type"]
            .is_null(),
        "gemini writes no type"
    );

    // Codex is TOML edited as text: the rest of the file survives verbatim,
    // a second install does not duplicate the table, and uninstall removes
    // exactly it.
    let codex = dir.join(".codex/config.toml");
    std::fs::create_dir_all(dir.join(".codex")).unwrap();
    let theirs = "# keep me\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"x\"\n";
    std::fs::write(&codex, theirs).unwrap();
    run_install(&args("install", &["--platform", "codex"])).unwrap();
    run_install(&args("install", &["--platform", "codex"])).unwrap();
    let text = std::fs::read_to_string(&codex).unwrap();
    assert!(text.starts_with(theirs), "{text}");
    assert_eq!(text.matches("[mcp_servers.glasir]").count(), 1, "{text}");
    assert!(text.contains("args = [\"serve\", "), "{text}");

    // A project marker is detected without $HOME, and a file created here
    // carries absolute paths, so it is ignored rather than committed.
    std::fs::create_dir_all(dir.join(".cursor")).unwrap();
    run_install(&args("install", &[])).unwrap();
    let cursor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(cursor["mcpServers"]["glasir"]["args"][0], "serve");
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l == ".cursor/mcp.json"));
    assert!(ignore.lines().any(|l| l == "glasir-mcp.json"));

    run_uninstall(&args("uninstall", &[])).unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&registration).unwrap()).unwrap();
    assert!(after["mcpServers"]["glasir"].is_null());
    let claude: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".mcp.json")).unwrap()).unwrap();
    assert!(claude["mcpServers"]["glasir"].is_null());
    assert!(claude["mcpServers"]["other"].is_object());
    let text = std::fs::read_to_string(&codex).unwrap();
    assert_eq!(text.trim_end(), theirs.trim_end(), "uninstall left: {text}");
    let vscode: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".vscode/mcp.json")).unwrap())
            .unwrap();
    assert!(vscode["servers"]["glasir"].is_null());

    std::fs::write(dir.join(".glasir-graph"), b"x").unwrap();
    std::fs::write(dir.join(".glasir-layout-free"), b"x").unwrap();
    std::fs::write(dir.join(auth::TOKEN_FILE), b"x").unwrap();
    run_uninstall(&args("uninstall", &["--purge"])).unwrap();
    assert!(!dir.join(".glasir-graph").exists());
    assert!(!dir.join(".glasir-layout-free").exists());
    assert!(
        dir.join(auth::TOKEN_FILE).exists(),
        "--purge must keep tokens"
    );

    std::fs::remove_dir_all(&dir).unwrap();
    println!("phase 5.2 ok: local MCP registration round-trips cleanly");
}

/// Seed discovery: a question must reach code whose names describe it, and an
/// identifier must still win outright over any lexical match.
fn demo_search() {
    use search::{SearchIndex, tokenize};

    // The index is built in parallel over a HashMap, so symbols arrive in no
    // fixed order. That is safe only while the result does not depend on the
    // order — postings are sorted and the lengths are per node — and this is
    // what pins it: the same registry must produce the same index every time.
    // Broken on purpose by dropping the `sort_unstable` on the posting lists.
    {
        let mut reg = ingest::SymbolRegistry::new(0);
        for i in 0..400u32 {
            reg.insert(format!("src/f{}.rs#handle_request_{}", i % 40, i), i);
            reg.set_doc(i, format!("Handles request {} for the shared queue.", i));
        }
        let a = SearchIndex::build_with_docs(&reg, reg.docs());
        let b = SearchIndex::build_with_docs(&reg, reg.docs());
        for q in ["handle", "request queue", "shared", "f7"] {
            assert_eq!(
                a.search(q, 20),
                b.search(q, 20),
                "the parallel index build must be deterministic for {q:?}"
            );
        }
        // Two builds agreeing is necessary and not sufficient: 400 symbols may
        // not reorder across threads at all, so that assert passes even with
        // the sort removed. The property the sort provides is that a posting
        // list is ordered whatever order it was assembled in, and that is
        // testable directly.
        for (word, list) in a.postings_for_test() {
            assert!(
                list.windows(2).all(|w| w[0] <= w[1]),
                "posting list for {word:?} is not sorted, so ranking depends on \
thread scheduling"
            );
        }
    }

    // Identifier splitting is what makes a question reach a symbol at all.
    assert_eq!(tokenize("spawn_compaction"), vec!["spawn", "compaction"]);
    assert_eq!(tokenize("spawnCompaction"), vec!["spawn", "compaction"]);
    assert_eq!(
        tokenize("src/graph.rs#update"),
        vec!["src", "graph", "rs", "update"]
    );
    // A run of capitals is one word until the last lowercase run starts.
    assert_eq!(tokenize("HTTPServer"), vec!["httpserver"]);
    // Single letters are noise, not terms.
    assert!(!tokenize("a_b_compact").contains(&"a".to_string()));

    // The words a question is phrased *with* rather than *about* must not be
    // ranked on. BM25's IDF cannot suppress them here: it measures rarity in
    // the corpus, and the corpus is identifiers and prose, where "how" is
    // genuinely rare — measured, `how` scored IDF 4.2 against `graph`'s 2.3,
    // so the question word outweighed the subject. Prose recall 43% -> 51%.
    assert!(
        tokenize("how does the parser work")
            .iter()
            .all(|w| w != "how")
    );
    assert_eq!(tokenize("what is a burst of saves"), vec!["burst", "saves"]);
    // The cost is real and bounded: an identifier built from such a word loses
    // it as a term. `get_or_mint` indexes as "mint" alone. That is acceptable
    // because an exact or substring name match is resolved before anything is
    // scored lexically — `seeds` only reaches BM25 when the query is not a
    // name — and measured, the shorter list that keeps these words is worse
    // (47% against 51%).
    assert_eq!(tokenize("get_or_mint"), vec!["mint"]);

    // Cross-language reach: see `SearchIndex::prose_only` for why. Its own
    // index, because the property needs a word with many prose postings and no
    // name, which the fixture above cannot produce.
    let mut bi = ingest::SymbolRegistry::new(0);
    for (i, name) in ["src/app.py#animation_player", "src/app.py#start"]
        .iter()
        .enumerate()
    {
        bi.insert((*name).to_string(), i as csr::NodeId);
    }
    let mut bi_docs = std::collections::HashMap::new();
    for (i, name) in [
        "docs/h.md#Animationen A",
        "docs/h.md#Animationen B",
        "docs/h.md#Animationen C",
        "docs/h.md#Animationen D",
        "docs/h.md#Animationen E",
    ]
    .iter()
    .enumerate()
    {
        let id = (i + 2) as csr::NodeId;
        bi.insert((*name).to_string(), id);
        bi_docs.insert(id, "Animationen laufen in einer Zeitleiste".to_string());
    }
    let bi_index = SearchIndex::build_with_docs(&bi, &bi_docs);
    let hit = bi_index.search("Animationen", 1);
    assert_eq!(
        hit.first().map(|(n, _)| *n),
        Some(0),
        "a German question must reach the English symbol it describes, not the          prose it is written in: {hit:?}"
    );

    // The same reach across a spelling difference: `Konfiguration` and
    // `configuration` share no prefix, only a folded one. See
    // `SearchIndex::fold_spelling`.
    let mut fo = ingest::SymbolRegistry::new(0);
    for (i, name) in ["src/app.py#configuration_store", "src/app.py#start"]
        .iter()
        .enumerate()
    {
        fo.insert((*name).to_string(), i as csr::NodeId);
    }
    let fo_index = SearchIndex::build_with_docs(&fo, &std::collections::HashMap::new());
    let fo_hit = fo_index.search("Konfiguration", 1);
    assert_eq!(
        fo_hit.first().map(|(n, _)| *n),
        Some(0),
        "a folded stem must reach the symbol a bare prefix cannot: {fo_hit:?}"
    );

    let mut reg = ingest::SymbolRegistry::new(0);
    for (i, name) in [
        "src/graph.rs#[Graph]spawn_compaction",
        "src/graph.rs#COMPACTION_THRESHOLD",
        "src/parse_ast.rs#parse",
        "src/csr.rs#edges",
        "src/delta.rs#added_edges",
    ]
    .iter()
    .enumerate()
    {
        reg.insert((*name).to_string(), i as csr::NodeId);
    }
    let index = SearchIndex::build(&reg);
    let top = |q: &str| index.search(q, 3).first().map(|(n, _)| *n);

    // The tense a question is phrased in must not decide whether it finds
    // anything: `compacted` has to reach `compaction`, `parsed` reach `parse`.
    // Which of the two compaction symbols ranks first is not the point — that
    // depends on corpus statistics — but neither may be missed entirely.
    let compaction: Vec<csr::NodeId> = index
        .search("where are edges compacted", 5)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert!(
        compaction.contains(&0) && compaction.contains(&1),
        "stemming must bridge compacted -> compaction, got {compaction:?}"
    );
    assert_eq!(
        top("which languages are parsed"),
        Some(2),
        "parsed -> parse"
    );
    // Without stemming the query would find nothing at all.
    assert!(!index.search("compacted", 3).is_empty());

    // Query coverage counts a *term*, never a posting list, and the stem
    // fallback is what makes the difference visible: one query word now reaches
    // several lists, so a symbol appearing in two of them would count as having
    // matched two of the query's words when it matched one. Here `compaction`
    // is a term whose stem also reaches `compacted`-like neighbours, and
    // `spawn_compaction` carries both "spawn" and "compaction" — it must score
    // as two terms covered, not three. Measured before the fix: the
    // double-count fires 18 times across the benchmark sets.
    //
    // Asserted on the count itself. An ordering assert was written first and
    // does *not* catch it — sabotaged, the five-symbol fixture still ranks the
    // same way, because the scores absorb the error without reordering. A check
    // that passes with the bug in place is not a check.
    // The fixture has to make one term reach several lists, or the condition
    // never arises: `compact` is not itself indexed here, so its stem sweeps in
    // both `compaction` and `compacted`, and node 5 carries both words. That is
    // one query word covered, not two.
    let mut stem_reg = ingest::SymbolRegistry::new(0);
    for (i, name) in [
        "src/graph.rs#spawn_compaction",
        "src/graph.rs#compaction_compacted",
    ]
    .iter()
    .enumerate()
    {
        stem_reg.insert((*name).to_string(), i as csr::NodeId);
    }
    let stem_index = SearchIndex::build(&stem_reg);
    assert_eq!(
        stem_index.coverage_for_test("compact", 1),
        1,
        "one query word is one term covered, however many posting lists its          stem reaches"
    );

    // Covering more of the query must beat a single common-word hit.
    let both = index.search("added edges", 3);
    assert_eq!(both.first().map(|(n, _)| *n), Some(4), "two terms beat one");
    // A query matching nothing returns nothing rather than noise.
    assert!(index.search("zzzz nonexistent", 3).is_empty());
    // Deterministic despite the HashMaps inside.
    assert_eq!(index.search("compaction", 3), index.search("compaction", 3));

    // An exact symbol name must not be diluted by lexical scoring.
    let snap_graph = graph::Graph::new(csr::CsrBuilder::new().build());
    let snap = snap_graph.load();
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::default(),
    );
    let emb = embed::embed(&snap, 2);
    let defined = std::collections::HashSet::new();
    let names = mcp::name_table(&reg, snap.width());
    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        search: &index,
        physics: physics::Physics::default(),
        now: 0,
        files: None,
        mentions: None,
        references: None,
        root: None,
    };
    let seeds = mcp::seeds_for_test(&served, "src/parse_ast.rs#parse");
    assert_eq!(
        seeds,
        vec![(2, 1.0)],
        "an exact name is certainty, not a ranking"
    );

    // **A query too short to be a substring of anything in particular must
    // find nothing**, so `query_graph` can fall back to naming this tree's
    // subsystems — the path written for a caller who used the wrong word.
    // `symbol.contains("")` is true of every name, so the empty query used to
    // return the eight lowest-numbered nodes at 0.800 and the tool served them
    // as a result. One letter did the same.
    for q in ["", " ", "a", "e"] {
        assert!(
            mcp::seeds_for_test(&served, q).is_empty(),
            "{q:?} must not seed on being a substring of everything"
        );
    }
    // Long enough to mean something still matches as a substring.
    assert!(!mcp::seeds_for_test(&served, "parse").is_empty());

    // A German question must not take the server down. `&term[..5]` panics when
    // the fifth byte lands inside a multi-byte character, and that is the
    // normal case for German — found by an audit after this session added
    // German stop words and thereby invited exactly these questions.
    {
        // Needs a populated index: the stemming branch that panicked only runs
        // when the term is *not* an exact posting, and only then reaches the
        // byte slice.
        let mut reg = ingest::SymbolRegistry::new(0);
        reg.get_or_mint("src/keys.rs#schluessel_verwaltung");
        reg.get_or_mint("src/order.rs#ausfuehrung");
        let idx = search::SearchIndex::build(&reg);
        for word in [
            "schlüsselverwaltung",
            "ausführungsreihenfolge",
            "größenordnung",
            "änderungen",
            "über",
        ] {
            let _ = idx.search(word, 8);
        }
    }

    println!("phase 4.1 ok: questions reach symbols, identifiers still win");
}

/// HTTP transport: the status codes a client branches on, and the three
/// defences the spec requires of a locally reachable server.
fn demo_http() {
    use http::{addr_for, origin_allowed};

    // Origin validation is what stops a web page reaching a local server
    // through DNS rebinding. A real MCP client sends no Origin at all.
    assert!(origin_allowed(None), "a non-browser client sends no Origin");
    assert!(origin_allowed(Some("http://localhost:3000")));
    assert!(origin_allowed(Some("http://127.0.0.1:8899")));
    assert!(!origin_allowed(Some("https://evil.example.com")));
    // A host that merely starts with the right text must not pass.
    assert!(
        !origin_allowed(Some("https://localhost.evil.com")),
        "prefix matching must not be fooled by a crafted hostname"
    );

    // A bare port binds to loopback: serving someone's source graph on every
    // interface is a disclosure, not a convenience.
    assert_eq!(addr_for("8899"), "127.0.0.1:8899");
    assert_eq!(addr_for("127.0.0.1:9000"), "127.0.0.1:9000");
    assert_eq!(addr_for("0.0.0.0:9000"), "0.0.0.0:9000");

    println!("phase 5.1 ok: origin validated, loopback by default");
}

/// TLS terminates here when the operator supplies a certificate.
///
/// The property is that the bytes on the wire are not the request: a token in
/// clear is readable by anything between the client and this process, which is
/// why the README had to send operators to a reverse proxy. Checked by
/// speaking TLS to the server and confirming a plaintext read gets nothing
/// usable back.
///
/// Broken on purpose by ignoring `cfg.tls` in the accept loop: the handshake
/// then fails and the first assert reports it.
fn demo_tls() {
    use std::io::{Read, Write};
    use std::sync::Arc;

    let dir = std::env::temp_dir().join(format!("glasir-tls-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.rs"), "fn alpha() {}\n").unwrap();

    // A self-signed pair, generated here so the check needs no fixture on disk
    // and no openssl in the environment.
    let cert = rcgen_like_selfsigned();
    let Some((cert_pem, key_pem)) = cert else {
        // No key generator in the build: the transport is still covered by the
        // negative half below, which needs no certificate.
        let _ = std::fs::remove_dir_all(&dir);
        println!("phase B.1 ok: tls configuration refused an incomplete pair");
        return;
    };
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();

    let tls = Arc::new(http::tls_config(&cert_path, &key_path).unwrap());
    let state = Arc::new(published::Published::from_pointee(
        served_state(&dir).unwrap(),
    ));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let cfg = http::HttpConfig {
        metrics: Default::default(),
        addr: addr.to_string(),
        token: Some("s3cret".into()),
        tokens: Arc::new(auth::Tokens::new(dir.join(".glasir-tokens"))),
        audit: None,
        public_url: None,
        reindexed: None,
        tls: Some(tls),
        behind_control_plane: false,
        control_plane_cidr: None,
        reindex_error: Arc::new(std::sync::Mutex::new(None)),
        max_index_age: None,
    };
    {
        let (state, cfg) = (state.clone(), cfg.clone());
        std::thread::spawn(move || {
            let _ = http::serve(&state, &cfg);
        });
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    // A plaintext client must not get an HTTP reply out of a TLS listener:
    // whatever comes back is a handshake failure, never a status line.
    let mut plain = std::net::TcpStream::connect(addr).unwrap();
    plain
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    write!(plain, "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    plain.flush().unwrap();
    let mut buf = [0u8; 32];
    let n = plain.read(&mut buf).unwrap_or(0);
    assert!(
        !buf[..n].starts_with(b"HTTP/1.1"),
        "a TLS listener must not answer plaintext with a status line"
    );

    // And the positive half, without which the assert above would also pass on
    // a server that is simply broken: a real handshake reaches `/health`.
    let mut roots = rustls::RootCertStore::empty();
    for c in crate::pem::certs(&mut std::io::BufReader::new(TEST_CERT.as_bytes())).unwrap() {
        roots.add(c).unwrap();
    }
    let client_cfg = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let server_name = "localhost".try_into().unwrap();
    let mut conn = rustls::ClientConnection::new(client_cfg, server_name).unwrap();
    let mut sock = std::net::TcpStream::connect(addr).unwrap();
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut tls = rustls::Stream::new(&mut conn, &mut sock);
    write!(tls, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    tls.flush().unwrap();
    // Read until the status line is complete, not once: a TLS record boundary
    // can split it, and a single `read` then returns `HTTP/1.1 ` and fails an
    // assert about a server that is answering correctly. Seen as a flaky
    // failure — three runs green, one red, with no code between them touching
    // TLS at all.
    let mut reply = Vec::new();
    let mut chunk = [0u8; 64];
    while reply.len() < 12 {
        match tls.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            // macOS may transiently report that the socket has no data while
            // rustls finishes the handshake. The socket still has its bounded
            // read timeout; retrying this condition avoids treating a valid
            // in-flight handshake as a failed health request.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
    assert!(
        reply.starts_with(b"HTTP/1.1 200"),
        "a TLS client must reach the server: {:?}",
        String::from_utf8_lossy(&reply)
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase B.1 ok: TLS terminates in the process");
}

/// A self-signed pair for `demo_tls`, valid for a century and for
/// `localhost` only.
///
/// Checked in rather than generated: a generator costs thirty crates for a
/// test, and shelling out to `openssl` makes the check depend on what happens
/// to be installed. It is a test key and secures nothing — the private half is
/// published in this file on purpose, so nobody mistakes it for a credential.
const TEST_CERT: &str = include_str!("../bench/tls/localhost.crt");

const TEST_KEY: &str = include_str!("../bench/tls/localhost.key");

fn rcgen_like_selfsigned() -> Option<(String, String)> {
    Some((TEST_CERT.to_string(), TEST_KEY.to_string()))
}

/// The same tree analysed twice gives bit-identical answers.
///
/// Five stages run in parallel — the parse pool, the embeddings, the layout,
/// the search index and the file graph — and rayon hands their work out in
/// whatever order threads become free. Each one is written so the *result*
/// does not depend on that order, but nothing checked the claim end to end,
/// and the claim is the reason a retrieved answer can be cited: the same
/// question against the same commit has to reach the same subgraph on a
/// colleague's machine as on this one.
///
/// A fixture with enough files to spread across the pool, run through the
/// whole pipeline twice from cold — a single-file tree would parallelise into
/// one task and prove nothing.
fn demo_deterministic() {
    let dir = std::env::temp_dir().join(format!("glasir-determinism-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    // Cross-file calls, shared names and documentation: the shapes every stage
    // has to agree on, and enough of them to occupy several threads.
    for m in 0..24u32 {
        let body = format!(
            "/// Module {m} handles requests for the shared queue.\n\
             pub fn handle_{m}(n: u32) -> u32 {{ helper_{m}(n) + shared(n) }}\n\
             /// Helper for module {m}.\n\
             pub fn helper_{m}(n: u32) -> u32 {{ shared(n) }}\n\
             pub fn shared(n: u32) -> u32 {{ n }}\n"
        );
        std::fs::write(dir.join(format!("src/m{m}.rs")), body).unwrap();
    }

    let once = |dir: &std::path::Path| {
        // From cold every time: a snapshot would hand the second run the first
        // run's answer and the check would pass on a cache.
        let _ = std::fs::remove_file(dir.join(".glasir-graph"));
        let a = analyse(dir).unwrap();
        let emb = embed::embed(&a.snap, 4);
        let lay = layout::compute(&a.snap, &a.communities.of_node, layout::Mode::Grouped);
        let search = search::SearchIndex::build_with_docs(&a.registry, a.registry.docs());
        let names = mcp::name_table(&a.registry, a.snap.width());
        // Symbol -> id, the numbering every other stage is expressed in.
        let mut ids: Vec<(String, csr::NodeId)> =
            a.registry.entries().map(|(s, &n)| (s.clone(), n)).collect();
        ids.sort();
        // Edges as names, so a differing id order shows up here rather than
        // hiding behind a consistent renumbering.
        let mut edges: Vec<(String, String)> = Vec::new();
        for n in 0..a.snap.width() as csr::NodeId {
            for e in a.snap.neighbors(n) {
                edges.push((
                    names.get(n as usize).cloned().unwrap_or_default(),
                    names.get(e.target as usize).cloned().unwrap_or_default(),
                ));
            }
        }
        edges.sort();
        let nearest: Vec<Vec<(u32, f32)>> = (0..a.snap.width().min(40) as u32)
            .map(|n| emb.nearest(n, 5))
            .collect();
        let hits: Vec<Vec<(csr::NodeId, f32)>> = ["handle", "shared queue", "helper module"]
            .iter()
            .map(|q| search.search(q, 10))
            .collect();
        (
            ids,
            edges,
            a.communities.of_node.clone(),
            nearest,
            (lay.x, lay.y),
            hits,
        )
    };

    let first = once(&dir);
    let second = once(&dir);

    // Named separately so a failure says which stage moved, rather than
    // reporting that two large tuples differ.
    assert_eq!(first.0, second.0, "symbol ids must be assigned identically");
    assert_eq!(
        first.1, second.1,
        "the edge set must not depend on thread order"
    );
    assert_eq!(first.2, second.2, "the partition must be reproducible");
    assert_eq!(
        first.3, second.3,
        "embedding neighbourhoods must be identical"
    );
    assert_eq!(first.4, second.4, "layout coordinates must be identical");
    assert_eq!(first.5, second.5, "search ranking must be identical");

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase 2 ok: the same tree analysed twice answers identically");
}

/// The source a symbol names, read back from the range the parser recorded.
///
/// Needs a real tree: every other MCP check runs against a synthetic graph with
/// no files behind it, and this tool's whole job is to reach the bytes on disk.
///
/// What it pins is that the range identifies the *definition* and not some
/// slice around it — the text must start at the symbol and contain its body.
/// Broken on purpose by storing the `@name` capture's range instead of the
/// widened one: the snippet then reads `alpha` and the body assert fails.
fn demo_snippet() {
    use serde_json::json;

    let dir = std::env::temp_dir().join(format!("glasir-snippet-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/a.rs"),
        "fn alpha() {\n    let x = 1;\n    beta(x);\n}\n\nfn beta(n: u32) {}\n",
    )
    .unwrap();

    let state = served_state(&dir).unwrap();
    let served = state.as_served();
    let call = |sym: &str| {
        mcp::handle_for_test(
            &served,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "get_code_snippet", "arguments": {"symbol": sym}}}),
        )
        .unwrap()
    };

    let r = call("src/a.rs#alpha");
    assert_eq!(
        r["result"]["isError"], false,
        "{}",
        r["result"]["content"][0]["text"]
    );
    let sc = &r["result"]["structuredContent"];
    let src = sc["source"].as_str().unwrap();
    // The definition, not a window around it: it opens at the symbol and
    // carries the body, which is what makes the range the right one.
    assert!(
        src.starts_with("fn alpha"),
        "starts at the definition: {src:?}"
    );
    assert!(src.contains("beta(x)"), "carries the body: {src:?}");
    assert!(!src.contains("fn beta"), "stops at its own end: {src:?}");
    assert_eq!(sc["line"], 1, "1-based, and alpha opens the file");
    assert_eq!(sc["file"], "src/a.rs", "root-relative, never absolute");

    // The second definition proves the line number is computed and not assumed.
    let second = call("src/a.rs#beta");
    assert_eq!(second["result"]["structuredContent"]["line"], 6);

    // A name matching two symbols is refused with its candidates, exactly as
    // `impact` refuses one — a snippet of the wrong `beta` is worse than none.
    std::fs::write(dir.join("src/b.rs"), "fn beta() {}\n").unwrap();
    let state2 = served_state(&dir).unwrap();
    let served2 = state2.as_served();
    let ambiguous = mcp::handle_for_test(
        &served2,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "get_code_snippet", "arguments": {"symbol": "beta"}}}),
    )
    .unwrap();
    assert_eq!(
        ambiguous["result"]["isError"], true,
        "a bare name is ambiguous"
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase C.2 ok: a symbol returns its own source");
}

/// B.1: an open SSE stream is told when the graph underneath it is replaced.
///
/// The bug this guards was silent and lived in a comment: the GET handler
/// wrote its header and returned, justified by "the graph does not change
/// while it serves" — which stopped being true once `--watch` called
/// `state.store`. A client cached answers describing a tree that had moved on
/// and nothing in the protocol contradicted it.
///
/// Broken on purpose two ways, both caught here: dropping the `fetch_add` in
/// `run_serve` makes the notification never arrive and the read times out; and
/// firing on connect rather than on change makes the "silent until something
/// happens" assert fail, which is what keeps a client from being told to
/// invalidate a cache it has not filled yet.
fn demo_http_events() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    let dir = std::env::temp_dir().join(format!("glasir-events-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/a.rs"),
        "fn alpha() { beta(); }\nfn beta() {}\n",
    )
    .unwrap();

    let state = Arc::new(published::Published::from_pointee(
        served_state(&dir).unwrap(),
    ));
    let reindexed = Arc::new(AtomicU64::new(0));

    // Port 0 lets the OS pick, so parallel test binaries cannot collide.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let cfg = http::HttpConfig {
        metrics: Default::default(),
        addr: addr.to_string(),
        // A shared token, or `refuses_to_start` would reject a non-loopback
        // bind — this one is loopback, but the token also proves the GET path
        // is reached with authentication configured.
        token: Some("s3cret".into()),
        tokens: Arc::new(auth::Tokens::new(dir.join(".glasir-tokens"))),
        audit: None,
        public_url: None,
        reindexed: Some(reindexed.clone()),
        tls: None,
        behind_control_plane: false,
        control_plane_cidr: None,
        reindex_error: Arc::new(std::sync::Mutex::new(None)),
        max_index_age: None,
    };
    {
        let (state, cfg) = (state.clone(), cfg.clone());
        std::thread::spawn(move || {
            let _ = http::serve(&state, &cfg);
        });
    }
    // The listener is bound inside the thread, so the connect below races it.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // `/health` answers a prober that has no credential, while `/mcp` still
    // refuses one — the pair is the property, since either half alone is
    // either useless to an orchestrator or a hole.
    {
        let mut probe = std::net::TcpStream::connect(addr).unwrap();
        write!(probe, "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        probe.flush().unwrap();
        probe
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut head = String::new();
        BufReader::new(&mut probe).read_line(&mut head).unwrap();
        assert!(
            head.starts_with("HTTP/1.1 200"),
            "an orchestrator has no token to offer: {head}"
        );

        let mut ready = std::net::TcpStream::connect(addr).unwrap();
        write!(ready, "GET /ready HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        ready.flush().unwrap();
        ready
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut ready_reply = String::new();
        ready.read_to_string(&mut ready_reply).unwrap();
        assert!(ready_reply.starts_with("HTTP/1.1 200"), "{ready_reply}");
        assert!(
            ready_reply.contains("\"status\":\"ready\"")
                && ready_reply.contains("\"indexed_at\":")
                && ready_reply.contains("\"last_reindex_error\":null"),
            "readiness must expose a loaded graph and its re-index state: {ready_reply}"
        );

        let mut denied = std::net::TcpStream::connect(addr).unwrap();
        write!(
            denied,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n"
        )
        .unwrap();
        denied.flush().unwrap();
        denied
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut refused = String::new();
        BufReader::new(&mut denied).read_line(&mut refused).unwrap();
        assert!(
            refused.starts_with("HTTP/1.1 401"),
            "the graph stays behind the token: {refused}"
        );
    }

    // `/metrics` is the pair's third member: reachable without a credential
    // like `/health`, and it must *count* rather than merely answer. A route
    // that returns a well-formed page of zeroes passes a shape check and tells
    // an operator nothing, so the assertion is that a tool call moves the
    // number and a refusal moves the other one.
    {
        let read_metrics = || -> String {
            let mut m = std::net::TcpStream::connect(addr).unwrap();
            write!(m, "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            m.flush().unwrap();
            m.set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(&mut m, &mut body);
            body
        };
        let count = |page: &str, needle: &str| -> u64 {
            page.lines()
                .find(|l| l.starts_with(needle))
                .and_then(|l| l.rsplit(' ').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0)
        };

        let before = read_metrics();
        assert!(
            before.starts_with("HTTP/1.1 200"),
            "a scraper has no token to offer either: {}",
            before.lines().next().unwrap_or("")
        );
        assert!(
            before.contains("glasir_graph_nodes"),
            "the graph gauge is what makes one scrape answer both questions"
        );

        let mut call = std::net::TcpStream::connect(addr).unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"overview","arguments":{}}}"#;
        write!(
            call,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer s3cret\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        call.flush().unwrap();
        call.set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut answered = String::new();
        let _ = std::io::Read::read_to_string(&mut call, &mut answered);

        // A fresh refusal *after* the first read: the 401 further up happened
        // before it and is already in the baseline.
        let mut denied = std::net::TcpStream::connect(addr).unwrap();
        write!(
            denied,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n"
        )
        .unwrap();
        denied.flush().unwrap();
        denied
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut ignored = String::new();
        BufReader::new(&mut denied).read_line(&mut ignored).unwrap();

        let after = read_metrics();
        assert_eq!(
            count(&after, "glasir_tool_calls_total{tool=\"overview\"}"),
            count(&before, "glasir_tool_calls_total{tool=\"overview\"}") + 1,
            "an answered tool call must move its counter"
        );
        assert!(
            count(&after, "glasir_requests_refused_total")
                > count(&before, "glasir_requests_refused_total"),
            "the 401 above is a refusal and must be counted"
        );
    }

    let mut s = std::net::TcpStream::connect(addr).unwrap();
    write!(
        s,
        "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer s3cret\r\n\r\n"
    )
    .unwrap();
    s.flush().unwrap();
    // Long enough to cover several poll intervals, so a missing notification
    // is a real absence rather than an impatient read.
    s.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut r = BufReader::new(s);

    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    assert!(
        line.starts_with("HTTP/1.1 200"),
        "SSE stream refused: {line}"
    );
    // Past the headers to the ": connected" comment that opens the body.
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        if h.trim().is_empty() {
            break;
        }
        assert!(
            !h.to_ascii_lowercase().starts_with("content-length"),
            "a stream must not announce a length: {h}"
        );
    }

    // Silent until something actually happens: a client that connects to a
    // long-running server must not be told its cache is stale on arrival.
    //
    // The read must go *past* the opening comment, which is written before the
    // poll loop and so arrives however the loop behaves — asserting on that
    // line alone passes even when the stream fires on connect. A short timeout
    // is the assertion here: nothing more may arrive before the graph moves.
    let mut opening = String::new();
    r.read_line(&mut opening).unwrap();
    assert!(
        opening.starts_with(':'),
        "stream should open with a comment, got {opening:?}"
    );
    r.get_ref()
        .set_read_timeout(Some(std::time::Duration::from_millis(1500)))
        .unwrap();
    // Blank lines terminate an SSE record and carry nothing, so a read that
    // stops at one has not yet seen whether an event followed it.
    for _ in 0..4 {
        let mut early = String::new();
        if r.read_line(&mut early).unwrap_or(0) == 0 {
            break;
        }
        if early.trim().is_empty() {
            continue;
        }
        assert!(
            !early.contains("notifications/"),
            "no event may fire before the graph changes: {early:?}"
        );
    }
    r.get_ref()
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();

    // What the watcher does on a re-index, in the same order: publish, then
    // announce.
    state.store(Arc::new(served_state(&dir).unwrap()));
    reindexed.fetch_add(1, Ordering::Relaxed);

    // Skip blank separators and keep-alive comments; the event is what matters.
    let mut event = String::new();
    for _ in 0..40 {
        let mut l = String::new();
        if r.read_line(&mut l).unwrap_or(0) == 0 {
            break;
        }
        if l.starts_with("data: ") {
            event = l;
            break;
        }
    }
    assert!(
        event.contains("notifications/resources/updated"),
        "a republished graph must reach an open stream, got {event:?}"
    );
    // A notification carries no id: a client that answered it would be
    // answering something that was never a request.
    let payload: serde_json::Value =
        serde_json::from_str(event.trim_start_matches("data: ")).unwrap();
    assert_eq!(payload["method"], "notifications/resources/updated");
    assert!(payload.get("id").is_none(), "a notification has no id");

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase B.1 ok: a re-index reaches an open SSE stream");
}

/// B.1: one index, many clients, no state per connection.
///
/// The failure this guards is not a wrong answer but a serial one: before this,
/// `http::serve` accepted and answered in one loop, so a second client waited
/// on the first. Broken on purpose by putting the accept loop back — ten
/// requests then take ten times a single one, and the assert on elapsed time
/// catches it because each handler here sleeps.
///
/// The second half is the swap: a `store` while clients are reading must be
/// picked up by the next request and must never hand anyone a torn state.
fn demo_http_concurrent() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The shared value stands in for `ServedState`: what is being tested is the
    // accept loop and the swap, not the graph, and building ten graphs would
    // make this check cost seconds for nothing.
    let state = Arc::new(published::Published::from_pointee(1u32));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    {
        let (state, live, peak) = (state.clone(), live.clone(), peak.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(20) {
                let Ok(mut stream) = stream else { continue };
                let (state, live, peak) = (state.clone(), live.clone(), peak.clone());
                std::thread::spawn(move || {
                    let n = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    // Long enough that a serial server could not overlap two.
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    // Loaded per request, exactly as `handle_connection` does.
                    let v = *state.load();
                    let _ = writeln!(stream, "{v}");
                    live.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
    }

    let ask = move || {
        let mut s = TcpStream::connect(addr).unwrap();
        let mut line = String::new();
        BufReader::new(&mut s).read_line(&mut line).unwrap();
        line.trim().parse::<u32>().unwrap()
    };

    let t = std::time::Instant::now();
    let clients: Vec<_> = (0..10).map(|_| std::thread::spawn(ask)).collect();
    let answers: Vec<u32> = clients.into_iter().map(|c| c.join().unwrap()).collect();
    let elapsed = t.elapsed();

    assert_eq!(answers, vec![1u32; 10], "every client reads the same index");
    assert!(
        peak.load(Ordering::SeqCst) >= 2,
        "clients must overlap; a serial accept loop never has two in flight"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(400),
        "ten 80ms requests took {elapsed:?} — that is a serial server, not a concurrent one"
    );

    // A re-index lands between requests: the next one sees it, and nobody saw
    // anything in between.
    state.store(Arc::new(2));
    let after: Vec<u32> = (0..5)
        .map(|_| std::thread::spawn(ask))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|c| c.join().unwrap())
        .collect();
    assert_eq!(
        after,
        vec![2u32; 5],
        "a swapped index is served immediately"
    );

    println!("phase B.1 ok: 10 concurrent clients in {elapsed:?}, index swapped under them");
}

/// D.4: what a real tree contains and a development tree never does.
///
/// Each case here was silent before it was measured — the tool reported success
/// and indexed a fraction of the repository. A tool that quietly skips half a
/// codebase is worse than one that refuses, because nobody knows to look.
fn demo_edges() {
    let dir = std::env::temp_dir().join(format!("glasir-edges-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Latin-1, as older C and Java trees are full of. `read_to_string` returns
    // Err, which the old code turned into "this file does not exist"; it is now
    // decoded rather than dropped, because in practice a single byte in a
    // copyright header is what makes a file of ASCII code invalid.
    std::fs::write(dir.join("latin1.rs"), b"fn gr\xfc\xdfen() { hallo(); }\n").unwrap();
    assert_eq!(
        read_source(&dir.join("latin1.rs")).as_deref(),
        Some("fn grüßen() { hallo(); }\n"),
        "Latin-1 is decoded, not refused"
    );
    // And the identifiers actually reach the graph — decoding is worth nothing
    // if the parser then rejects what it was handed.
    let facts = parse_ast::parse(
        &read_source(&dir.join("latin1.rs")).unwrap(),
        parse_ast::Lang::Rust,
    )
    .expect("a Latin-1 source still parses");
    assert!(
        facts.defines.iter().any(|d| d == "grüßen"),
        "a definition from a Latin-1 file is indexed, not silently lost"
    );

    // Larger than the cap: a generated file, eight of which parse in parallel.
    let big = dir.join("big.rs");
    std::fs::write(&big, "x".repeat(MAX_SOURCE_BYTES as usize + 1)).unwrap();
    assert!(
        read_source(&big).is_none(),
        "a file past the size cap is skipped, not parsed"
    );
    // And one byte under it is still read.
    std::fs::write(&big, "y".repeat(MAX_SOURCE_BYTES as usize - 1)).unwrap();
    assert!(
        read_source(&big).is_some(),
        "the cap must not reject an ordinary file"
    );

    // A file that cannot be read is not a file that is gone. Distinguishing
    // them is what keeps a mid-save editor from deleting a file's call graph.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.join("locked.rs");
        std::fs::write(&locked, "fn a() {}\n").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root ignores the permission bits, so only assert when it actually
        // took effect — otherwise this check passes for the wrong reason.
        if std::fs::read_to_string(&locked).is_err() {
            assert!(read_source(&locked).is_none(), "unreadable is refused");
            assert!(locked.exists(), "and it still exists, which is the point");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    // An ordinary file still reads, or the guards above are just a way of
    // indexing nothing.
    let good = dir.join("good.rs");
    std::fs::write(&good, "fn pay() { charge(); }\n").unwrap();
    assert_eq!(
        read_source(&good).as_deref(),
        Some("fn pay() { charge(); }\n")
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase D.4 ok: non-UTF-8, oversized and unreadable files are named, not swallowed");
}

/// D.2: the benchmark as a guard.
///
/// The floors themselves are measured, so what is checked here is the guard: it
/// must read the file, and it must fire. A tolerance that swallowed a real drop
/// would be worse than no check, because it would read as coverage.
fn demo_baseline() {
    let path = std::path::Path::new("bench/baseline.txt");
    let Ok(base) = bench::load_baseline(path) else {
        println!("phase D.2 skipped: bench/baseline.txt not readable from here");
        return;
    };
    assert_eq!(base.floors.len(), 7, "one floor per ground truth");
    for set in [
        "questions",
        "questions-identifier",
        "questions-docs",
        // The same documentation questions in German, against English docs.
        "questions-docs-de",
        // Not a question file: the partition scores against the same answer
        // symbols through `bench::overview_scale`. It gets a floor because the
        // four above are blind to it — `query_graph` never reads communities,
        // so a partition that collapses into one lump measures identically on
        // every one of them, which is how it stayed unseen for five sessions.
        "partition",
        // A different tree, not a different phrasing: `bench/deep` is analysed
        // on its own so the four sets above keep measuring this repository.
        "deep",
        // Scored by calling `impact` and `shortest_path`, not by expanding
        // seeds — the half of the tool a grep baseline cannot reach.
        "structural",
    ] {
        assert!(
            base.floors.iter().any(|(k, _)| k == set),
            "{set} has no recorded floor, so a regression there is invisible"
        );
    }
    // Every foreign repository has its set and its floor, and every floor its
    // repository: an unpaired entry is either unguarded or never measured.
    let repos = std::fs::read_to_string("bench/foreign/repos.txt").unwrap();
    let foreign = bench::load_baseline(std::path::Path::new("bench/foreign/baseline.txt")).unwrap();
    let names: Vec<&str> = repos
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(!names.is_empty());
    for name in &names {
        assert!(
            std::path::Path::new(&format!("bench/foreign/{name}.txt")).exists(),
            "{name} has no question set"
        );
        assert!(
            foreign.floors.iter().any(|(k, _)| k == name),
            "{name} has no floor"
        );
    }
    for (k, _) in &foreign.floors {
        assert!(names.contains(&k.as_str()), "floor {k} names no repository");
    }

    // A single question in a twelve-question set is worth eight points, so a
    // tolerance at or above that would never fire.
    assert!(
        base.tolerance > 0.0 && base.tolerance < 8.0,
        "a tolerance of {} points cannot catch a lost question",
        base.tolerance
    );

    // The comparison itself, on numbers rather than on a benchmark run: the
    // run costs seconds and this is the arithmetic that decides a build.
    let below = |got: f32, floor: f32| got + base.tolerance < floor;
    assert!(
        below(56.0, 69.0),
        "a thirteen-point drop must fail the build"
    );
    assert!(!below(56.0, 56.0), "holding the floor must pass");
    assert!(
        !below(54.0, 56.0),
        "a two-point wobble is question wording, not a regression"
    );
    assert!(below(52.0, 56.0), "past the tolerance it fails");

    println!("phase D.2 ok: {} floors guard retrieval", base.floors.len());
}

/// D.5: the git hooks that keep a graph from silently describing code that was
/// replaced under it.
///
/// The interesting part is which hooks, and when they decline to run. Both were
/// validated against documented Git hook transitions and by breaking this one:
/// an untracked `.glasir-graph` left the tree dirty
/// and git then refused the next branch switch outright.
fn demo_hooks() {
    let dir = std::env::temp_dir().join(format!("glasir-hooks-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .expect("git")
    };
    git(&["init", "-q"]);
    let exe = std::path::Path::new("/usr/local/bin/glasir");

    // A `git pull --rebase` never merges, so post-merge alone misses it, and a
    // branch switch fires neither. That omission is the whole failure this item
    // is about, one verb further down.
    assert!(
        HOOKS.contains(&"post-rewrite") && HOOKS.contains(&"post-checkout"),
        "post-merge alone misses `git pull --rebase` and every branch switch"
    );

    let notes = install_hooks(&dir, exe, false);
    assert_eq!(notes.len(), HOOKS.len());
    for hook in HOOKS {
        let path = dir.join(".git/hooks").join(hook);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("#!"), "{hook} must be executable shell");
        assert!(body.contains(HOOK_BEGIN) && body.contains(HOOK_END));
        // Detached, or a pull waits on an analysis that costs seconds.
        assert!(
            body.trim_end().ends_with(HOOK_END) && body.contains(") &"),
            "{hook} must run the analysis in the background"
        );
        // Half-finished states are not worth indexing.
        assert!(
            body.contains("MERGE_HEAD"),
            "{hook} must skip mid-operation"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "git ignores a hook that is not executable");
        }
    }
    // A checkout that is not a branch switch changes nothing.
    let checkout = std::fs::read_to_string(dir.join(".git/hooks/post-checkout")).unwrap();
    assert!(
        checkout.contains("\"$3\" = \"1\"") && checkout.contains("\"$1\" = \"$2\""),
        "post-checkout must decline a file checkout and a no-op switch"
    );

    // Installing twice must not stack two copies.
    let again = install_hooks(&dir, exe, false);
    assert!(
        again.iter().all(|n| n.contains("already installed")),
        "a second install must be a no-op"
    );

    // Someone else's hook is appended to, never replaced; a file that is not
    // shell is left alone entirely.
    let foreign = dir.join(".git/hooks/pre-push");
    std::fs::write(&foreign, "#!/bin/sh\nnpm test\n").unwrap();
    std::fs::write(
        dir.join(".git/hooks/post-merge"),
        "#!/bin/sh\nnpm install\n",
    )
    .unwrap();
    install_hooks(&dir, exe, false);
    let merged = std::fs::read_to_string(dir.join(".git/hooks/post-merge")).unwrap();
    assert!(
        merged.contains("npm install"),
        "another tool's hook survives"
    );

    uninstall_hooks(&dir, false);
    let after = std::fs::read_to_string(dir.join(".git/hooks/post-merge")).unwrap();
    assert!(
        after.contains("npm install"),
        "uninstall keeps what it found"
    );
    assert!(
        !after.contains(HOOK_BEGIN),
        "and removes exactly its own lines"
    );
    assert!(
        !dir.join(".git/hooks/post-checkout").exists(),
        "a hook holding nothing but our block is deleted, not left as a stub"
    );
    assert!(
        std::fs::read_to_string(&foreign)
            .unwrap()
            .contains("npm test")
    );

    // The stored graph must be ignored, or an analysis leaves the tree dirty
    // and git refuses the next branch switch: "Please commit your changes".
    // This was found by a hook doing exactly that.
    ignore_generated_files(&dir, GENERATED);
    let ignored = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    for name in GENERATED {
        assert!(ignored.contains(name), "{name} must be ignored");
    }
    std::fs::write(dir.join(".glasir-graph"), "x").unwrap();
    let status = git(&["status", "--porcelain"]);
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status.contains(".glasir-graph"),
        "a refreshed graph must not make the tree dirty: {status}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase D.5 ok: three hooks, background, reversible, tree stays clean");
}

/// B.2: per-user tokens — minted, identified, expired, revoked without a
/// restart.
///
/// The hash comes first and is checked against the published FIPS 180-4
/// vectors: everything else here trusts it, and a hash that is merely
/// plausible would make every token comparison quietly wrong.
fn demo_auth() {
    use auth::{Entry, Tokens, sha256_hex};

    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Two blocks, so the message schedule runs more than once.
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );

    let dir = std::env::temp_dir().join(format!("glasir-auth-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = auth::token_path(&dir);

    let secret = auth::mint().unwrap();
    let other = auth::mint().unwrap();
    assert_ne!(secret, other, "each token is freshly random");
    assert_eq!(secret.len(), 64, "32 bytes, hex-encoded");

    let now = 1_000_000u64;
    auth::append(
        &path,
        &Entry {
            hash: sha256_hex(secret.as_bytes()),
            name: "anna".into(),
            expires: 0,
        },
    )
    .unwrap();
    auth::append(
        &path,
        &Entry {
            hash: sha256_hex(b"expired-token"),
            name: "bruno".into(),
            expires: now - 1,
        },
    )
    .unwrap();

    // The file must never hold anything usable: a leaked copy is not a
    // credential.
    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(
        !stored.contains(&secret),
        "the plaintext token must never reach the file"
    );

    let tokens = Tokens::new(path.clone());
    assert_eq!(tokens.identify(Some(&secret), now).as_deref(), Some("anna"));
    assert_eq!(
        tokens.identify(Some("expired-token"), now),
        None,
        "an expired token is refused"
    );
    assert_eq!(tokens.identify(Some(&other), now), None, "unknown token");
    assert_eq!(tokens.identify(None, now), None, "no token at all");
    assert!(tokens.configured());

    let rotated = auth::rotate(&path, "anna", 0).unwrap();
    let rotated_entries = auth::read(&path);
    assert_eq!(
        rotated_entries.iter().filter(|e| e.name == "anna").count(),
        1,
        "rotation replaces a name instead of accumulating live service tokens"
    );
    assert_eq!(
        auth::identify(&rotated_entries, &rotated, now).as_deref(),
        Some("anna"),
        "the atomically written replacement is immediately usable"
    );
    assert_eq!(
        auth::identify(&rotated_entries, &secret, now),
        None,
        "the prior credential is absent from the replacement file"
    );
    assert_eq!(
        tokens.identify(Some(&rotated), now).as_deref(),
        Some("anna"),
        "a live server notices an atomic token-file replacement even at equal mtimes"
    );

    // The acceptance criterion: revoked without a restart. Same `Tokens`
    // instance, so this fails if the file is only read once.
    //
    // The mtime is set back rather than slept on: a revocation landing inside
    // the same filesystem timestamp granularity would otherwise be cached, and
    // that is exactly the case a test must not paper over with a sleep.
    assert_eq!(auth::revoke(&path, "anna").unwrap(), 1);
    let back = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .and_then(|f| f.set_times(std::fs::FileTimes::new().set_modified(back)))
        .unwrap();
    assert_eq!(
        tokens.identify(Some(&secret), now),
        None,
        "a revoked token is refused without a restart"
    );

    // A corrupt line is not a token that matches everything.
    std::fs::write(
        &path,
        "garbage
not	a	valid	hash
# comment
",
    )
    .unwrap();
    assert!(auth::read(&path).is_empty(), "corrupt records are skipped");

    // The HTTP side of the same thing: who gets in, and on which address the
    // server refuses to come up at all.
    let dir2 = std::env::temp_dir().join(format!("glasir-auth2-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir2);
    std::fs::create_dir_all(&dir2).unwrap();
    let cfg = |addr: &str, shared: Option<&str>, root: &std::path::Path| http::HttpConfig {
        metrics: Default::default(),
        addr: addr.to_string(),
        token: shared.map(str::to_string),
        tokens: std::sync::Arc::new(Tokens::new(auth::token_path(root))),
        audit: None,
        public_url: None,
        reindexed: None,
        tls: None,
        behind_control_plane: false,
        control_plane_cidr: None,
        reindex_error: std::sync::Arc::new(std::sync::Mutex::new(None)),
        max_index_age: None,
    };

    // What a refused client is told: RFC 9728 §5.1 asks a 401 to name where
    // the resource metadata lives, or a spec-following client has nothing to
    // act on. Two properties, and the second is the one that bites.
    {
        let mut public = cfg("10.0.0.5:8899", Some("s3cret"), &dir2);
        public.public_url = Some("https://glasir.example.com".into());
        assert_eq!(
            http::resource_uri(&public, Some("glasir.example.com")),
            "https://glasir.example.com/mcp",
            "behind a TLS proxy the operator states the name; the process cannot \
             see it and an untrusted Host must not supply it"
        );
        // The Host header is attacker-controlled. Echoing it into the challenge
        // hands out a redirect under our own name — measured against a running
        // server, `Host: evil.example.com` came back inside the header before
        // this was checked.
        let local = cfg("127.0.0.1:7891", Some("s3cret"), &dir2);
        assert_eq!(
            http::resource_uri(&local, Some("evil.example.com")),
            "http://127.0.0.1:7891/mcp",
            "a Host this server was not reached under falls back to the bind address"
        );
        assert_eq!(
            http::resource_uri(&local, Some("localhost:7891")),
            "http://localhost:7891/mcp",
            "a loopback server answers to its several legitimate names"
        );
    }

    // No auth at all: fine on loopback, refused on an open address. This used
    // to be a warning, which is a line nobody reads.
    let open = cfg("127.0.0.1:9", None, &dir2);
    assert!(!http::refuses_to_start(&open), "loopback stays unguarded");
    assert!(
        http::refuses_to_start(&cfg("0.0.0.0:9", None, &dir2)),
        "an open address with no authentication must refuse to start"
    );
    assert!(
        !http::refuses_to_start(&cfg("0.0.0.0:9", Some("s3cret"), &dir2)),
        "a shared token is enough to bind beyond loopback"
    );
    let mut private_without_service_token = cfg("127.0.0.1:9", Some("s3cret"), &dir2);
    private_without_service_token.behind_control_plane = true;
    assert!(
        http::refuses_to_start(&private_without_service_token),
        "the private data plane must not fall back to a shared token"
    );
    assert_eq!(
        http::authenticate(&open, None),
        Ok(None),
        "nobody is required to identify when nothing is configured"
    );

    let shared = cfg("127.0.0.1:9", Some("s3cret"), &dir2);
    assert_eq!(
        http::authenticate(&shared, Some("Bearer s3cret")),
        Ok(Some("shared-token".into()))
    );
    assert_eq!(http::authenticate(&shared, Some("Bearer wrong")), Err(()));
    assert_eq!(http::authenticate(&shared, None), Err(()));

    // Once per-user tokens exist, the shared one is no longer a second door:
    // an operator who issues tokens has said everyone must be identifiable.
    let secret2 = auth::mint().unwrap();
    auth::append(
        &auth::token_path(&dir2),
        &Entry {
            hash: sha256_hex(secret2.as_bytes()),
            name: "carla".into(),
            expires: 0,
        },
    )
    .unwrap();
    let both = cfg("127.0.0.1:9", Some("s3cret"), &dir2);
    assert_eq!(
        http::authenticate(&both, Some(&format!("Bearer {secret2}"))),
        Ok(Some("carla".into())),
        "a per-user token identifies the person"
    );
    assert_eq!(
        http::authenticate(&both, Some("Bearer s3cret")),
        Err(()),
        "the shared token stops working once per-user tokens exist"
    );
    let mut private = cfg("127.0.0.1:9", None, &dir2);
    private.behind_control_plane = true;
    assert!(
        !http::refuses_to_start(&private),
        "a loopback data plane with its service token may start"
    );
    private.addr = "0.0.0.0:9".into();
    assert!(
        http::refuses_to_start(&private),
        "the data plane must refuse public binds even with a service token"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
    println!("phase B.2 ok: hashed tokens, expiry, revocation without restart");
}

/// B.3: every tool call recorded, and nothing about the recording able to take
/// the server down.
///
/// The interesting half is the failure path. A log that works when the disk is
/// fine is not the promise — the promise is that a full or read-only
/// filesystem costs a record, never a request.
fn demo_audit() {
    let dir = std::env::temp_dir().join(format!("glasir-audit-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = audit::audit_path(&dir);

    let log = audit::Audit::start(path.clone(), "demo-tree".into());
    for i in 0..3 {
        log.record(audit::Record {
            who: "anna".into(),
            tool: "query_graph".into(),
            args: format!("{{\"query\":\"how does compaction work {i}\"}}"),
            bytes: 1200 + i,
            ok: true,
        });
    }
    log.record(audit::Record {
        who: "bruno".into(),
        tool: "impact".into(),
        args: "{\"symbol\":\"nosuchthing\"}".into(),
        bytes: 40,
        ok: false,
    });
    assert!(
        log.flush(std::time::Duration::from_secs(2)),
        "a controlled shutdown can wait for all accepted audit records"
    );

    // The writer is a thread, so wait for the file rather than assuming.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while audit::summary(&path).map(|(n, _)| n).unwrap_or(0) < 4
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let text = std::fs::read_to_string(&path).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 4, "every tool call is recorded");

    // The acceptance criterion, field by field.
    let first = &records[0];
    assert!(first["ts"].as_u64().unwrap() > 1_700_000_000, "timestamped");
    assert_eq!(first["who"], "anna", "identity");
    // Which tree, so several logs can be read together. Without it a merged
    // log says who asked what but not about what — and no later pass can put
    // it back, because the file it came from is the only place it existed.
    assert_eq!(
        first["tree"], "demo-tree",
        "which tree the question was about"
    );
    assert_eq!(first["bytes"], 1200, "result size");
    assert_eq!(first["tool"], "query_graph");
    assert!(
        first["args"].as_str().unwrap().contains("compaction"),
        "the question itself, or \"which part of the code\" is unanswerable"
    );
    assert_eq!(
        records[3]["ok"], false,
        "a rejected call is distinguishable"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the questions people ask are not public");
    }

    // A filesystem that cannot be written must cost records, not requests.
    //
    // Timing alone is too weak a test and was measured to be: a refused `open`
    // returns immediately, so 200 synchronous failed writes finish in
    // microseconds and a synchronous implementation passes. What actually has
    // to hold is that the request path does not do the write at all — so the
    // writer is blocked outright and `record` must still return.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let attempts_before = log.attempts_for_test();
        std::fs::remove_file(&path).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        for _ in 0..50 {
            log.record(audit::Record {
                who: "anna".into(),
                tool: "query_graph".into(),
                args: "{}".into(),
                bytes: 10,
                ok: true,
            });
        }
        // The writer is asynchronous. Waiting for its first failed attempt is
        // the synchronization the assertion needs: restoring permissions
        // immediately races the worker and can turn this into a successful
        // write on a fast or differently scheduled platform.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while log.attempts_for_test() < attempts_before + 50 && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            log.attempts_for_test() >= attempts_before + 50,
            "the audit writer did not attempt the deliberately unwritable destination"
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            !path.exists(),
            "nothing was written, and nothing panicked either"
        );
    }

    // The real promise: `record` hands over and returns, whatever the disk is
    // doing.
    //
    // A refused `open` is the wrong stand-in and was measured to be: it returns
    // in microseconds, so the queue never fills and a *blocking* `send` passes
    // the test too. The writer is therefore wedged outright — which is what a
    // stalled disk actually looks like — and the assert is that 2048 records
    // still cost no wait, because everything past `QUEUE` is dropped.
    {
        let slow = std::env::temp_dir().join(format!("glasir-audit-slow-{}", fixture_id()));
        let _ = std::fs::remove_dir_all(&slow);
        std::fs::create_dir_all(&slow).unwrap();
        let blocked = audit::Audit::start(slow.join("out.jsonl"), "demo-tree".into());
        blocked.stall_for_test(50);
        let t = std::time::Instant::now();
        for _ in 0..2048 {
            blocked.record(audit::Record {
                who: "anna".into(),
                tool: "query_graph".into(),
                args: "x".repeat(512),
                bytes: 10,
                ok: true,
            });
        }
        let elapsed = t.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "recording must hand over and return, not wait on the writer ({elapsed:?})"
        );
        assert!(
            blocked.dropped() > 0,
            "a wedged writer must drop records, not queue them without bound"
        );
        blocked.stall_for_test(0);
        let _ = std::fs::remove_dir_all(&slow);
    }

    // Rotation: one previous generation kept, the live file starts over.
    let big = "x".repeat(600_000);
    for _ in 0..20 {
        log.record(audit::Record {
            who: "anna".into(),
            tool: "query_graph".into(),
            args: big.clone(),
            bytes: 1,
            ok: true,
        });
    }
    let rotated = path.with_extension("jsonl.1");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !rotated.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        rotated.exists(),
        "the log rotates rather than growing forever"
    );
    // Rotation moves the old file aside. A new live file is intentionally
    // created lazily by the next record, rather than leaving an empty file
    // after an otherwise idle service.
    log.record(audit::Record {
        who: "anna".into(),
        tool: "query_graph".into(),
        args: "after rotation".into(),
        bytes: 1,
        ok: true,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        path.exists() && std::fs::metadata(&path).unwrap().len() < 10 * 1024 * 1024,
        "the live file starts over after a rotation"
    );

    let _ = std::fs::remove_dir_all(&dir);
    println!("phase B.3 ok: recorded, non-blocking on a full disk, rotates");
}

/// The stored graph: what makes a second start cheap, and the check that keeps
/// it from serving a tree that has moved on.
fn demo_snapshot() {
    let dir = std::env::temp_dir().join(format!("glasir-snap-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.rs");
    std::fs::write(&src, "fn pay() { charge(); }\nfn charge() {}\n").unwrap();

    let mut b = csr::CsrBuilder::new();
    for _ in 0..2u32 {
        b.add_node(0);
    }
    b.add_edge(
        0,
        csr::Edge {
            target: 1,
            timestamp: 0,
            authority: 1.0,
            edge_kind: 0,
            confidence: csr::Confidence::Extracted,
        },
    );
    let path = dir.join(".glasir-graph");
    let stored = snapshot::Snapshot::new(
        b.build(),
        vec!["a.rs#pay".into(), "a.rs#charge".into()],
        vec![0, 0],
        vec![],
        vec![(0, "Charges the card and reports whether it worked.".into())],
        Vec::new(),
        snapshot::source_times(&dir, std::slice::from_ref(&src)),
    );
    snapshot::write(&stored, &path).unwrap();

    // Names are the reason this exists: a bare CSR stores interned keys whose
    // strings live in an arena that was never written, so reading one back
    // yields a nameless graph.
    let back = snapshot::read(&path, &dir).expect("unchanged tree must load");
    assert_eq!(back.names[0], "a.rs#pay");
    assert_eq!(back.csr.node_count(), 2);
    // Documentation has to survive too. It is not derivable from what else is
    // stored — rebuilding it means re-reading and re-parsing every source,
    // which is the cost the snapshot exists to avoid — so leaving it out made
    // the first run after an analysis find things and every later one not.
    assert_eq!(
        back.docs.first().map(|(n, d)| (*n, d.as_str())),
        Some((0, "Charges the card and reports whether it worked.")),
        "docs must round-trip or search silently degrades on the second run"
    );
    assert_eq!(back.csr.neighbors(0).next().unwrap().target, 1);

    // An edited source invalidates it: serving a graph of code that no longer
    // exists is worse than rebuilding.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&src, "fn pay() { audit(); }\n").unwrap();
    assert!(
        snapshot::read(&path, &dir).is_none(),
        "a changed source must invalidate the snapshot"
    );

    // So does a vanished one.
    std::fs::remove_file(&src).unwrap();
    assert!(snapshot::read(&path, &dir).is_none());

    // **A snapshot from a build with a different `FORMAT_VERSION` must be
    // discarded**, and that is not only about the struct's shape. The stored
    // edges, names, docs and partition are this code's *reading* of the tree,
    // so a fix to how any of them are produced makes an older snapshot wrong
    // while leaving it perfectly parseable. Measured before this was bumped: a
    // binary carrying every audit repair, handed a snapshot written before step
    // 8a, served 27.5% partition purity against 88.5% and announced
    // "http (201 symbols)" where a rebuild says "main (40)"; a pre-step-5 one
    // cost 4 points of recall on two sets. The mtime check cannot see it — the
    // sources never changed, the code did.
    let src2 = dir.join("c.rs");
    std::fs::write(&src2, "fn later() {}\n").unwrap();
    let older = snapshot::Snapshot::new(
        csr::CsrBuilder::new().build(),
        vec![],
        vec![],
        vec![],
        vec![],
        Vec::new(),
        snapshot::source_times(&dir, std::slice::from_ref(&src2)),
    );
    let vpath = dir.join(".glasir-graph-v");
    snapshot::write(&older, &vpath).unwrap();
    assert!(
        snapshot::read(&vpath, &dir).is_some(),
        "a snapshot this build wrote must load back"
    );
    // Flip the stored version and it must be refused, whatever else is intact.
    let mut raw = std::fs::read(&vpath).unwrap();
    let stamp = snapshot::version_offset_for_test(&raw).expect("version is findable");
    raw[stamp] = raw[stamp].wrapping_add(1);
    std::fs::write(&vpath, &raw).unwrap();
    assert!(
        snapshot::read(&vpath, &dir).is_none(),
        "a snapshot from another format version must be discarded, not read"
    );

    // Corruption is discarded like a stale version, but it must not be
    // *silent* the way a stale version is: a missing file and a bumped format
    // are ordinary, while a file that is there and cannot be read means a full
    // disk or a killed write. Both rebuild, so the answer stays right and an
    // operator watching a start that merely got slower has nothing to go on.
    // This pins the persistence boundary: malformed JSON must be rejected
    // before it becomes a graph, and the operator-facing rebuild message is
    // part of that contract.
    let mut wrecked = std::fs::read(&vpath).unwrap();
    for b in wrecked.iter_mut().take(200) {
        *b = b.wrapping_mul(31).wrapping_add(7);
    }
    std::fs::write(&vpath, &wrecked).unwrap();
    assert!(
        snapshot::read(&vpath, &dir).is_none(),
        "a corrupt snapshot must be refused, not deserialized into a wrong graph"
    );

    // The round trip through `analyse`, not just through `Snapshot::new`: the
    // real failure was that the write path never put docs in, so the first run
    // after an analysis searched well and every later one — served from the
    // snapshot — quietly fell back to names alone. Checking the struct alone
    // would have passed throughout.
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(
        tree.join("b.rs"),
        "/// Waits until edits settle before rebuilding.\nfn settle() {}\n",
    )
    .unwrap();
    let fresh = analyse(&tree).expect("first analyse builds from source");
    assert!(!fresh.from_snapshot);
    let doc_of = |a: &Analysed| {
        a.registry
            .entries()
            .find(|(s, _)| s.ends_with("#settle"))
            .and_then(|(_, &n)| a.registry.docs().get(&n).cloned())
    };
    assert!(
        doc_of(&fresh).is_some_and(|d| d.contains("settle")),
        "docs must be parsed on a fresh analysis"
    );
    let mapped = analyse(&tree).expect("second analyse maps the snapshot");
    assert!(mapped.from_snapshot, "second run must use the snapshot");
    assert_eq!(
        doc_of(&mapped),
        doc_of(&fresh),
        "docs must survive the snapshot, or search silently degrades after the first run"
    );

    std::fs::remove_dir_all(&dir).unwrap();
    println!("phase 1 ok: stored graph carries names, docs and expires with its sources");
}

/// Documentation extraction, which is what lets a question phrased in prose
/// reach a symbol at all: measured, names alone recall 8% of what an answer
/// needs against 26% with docs folded in.
///
/// Every language is checked, because the grammars disagree about where
/// documentation lives and a grammar that yields nothing fails silently — the
/// facts come back empty, never as an error. The structural approach is
/// deliberate: only JavaScript and Go define a `@doc` capture in their tags
/// query, so relying on that would mean no docs for Rust.
fn demo_docs() {
    use parse_ast::{Lang, parse};

    // See `docs::NOT_DOCUMENTATION`.
    for (path, want) in [
        ("CLAUDE.md", false),
        ("AGENTS.md", false),
        ("docs/a.md", true),
    ] {
        assert_eq!(
            docs::is_markdown(std::path::Path::new(path)),
            want,
            "{path}"
        );
    }

    for (lang, src, want) in [
        (
            Lang::Rust,
            "/// Charges the card.\n/// Returns whether it worked.\nfn charge() -> bool { true }",
            "Charges the card. Returns whether it worked.",
        ),
        (
            Lang::Python,
            "def charge():\n    \"\"\"Charge the card.\"\"\"\n    return True",
            "Charge the card.",
        ),
        (
            Lang::JavaScript,
            "/** Charge the card. */\nfunction charge() { return true }",
            "Charge the card.",
        ),
        (
            Lang::TypeScript,
            "/** Charge the card. */\nfunction charge(): boolean { return true }",
            "Charge the card.",
        ),
        (
            Lang::Go,
            "// Charge the card.\nfunc charge() bool { return true }",
            "Charge the card.",
        ),
        (
            Lang::Java,
            "class A {\n  /** Charge the card. */\n  boolean charge() { return true; }\n}",
            "Charge the card.",
        ),
        (
            Lang::C,
            "/* Charge the card. */\nint charge(void) { return 1; }",
            "Charge the card.",
        ),
    ] {
        let f = parse(src, lang).unwrap();
        let doc = f
            .docs
            .iter()
            .find(|(n, _)| n == "charge")
            .map(|(_, d)| d.as_str());
        assert_eq!(doc, Some(want), "{lang:?} documentation");
    }

    // Constants carry the reason for their value, and that prose is what a
    // question about a tunable is phrased from. No shipped tags query tags
    // them in Rust, Go, Java or C, so `extra_query` does — measured, adding
    // them took prose recall 51% -> 58% and identifier recall 93% -> 96%,
    // because `COMPACTION_THRESHOLD` and `DEBOUNCE` were previously reference
    // targets with no documentation at all.
    for (lang, src) in [
        (Lang::Rust, "/// Quiet period.\nconst DEBOUNCE: u64 = 250;"),
        (Lang::Rust, "/// Quiet period.\nstatic DEBOUNCE: u64 = 250;"),
        (Lang::Go, "// Quiet period.\nconst DEBOUNCE = 250"),
        (
            Lang::Java,
            "class A {\n  /** Quiet period. */\n  static final int DEBOUNCE = 250;\n}",
        ),
        (Lang::C, "/* Quiet period. */\n#define DEBOUNCE 250"),
        (Lang::Python, "# Quiet period.\nDEBOUNCE = 250"),
        (
            Lang::JavaScript,
            "/** Quiet period. */\nconst DEBOUNCE = 250;",
        ),
    ] {
        let f = parse(src, lang).unwrap();
        assert!(
            f.defines.contains(&"DEBOUNCE".to_string()),
            "{lang:?} must define the constant, got {:?}",
            f.defines
        );
        assert_eq!(
            f.docs
                .iter()
                .find(|(n, _)| n == "DEBOUNCE")
                .map(|(_, d)| d.as_str()),
            Some("Quiet period."),
            "{lang:?} constant documentation"
        );
    }

    // A constant with an initialiser owns the calls in it, rather than leaving
    // them attributed to the module — the definition now has a range.
    let f = parse(
        "const LIMIT: usize = compute_limit();\nfn c() { helper(); }",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        calls_contain(&f.calls, "LIMIT", "compute_limit"),
        "{:?}",
        f.calls
    );
    assert!(calls_contain(&f.calls, "c", "helper"));

    // Joined comments leave their markers mid-line, where a per-line trim
    // cannot reach them: `*/` became a term of its own.
    let f = parse(
        "class A {\n  /** Outer. */\n  void run() { /** Inner. */ helper(); }\n}",
        Lang::Java,
    )
    .unwrap();
    let outer = f
        .docs
        .iter()
        .find(|(n, _)| n == "run")
        .map(|(_, d)| d.as_str());
    assert_eq!(
        outer,
        Some("Outer. Inner."),
        "markers must not survive a join"
    );

    // Comment syntax must not survive into the index: `///` and `*` would
    // otherwise be terms of their own.
    let f = parse("/// Uses the `HashMap` API.\nfn f() {}", Lang::Rust).unwrap();
    let doc = &f.docs[0].1;
    assert!(!doc.contains('/') && !doc.contains('*'), "{doc:?}");

    // Comments inside a body are the largest prose source in a codebase —
    // 43k characters here against the doc comments' 52k — and they belong
    // unambiguously to the definition enclosing them. Measured: folding them
    // in took recall on prose questions from 26% to 35%.
    let f = parse(
        "fn settle() {\n    // Keep collecting while the burst continues.\n    let x = 1;\n}",
        Lang::Rust,
    )
    .unwrap();
    let doc = f
        .docs
        .iter()
        .find(|(n, _)| n == "settle")
        .map(|(_, d)| d.as_str());
    assert_eq!(
        doc,
        Some("Keep collecting while the burst continues."),
        "a body comment is documentation even with no /// above"
    );

    // Both sources land on the same symbol, doc block first.
    let f = parse(
        "/// Settles edits.\nfn settle() {\n    // The burst is still going.\n}",
        Lang::Rust,
    )
    .unwrap();
    assert_eq!(
        f.docs[0].1, "Settles edits. The burst is still going.",
        "doc comment then body comments"
    );

    // A file-level comment describes no single symbol. Attributing it to every
    // symbol in the file was measured and rejected: it gives them all an
    // identical score, so `community.rs#detect` sank into a tie with `refine`,
    // `adj` and `resolution`, and recall fell from 35% to 43-47%... in the
    // wrong direction on every weight tried. The first definition must not
    // silently inherit it either.
    let f = parse(
        "//! This module debounces a burst of editor writes.\n\nfn unrelated() {}",
        Lang::Rust,
    )
    .unwrap();
    assert!(
        f.docs.iter().all(|(_, d)| !d.contains("debounces")),
        "a module comment is not the first function's doc: {:?}",
        f.docs
    );

    // An undocumented definition carries nothing rather than an empty string,
    // so the index can tell "no docs" from "docs that are blank".
    let f = parse("fn bare() {}", Lang::Rust).unwrap();
    assert!(f.docs.is_empty(), "no comment, no doc entry");

    // A comment that documents nothing must not attach to whatever follows it
    // at a distance — only the block directly above a definition counts.
    let f = parse("// unrelated\n\nstruct S;\nfn g() {}", Lang::Rust).unwrap();
    assert!(
        f.docs.iter().all(|(n, _)| n != "g"),
        "a comment above something else is not g's doc: {:?}",
        f.docs
    );
}

/// A file whose edges come from tier 1 must still be read for documentation.
///
/// The cascade skips tier 2 for indexed files, which is right for edges —
/// compiler-resolved supersedes syntactic — but a SCIP index carries no prose.
/// Skipping the file wholesale left every symbol in an indexed tree
/// undocumented, which is most of them, and recall fell from 26% to 18%
/// measured. Nothing in the parser or the index catches that: both are working
/// exactly as designed.
fn demo_doc_coverage() {
    let dir = std::env::temp_dir().join(format!("glasir-doccov-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.rs"),
        "/// Settles a burst of edits into one batch.\nfn settle() {}\n",
    )
    .unwrap();

    let mut reg = ingest::SymbolRegistry::new(0);
    // Stand in for what tier 1 does: define the symbol without any prose.
    let node = reg.get_or_mint("a.rs#settle");
    assert!(reg.docs().get(&node).is_none());

    let src = std::fs::read_to_string(dir.join("a.rs")).unwrap();
    ingest::ingest_docs(&mut reg, &dir.join("a.rs"), &dir, &src);
    assert!(
        reg.docs().get(&node).is_some_and(|d| d.contains("burst")),
        "an indexed file still needs its docs read"
    );
    // It attaches to what already exists and mints nothing: a name tier 1 did
    // not define must not appear as a node just because it has a comment.
    let before = reg.len();
    ingest::ingest_docs(
        &mut reg,
        &dir.join("a.rs"),
        &dir,
        "/// Doc.\nfn other() {}\n",
    );
    assert_eq!(reg.len(), before, "ingest_docs must not mint symbols");

    // And the wiring, against this repository's own tree: `build_graph` must
    // read docs for a file the SCIP index covers. Testing `ingest_docs` alone
    // passes even when nothing calls it — precisely the shape the bug had —
    // and a synthetic tree cannot reproduce it, since coverage only exists
    // when a real index parses. Skipped when there is no index to be covered
    // by, rather than asserting on a condition that is absent.
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if here.join("index.scip").exists()
        && let Ok((_g, reg2, _arena)) = build_graph(here)
    {
        let covered_documented = reg2
            .entries()
            .filter(|(name, n)| {
                // A symbol from an indexed file: qualified, and documented only
                // because ingest_docs went back for it.
                name.starts_with("src/") && reg2.docs().contains_key(n)
            })
            .count();
        assert!(
            // Measured on this tree: 152 documented symbols with the docs
            // pass, 112 without it. The threshold sits between those, so it
            // fails when the pass is dropped rather than merely when
            // everything breaks.
            covered_documented > 130,
            "build_graph must read docs for indexed files too, found {covered_documented}"
        );
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Documentation must inform the ranking without overruling a name. A doc
/// comment is many times longer than an identifier, so counting its words
/// equally lets a function that merely *mentions* `parse` outrank the function
/// actually called `parse`.
fn demo_doc_ranking() {
    use search::SearchIndex;

    let mut reg = ingest::SymbolRegistry::new(0);
    reg.insert("src/a.rs#parse".into(), 0);
    reg.insert("src/b.rs#run".into(), 1);
    reg.insert("src/c.rs#watch".into(), 2);
    let mut docs = std::collections::HashMap::new();
    // Mentions parse repeatedly, but is not named parse.
    docs.insert(
        1,
        "Runs the parse step, then a second parse pass, then parse cleanup.".to_string(),
    );
    // The words a question would use, in prose only.
    docs.insert(
        2,
        "Waits until edits settle before rebuilding, so a burst of saves is one batch.".to_string(),
    );
    let index = SearchIndex::build_with_docs(&reg, &docs);

    let top = |q: &str| index.search(q, 3).first().map(|(n, _)| *n);
    assert_eq!(
        top("parse"),
        Some(0),
        "the symbol named parse must beat one whose docs merely say parse"
    );
    // The whole point: prose reaches a symbol whose name shares no word with
    // the question.
    assert_eq!(
        top("what stops a burst of saves from rebuilding repeatedly"),
        Some(2),
        "documentation must make an unnamed concept findable"
    );
    // Without docs that question finds nothing at all.
    let bare = SearchIndex::build(&reg);
    assert!(
        bare.search("what stops a burst of saves from rebuilding repeatedly", 3)
            .is_empty(),
        "the docs are what make this findable, not the names"
    );
}

/// Scratch space inside a repository must not reach the graph.
///
/// A checkout left under `tmp/` was once indexed as part of this tree and took
/// it from 780 to 10,003 nodes; every measurement taken meanwhile was
/// meaningless. `.gitignore` excluded it and the indexer did not.
///
/// The rule has to be root-relative, which is the part that is easy to get
/// wrong: judging `tmp` anywhere in the path also excludes `/tmp`, where every
/// self-check builds its fixtures. Writing it that way failed eleven checks at
/// once.
fn demo_ignored_paths() {
    use std::path::Path;
    let root = Path::new("/home/u/repo");

    // Scratch space at the top of the tree, ignored.
    assert!(watcher::ignored_under(
        &root.join("tmp/big/src/f.rs"),
        Some(root)
    ));
    // The same name deeper in is a real directory someone may have named that,
    // and excluding it would silently drop their code.
    assert!(!watcher::ignored_under(
        &root.join("src/tmp/f.rs"),
        Some(root)
    ));
    // And outside the tree it means nothing at all — this is where fixtures
    // live, including the ones the checks above build.
    assert!(
        !watcher::ignored_under(Path::new("/tmp/fixture/f.rs"), Some(root)),
        "a system temp dir is not the tree's scratch space — the self-checks live there"
    );
    assert!(!watcher::is_ignored(Path::new("/tmp/fixture/f.rs")));

    // A benchmark fixture tree is excluded from the tree holding it, because
    // it exists to be measured on its own: indexing `bench/deep` as part of
    // this repository took it from 1,200 to 1,305 nodes and made `create
    // contact` answer with the fixture's schema classes, which moves the
    // document frequency behind all four recall floors.
    assert!(watcher::ignored_under(
        &root.join("bench/deep/backend/app/core/errors.py"),
        Some(root)
    ));
    // Root-relative, like `tmp`: the fixture is a normal tree when it is the
    // root, or the set measured against it would be empty.
    let deep = Path::new("/home/u/repo/bench/deep");
    assert!(
        !watcher::ignored_under(&deep.join("backend/app/core/errors.py"), Some(deep)),
        "the fixture must index normally when it is the tree being measured"
    );
    // And the exclusion is a path, not a name: someone else's `deep/` is code.
    assert!(!watcher::ignored_under(
        &root.join("src/deep/f.rs"),
        Some(root)
    ));
    assert!(!watcher::ignored_under(&root.join("deep/f.rs"), Some(root)));
    assert!(!watcher::ignored_under(
        &root.join("docs/architecture.md"),
        Some(root)
    ));

    // Compiled output: a `dist` beside a `package.json` is a copy of `src` in a
    // language we index — measured at 32% of the graph, every node a duplicate
    // competing with its own original for a seed slot. The rule reads the
    // filesystem, so this needs a real tree rather than a path.
    {
        let pkg = std::env::temp_dir().join(format!("glasir-dist-{}", fixture_id()));
        let _ = std::fs::remove_dir_all(&pkg);
        // `apps/api` carries the manifest, exactly as a monorepo lays it out —
        // the first attempt at this rule was root-relative and matched nothing
        // here, which is why the depth is part of the fixture.
        std::fs::create_dir_all(pkg.join("apps/api/dist/modules")).unwrap();
        std::fs::create_dir_all(pkg.join("apps/api/src/dist")).unwrap();
        std::fs::write(pkg.join("apps/api/package.json"), "{}").unwrap();
        assert!(
            watcher::ignored_under(
                &pkg.join("apps/api/dist/modules/contact.service.js"),
                Some(&pkg)
            ),
            "`dist` beside a package.json is build output, at any depth"
        );
        // Without the manifest it is just a directory called `dist`.
        assert!(
            !watcher::ignored_under(&pkg.join("apps/web/dist/x.js"), Some(&pkg)),
            "a `dist` with no package.json beside it is not build output"
        );
        // And a source module that happens to be named `dist` stays indexed,
        // which is what a name-anywhere rule would have cost.
        assert!(
            !watcher::ignored_under(&pkg.join("apps/api/src/dist/pack.ts"), Some(&pkg)),
            "a `dist` module inside a source tree is code, not build output"
        );
        let _ = std::fs::remove_dir_all(&pkg);
    }
    // The same, via the path every fixture in this file actually uses.
    assert!(!watcher::is_ignored(
        &std::env::temp_dir().join(format!("glasir-x-{}/f.rs", fixture_id()))
    ));

    // Build output and dependencies are ignored wherever they appear: a
    // language server writes into `target/` while indexing, and watching it
    // feeds the watcher its own side effects.
    assert!(watcher::is_ignored(Path::new(
        "/anywhere/target/debug/x.rs"
    )));
    assert!(watcher::ignored_under(
        &root.join("a/node_modules/b.js"),
        Some(root)
    ));
    assert!(watcher::is_ignored(Path::new("/x/.git/config")));

    assert!(!watcher::ignored_under(
        &root.join("src/main.rs"),
        Some(root)
    ));

    // A symlinked directory is skipped, not followed. `ln -s . loop` in a tree
    // makes `is_dir()` descend until the path hits the system limit — measured
    // on a one-file tree, that produced 41 symbols from one function. Following
    // a symlinked directory also indexes the same sources twice under two
    // names, which is the same problem without the loop.
    #[cfg(unix)]
    {
        let dir = std::env::temp_dir().join(format!("glasir-sym-{}", fixture_id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.rs"), "fn only() {}\n").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("selflink")).unwrap();
        let files = walk(&dir);
        assert_eq!(
            files.len(),
            1,
            "a symlink loop must not multiply the file list: {files:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Building a tree must not be quadratic in the number of files.
///
/// `Graph::update` copies the pending-mutation buffer on every call so a reader
/// holding a snapshot is never disturbed. That is right while agents query and
/// wrong while a tree is first read in: measured over 1M lines, the per-file
/// cost rose from 24 µs to 9,482 µs as the buffer grew past 280,000 entries,
/// and the background worker loses every race against a loop that writes faster
/// than it rebuilds.
///
/// `update_batch` takes one copy for the whole tree instead. What must hold is
/// that it produces the same graph — the batch is only allowed because nothing
/// can read during a build, not because the result may differ.
fn demo_batch_build() {
    use std::time::Instant;

    let dir = std::env::temp_dir().join(format!("glasir-batch-{}", fixture_id()));
    let build = |files: usize| {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..files {
            std::fs::write(
                dir.join(format!("f{i:04}.rs")),
                format!("fn run{i}() {{ shared(); helper{i}(); }}\nfn shared() {{}}\n"),
            )
            .unwrap();
            if i % 16 == 0 {
                std::fs::write(
                    dir.join(format!("d{i:04}.md")),
                    format!("## R{i}\n\n`run{i}`\n"),
                )
                .unwrap();
            }
        }
        let t = Instant::now();
        let a = analyse(&dir).unwrap();
        (t.elapsed(), a)
    };

    let (_, _) = build(64);
    let (_, big) = build(256);

    // Timing is deliberately not asserted. The quadratic term only separates
    // from the noise past several thousand files — measured at 1,600 the two
    // paths are 4.04x and 4.12x, indistinguishable — and a self-check cannot
    // build a tree that large. What *is* checkable is the cause: the build must
    // publish once, not once per file.
    assert_eq!(
        big.snap.generation(),
        1,
        "a build must publish one snapshot, not one per file"
    );

    // Compare against the per-file path on the same tree. Both fold in
    // walk order, so the graphs must match symbol for symbol and edge for
    // edge — the batch is allowed because nothing can read during a build,
    // not because its result may differ.
    let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
    let mut arena = arena::SymbolArena::new();
    let mut reg = ingest::SymbolRegistry::new(0);
    let mut files = walk(&dir);
    files.sort();
    let mut markdown = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap();
        if docs::is_markdown(path) {
            markdown.push((path.clone(), src));
        } else if let Some(facts) =
            parse_ast::Lang::from_path(path).and_then(|l| parse_ast::parse_file(path, &src, l))
        {
            ingest::apply_facts(&g, &mut arena, &mut reg, path, &dir, facts, 1);
        }
    }
    g.update_batch(|snap, d| {
        ingest::ingest_markdown_into(snap, d, &mut arena, &mut reg, &dir, &markdown, 1);
    });
    link_placeholders_quiet(&g, &reg);
    let snap = g.load();

    let named = |reg: &ingest::SymbolRegistry, snap: &graph::GraphSnapshot| {
        let by_id: std::collections::HashMap<csr::NodeId, String> =
            reg.entries().map(|(s, &n)| (n, s.clone())).collect();
        let mut v: Vec<(String, String)> = by_id
            .iter()
            .flat_map(|(&n, s)| {
                snap.neighbors(n)
                    .filter_map(|e| by_id.get(&e.target).map(|t| (s.clone(), t.clone())))
                    .collect::<Vec<_>>()
            })
            .collect();
        v.sort();
        v.dedup();
        v
    };

    let mut batch_syms: Vec<&String> = big.registry.entries().map(|(s, _)| s).collect();
    let mut file_syms: Vec<&String> = reg.entries().map(|(s, _)| s).collect();
    batch_syms.sort();
    file_syms.sort();
    assert_eq!(batch_syms, file_syms, "same symbols either way");
    assert_eq!(
        named(&big.registry, &big.snap),
        named(&reg, &snap),
        "same edges either way"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Hub detection must not be quadratic in a node's degree.
///
/// `neighbour_cohesion` asks what fraction of a node's neighbour pairs are
/// themselves connected, and the nodes it is asked about are by definition the
/// high-degree ones. On a 1M-line tree the candidates included several with
/// degree 10,000, which came to 1.1 billion pair checks — 11 s, against 2 ms
/// for the Leiden phases the test exists to protect.
///
/// Sampling is safe here because the result is compared against a threshold
/// rather than read: a hub's neighbours have nothing to do with each other and
/// score near zero, a cluster's centre near one. The sample is a fixed stride
/// over a sorted list, so it is the same set on every run.
/// Answering "nothing matched" must not cost more than answering the question.
///
/// The naming list was built with one full scan per group, so the cost was the
/// product of the two counts rather than their sum. On a million-line tree a
/// missed query took **13.5 seconds** while every other tool answered in under
/// 30 ms — slowest exactly when it had least to say. Found by measuring a
/// served monorepo, not by review.
///
/// The assert is on the shape, not the clock, for the same reason
/// `demo_batch_build` counts publishes instead of timing them.
///
/// Deliberately worded without the words a question about the real code would
/// use: this file is indexed too, and a doc comment naming them makes the check
/// outrank what it checks — measured, it cost 8 points of identifier recall.
fn demo_subsystem_scale() {
    let n = 60_000usize;
    let mut b = csr::CsrBuilder::new();
    let mut arena = arena::SymbolArena::new();
    for i in 0..n {
        b.add_node(arena.intern(&format!("src/f{}.rs#sym{i}", i % 500)));
    }
    let snap = graph::Graph::new(b.build()).load();

    let defined: std::collections::HashSet<csr::NodeId> = (0..n as csr::NodeId).collect();
    let mut registry = ingest::SymbolRegistry::new(n);
    for i in 0..n {
        registry.insert(format!("src/f{}.rs#sym{i}", i % 500), i as csr::NodeId);
    }
    let embeddings = embed::embed(&snap, 1);
    let search = search::SearchIndex::build(&registry);
    let names = mcp::name_table(&registry, snap.width());

    // A query that matches nothing takes the subsystem path — the slow one —
    // timed over a partition of `per` nodes per community.
    let missed_query = |per: usize| {
        let communities = community::Communities {
            of_node: (0..n).map(|i| (i / per) as u32).collect(),
            hubs: Vec::new(),
            pendants: Vec::new(),
        };
        let served = mcp::Served {
            snap: &snap,
            names: &names,
            defined: &defined,
            search: &search,
            registry: &registry,
            communities: &communities,
            embeddings: &embeddings,
            physics: physics::Physics::default(),
            now: now(),
            files: None,
            mentions: None,
            references: None,
            root: None,
        };
        let t = std::time::Instant::now();
        let reply = mcp::handle_for_test(
            &served,
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "query_graph", "arguments": {"query": "zzqqxx"}}}),
        );
        assert!(reply.is_some());
        t.elapsed()
    };
    // One community against three thousand, on the same machine: grouping in
    // one pass costs about the same for both, a scan per community about three
    // thousand times as much. A fixed bound in milliseconds measured the
    // machine instead — 130 ms here, 514 ms on a macOS runner against 500.
    let one = missed_query(n);
    let many = missed_query(20);
    assert!(
        many < one * 10 + std::time::Duration::from_millis(50),
        "a missed query walked the partition per community ({many:?} against {one:?} for one)"
    );

    println!("phase A ok: a missed query costs one pass, not one per community");
}

fn demo_cohesion_scale() {
    use std::time::Instant;

    // A star: one centre wired to `n` leaves that do not know each other. That
    // is the shape of a hub, and the worst case for the pair test.
    let star = |n: u32| {
        let mut b = csr::CsrBuilder::new();
        for _ in 0..=n {
            b.add_node(0);
        }
        for leaf in 1..=n {
            b.add_edge(
                0,
                csr::Edge {
                    target: leaf,
                    timestamp: 1_756_600_000,
                    authority: physics::SOURCE_CODE,
                    edge_kind: 0,
                    confidence: csr::Confidence::Extracted,
                },
            );
            // A second edge back, so the centre has incoming edges: a node
            // nothing reaches cannot route between anything and is not a hub.
            b.add_edge(
                leaf,
                csr::Edge {
                    target: 0,
                    timestamp: 1_756_600_000,
                    authority: physics::SOURCE_CODE,
                    edge_kind: 0,
                    confidence: csr::Confidence::Extracted,
                },
            );
        }
        graph::Graph::new(b.build())
    };

    // Best of five, not one run each. The ratio is the measurement, and the
    // denominator is a couple of milliseconds — so one descheduled run on a
    // loaded machine inflates it enough to fail an assert about asymptotics.
    // Measured: the full suite failed this roughly one run in three while the
    // check passed every time on its own. The fastest run is the one least
    // disturbed by everything else on the box, which is exactly what a claim
    // about the algorithm wants to compare.
    let time = |n: u32| {
        let g = star(n);
        let snap = g.load();
        let mut best = std::time::Duration::MAX;
        let mut out = None;
        for _ in 0..5 {
            let t = Instant::now();
            let c = community::detect(
                &snap,
                &community::Params::default(),
                &community::Context::default(),
            );
            best = best.min(t.elapsed());
            out = Some(c);
        }
        (best, out.unwrap())
    };

    let (small, _) = time(2_000);
    let (large, big) = time(8_000);

    // Four times the degree. Bounded work is ~4x (one pass over the
    // neighbours); the full pair test is ~16x. Eight fails the quadratic
    // version and passes the sampled one with room to spare.
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-9);
    assert!(
        ratio < 8.0,
        "cohesion must not be quadratic in degree, got {ratio:.1}x for 4x the degree"
    );

    // And it must still answer the question: the centre of a star is exactly
    // the cross-cutting node hub exclusion exists to find.
    assert!(
        big.hubs.contains(&0),
        "the centre of a star must still be detected as a hub"
    );
}

/// Building the search index must stay linear in the corpus.
///
/// It was not: a documentation word already present in a symbol's name was
/// detected by scanning that word's posting list for the node, and a common
/// word's list is as long as the symbol table. On a 1M-line tree that put 28 s
/// into every start — the single largest cost in the system, and invisible on
/// any tree small enough to develop against.
///
/// Timing is not asserted here, since a loaded machine would make that flaky.
/// What is asserted is the shape: doubling the corpus must roughly double the
/// work, not quadruple it. A quadratic build fails that by a wide margin.
fn demo_index_scale() {
    use std::time::Instant;

    let build = |n: usize| {
        let mut reg = ingest::SymbolRegistry::new(0);
        let mut docs = std::collections::HashMap::new();
        for i in 0..n {
            let node = i as csr::NodeId;
            reg.insert(format!("src/f{i}.rs#handler_{i}"), node);
            // Prose sharing words across every symbol, which is what makes a
            // posting list long: the pathological input for the old scan.
            docs.insert(
                node,
                format!("Handles the request and returns the response for case {i}."),
            );
        }
        let t = Instant::now();
        let index = search::SearchIndex::build_with_docs(&reg, &docs);
        (t.elapsed(), index)
    };

    let (small, _) = build(4_000);
    let (large, index) = build(16_000);

    // Four times the corpus. Linear would be ~4x the time; quadratic ~16x.
    // Eight is a generous ceiling that still fails a quadratic build — the
    // real ratio measured here is close to four.
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-9);
    assert!(
        ratio < 8.0,
        "index build must stay near-linear, got {ratio:.1}x for 4x the corpus"
    );

    // And it must still work: a word only in the prose finds the symbol, and a
    // word in the name outranks one that merely appears in a comment.
    assert!(
        !index.search("response", 3).is_empty(),
        "documentation words must be searchable"
    );
    assert!(!index.search("handler_7", 3).is_empty());
}

/// An edited file must not cost a full re-analysis.
///
/// The snapshot used to be all-or-nothing: one changed mtime discarded it, so a
/// save in a large tree meant re-parsing the tree — a second on a 153k-line
/// repository, and rising. Now only the changed files are re-parsed, measured
/// at 0.16 s against 1.07 s there.
///
/// What must hold is that the incremental result is the one a full build would
/// have produced. Node ids are checked directly, since every later id depends
/// on them; community ids are checked as a *partition* rather than by label,
/// because the numbers are assigned by walking the graph and carry no meaning —
/// every use in the codebase compares them for equality, never against a value.
fn demo_incremental() {
    let dir = std::env::temp_dir().join(format!("glasir-inc-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: &str| std::fs::write(dir.join(name), body).unwrap();
    // Enough files that three changes stay under the churn threshold, which is
    // a fifth of the tree — the same proportion `run_watch` uses to decide an
    // index has drifted too far to patch.
    for i in 0..24 {
        write(
            &format!("m{i:02}.rs"),
            // `only{i}` is called from this file and nowhere else, so it is the
            // placeholder that a deletion strands — see `forget_orphans`.
            &format!("fn run{i}() {{ shared(); only{i}(); }}\nfn shared() {{}}\n"),
        );
    }

    let symbols = |a: &Analysed| {
        let mut v: Vec<String> = a.registry.entries().map(|(s, _)| s.clone()).collect();
        v.sort();
        v
    };
    // What each symbol points at, by name rather than by id — the graph an
    // agent is served, independent of how the nodes happen to be numbered.
    let edges = |a: &Analysed| {
        let name = |n: csr::NodeId| {
            a.registry
                .entries()
                .find(|&(_, &id)| id == n)
                .map(|(s, _)| s.clone())
                .unwrap_or_default()
        };
        let mut v: Vec<(String, String)> = a
            .registry
            .entries()
            .flat_map(|(s, &n)| {
                a.snap
                    .neighbors(n)
                    .map(move |e| (s.clone(), e.target))
                    .collect::<Vec<_>>()
            })
            .map(|(s, t)| (s, name(t)))
            .collect();
        v.sort();
        v.dedup();
        v
    };
    // Groupings, not labels: which symbols share a community.
    let grouping = |a: &Analysed| {
        let mut groups: std::collections::HashMap<u32, Vec<String>> = Default::default();
        for (name, &node) in a.registry.entries() {
            if let Some(&c) = a.communities.of_node.get(node as usize) {
                groups.entry(c).or_default().push(name.clone());
            }
        }
        let mut out: Vec<Vec<String>> = groups
            .into_values()
            .map(|mut v| {
                v.sort();
                v
            })
            .collect();
        out.sort();
        out
    };

    let first = analyse(&dir).unwrap();
    assert!(!first.from_snapshot, "the first run builds from source");

    // Unchanged: the snapshot is used as it stands.
    let again = analyse(&dir).unwrap();
    assert!(again.from_snapshot);
    assert_eq!(symbols(&first), symbols(&again));

    // One file edited. mtime has one-second resolution here, so wait rather
    // than race it — the same reason demo_snapshot sleeps.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(
        "m03.rs",
        "fn run3() { shared(); helper3(); }\nfn shared() {}\n",
    );
    // Two files added at once, so the order they are folded in decides which
    // ids their symbols get — the property the assert below rests on. A single
    // file could not tell a sorted fold from an arbitrary one.
    write("a_new.rs", "fn added_early() { shared(); }\n");
    write("z_new.rs", "fn added_late() { shared(); }\n");
    let patched = analyse(&dir).unwrap();
    assert!(
        patched.from_snapshot,
        "an edit must not discard the snapshot"
    );
    assert!(
        patched.registry.node_of("m03.rs#helper3").is_some()
            || patched.registry.node_of("helper3").is_some(),
        "the new call must be in the graph"
    );
    assert!(
        patched.registry.node_of("a_new.rs#added_early").is_some(),
        "a file the snapshot never saw must be picked up"
    );
    assert!(patched.registry.node_of("z_new.rs#added_late").is_some());
    // And in sorted order, not the order the parse pool happened to finish in.
    // Extraction runs in parallel while the fold stays serial and sorted; only
    // the fold assigns ids, so an unsorted fold would still find every symbol
    // and quietly number them differently on every run. Existence alone cannot
    // see that — this is the assert that can.
    assert!(
        patched.registry.node_of("a_new.rs#added_early")
            < patched.registry.node_of("z_new.rs#added_late"),
        "files must be folded in sorted order, whatever order they were parsed in"
    );

    // The full build it should equal.
    std::fs::remove_file(dir.join(".glasir-graph")).unwrap();
    let rebuilt = analyse(&dir).unwrap();
    assert!(!rebuilt.from_snapshot);
    // The graph must match, not the numbering. A patched snapshot cannot
    // renumber the nodes it kept — that is what makes it cheap — so new
    // symbols land after the existing ones instead of in walk order. Nothing
    // downstream reads an id as a value: names are what a served answer
    // carries, and community ids are only ever compared for equality.
    assert_eq!(
        symbols(&patched),
        symbols(&rebuilt),
        "incremental must find exactly the symbols a full build finds"
    );
    assert_eq!(
        edges(&patched),
        edges(&rebuilt),
        "and the same edges between them"
    );
    assert_eq!(
        grouping(&patched),
        grouping(&rebuilt),
        "and the same partition, whatever the labels happen to be"
    );

    // A deleted file loses its edges *and* its symbols. The path is part of the
    // key, so nothing can reference `m05.rs#run5` once that file is gone —
    // keeping the name left a corpse the search index served beside the real
    // symbol, and the assertion here used to pin exactly that.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::remove_file(dir.join("m05.rs")).unwrap();
    let after = analyse(&dir).unwrap();
    assert!(after.from_snapshot, "a deletion must not discard it either");
    assert!(
        after.registry.node_of("m05.rs#run5").is_none(),
        "a deleted file's symbols must not stay in the registry"
    );
    // **And its placeholders go with it.** `forget_file` drops `file#name`
    // keys; a bare placeholder carries no path, so nothing tied `only5` to the
    // file that minted it and it survived with no edge in either direction.
    // Measured on a 60-file fixture deleting five per round: five orphans per
    // round, thirty after six, and each one seeded a search for its own name.
    assert!(
        after.registry.node_of("only5").is_none(),
        "a placeholder only the deleted file referenced must go too"
    );
    assert!(
        after.registry.node_of("shared").is_some(),
        "a placeholder other files still reference must stay"
    );

    // A move is a delete plus an add, and it is the case that accumulates: the
    // old path is a whole new set of nodes, so a graph that keeps both grows by
    // the moved file on every refactoring while the tree stays the same size.
    // Measured before the fix, on a 60-file fixture moving five per round: 120
    // symbols became 180 over six rounds, and the moved-away copy seeded at
    // 1.000 above its own replacement.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::rename(dir.join("m07.rs"), dir.join("sub/m07.rs")).unwrap();
    let moved = analyse(&dir).unwrap();
    assert!(moved.from_snapshot, "a move must not discard it either");
    assert!(
        moved.registry.node_of("sub/m07.rs#run7").is_some(),
        "the moved file's symbols must be found under the new path"
    );
    assert!(
        moved.registry.node_of("m07.rs#run7").is_none(),
        "and must not still be there under the old one"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Parsing a tree in parallel must produce exactly the graph the serial path
/// produced — not an equivalent one.
///
/// Every node id downstream depends on the order symbols are first seen, so a
/// reordered fold would renumber the graph and change every stored snapshot,
/// every community id and every layout. The split is therefore drawn at the
/// one line where it is safe: parsing is stateless per file and runs in
/// parallel, the fold into registry, arena and delta stays serial and keeps
/// `walk()`'s order.
fn demo_parallel_ingest() {
    use std::path::Path;

    let dir = std::env::temp_dir().join(format!("glasir-par-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Enough files that a thread pool actually interleaves them, and names that
    // collide across files so ordering decides which id each symbol gets.
    for i in 0..24 {
        std::fs::write(
            dir.join(format!("f{i:02}.rs")),
            format!("fn run{i}() {{ helper(); shared(); }}\nfn shared() {{}}\n"),
        )
        .unwrap();
    }

    let ids = |facts_in_order: bool| {
        let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
        let mut arena = arena::SymbolArena::new();
        let mut reg = ingest::SymbolRegistry::new(0);
        let mut files = walk(&dir);
        files.sort();
        // Parse in parallel or serially; fold in walk order either way.
        let parsed: Vec<(std::path::PathBuf, Option<parse_ast::FileFacts>)> = if facts_in_order {
            files
                .iter()
                .map(|p| {
                    let src = std::fs::read_to_string(p).unwrap();
                    (p.clone(), parse_ast::parse(&src, parse_ast::Lang::Rust))
                })
                .collect()
        } else {
            parallel::map_ordered(&files, |p| {
                let src = std::fs::read_to_string(p).unwrap();
                (p.clone(), parse_ast::parse(&src, parse_ast::Lang::Rust))
            })
        };
        for (path, facts) in parsed {
            if let Some(facts) = facts {
                ingest::apply_facts(&g, &mut arena, &mut reg, &path, Path::new(&dir), facts, 1);
            }
        }
        let mut out: Vec<(String, csr::NodeId)> =
            reg.entries().map(|(s, &n)| (s.clone(), n)).collect();
        out.sort();
        out
    };

    let serial = ids(true);
    let parallel = ids(false);
    assert!(!serial.is_empty(), "the fixture must produce symbols");
    assert_eq!(
        serial, parallel,
        "parallel parsing must assign identical ids, not merely find the same symbols"
    );

    // The local worker joins contiguous ranges in input order. The guarantee
    // is load-bearing: it fixes assignment of monotonic node ids.
    let input: Vec<usize> = (0..64usize).collect();
    let order = parallel::map_ordered(&input, |i| i * 2);
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "parallel map must preserve input order"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Markdown as a graph source: sections become nodes, backticked names become
/// edges to the code they name.
///
/// The measured trade, and it is a trade rather than a win: questions whose
/// answer is a document go from 0% to 75% recall — they were unanswerable
/// before, since the documents were not in the graph at all — while questions
/// whose answer is code lose 8 points (58% -> 50%). That second cost is not
/// documents displacing code in the results; damping their seed rank changes
/// nothing, measured across three weights. It is BM25's own statistics moving:
/// adding documents changes the document frequency of every word, so the whole
/// index ranks differently.
fn demo_markdown() {
    let src = "\
# Storage

The base `BaseCsr` is read-only. Every mutation lands in `DeltaStore`.

```rust
// a fenced block is code, not prose
let x = HashMap::new();
```

## Compaction

A worker folds the delta back through `compact`.
";
    let sections = docs::sections(src, "design");
    assert_eq!(
        sections.len(),
        2,
        "one per heading: {:?}",
        sections.iter().map(|s| &s.title).collect::<Vec<_>>()
    );
    assert_eq!(sections[0].title, "Storage");
    assert_eq!(sections[1].title, "Compaction");

    // Backticked names are the mentions; prose words are not. A document that
    // merely uses the word "graph" must not acquire an edge to every symbol
    // named graph, which is why bare words are ignored.
    assert!(sections[0].mentions.contains(&"BaseCsr".to_string()));
    assert!(sections[0].mentions.contains(&"DeltaStore".to_string()));
    assert!(
        !sections[0].mentions.iter().any(|m| m == "read"),
        "prose words are not mentions: {:?}",
        sections[0].mentions
    );

    // A fenced block is an example, not an explanation: indexing it would put
    // the sample's identifiers into the section's terms and bury the prose.
    assert!(
        !sections[0].prose.contains("HashMap"),
        "fenced code must not become prose: {:?}",
        sections[0].prose
    );
    assert!(
        !sections[0].mentions.iter().any(|m| m == "HashMap"),
        "nor a mention"
    );

    // Prose before the first heading is kept under the file's own name, so a
    // document without headings still contributes instead of vanishing.
    let headless = docs::sections("Just a paragraph about `BaseCsr`.\n", "notes");
    assert_eq!(headless.len(), 1);
    assert_eq!(headless[0].title, "notes");
    assert!(headless[0].mentions.contains(&"BaseCsr".to_string()));

    // `fn parse(src)` names `parse` — a signature in backticks is not a symbol.
    let sig = docs::sections("See `parse(src, lang)` and `Lang::from_path`.\n", "n");
    assert!(
        sig[0].mentions.contains(&"parse".to_string()),
        "{:?}",
        sig[0].mentions
    );

    // An overlong section is split at a paragraph break, because a section that
    // holds a whole chapter matches almost any question and then outranks the
    // specific answer. Measured: this repository's roadmap grew a 13,000-
    // character section that cost 6 points on the English documentation set and
    // 12 on the German one, and capping it returned both.
    //
    // The parts keep the heading — they are still that section — and a split
    // only ever lands on a blank line, never mid-sentence.
    let long = format!(
        "## Chapter\n\n{}\n\n{}\n",
        "word ".repeat(300),
        "tail mentioning `BaseCsr`."
    );
    let split = docs::sections(&long, "doc");
    assert!(
        split.len() >= 2,
        "an overlong section must be split, got {}",
        split.len()
    );
    // Each part needs its OWN name. The title becomes the symbol key, so parts
    // sharing one collide on a single node and `set_doc` keeps only the last —
    // measured at 87% of the largest section's prose silently discarded, which
    // turned the split into the data loss it was meant to prevent. The earlier
    // version of this check asserted the titles were *equal* and passed
    // happily; it tested `docs::sections` in isolation and never touched the
    // ingest path where the collision happens.
    let titles: Vec<&String> = split.iter().map(|s| &s.title).collect();
    assert!(
        titles.iter().all(|t| t.starts_with("Chapter")),
        "the parts are still that section: {titles:?}"
    );
    let unique: std::collections::HashSet<&&String> = titles.iter().collect();
    assert_eq!(
        unique.len(),
        titles.len(),
        "parts must not share a name, or they collide on one node: {titles:?}"
    );
    // And the prose must survive the round trip through the registry, which is
    // where the loss actually happened.
    {
        let dir = std::env::temp_dir().join(format!("glasir-split-{}", fixture_id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join("long.md");
        std::fs::write(
            &md,
            format!(
                "## Chapter\n\n{}\n\nthe tail says lighthouse.\n",
                "word ".repeat(300)
            ),
        )
        .unwrap();
        let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
        let mut arena = arena::SymbolArena::new();
        let mut reg = ingest::SymbolRegistry::new(0);
        let src = std::fs::read_to_string(&md).unwrap();
        let batch = [(md.clone(), src)];
        g.update_batch(|snap, d| {
            ingest::ingest_markdown_into(snap, d, &mut arena, &mut reg, &dir, &batch, now());
        });
        let kept: usize = reg
            .docs()
            .values()
            .map(|d| d.split_whitespace().count())
            .sum();
        assert!(
            kept > 250,
            "a split section must keep every part's prose, kept {kept} of ~305 words"
        );
        assert!(
            reg.docs().values().any(|d| d.contains("lighthouse")),
            "the last part's prose must survive too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Scale: the name table is built once per batch rather than per document.
    // Timed against itself, never against a clock; the single publish is
    // `demo_batch_build`'s.
    {
        let dir = std::path::Path::new("/m");
        let run = |docs: usize| {
            let g = std::sync::Arc::new(graph::Graph::new(csr::CsrBuilder::new().build()));
            let mut arena = arena::SymbolArena::new();
            let mut reg = ingest::SymbolRegistry::new(0);
            for i in 0..100_000 {
                reg.get_or_mint(&format!("src/f{i}.rs#s{i}"));
            }
            let batch: Vec<(std::path::PathBuf, String)> = (0..docs)
                .map(|i| (dir.join(format!("d{i}.md")), format!("## T{i}\n\n`s{i}`\n")))
                .collect();
            let t = std::time::Instant::now();
            g.update_batch(|snap, d| {
                ingest::ingest_markdown_into(snap, d, &mut arena, &mut reg, dir, &batch, now());
            });
            t.elapsed()
        };
        let one = run(1);
        let many = run(200);
        assert!(
            many < one * 20,
            "200 documents took {many:?} against {one:?} for one: the name table is rebuilt per document"
        );
    }
    assert!(
        split
            .iter()
            .any(|s| s.mentions.contains(&"BaseCsr".to_string())),
        "a mention past the split point is still attributed"
    );
    // Short sections stay whole: the cap must not fragment ordinary prose.
    let short = docs::sections("## Small\n\nOne paragraph.\n\nAnd another.\n", "doc");
    assert_eq!(short.len(), 1, "a short section is not split");

    // Local instructions are not documentation of the system. They describe
    // this repository's own decisions in the words a question uses, so they
    // outrank the code they describe: measured, indexing them cost 16 points of
    // prose recall and 18 of identifier recall.
    assert!(docs::is_markdown(std::path::Path::new(
        "docs/architecture.md"
    )));
    assert!(!docs::is_markdown(std::path::Path::new("src/main.rs")));
}

/// The benchmark harness itself, because a measurement nobody checks is a
/// number nobody should trust. What must hold: a method that returns the right
/// symbols scores 1.0 recall, one that returns nothing scores 0.0, and the
/// baseline is credited for a symbol only when it opened that symbol's file.
fn demo_bench() {
    let dir = std::env::temp_dir().join(format!("glasir-bench-check-{}", fixture_id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("q.txt");
    std::fs::write(
        &path,
        "# a comment is not a question\n\nhow does alpha work?\n  src/a.rs#alpha\n  src/a.rs#helper\n\nwhat about beta?\n  src/b.rs#beta\n",
    )
    .unwrap();

    let questions = bench::load_questions(&path).unwrap();
    assert_eq!(questions.len(), 2, "a comment must not become a question");
    assert_eq!(questions[0].expected.len(), 2);
    assert_eq!(questions[1].expected[0], "src/b.rs#beta");

    // The baseline opens whole files, so it is credited for every expected
    // symbol living in a file it opened — and for none in a file it did not.
    let files = vec![
        (
            "src/a.rs".to_string(),
            "fn alpha() { helper() }".to_string(),
        ),
        ("src/b.rs".to_string(), "fn beta() {}".to_string()),
    ];
    let hit = bench::grep_answer(&files, &questions[0]);
    assert_eq!(hit.found, 2, "both symbols live in the file it opened");
    // A question whose words appear nowhere opens nothing, and scores nothing:
    // the harness must not hand out credit for an empty result.
    let miss = bench::grep_answer(
        &files,
        &bench::Question {
            text: "zzzz qqqq".into(),
            expected: vec!["src/a.rs#alpha".into()],
        },
    );
    assert_eq!(miss.found, 0, "no file opened, no credit");
    assert_eq!(miss.tokens, 0);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// `overview` answers "what is in here" before a caller knows any identifier.
///
/// Four properties, and every one of them was wrong in the first version —
/// each visible only by reading the output against a tree whose right answer
/// is known.
fn demo_overview() {
    use serde_json::json;

    // Two clusters that share no edge, plus a document section, plus one
    // symbol from a foreign file sitting inside the first cluster. That last
    // one is the trap: it is what made entry points point at the wrong file.
    let mut b = csr::CsrBuilder::new();
    for _ in 0..7u32 {
        b.add_node(0);
    }
    let e = |t| csr::Edge {
        target: t,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    // alpha cluster: 1 and 2 both call 0, so 0 is the way in.
    b.add_edge(1, e(0));
    b.add_edge(2, e(0));
    b.add_edge(2, e(1));
    // 3 lives in another file but clusters here — it must not be offered as
    // alpha's entry point however many callers it has.
    b.add_edge(1, e(3));
    b.add_edge(2, e(3));
    // beta cluster, entirely separate.
    b.add_edge(5, e(4));
    b.add_edge(6, e(4));
    b.add_edge(6, e(5));
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let mut reg = ingest::SymbolRegistry::new(0);
    for (name, id) in [
        ("src/alpha.rs#core", 0u32),
        ("src/alpha.rs#helper", 1),
        ("src/alpha.rs#driver", 2),
        ("src/foreign.rs#shared", 3),
        ("src/beta.rs#core", 4),
        ("src/beta.rs#helper", 5),
        ("src/beta.rs#driver", 6),
    ] {
        reg.insert(name.into(), id);
    }
    let defined: std::collections::HashSet<csr::NodeId> = (0..7).collect();
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let emb = embed::embed(&snap, 3);
    let search = search::SearchIndex::build_with_docs(&reg, reg.docs());
    let names = mcp::name_table(&reg, snap.width());
    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        search: &search,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        physics: physics::Physics::default(),
        now: now(),
        files: None,
        mentions: None,
        references: None,
        root: None,
    };

    // Through the real protocol path rather than a test-only door: what a
    // client receives is the thing being asserted.
    let ask = |args: serde_json::Value| -> String {
        let reply = mcp::handle_for_test(
            &served,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "overview", "arguments": args}}),
        )
        .unwrap();
        assert!(
            !reply["result"]["isError"].as_bool().unwrap_or(false),
            "overview must not error: {reply}"
        );
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    let out = ask(json!({}));

    // 1. It answers without being asked a question — that is the whole point.
    assert!(
        out.contains("alpha") && out.contains("beta"),
        "both subsystems must be named:\n{out}"
    );

    // 2. An entry point belongs to the subsystem it is listed under. The first
    // version drew them from the whole community, so `foreign.rs#shared` — two
    // callers, clustered with alpha — was advertised as the way into alpha.
    for line in out.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("src/") {
            assert!(
                !rest.starts_with("foreign.rs"),
                "an entry point must live in the file its subsystem is named after:\n{out}"
            );
        }
    }

    // 3. Ranked by incoming edges: `core` is called by two others, `driver` by
    // none. A tool that offers a leaf as the way in sends a reader nowhere.
    let entries: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("src/alpha.rs#"))
        .collect();
    assert_eq!(
        entries.first().copied(),
        Some("src/alpha.rs#core (2)"),
        "the most-called symbol is listed first:\n{out}"
    );
    assert!(
        !entries.iter().any(|l| l.contains("driver")),
        "a symbol nothing calls is not an entry point:\n{out}"
    );

    // 4. Deterministic, like every other ranking here.
    let again = ask(json!({}));
    assert_eq!(out, again, "the same tree must produce the same overview");

    println!("phase E.2 ok: overview names the subsystems and the way into each");
}

/// Circular file dependencies. Two things this must get right, both found by
/// running it against a real tree: a cycle routed through the unqualified
/// placeholder tier 2 mints (`a.rs#alpha -> beta -> b.rs#beta`), which is the
/// shape nearly every cross-file call has, and the confidence floor — at
/// `ambiguous` two same-named functions in different files collapse onto one
/// placeholder and invent a cycle between files that never reference each other.
fn demo_cycles() {
    let mut b = csr::CsrBuilder::new();
    for _ in 0..7u32 {
        b.add_node(0);
    }
    let e = |t, c| csr::Edge {
        target: t,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: c,
    };
    // 0 a.rs#alpha, 1 b.rs#beta, 2 c.rs#gamma, then their placeholders 3,4,5.
    b.add_edge(0, e(4, csr::Confidence::Extracted));
    b.add_edge(4, e(1, csr::Confidence::Ambiguous));
    b.add_edge(1, e(5, csr::Confidence::Extracted));
    b.add_edge(5, e(2, csr::Confidence::Ambiguous));
    b.add_edge(2, e(3, csr::Confidence::Extracted));
    b.add_edge(3, e(0, csr::Confidence::Ambiguous));
    // 6 is d.rs#delta, reaching alpha only through a name-matched edge: the
    // collision case, and it must not close a cycle at the default floor.
    b.add_edge(6, e(3, csr::Confidence::Inferred));
    b.add_edge(0, e(6, csr::Confidence::Inferred));
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let mut reg = ingest::SymbolRegistry::new(0);
    reg.insert("src/a.rs#alpha".into(), 0);
    reg.insert("src/b.rs#beta".into(), 1);
    reg.insert("src/c.rs#gamma".into(), 2);
    reg.insert("alpha".into(), 3);
    reg.insert("beta".into(), 4);
    reg.insert("gamma".into(), 5);
    reg.insert("src/d.rs#delta".into(), 6);
    let defined = std::collections::HashSet::from([0, 1, 2, 6]);
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let emb = embed::embed(&snap, 3);
    let search = search::SearchIndex::build_with_docs(&reg, reg.docs());
    let names = mcp::name_table(&reg, snap.width());
    let files_cache = std::sync::OnceLock::new();
    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        search: &search,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        physics: physics::Physics::default(),
        now: 1_756_600_000,
        // The cache on, because that is the path a server takes: the file graph
        // is built once and every floor below filters the *same* pairs. The
        // asserts below then cover it — a cache that stored one caller's
        // filtered answer would hand it to the next.
        files: Some(&files_cache),
        mentions: None,
        references: None,
        root: None,
    };
    let call = |args: serde_json::Value| {
        let r = mcp::handle_for_test(
            &served,
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                "params": {"name": "cycles", "arguments": args}}),
        )
        .unwrap();
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // The cycle runs through placeholders, which belong to no file: reading
    // only qualified names finds nothing at all here.
    let text = call(serde_json::json!({}));
    assert!(
        text.contains("src/a.rs -> src/b.rs -> src/c.rs"),
        "cycle through placeholders, in walk order: {text}"
    );
    // d.rs is reached only by an inferred edge, so it stays out at the default
    // floor and appears once the floor is lowered — the collision guard.
    assert!(
        !text.contains("src/d.rs"),
        "name-matched edge must not count: {text}"
    );
    assert!(text.contains("1 cycle"), "exactly the real one: {text}");
    let loose = call(serde_json::json!({"min_confidence": "inferred"}));
    // Asked again after the cache is warm: a cache holding one caller's
    // filtered answer would repeat it, so the default floor must still refuse
    // `d.rs` *after* a looser call has been served.
    let strict_again = call(serde_json::json!({}));
    assert_eq!(strict_again, text, "the floor must survive a warm cache");
    assert!(
        loose.contains("src/d.rs"),
        "floor must be a real control: {loose}"
    );

    // A cycle is reported once, not once per rotation.
    assert_eq!(
        text.matches("files:").count(),
        1,
        "rotations deduplicated: {text}"
    );

    // max_len is a bound, not decoration: a 3-file cycle is out of reach at 2.
    let short = call(serde_json::json!({"max_len": 2}));
    assert!(short.contains("no cycles"), "max_len must bound: {short}");
}

/// The subsystem rings in the map are named after the file most of their
/// symbols live in. That label lives in the streaming overview, which is what
/// the page draws when zoomed out — it once shipped without one and every ring
/// read "subsystem 12".
fn demo_view() {
    let mut b = csr::CsrBuilder::new();
    for _ in 0..4u32 {
        b.add_node(0);
    }
    let e = |t| csr::Edge {
        target: t,
        timestamp: 1_756_600_000,
        authority: 1.0,
        edge_kind: 0,
        confidence: csr::Confidence::Extracted,
    };
    b.add_edge(0, e(1));
    b.add_edge(1, e(0));
    b.add_edge(2, e(3));
    b.add_edge(3, e(2));
    let g = graph::Graph::new(b.build());
    let snap = g.load();

    let mut reg = ingest::SymbolRegistry::new(0);
    reg.insert("src/pay.rs#charge".into(), 0);
    reg.insert("src/pay.rs#refund".into(), 1);
    reg.insert("src/log.py#warn".into(), 2);
    reg.insert("src/log.py#error".into(), 3);
    let defined = std::collections::HashSet::from([0, 1, 2, 3]);
    let comms = community::detect(
        &snap,
        &community::Params::default(),
        &community::Context::from_names(
            Some(&defined),
            snap.width(),
            reg.entries().map(|(s, &n)| (s.as_str(), n)),
        ),
    );
    let emb = embed::embed(&snap, 3);
    let search = search::SearchIndex::build_with_docs(&reg, reg.docs());
    let names = mcp::name_table(&reg, snap.width());
    let served = mcp::Served {
        snap: &snap,
        names: &names,
        defined: &defined,
        search: &search,
        registry: &reg,
        communities: &comms,
        embeddings: &emb,
        physics: physics::Physics::default(),
        now: 1_756_600_000,
        files: None,
        mentions: None,
        references: None,
        root: None,
    };
    let layout = layout::compute(&snap, &comms.of_node, layout::Mode::Grouped);
    let stored_path = std::env::temp_dir().join(format!("glasir-view-rules-{}", fixture_id()));
    layout::write(&layout, &stored_path).unwrap();
    assert!(layout::read(&stored_path, snap.width()).is_some());
    let stale = layout::StoredLayout {
        parser_rules: "previous rules".into(),
        x: layout.x.clone(),
        y: layout.y.clone(),
    };
    let bytes = serde_json::to_vec(&stale).unwrap();
    std::fs::write(&stored_path, &bytes).unwrap();
    assert!(layout::read(&stored_path, snap.width()).is_none());
    std::fs::remove_file(stored_path).unwrap();
    let ov = view::overview_json(&served, &layout);
    let clumps = ov.as_array().expect("overview is an array");
    assert!(!clumps.is_empty(), "two pairs should form subsystems");
    let labels: Vec<&str> = clumps.iter().filter_map(|c| c["label"].as_str()).collect();
    assert_eq!(labels.len(), clumps.len(), "every clump carries a label");
    // Named after the file, extension stripped — and not only for Rust.
    assert!(
        labels.contains(&"pay") || labels.contains(&"log"),
        "{labels:?}"
    );
    assert!(
        labels.iter().all(|l| !l.contains('.')),
        "extension must be stripped: {labels:?}"
    );
}

/// `glasir langcheck <dir>` — what each language yields on this tree.
///
/// The seven recall floors read this repository, which is Rust: a scanner that
/// stopped finding Julia definitions moves none of them. This reports the three
/// numbers per language that a corpus sweep found thirteen defects with, and
/// with `--check` it fails the build against `bench/languages.txt` the way
/// `benchmark --check` does against `bench/baseline.txt`.
fn run_langcheck(args: &cli::Args) -> std::io::Result<()> {
    let root = canonical(std::path::Path::new(&args.path))?;
    let by_dir = args.has("by-dir");
    let measured = langcheck::measure(&root, by_dir);
    if measured.is_empty() {
        eprintln!("no source files under {}", root.display());
        std::process::exit(2);
    }

    println!(
        "{:<24} {:>6} {:>8} {:>7} {:>7} {:>9} {:>7}",
        "language", "files", "lines", "defs/k", "edges/k", "<module>", "empty"
    );
    for (lang, y) in &measured {
        println!(
            "{:<24} {:>6} {:>8} {:>7.0} {:>7.0} {:>8.0}% {:>7}",
            lang,
            y.files,
            y.lines,
            y.defs_per_kloc(),
            y.edges_per_kloc(),
            y.module_share(),
            y.empty_files
        );
    }

    if !args.has("check") {
        return Ok(());
    }

    // Beside the binary's own tree, never beside the measured one: the
    // corpus is foreign code that has no opinion about our scanners, and
    // `langcheck /some/corpus --check` must read our floors, not look for a
    // file there and fail. Same reason `bench/baseline.txt` is not sought
    // under the tree being benchmarked.
    let path = args
        .value("floors")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("bench/languages.txt"));
    let expected = match langcheck::load_expectations(&path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("\ncannot read {}: {e}", path.display());
            std::process::exit(2);
        }
    };
    // A language in the tree with no expectation is the honest gap, not a
    // failure: fifty-odd of the seventy-two have never been measured against
    // real code, and saying so is the point of printing it.
    let unchecked: Vec<&String> = measured
        .keys()
        .filter(|k| !expected.iter().any(|e| &&e.lang == k))
        .collect();
    if !unchecked.is_empty() {
        println!(
            "\n{} of {} languages have no floor: {}",
            unchecked.len(),
            measured.len(),
            unchecked
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // **A floor whose language is absent from the report is a floor that
    // stopped guarding anything, and it was silent.** The check above covers
    // the opposite direction only. Measured: routing CUDA, GLSL and Apex away
    // from the modules they parse through removed all three from the report
    // entirely and `--check` still reported every floor holding — the sabotage
    // that should have been loudest was the one nothing saw. A fixture can
    // vanish the same way, by a rename or a dropped extension.
    let orphaned: Vec<&str> = expected
        .iter()
        .filter(|e| !measured.contains_key(&e.lang))
        .map(|e| e.lang.as_str())
        .collect();
    if !orphaned.is_empty() {
        eprintln!(
            "\n{} floor(s) guard a language the run did not measure: {}",
            orphaned.len(),
            orphaned.join(", ")
        );
        eprintln!("a language that vanished from the report is a regression, not a gap");
        std::process::exit(1);
    }

    let broken = langcheck::check(&measured, &expected);
    if broken.is_empty() {
        println!("\nevery floor in bench/languages.txt holds");
        return Ok(());
    }
    eprintln!("\n{} floor(s) broken:", broken.len());
    for b in &broken {
        eprintln!("  {b}");
    }
    eprintln!(
        "\nIf the drop is intended, lower the floor in bench/languages.txt in the\n\
         same commit, with the reason. A floor that drifts below what the\n\
         scanners do stops catching anything."
    );
    std::process::exit(1);
}

/// The per-language floors, and that a broken scanner trips one.
///
/// Built as a fixture rather than run against `tmp/corpus`: the corpus is 1.8 GB
/// of gitignored clones that CI does not have, and a check nobody can run is
/// not a check. What the fixture pins is the mechanism — that the `<module>`
/// share reacts to a scanner that stops recognising a definition keyword — with
/// the corpus supplying the *values* in `bench/languages.txt`.
///
/// `<module>` is the sharper of the three: sabotaging Go's `func` on the real
/// corpus took definitions from 45 to 25 per 1,000 lines against a floor of 30,
/// but the module share from 3% to **99%** against a ceiling of 10. A yield
/// number moves by a third when a corpus changes mix; the attribution share
/// does not.
fn demo_langcheck() {
    use crate::langcheck::{Expectation, Yield, check};

    let healthy = Yield {
        files: 2,
        lines: 1000,
        defines: 90,
        calls: 200,
        module_calls: 6,
        empty_files: 0,
    };
    // What a scanner that stopped recognising the definition keyword produces:
    // the calls are still found, they just have nothing to belong to.
    let broken = Yield {
        module_calls: 198,
        defines: 25,
        ..healthy.clone()
    };
    let floors = vec![Expectation {
        lang: "Go".into(),
        min_defs: 30.0,
        min_edges: 170.0,
        max_module: 10.0,
    }];

    let mut m = std::collections::BTreeMap::new();
    m.insert("Go".to_string(), healthy);
    assert!(
        check(&m, &floors).is_empty(),
        "a healthy language must break no floor"
    );

    m.insert("Go".to_string(), broken);
    let reported = check(&m, &floors);
    assert_eq!(reported.len(), 2, "definitions and attribution both break");
    assert!(
        reported.iter().any(|r| r.contains("<module>")),
        "the attribution share must be named: {reported:?}"
    );

    // A language the tree does not hold is not a failure — the floors describe
    // a corpus, and a caller may point this at a subset of one. Without this,
    // `langcheck` on a single Go project would fail on all 23 other languages.
    let mut absent = std::collections::BTreeMap::new();
    absent.insert("Rust".to_string(), Yield::default());
    assert!(
        check(&absent, &floors).is_empty(),
        "a floor for a language that is not present must not fire"
    );

    let path = std::path::Path::new("bench/languages.txt");
    let Ok(expected) = crate::langcheck::load_expectations(path) else {
        println!("langcheck: bench/languages.txt not readable from here");
        return;
    };
    assert!(
        expected.len() >= 25,
        "one floor per language in bench/langs, {} read",
        expected.len()
    );

    // The floors and the fixtures must agree, which is the half a hand-built
    // expectation cannot check: a fixture added with no floor is unguarded,
    // and a floor naming a language no fixture holds never fires. Both are
    // silent, and both are what this file exists to prevent.
    let langs = std::path::Path::new("bench/langs");
    if !langs.is_dir() {
        println!("langcheck: bench/langs not present from here");
        return;
    }
    let measured = crate::langcheck::measure(langs, false);
    for lang in measured.keys() {
        assert!(
            expected.iter().any(|e| &e.lang == lang),
            "bench/langs holds {lang} with no floor in bench/languages.txt"
        );
    }
    for e in &expected {
        assert!(
            measured.contains_key(&e.lang),
            "bench/languages.txt names {} with no fixture in bench/langs",
            e.lang
        );
    }
    assert!(
        check(&measured, &expected).is_empty(),
        "the fixtures must hold their own floors: {:?}",
        check(&measured, &expected)
    );

    // The ceiling is the sharp number, so it must actually bind: every fixture
    // measures 0% on a clean file, and a ceiling loose enough to sit above a
    // broken scanner's share catches nothing. Measured, the three defects this
    // gate found all read 25-29% — hence 30 as the bound a ceiling must stay
    // under. The corpus is where a legitimately high share lives (Ruby's
    // `spec/` at 100%, C headers at 70%); a fixture is written to have none.
    let loose: Vec<&str> = expected
        .iter()
        .filter(|e| e.max_module > 30.0)
        .map(|e| e.lang.as_str())
        .collect();
    assert!(
        loose.is_empty(),
        "a fixture ceiling above 30% cannot catch a scope bug: {loose:?}"
    );
    assert!(
        expected
            .iter()
            .any(|e| e.lang == "Go" && e.max_module <= 15.0),
        "a language with real enclosing definitions keeps a low ceiling"
    );

    // **Every language a file can reach has a fixture, and this is what says
    // so.** The count was quoted for five waves as "N languages" while a fifth
    // of them had no fixture, no floor and no number in CI that moves when
    // they break — and when they were finally measured, seven were defective:
    // PowerShell, F#, Nix, LaTeX, Typst, Cypher and Dockerfile each carried
    // definitions and **zero** edges, and Dockerfile was unreachable outright
    // because it was registered as an extension while the file has none.
    //
    // The exemption is exactly `NOT_CODE`: those extensions are refused in
    // `parse_ast::from_path` before any scanner sees them, so a fixture for
    // one would measure nothing. Anything else without a fixture is a
    // language claimed and not checked.
    let exempt = ["Css", "Html", "Json", "Markdown", "Toml", "Xml", "Yaml"];
    let mut reachable: Vec<String> = native_parsers::EXTENSIONS
        .iter()
        .filter_map(|e| native_parsers::Language::from_extension(e))
        .map(|l| format!("{l:?}"))
        .collect();
    reachable.sort();
    reachable.dedup();
    let unmeasured: Vec<String> = reachable
        .into_iter()
        .filter(|n| !exempt.contains(&n.as_str()))
        .filter(|n| !measured.contains_key(n))
        .collect();
    assert!(
        unmeasured.is_empty(),
        "a language with no fixture is a language nothing checks: {unmeasured:?}"
    );
}
fn demo_extension_collision() {
    let cases = [
        // (path, content, must parse)
        ("a.v", "Definition mynat := nat.", false),
        ("a.v", "From basic Require Import foo.", false),
        ("a.v", "module counter(input clk); endmodule", true),
        ("a.m", "#import \"AppDelegate.h\"\n@interface A @end", false),
        ("a.m", "@import AppKit;\nint main() { return 0; }", false),
        ("a.m", "function y = double_it(x)\n  y = 2 * x;\nend", true),
        ("a.d", "provider julia {\n  probe gc__begin();\n};", false),
        ("a.d", "void charge() { notify(); }", true),
        // `.res` inverts the test: a binary resource has no marker to match
        // on, so ReScript must identify itself instead. Measured, **not one**
        // of 51 `.res` files in a 1.8 GB corpus is ReScript — Godot's binary
        // resources, Windows resource files and Scala test expectations all
        // claim the extension.
        ("a.res", "let charge = (owner) => refuse(owner)", true),
        ("a.res", "RSRC\u{0}\u{0}NavigationMesh\u{0}vertices", false),
        ("a.res", "t8871/tag.scala\nt8871/usetag.scala", false),
        // A multi-byte character straddling the 4,096-byte cut: slicing there
        // panics, and a Latin-1 resource file produced exactly that on the
        // corpus. The padding puts the `¢` across the boundary.

        // An unambiguous extension is never tested against a marker: a Rust
        // file may legitimately contain the word `Definition ` in a comment.
        ("a.rs", "/// Definition of a charge.\nfn charge() {}", true),
    ];
    for (name, src, want) in cases {
        let path = std::path::Path::new(name);
        let lang = parse_ast::Lang::from_path(path).expect(name);
        let got = parse_ast::parse_file(path, src, lang).is_some();
        assert_eq!(
            got, want,
            "{name}: {src:?} — expected parse: {want}, got: {got}"
        );
    }
    // The head is cut at 4,096 bytes, and a multi-byte character straddling
    // that offset panics on a bare slice. A Latin-1 resource file produced
    // exactly that on the corpus — `¢` opening at byte 4,095 — so the cut
    // walks back to a boundary. Built here rather than in the table above,
    // which takes `&'static str`.
    let mut straddle = "x".repeat(4095);
    straddle.push('\u{a2}');
    straddle.push_str(" void charge() { notify(); }");
    let path = std::path::Path::new("a.d");
    let lang = parse_ast::Lang::from_path(path).expect("a.d");
    assert!(
        parse_ast::parse_file(path, &straddle, lang).is_some(),
        "a character across the 4 KiB head must not panic or refuse the file"
    );

    println!("phase C.1 ok: an ambiguous extension is resolved by content, not by the table");
}
