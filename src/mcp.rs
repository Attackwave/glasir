//! MCP server over stdio: the gateway agents actually reach the graph through.
//!
//! Dual-era, because the protocol changed shape. Revisions up to `2025-11-25`
//! open with an `initialize` handshake and hold session state; `2026-07-28`
//! removed the handshake and carries version and identity as `_meta` on every
//! request instead. Both are answered here — a request with modern `_meta` (or
//! `server/discover`) is served statelessly, an `initialize` selects legacy
//! semantics — because current editors are legacy clients while the
//! specification has already moved on.
//!
//! Transport is line-delimited JSON-RPC on stdin/stdout, so **nothing may ever
//! print to stdout except a response**: a stray `println!` corrupts the stream
//! and the client sees a parse error rather than a message. Diagnostics go to
//! stderr.

use crate::community::Communities;
use crate::csr::{Confidence, NodeId};
use crate::embed::Embeddings;
use crate::graph::GraphSnapshot;
use crate::ingest::SymbolRegistry;
use crate::physics::{self, Physics};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// How many nodes a lexical search may seed the expansion with.
///
/// Measured, holding the node budget fixed so this is not just "return more":
/// at 8 seeds a question recalls 26% of the symbols an answer needs, at 24 it
/// recalls 43%, and past 24 the curve is flat. A prose question spreads its
/// evidence over many weakly-matching names — no single one is the obvious
/// entry point the way an identifier is — so a handful of seeds lands in one
/// corner of the graph and expansion never reaches the rest.
const SEED_LIMIT: usize = 24;

/// What `query_graph` returns unless a caller says otherwise. Named so the
/// benchmark measures the same budget a client actually gets.
pub const DEFAULT_MAX_NODES: usize = 40;
pub const DEFAULT_MAX_HOPS: usize = 3;

/// What a caller may ask for, whatever they send.
///
/// `max_nodes` and `max_hops` are request arguments, and an authenticated
/// caller could set them to anything: measured, `max_nodes: 100_000_000` on a
/// 300,000-line tree costs nothing today, because `physics::expand` cuts a
/// branch scoring below its threshold and the answer is the same 6 KB either
/// way. So this closes no leak — it makes the promise explicit rather than
/// leaving it to a scoring constant that exists for a different reason. A
/// limit a caller can lift is not a limit, and the next tuning of that
/// threshold should not silently become a denial-of-service surface.
///
/// Ten times the defaults: enough that a caller wanting a wider answer gets
/// one, bounded enough that the cost stays in the range this server is built
/// for.
pub const MAX_NODES_CEILING: usize = DEFAULT_MAX_NODES * 10;
pub const MAX_HOPS_CEILING: usize = DEFAULT_MAX_HOPS * 10;

/// Legacy revision we answer an `initialize` handshake with.
const LEGACY_VERSION: &str = "2025-06-18";
/// Modern revision, carried per request rather than negotiated.
const MODERN_VERSION: &str = "2026-07-28";

/// Everything the tools read. Built once, then served read-only.
pub struct Served<'a> {
    pub snap: &'a GraphSnapshot,
    /// Symbol name per node id.
    ///
    /// A flat table rather than a scan of the registry: `name` is called for
    /// every returned node and again for the vocabulary, and the registry is a
    /// HashMap keyed the other way round. Measured on a million-line tree —
    /// 200,000 symbols — one `query_graph` took **15 seconds** while every
    /// other tool answered in under 30 ms. The fifth quadratic wall in this
    /// project, and the same shape as the other four: invisible on a
    /// development tree, fatal on a real one.
    pub names: &'a [String],
    /// Nodes the indexed tree defines. References to anything else are ranked
    /// far below, so a result is symbols to read rather than standard-library
    /// names.
    pub defined: &'a std::collections::HashSet<NodeId>,
    pub registry: &'a SymbolRegistry,
    pub communities: &'a Communities,
    pub embeddings: &'a Embeddings,
    /// Lexical index over symbol names, for queries that are a question rather
    /// than an identifier.
    pub search: &'a crate::search::SearchIndex,
    pub physics: Physics,
    pub now: u64,
    /// Where `file_graph()` memoises. `None` in fixtures, which rebuild.
    pub files: Option<&'a std::sync::OnceLock<FileGraph>>,
    /// Where `mentions()` memoises. `None` in fixtures, which rebuild.
    pub mentions: Option<&'a std::sync::OnceLock<Mentions>>,
    /// Where `references()` memoises. `None` in fixtures, which rebuild.
    pub references: Option<&'a std::sync::OnceLock<References>>,
    /// The indexed tree, for the one tool that reads a file rather than the
    /// graph. Symbol names are root-relative — that is what keeps an absolute
    /// path out of every answer — so returning source needs the root back.
    /// `None` where there is no tree on disk, and the tool then refuses.
    pub root: Option<&'a std::path::Path>,
}

/// The same thing, owning its parts, so it can live behind an `ArcSwap` and be
/// replaced while clients are reading.
///
/// `Served` borrows; a server that re-indexes needs a value it can swap. Rather
/// than making every tool generic over ownership, this owns the pieces and
/// lends a `Served` per request — the borrow lasts one call, the swap replaces
/// the whole state at once, and no reader ever sees a half-updated graph.
/// What a cycle report is about: files, and which depends on which.
///
/// Derived from `snap`, `registry` and `defined`, all replaced together on a
/// re-index, so it cannot go stale. Held per state because folding symbol
/// edges onto pairs was 66 ms of `cycles`' 66 ms at 1M lines — the search
/// itself is 40 µs. `edges` keeps the strongest confidence per pair, so a
/// floor still filters, against thousands rather than 400,000.
pub struct FileGraph {
    /// File paths, sorted, indexed by the positions used in `edges`.
    pub files: Vec<String>,
    /// `(from, to) -> strongest confidence on any edge between those files`.
    pub edges: std::collections::HashMap<(usize, usize), Confidence>,
}

pub struct ServedState {
    pub snap: std::sync::Arc<GraphSnapshot>,
    pub names: Vec<String>,
    pub defined: std::collections::HashSet<NodeId>,
    pub registry: SymbolRegistry,
    pub communities: Communities,
    pub embeddings: Embeddings,
    pub search: crate::search::SearchIndex,
    pub physics: Physics,
    pub now: u64,
    /// The file graph, built on first use. See `FileGraph`.
    pub files: std::sync::OnceLock<FileGraph>,
    /// Which files name each uniquely defined symbol, built on first use.
    pub mentions: std::sync::OnceLock<Mentions>,
    /// Who uses each symbol without calling it, built on first use.
    pub references: std::sync::OnceLock<References>,
    pub root: std::path::PathBuf,
}

/// Builds the node-id -> name table the tools read.
///
/// One pass over the registry instead of a scan per lookup.
pub fn name_table(registry: &SymbolRegistry, width: usize) -> Vec<String> {
    let mut names = vec![String::new(); width];
    for (symbol, &node) in registry.entries() {
        if let Some(slot) = names.get_mut(node as usize) {
            *slot = symbol.clone();
        }
    }
    names
}

impl ServedState {
    pub fn as_served(&self) -> Served<'_> {
        Served {
            snap: &self.snap,
            names: &self.names,
            defined: &self.defined,
            registry: &self.registry,
            communities: &self.communities,
            embeddings: &self.embeddings,
            search: &self.search,
            physics: self.physics,
            now: self.now,
            files: Some(&self.files),
            mentions: Some(&self.mentions),
            references: Some(&self.references),
            root: Some(&self.root),
        }
    }
}

impl Served<'_> {
    /// Qualified symbol for a node, or a synthetic name if it has none.
    pub fn name(&self, node: NodeId) -> String {
        self.names
            .get(node as usize)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("node:{node}"))
    }

    /// What this tree's subsystems are called, largest first.
    ///
    /// Named after the file most of a community's symbols live in — the same
    /// honest label the map uses. This is the orientation a caller needs when
    /// a question matched nothing at all: not "no results", but "here is the
    /// vocabulary this codebase is written in".
    fn subsystem_names(&self, limit: usize) -> Vec<String> {
        // One pass to group defined nodes by community, rather than
        // `members(cid)` per community — that scans every node once per
        // community, which is O(communities x nodes). Measured on a
        // million-line tree: this path took **13 seconds** and it runs exactly
        // when a query matched nothing, so the tool was slowest at the moment
        // it had the least to say. The fifth quadratic wall in this project,
        // and again invisible on a development tree.
        let mut by_community: std::collections::HashMap<u32, Vec<NodeId>> = Default::default();
        for (node, &cid) in self.communities.of_node.iter().enumerate() {
            let node = node as NodeId;
            if self.defined.contains(&node) {
                by_community.entry(cid).or_default().push(node);
            }
        }
        let mut sized: Vec<(usize, String)> = Vec::new();
        // Deterministic: a HashMap has no order, and this list is user-visible.
        let mut cids: Vec<u32> = by_community.keys().copied().collect();
        cids.sort_unstable();
        for cid in cids {
            let members = &by_community[&cid];
            if members.len() < 2 {
                continue;
            }
            // The file most of them live in, stripped to its stem.
            let mut counts: std::collections::HashMap<String, usize> = Default::default();
            for &m in members {
                if let Some((file, _)) = self.name(m).split_once('#') {
                    let stem = file.rsplit('/').next().unwrap_or(file);
                    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
                    *counts.entry(stem.to_string()).or_insert(0) += 1;
                }
            }
            let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
            // Ties on name, so the list is the same on every run.
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            if let Some((label, _)) = ranked.first() {
                sized.push((members.len(), label.clone()));
            }
        }
        sized.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out: Vec<String> = Vec::new();
        for (_, label) in sized {
            if !out.contains(&label) {
                out.push(label);
            }
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// Resolves a query to exactly one node, or says why it could not.
    ///
    /// Deliberately stricter than `seeds`: that one falls back to lexical
    /// scoring, which is right for a question and wrong here. A blast radius
    /// computed from a typo would answer "nothing depends on this" about a
    /// symbol with sixteen dependents — indistinguishable from a genuine zero,
    /// and the one answer a change-impact tool must never give by accident.
    /// An ambiguous name is reported with its candidates rather than resolved.
    fn resolve_one(&self, query: &str) -> Result<NodeId, String> {
        // An exact key wins — but a *bare* one is a placeholder, and a
        // placeholder stands for every definition of that name. Taking it
        // silently answered a different question than the caller asked:
        // measured, `impact compact` reported 22 dependents against
        // `src/graph.rs#compact`'s 10 and `impact new` 284, and the caller was
        // never told which. 37% of the placeholders on this tree also name a
        // real definition (38% in office4u, 16% in epoch-engine), so the case
        // is common rather than exotic. Fall through to the ambiguity check,
        // which already refuses and lists the candidates.
        if let Some(node) = self.registry.node_of(query)
            && query.contains('#')
        {
            return Ok(node);
        }
        let mut exact = Vec::new();
        let mut partial = Vec::new();
        for (symbol, &node) in self.registry.entries() {
            // `split_once`, not `rsplit`: the path is built first, so the
            // *first* `#` is the separator. A Markdown heading may hold one of
            // its own — `#` is how issues and pull requests are written, and
            // 14 of 1,869 headings across 35 real repositories carry one. With
            // `rsplit`, a section titled `Notes #compact` was an exact match
            // for the function `compact` and `impact` refused a question it
            // could answer, naming a paragraph of prose as the other candidate.
            let tail = symbol.split_once('#').map_or(symbol.as_str(), |(_, t)| t);
            if tail == query {
                exact.push(node);
            } else if symbol.contains(query) {
                partial.push(node);
            }
        }
        let mut candidates = if exact.is_empty() { partial } else { exact };
        candidates.sort_unstable();
        match candidates.len() {
            0 => Err(format!(
                "unknown symbol: {query:?} — nothing in the graph carries that \
name, which is not the same as nothing depending on it"
            )),
            1 => Ok(candidates[0]),
            _ => {
                let names: Vec<String> = candidates.iter().take(8).map(|&n| self.name(n)).collect();
                Err(format!(
                    "{query:?} matches {} symbols; name one of them:\n  {}",
                    candidates.len(),
                    names.join("\n  ")
                ))
            }
        }
    }

    /// Resolves a query string to seed nodes.
    ///
    /// Precision first: an exact symbol name wins outright, then a bare name,
    /// then a substring. Only when none of those hit does the query get scored
    /// lexically — someone who types an identifier means that identifier, and
    /// ranking it against every other name would bury it.
    pub fn seeds(&self, query: &str) -> Vec<(NodeId, f32)> {
        if let Some(node) = self.registry.node_of(query) {
            return vec![(node, 1.0)];
        }
        let mut exact = Vec::new();
        let mut partial = Vec::new();
        // A substring test needs something to be a substring *of*. Below this,
        // `symbol.contains(query)` is true of nearly every name and the branch
        // returns the eight lowest-numbered nodes — which are the ones parsed
        // first, not the ones that match. Measured: the empty query answered
        // with eight symbols at 0.800 and `query_graph` served them as a
        // result, so the "nothing matched, here is this tree's vocabulary"
        // path — the one written for exactly this case — could never fire.
        // One letter did the same: `a` returned `build.rs#main` and
        // `make_tree.py#FUNCS_PER_FILE`.
        const MIN_SUBSTRING: usize = 3;
        for (symbol, &node) in self.registry.entries() {
            // The first `#` is the separator — see `resolve_one`.
            let tail = symbol.split_once('#').map_or(symbol.as_str(), |(_, t)| t);
            if tail == query {
                exact.push(node);
            } else if query.len() >= MIN_SUBSTRING && symbol.contains(query) {
                partial.push(node);
            }
        }
        // Deterministic: the registry is a HashMap, so iteration order is not.
        exact.sort_unstable();
        partial.sort_unstable();
        if !exact.is_empty() {
            exact.truncate(8);
            return exact.into_iter().map(|n| (n, 1.0)).collect();
        }
        if !partial.is_empty() {
            partial.truncate(8);
            // A substring is weaker evidence than a whole name.
            return partial.into_iter().map(|n| (n, 0.8)).collect();
        }
        // A question, not an identifier: score it against symbol words and
        // carry that confidence into the expansion, normalised against the best
        // hit so scores stay comparable across queries.
        let hits = self.search.search(query, SEED_LIMIT);
        let best = hits
            .first()
            .map(|(_, s)| *s)
            .unwrap_or(1.0)
            .max(f32::EPSILON);
        hits.into_iter()
            .map(|(node, score)| (node, (score / best).clamp(0.05, 1.0)))
            .collect()
    }
}

/// The tools this server exposes. Every one reads the graph or the working
/// tree and changes nothing, so a client in a read-only mode may call them all.
fn tool_definitions() -> Value {
    let mut tools = tool_list();
    for tool in tools.as_array_mut().into_iter().flatten() {
        tool["annotations"] = json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        });
    }
    tools
}

fn tool_list() -> Value {
    json!([
        {
            "name": "get_code_snippet",
            "title": "The source a symbol names",
            "description": "Return the definition's own text for a symbol, from the \
    range the parser recorded when the graph was built. Use it after `query_graph` \
    or `impact` names something worth reading: the answer is the definition itself, \
    so no file has to be opened and no line number has to be guessed. A name that \
    does not resolve uniquely is refused with its candidates, exactly as `impact` \
    refuses one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Qualified `path#name`, or a name that resolves uniquely"}
                },
                "required": ["symbol"]
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "file": {"type": "string", "description": "Relative to the indexed root"},
                    "line": {"type": "integer", "description": "1-based, where the definition starts"},
                    "bytes": {"type": "array", "items": {"type": "integer"},
                        "description": "Start and end offset in the file"},
                    "source": {"type": "string"}
                },
                "required": ["symbol", "file", "line", "bytes", "source"]
            }
        },
        {
            "name": "query_graph",
            "title": "Query the code graph",
            "description": "Find the subgraph relevant to a symbol or a question. \
    Expands from matching nodes and filters by recency, provenance and distance, \
    returning only what is worth reading. Takes a symbol name (`pay`, \
    `src/checkout.rs#pay`), a substring, or a plain-language question — a name is \
    matched exactly first, a question is scored against identifiers and the prose \
    documenting them.\n\nSearch here is lexical, so it finds what a codebase calls \
    things, not what you called them. Every result ends with the words that code is \
    actually written in; if they suggest this tree uses different terms for your \
    concept, ask again with those. One re-query costs far less than reading files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Symbol name, substring, or a plain-language question"},
                    "max_nodes": {"type": "integer", "description": "Result cap (default 40)"},
                    "max_hops": {"type": "integer", "description": "Expansion depth (default 3)"}
                },
                "required": ["query"]
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "returned": {"type": "integer"},
            "total": {"type": "integer", "description": "Nodes in the graph"},
            "filtered_out_percent": {"type": "number"},
            "nodes": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Qualified as path#name"},
                    "file": {"type": "string", "description": "Empty for a bare placeholder"},
                    "score": {"type": "number"},
                    "hops": {"type": "integer", "description": "Distance from a seed"},
                    "community": {"type": "integer"}
                },
                "required": ["symbol", "file", "score", "hops", "community"]
            }},
            "vocabulary": {"type": "array", "items": {"type": "string"},
                "description": "This tree's own words for the query"}
        },
        "required": ["returned", "total", "filtered_out_percent", "nodes", "vocabulary"]
    }
        },
        {
            "name": "overview",
            "title": "What is in this repository",
            "description": "The parts of this tree and the way into each one, \
    without asking a question first. Start here when you do not yet know what \
    a codebase calls things — the other tools need that, this one supplies it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "How many subsystems (default 12)"},
                    "entry_points": {"type": "integer", "description": "Entry points per subsystem (default 3)"}
                },
                "required": []
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "symbols": {"type": "integer"},
            "files": {"type": "integer"},
            "shown": {"type": "integer", "description": "How many the text renders; the array is complete"},
            "subsystems": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The file most of its symbols live in"},
                    "symbols": {"type": "integer"},
                    "entry_points": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "symbol": {"type": "string"},
                            "callers": {"type": "integer"}
                        },
                        "required": ["symbol", "callers"]
                    }}
                },
                "required": ["name", "symbols", "entry_points"]
            }}
        },
        "required": ["symbols", "files", "subsystems"]
    }
        },
        {
            "name": "shortest_path",
            "title": "Path between two symbols",
            "description": "Shortest call path between two symbols, preferring \
    compiler-resolved edges over guessed ones. Answers 'how does A reach B'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": {"type": "string"},
                    "to": {"type": "string"},
                    "min_confidence": {
                        "type": "string",
                        "enum": ["extracted", "inferred", "ambiguous"],
                        "description": "Weakest edge kind to walk (default ambiguous)"
                    }
                },
                "required": ["from", "to"]
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "from": {"type": "string"},
            "to": {"type": "string"},
            "min_confidence": {"type": "string"},
            "found": {"type": "boolean", "description": "False when no route exists, which is an answer"},
            "hops": {"type": ["integer", "null"]},
            "path": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "via": {"type": ["string", "null"],
                        "description": "Confidence of the edge that reached it; null at the start"}
                },
                "required": ["symbol", "via"]
            }}
        },
        "required": ["from", "to", "min_confidence", "found", "hops", "path"]
    }
        },
        {
            "name": "impact",
            "title": "What breaks if I change this",
            "description": "Everything that depends on a symbol, transitively: the \
    callers and the definitions using it by name, then their callers, and so on, \
    grouped by distance. Answers 'what do I have to \
    check before changing this'. Each dependent is tagged with how the edge reaching it \
    was resolved.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Symbol name or qualified path#name"},
                    "depth": {"type": "integer", "description": "How many hops back to follow (default 3)"},
                    "min_confidence": {
                        "type": "string",
                        "enum": ["extracted", "inferred", "ambiguous"],
                        "description": "Weakest edge kind to follow (default ambiguous)"
                    },
                    "limit": {"type": "integer", "description": "How many dependents to name (default 200)"},
                    "offset": {"type": "integer", "description": "Skip this many, nearest first: the `next_offset` of a previous answer"}
                },
                "required": ["symbol"]
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "symbol": {"type": "string"},
            "dependents": {"type": "integer"},
            "within": {"type": "integer", "description": "Hops the dependents span"},
            "next_offset": {"type": "integer", "description": "Present when more remain"},
            "min_confidence": {"type": "string"},
            "hops": {"type": "array", "description": "Grouped by distance: a direct caller almost certainly breaks",
                "items": {
                    "type": "object",
                    "properties": {
                        "hop": {"type": "integer"},
                        "symbols": {"type": "array", "items": {
                            "type": "object",
                            "properties": {
                                "symbol": {"type": "string"},
                                "confidence": {"type": "string"}
                            },
                            "required": ["symbol", "confidence"]
                        }}
                    },
                    "required": ["hop", "symbols"]
                }}
        },
        "required": ["symbol", "dependents", "within", "min_confidence", "hops"]
    }
        },
        {
            "name": "cycles",
            "title": "Circular dependencies between files",
            "description": "Files that depend on each other in a loop, tightest \
    first. A cycle is where a change cannot be made in one place, so these are the \
    knots worth untying before a refactor.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_len": {"type": "integer", "description": "Longest cycle to report, in files (default 5)"},
                    "limit": {"type": "integer", "description": "How many cycles to return (default 20)"},
                    "min_confidence": {
                        "type": "string",
                        "enum": ["extracted", "inferred", "ambiguous"],
                        "description": "Weakest edge to trust (default extracted: name-matched edges invent cycles between unrelated files)"
                    }
                },
                "required": []
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "min_confidence": {"type": "string"},
            "max_len": {"type": "integer"},
            "cycles": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "files": {"type": "array", "items": {"type": "string"},
                        "description": "In loop order; the first is not repeated at the end"},
                    "length": {"type": "integer"}
                },
                "required": ["files", "length"]
            }}
        },
        "required": ["min_confidence", "max_len", "cycles"]
    }
        },
        {
            "name": "explain_node",
            "title": "Explain one symbol",
            "description": "Callers, callees, subsystem and structurally similar \
    symbols for one node, each edge tagged with how it was resolved.",
            "inputSchema": {
                "type": "object",
                "properties": {"symbol": {"type": "string"}},
                "required": ["symbol"]
            },
            "outputSchema": {
        "type": "object",
        "properties": {
            "symbol": {"type": "string"},
            "role": {"type": "string", "enum": ["member", "connector"]},
            "community": {"type": "integer"},
            "siblings": {"type": "array", "items": {"type": "string"}},
            "calls": {"type": "array", "items": {"$ref": "#/$defs/edge"}},
            "called_by": {"type": "array", "items": {"$ref": "#/$defs/edge"}},
            "structurally_similar": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "score": {"type": "number"}
                },
                "required": ["symbol", "score"]
            }}
        },
        "required": ["symbol", "role", "community", "siblings", "calls", "called_by",
                     "structurally_similar"],
        "$defs": {"edge": {
            "type": "object",
            "properties": {
                "symbol": {"type": "string"},
                "confidence": {"type": "string", "enum": ["extracted", "inferred", "ambiguous"]}
            },
            "required": ["symbol", "confidence"]
        }}
    }
        },
        {
            "name": "find_callers",
            "title": "Who calls this",
            "description": "The direct callers of one symbol, as one list, and the \
    definitions that use it by name without calling it — a type in a signature, a \
    constant read — tagged ambiguous, since a name is matched by name. \
    `impact` answers the wider question — everything reachable backwards within k \
    hops, grouped by distance — which is right for a blast radius and too much when \
    the question is simply who calls this. An ambiguous bare name is refused with \
    its candidates rather than answered about whichever sorted first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Qualified `path#name`, or a name that resolves uniquely"},
                    "min_confidence": {"type": "string", "enum": ["extracted", "inferred", "ambiguous"],
                        "description": "Lowest provenance tier to count (default ambiguous)"},
                    "limit": {"type": "integer", "description": "How many callers to name (default 200)"},
                    "offset": {"type": "integer", "description": "Skip this many: the `next_offset` of a previous answer"}
                },
                "required": ["symbol"]
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "count": {"type": "integer"},
                    "next_offset": {"type": "integer", "description": "Present when more remain"},
                    "min_confidence": {"type": "string"},
                    "callers": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "symbol": {"type": "string"},
                            "file": {"type": "string", "description": "Empty for a bare placeholder"},
                            "confidence": {"type": "string", "enum": ["extracted", "inferred", "ambiguous"]}
                        },
                        "required": ["symbol", "file", "confidence"]
                    }}
                },
                "required": ["symbol", "count", "min_confidence", "callers"]
            }
        },
        {
            "name": "detect_changes",
            "title": "What a diff puts at risk",
            "description": "Maps a git diff to the symbols the changed files \
    define, then to what depends on those, with a risk level and its reasons: \
    changed code that has callers but no test reaching it, and files the git \
    history says usually change too but this change leaves out. Reads the working \
    tree as well as the graph, new untracked files included. Defaults to \
    uncommitted changes against HEAD; \
    pass a revision to compare that revision to HEAD.\n\nA changed file the graph \
    does not carry yet — added since the last analysis — is named rather than \
    silently ignored, because \"defines nothing\" and \"not indexed\" are different \
    answers and only one of them means nothing depends on it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rev": {"type": "string", "description": "Revision to compare against HEAD; omit for uncommitted changes"},
                    "depth": {"type": "integer", "description": "Backwards hops (default 3, max 10)"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "risk": {
                        "type": "object",
                        "description": "high: a changed symbol has callers and no test reaches it, or a file history says usually changes too is left out. medium: something depends on the change. low: nothing does.",
                        "properties": {
                            "level": {"type": "string", "enum": ["low", "medium", "high"]},
                            "untested": {"type": "array", "items": {"type": "string"}},
                            "evaluated": {"type": "integer", "description": "Changed symbols checked for a reaching test"},
                            "missed_partners": {"type": "array", "items": {
                                "type": "object",
                                "properties": {
                                    "file": {"type": "string"},
                                    "partner": {"type": "string"},
                                    "together": {"type": "integer"},
                                    "commits": {"type": "integer"}
                                },
                                "required": ["file", "partner", "together", "commits"]
                            }}
                        },
                        "required": ["level", "untested", "evaluated", "missed_partners"]
                    },
                    "changed_files": {"type": "integer"},
                    "changed_symbols": {"type": "array", "items": {"type": "string"}},
                    "files_without_known_symbols": {"type": "array", "items": {"type": "string"}},
                    "dependents": {"type": "integer"},
                    "hops": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "hop": {"type": "integer"},
                            "symbols": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["hop", "symbols"]
                    }}
                },
                "required": ["risk", "changed_files", "changed_symbols",
                             "files_without_known_symbols", "dependents", "hops"]
            }
        },
        {
            "name": "affected_tests",
            "title": "Which tests to run",
            "description": "The tests that reach a symbol or a git diff, nearest \
    first, with the files to run. Answers \"what do I run before I commit this\" \
    without running the suite. Pass `symbol` for one definition, or nothing (or a \
    `rev`) for the uncommitted diff. Tests are recognised by the conventions of \
    each language: tests/, spec/, __tests__/, test_x, x_test, x.test, x.spec, \
    FooTest, TestX. An empty answer is not proof of no coverage: a test in an \
    unindexed file, or one reaching the code through dynamic dispatch, is \
    invisible to the graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string", "description": "Qualified `path#name`; omit to use the git diff"},
                    "rev": {"type": "string", "description": "Revision to compare against HEAD when no symbol is given; omit for uncommitted changes"},
                    "depth": {"type": "integer", "description": "Backwards hops (default 12, max 30)"},
                    "min_confidence": {"type": "string", "enum": ["extracted", "inferred", "ambiguous"],
                        "description": "Lowest provenance tier to follow (default ambiguous)"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string"},
                    "count": {"type": "integer"},
                    "files": {"type": "array", "items": {"type": "string"}},
                    "tests": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "symbol": {"type": "string"},
                            "file": {"type": "string"},
                            "hop": {"type": "integer", "description": "0 for a test that is itself part of the change"}
                        },
                        "required": ["symbol", "file", "hop"]
                    }},
                    "depth": {"type": "integer"},
                    "files_without_known_symbols": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["target", "count", "files", "tests", "depth", "files_without_known_symbols"]
            }
        },
        {
            "name": "co_changes",
            "title": "What changes with this",
            "description": "Files that changed in the same commits as one file, \
    from the git history, with how often and whether the graph connects them at \
    all. A coupled file with no edge in the graph is hidden coupling — a template, \
    a migration, a fixture, a client in another language — that `impact` cannot \
    see because no code names it. Commits touching more than 30 files are left \
    out, and a pair must occur twice to count.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": {"type": "string", "description": "Path relative to the tree"},
                    "symbol": {"type": "string", "description": "Alternatively a symbol; its file is used"},
                    "commits": {"type": "integer", "description": "How many recent commits to read (default 500)"},
                    "top": {"type": "integer", "description": "How many coupled files to return (default 10)"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "file": {"type": "string"},
                    "commits": {"type": "integer", "description": "Commits in the window that touched the file"},
                    "window": {"type": "integer", "description": "Commits read, after dropping oversized ones"},
                    "coupled": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "file": {"type": "string"},
                            "together": {"type": "integer"},
                            "share": {"type": "number", "description": "Fraction of the file's commits that also touched this one"},
                            "in_graph": {"type": "boolean", "description": "Whether any edge in the graph joins the two files"}
                        },
                        "required": ["file", "together", "share", "in_graph"]
                    }}
                },
                "required": ["file", "commits", "window", "coupled"]
            }
        },
        {
            "name": "check_architecture",
            "title": "Does this break the architecture",
            "description": "Checks the tree's architecture contract \
    (`glasir-rules.txt`: `deny <path> -> <path>`, `no-cycles`) against the graph, \
    and returns every broken rule with the edges that break it. Pass `rules` to \
    test a boundary that is not written down yet. With `serve --watch` the graph \
    follows the working tree, so a change can be checked before it is committed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rules": {"type": "string", "description": "Rules to check instead of the tree's file, one per line"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "source": {"type": "string"},
                    "rules": {"type": "integer"},
                    "holds": {"type": "boolean"},
                    "violations": {"type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "rule": {"type": "string"},
                            "evidence": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["rule", "evidence"]
                    }}
                },
                "required": ["source", "rules", "holds", "violations"]
            }
        },
        {
            "name": "find_unused",
            "title": "What nothing uses",
            "description": "Definitions nothing in the tree refers to: no call \
    reaches them, no call anywhere uses the name, and the name appears in no text \
    file of the tree more often than it is defined. Tests, entry points, \
    annotated or decorated definitions (a framework calls those), overrides, \
    generated files, headers, data formats, JavaBeans accessors, visitor and \
    event-handler conventions and Godot callbacks are left out. Not proof of \
    dead code: a library's public API, or a name a library builds at run time \
    (`getattr`, reflection), is invisible here. Check before deleting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Only definitions under this path prefix"},
                    "limit": {"type": "integer", "description": "How many to name (default 50)"},
                    "offset": {"type": "integer", "description": "Skip this many: the `next_offset` of a previous answer"}
                }
            },
            "outputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "checked": {"type": "integer", "description": "Definitions considered"},
                    "count": {"type": "integer"},
                    "next_offset": {"type": "integer", "description": "Present when more remain"},
                    "unused": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["path", "checked", "count", "unused"]
            }
        }
    ])
}

fn confidence_label(c: Confidence) -> &'static str {
    match c {
        Confidence::Extracted => "extracted",
        Confidence::Inferred => "inferred",
        Confidence::Ambiguous => "ambiguous",
    }
}

fn parse_confidence(s: &str) -> Confidence {
    match s {
        "extracted" => Confidence::Extracted,
        "inferred" => Confidence::Inferred,
        _ => Confidence::Ambiguous,
    }
}

/// How many of the top results the suggested vocabulary is drawn from.
///
/// Measured on this repository: the useful term is reliably in the first ten
/// or not there at all, while taking all forty buries it in whatever the
/// expansion drifted into.
const VOCAB_FROM: usize = 10;

/// Symbols `detect_changes` names per section before it stops listing them.
/// One changed file here defines 54, which cost 972 tokens to print in full.
/// Counts stay exact; only the naming is capped.
const CHANGE_LIST_LIMIT: usize = 20;

/// Words the result is written in that the question did not use.
///
/// This is the answer to vocabulary mismatch, and deliberately not an attempt
/// to solve it here. A reader who asks about "splitting a codebase into
/// subsystems" cannot reach the code that does it, because that code is named
/// and documented in the terms of the literature instead. Measured on the
/// benchmark: phrasing the same question either way is the whole difference
/// between rank 1 and no hit at all.
///
/// Both standard fixes fail under this project's constraints. Mining synonyms
/// from the corpus needs one far larger than a single repository — the
/// technique's own evaluation used 3M lines across 12,070 projects, and this
/// tree yields no pair above the frequency threshold it requires. Document
/// expansion needs a neural model at index time, which rules it out outright.
///
/// What is left is the caller. The client is a language model that already
/// knows which words mean the same thing; it just has no way to know which of
/// them this codebase chose. So a result carries the vocabulary it is actually
/// written in, and re-asking is the model's own move to make. Terms already in
/// the question are omitted — repeating them says nothing.
///
/// (Kept free of the concrete example pair on purpose: this file is indexed
/// like any other, and naming both words here made this function itself the
/// top hit for them, costing 8 points of benchmark recall.)
/// Whether a symbol key names a section of a document rather than code.
///
/// The path half is what decides: `docs/a.md#Heading` is prose, `src/a.rs#f`
/// is not. Used to tell a question that failed from one that was answered by
/// a document on purpose.
fn is_documentation(key: &str) -> bool {
    key.split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(key)
        .rsplit_once('.')
        .is_some_and(|(_, ext)| {
            ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
        })
}

fn vocabulary(served: &Served, query: &str, kept: &[physics::Scored]) -> Vec<String> {
    let asked: std::collections::HashSet<String> =
        crate::search::tokenize(query).into_iter().collect();

    // **A question that reaches only one of its own words has failed, and the
    // words of what it reached are the worst thing to offer.** The empty-seed
    // path above already hands back the tree's subsystems, but a *partial*
    // miss never reaches it: one term matches something incidental, two nodes
    // come back, and the suggestion is drawn from exactly those two.
    //
    // Measured on a German question against an English Godot tree: the answer
    // was two sections of a game-design document, and the words offered were
    // `gassen`, `cinematic`, `runenbrück` — further from the code than the
    // question started. The same tree answers `save room to disk` at 87 BM25
    // against that miss's 10.4, but the score is not the signal: a good German
    // question here reads 28. What separates them is coverage — the miss
    // reaches 1 of 2 terms while every question that works reaches 2 of 4, 3
    // of 5, or all of them.
    //
    // So a single covered term out of several asked is treated as the miss it
    // is, and the subsystems are offered instead — the same answer the
    // empty-seed path gives, for the same reason.
    // Coverage alone is too coarse, and the self-check said so: `charge
    // settlement` also reaches one term of two, and there the one it reaches
    // is the answer. What separates the two is *where* it landed — a question
    // in the wrong language lands in prose, because a document is the only
    // thing in the tree written in that language at all. So both conditions
    // have to hold: one term covered, and nothing but documentation reached.
    if asked.len() > 1 && !kept.is_empty() {
        let covered = kept
            .first()
            .map(|s| served.search.coverage_of(query, s.node))
            .unwrap_or(0);
        let all_prose = kept.iter().all(|s| {
            served
                .names
                .get(s.node as usize)
                .is_some_and(|n| is_documentation(n))
        });
        if covered <= 1 && all_prose {
            return served.subsystem_names(8);
        }
    }
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // Only the strongest results: a query that half-missed still returns forty
    // nodes, and the tail of those is what the expansion drifted into rather
    // than what the query was about. Taking all of them made the suggestion
    // read `main, ingest, community, mcp, view, demo, json` — the useful word
    // was in there and indistinguishable from the noise around it.
    let strong = kept
        .iter()
        .filter(|s| served.defined.contains(&s.node))
        .take(VOCAB_FROM);
    for s in strong {
        let name = served.name(s.node);
        // The symbol's own words, not its documentation: a doc comment runs to
        // hundreds of words and would bury the names in prose. What the caller
        // needs is the terms to search *with*.
        let (file, symbol) = name.split_once('#').unwrap_or(("", name.as_str()));
        // The symbol counts double: `community_label` says more about what a
        // result is than the file it happens to live in, and a large file
        // otherwise votes once per symbol it contributed.
        for word in crate::search::tokenize(symbol) {
            if word.len() > 2 && !asked.contains(&word) {
                *counts.entry(word).or_insert(0) += 2;
            }
        }
        let stem = file.rsplit('/').next().unwrap_or(file);
        for word in crate::search::tokenize(stem) {
            if word.len() > 2 && word != "rs" && !asked.contains(&word) {
                *counts.entry(word).or_insert(0) += 1;
            }
        }
    }
    // A word common across the whole tree names nothing in particular: `len`,
    // `contains` and `counts` appeared in every suggestion until this, because
    // they appear in every part of the code. Weighting by how *specific* a word
    // is — the same inverse-frequency idea BM25 uses for ranking — leaves the
    // terms that distinguish this result from the rest of the repository.
    let mut ranked: Vec<(String, f32)> = counts
        .into_iter()
        .map(|(w, n)| {
            let spread = served.search.document_frequency(&w).max(1) as f32;
            (w, n as f32 / spread.sqrt())
        })
        .collect();
    // Score first, then alphabetical: the registry is a HashMap, so an
    // unbroken tie would make the suggestion differ between runs.
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked.into_iter().take(6).map(|(w, _)| w).collect()
}

fn query_graph(served: &Served, args: &Value) -> Result<Value, String> {
    let query = args["query"].as_str().ok_or("query must be a string")?;
    let seeds = served.seeds(query);
    if seeds.is_empty() {
        // Nothing matched, which is exactly when the caller most needs to know
        // what this tree calls things — a question in the wrong vocabulary
        // fails here, not with a weak result. Returning only "no match" left
        // the model with nowhere to go but reading files.
        let subsystems = served.subsystem_names(8);
        if subsystems.is_empty() {
            return Err(format!("no symbol matches {query:?}"));
        }
        return Err(format!(
            "no symbol matches {query:?}\n\nthis tree's subsystems: {}\n\
Search is lexical, so it finds what this codebase calls things rather than \
what you called them. If your concept is here under another name, ask again \
using one of these.",
            subsystems.join(", ")
        ));
    }

    let cfg = Physics {
        max_nodes: (args["max_nodes"]
            .as_u64()
            .unwrap_or(DEFAULT_MAX_NODES as u64) as usize)
            .min(MAX_NODES_CEILING),
        max_hops: (args["max_hops"].as_u64().unwrap_or(DEFAULT_MAX_HOPS as u64) as usize)
            .min(MAX_HOPS_CEILING),
        ..served.physics
    };
    let kept = physics::expand(served.snap, &seeds, served.now, &cfg, Some(served.defined));

    let nodes: Vec<Value> = kept
        .iter()
        .map(|s| {
            let name = served.name(s.node);
            json!({
                "symbol": name,
                // The qualified name splits on the *first* `#`, never the last:
                // a Markdown heading may carry a `#` of its own. See `qualify`.
                "file": name.split_once('#').map(|(f, _)| f).unwrap_or(""),
                "score": round_to(s.score as f64, 1000.0),
                "hops": s.hops,
                "community": served
                    .communities
                    .of_node
                    .get(s.node as usize)
                    .copied()
                    .unwrap_or(0),
            })
        })
        .collect();

    // Bound before the macro, not called inside it: a `json!` body is a token
    // tree, and the former tree-sitter tags query captured no calls within one.
    // Keep the explicit binding that fixed that regression. Measured: this
    // was `"vocabulary": vocabulary(...)` and the `query_graph -> vocabulary`
    // edge disappeared from the graph, taking a `shortest_path` answer with it
    // (`structural` 100% -> 97%). The tools describe this repository too.
    let vocab = vocabulary(served, query, &kept);
    let filtered = round_to(
        physics::reduction(kept.len(), served.snap.width()) as f64,
        10.0,
    );
    Ok(json!({
        "returned": kept.len(),
        "total": served.snap.width(),
        "filtered_out_percent": filtered,
        "nodes": nodes,
        "vocabulary": vocab,
    }))
}

fn render_query_graph(v: &Value) -> String {
    let mut out = format!(
        "{} of {} nodes ({:.1}% filtered out)\n\n",
        v["returned"].as_u64().unwrap_or(0),
        v["total"].as_u64().unwrap_or(0),
        v["filtered_out_percent"].as_f64().unwrap_or(0.0)
    );
    for n in v["nodes"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{:.3}  {:2} hop  {}  [community {}]\n",
            n["score"].as_f64().unwrap_or(0.0),
            n["hops"].as_u64().unwrap_or(0),
            n["symbol"].as_str().unwrap_or(""),
            n["community"].as_u64().unwrap_or(0)
        ));
    }
    let vocab: Vec<&str> = v["vocabulary"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !vocab.is_empty() {
        out.push_str(&format!(
            "\nthis code's words for it: {}\nIf that is not what you meant, ask again using them — \
this tree may name your concept differently.\n",
            vocab.join(", ")
        ));
    }
    out
}

fn shortest_path(served: &Served, args: &Value) -> Result<Value, String> {
    let from = args["from"].as_str().ok_or("from must be a string")?;
    let to = args["to"].as_str().ok_or("to must be a string")?;
    let floor = parse_confidence(args["min_confidence"].as_str().unwrap_or("ambiguous"));

    // `resolve_one`, not `seeds().first()`. A bare name is a registry key, so
    // the seed path took the *placeholder* and answered about it without
    // saying so — the same mistake `impact` was fixed for in the step 5 audit,
    // left in the one tool that also reports a distance.
    //
    // It is not only a wrong label: a placeholder routes straight into its own
    // definition, so entering through one adds a hop the caller then reads as
    // structure. Measured, `compact -> src/delta.rs#added` reports 4 hops
    // where `src/graph.rs#compact` reports 3, and the first hop is
    // `compact -> src/graph.rs#compact`.
    //
    // The population is not exotic: of this tree's 531 bare placeholders, 190
    // also name a definition and **165 of those route straight into it**
    // (office4u 403 of 623, epoch-engine 39 of 43).
    //
    // A placeholder is still a legitimate *endpoint* — a path through one is
    // how cross-file calls are routed, and `shortest_path X tokenize` is a
    // reasonable question. What `resolve_one` refuses is the ambiguous case,
    // where a bare name stands for several definitions and picking one
    // silently answers a question nobody asked.
    let start = served.resolve_one(from)?;
    let goal = served.resolve_one(to)?;

    // Plain BFS: every edge is one hop, so BFS already yields a shortest path
    // and Dijkstra would only add weight handling nothing here needs.
    let mut prev: std::collections::HashMap<NodeId, (NodeId, Confidence)> =
        std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::from([start]);
    let mut seen = std::collections::HashSet::from([start]);

    while let Some(node) = queue.pop_front() {
        if node == goal {
            break;
        }
        let mut edges: Vec<_> = served
            .snap
            .neighbors(node)
            .filter(|e| e.confidence >= floor)
            .collect();
        // Strongest provenance first, so a compiler-resolved route wins over a
        // guessed one of the same length. Node id breaks ties for determinism.
        edges.sort_by(|a, b| {
            b.confidence
                .cmp(&a.confidence)
                .then(a.target.cmp(&b.target))
        });
        for e in edges {
            if seen.insert(e.target) {
                prev.insert(e.target, (node, e.confidence));
                queue.push_back(e.target);
            }
        }
    }

    if goal != start && !prev.contains_key(&goal) {
        return Ok(json!({
            "from": served.name(start),
            "to": served.name(goal),
            "min_confidence": confidence_label(floor),
            // A refusal is an answer, not an empty result: two symbols with no
            // route between them is a fact about the architecture. `found`
            // distinguishes it from a path of length zero.
            "found": false,
            "hops": Value::Null,
            "path": [],
        }));
    }

    let mut chain = vec![(goal, None)];
    let mut cur = goal;
    while let Some(&(parent, conf)) = prev.get(&cur) {
        chain.push((parent, Some(conf)));
        cur = parent;
    }
    chain.reverse();

    let steps: Vec<Value> = chain
        .iter()
        .enumerate()
        .map(|(i, (node, _))| {
            // The edge shown before a node is the one that reached it, which is
            // this entry's own confidence — falling back to the previous step's
            // for the terminal node, exactly as the prose did.
            let conf = if i > 0 {
                chain[i].1.or(chain[i - 1].1).map(confidence_label)
            } else {
                None
            };
            json!({"symbol": served.name(*node), "via": conf})
        })
        .collect();
    Ok(json!({
        "from": served.name(start),
        "to": served.name(goal),
        "min_confidence": confidence_label(floor),
        "found": true,
        "hops": chain.len().saturating_sub(1),
        "path": steps,
    }))
}

fn render_shortest_path(v: &Value) -> String {
    if !v["found"].as_bool().unwrap_or(false) {
        return format!(
            "no path from {} to {} over {}+ edges",
            v["from"].as_str().unwrap_or(""),
            v["to"].as_str().unwrap_or(""),
            v["min_confidence"].as_str().unwrap_or("")
        );
    }
    let mut out = format!("{} hops\n\n", v["hops"].as_u64().unwrap_or(0));
    for step in v["path"].as_array().into_iter().flatten() {
        if let Some(via) = step["via"].as_str() {
            out.push_str(&format!("  --[{via}]-->\n"));
        }
        out.push_str(&format!("{}\n", step["symbol"].as_str().unwrap_or("")));
    }
    out
}

fn explain_node(served: &Served, args: &Value) -> Result<Value, String> {
    let symbol = args["symbol"].as_str().ok_or("symbol must be a string")?;
    // The third tool that took `seeds().first()` and answered about whichever
    // node sorted first. The two answers are not variations of each other:
    // `explain_node compact` reported "no subsystem, 1 call, 4 callers" while
    // `src/graph.rs#compact` reports "3 siblings, 8 calls, 1 caller", and
    // nothing said which question had been answered. `resolve_one` refuses the
    // ambiguity and lists the candidates, exactly as `impact` and
    // `shortest_path` do.
    let node = served.resolve_one(symbol)?;

    let community = served
        .communities
        .of_node
        .get(node as usize)
        .copied()
        .unwrap_or(0);
    let peers: Vec<String> = served
        .communities
        .members(community)
        .into_iter()
        .filter(|&m| m != node)
        .take(8)
        .map(|p| served.name(p))
        .collect();

    // Sorted and deduplicated as formatted lines, which is what the prose has
    // always done — the pair is what repeats, so a call written twice in the
    // source collapses to one entry while two different confidences do not.
    let mut outgoing: Vec<(String, String)> = served
        .snap
        .neighbors(node)
        .map(|e| {
            (
                confidence_label(e.confidence).to_string(),
                served.name(e.target),
            )
        })
        .collect();
    // The prose sorted the rendered line, which begins with the confidence —
    // so confidence orders before name, and the JSON must keep that or the
    // rendered text changes order.
    outgoing.sort();
    outgoing.dedup();

    // The CSR stores outgoing edges only, so callers come from the reverse
    // index built from this snapshot.
    let mut incoming: Vec<(String, String)> = served
        .snap
        .reverse()
        .callers(node)
        .filter(|&(n, _)| n != node)
        .map(|(n, e)| (confidence_label(e.confidence).to_string(), served.name(n)))
        .collect();
    incoming.sort();

    // A direct caller or callee is structural evidence, not merely a keyword
    // match. Keep a bounded set beside the five embedding neighbours: a small
    // graph growth must not make `configured -> current` disappear from an
    // explanation just because it moved from rank five to six. The score is
    // still the actual deterministic cosine similarity; directness controls
    // inclusion, never invents a score.
    let mut similar = served.embeddings.nearest(node, 5);
    let mut direct: Vec<_> = served
        .snap
        .neighbors(node)
        .map(|edge| edge.target)
        .chain(
            served
                .snap
                .reverse()
                .callers(node)
                .map(|(caller, _)| caller),
        )
        .filter(|&other| other != node && !similar.iter().any(|&(known, _)| known == other))
        .collect();
    // Scanner-tier call extraction intentionally refuses receiver-qualified
    // calls such as `self.current()`: guessing their target across impls would
    // create false edges. For an explanation, however, definitions in the same
    // source file are useful bounded structural context. This covers that
    // honest uncertainty without inventing a call edge or changing graph-wide
    // retrieval and impact results.
    if let Some((file, _)) = served.name(node).split_once('#') {
        for candidate in 0..served.snap.width() as u32 {
            if served.name(candidate).starts_with(&format!("{file}#"))
                && candidate != node
                && !similar.iter().any(|&(known, _)| known == candidate)
            {
                direct.push(candidate);
            }
        }
    }
    direct.sort_unstable();
    direct.dedup();
    // A source file can be large; 32 keeps the response bounded while still
    // covering ordinary impl blocks whose method order is meaningful context.
    direct.truncate(32);
    similar.extend(
        direct
            .into_iter()
            .map(|other| (other, served.embeddings.similarity(node, other))),
    );

    Ok(json!({
        "symbol": served.name(node),
        "role": if served.communities.hubs.contains(&node) { "connector" } else { "member" },
        "community": community,
        "siblings": peers,
        "calls": outgoing
            .iter()
            .map(|(c, n)| json!({"symbol": n, "confidence": c}))
            .collect::<Vec<_>>(),
        "called_by": incoming
            .iter()
            .map(|(c, n)| json!({"symbol": n, "confidence": c}))
            .collect::<Vec<_>>(),
        "structurally_similar": similar
            .into_iter()
            .map(|(n, score)| json!({
                "symbol": served.name(n),
                "score": round_to(score as f64, 100.0),
            }))
            .collect::<Vec<_>>(),
    }))
}

fn render_explain_node(v: &Value) -> String {
    let mut out = format!("{}\n\n", v["symbol"].as_str().unwrap_or(""));
    let peers = v["siblings"].as_array().map_or(&[][..], Vec::as_slice);
    if v["role"] == "connector" {
        // "hub" here means structural, not popular: it bridges parts of the
        // codebase that are otherwise unrelated, which is why it is left out of
        // the partition.
        out.push_str(
            "role: connector — bridges otherwise unrelated parts, so it is not \
placed in a subsystem\n\n",
        );
    } else if peers.is_empty() {
        out.push_str("subsystem: none (not grouped with other symbols)\n\n");
    } else {
        out.push_str(&format!("subsystem ({} others):\n", peers.len()));
        for p in peers {
            out.push_str(&format!("  {}\n", p.as_str().unwrap_or("")));
        }
        out.push('\n');
    }

    let calls = v["calls"].as_array().map_or(&[][..], Vec::as_slice);
    out.push_str(&format!("calls ({}):\n", calls.len()));
    let lines: Vec<String> = calls
        .iter()
        .map(|c| {
            format!(
                "  --[{}]--> {}",
                c["confidence"].as_str().unwrap_or(""),
                c["symbol"].as_str().unwrap_or("")
            )
        })
        .collect();
    out.push_str(&lines.join("\n"));

    let callers = v["called_by"].as_array().map_or(&[][..], Vec::as_slice);
    out.push_str(&format!("\n\ncalled by ({}):\n", callers.len()));
    let lines: Vec<String> = callers
        .iter()
        .map(|c| {
            format!(
                "  {} --[{}]-->",
                c["symbol"].as_str().unwrap_or(""),
                c["confidence"].as_str().unwrap_or("")
            )
        })
        .collect();
    out.push_str(&lines.join("\n"));

    let similar = v["structurally_similar"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if !similar.is_empty() {
        out.push_str("\n\nstructurally similar:\n");
        for sm in similar {
            out.push_str(&format!(
                "  {:.2}  {}\n",
                sm["score"].as_f64().unwrap_or(0.0),
                sm["symbol"].as_str().unwrap_or("")
            ));
        }
    }
    out
}

/// "What breaks if I change this" — the transitive closure backwards.
///
/// Three things learned the hard way, all visible in the output:
///  * a caller is reported with the confidence of the edge that reached it, so
///    a guessed dependency is never presented as a compiler-resolved one;
///  * the seed must resolve uniquely or the call fails — see `resolve_one`;
///  * results are grouped by hop, because distance is the whole signal: a
///    direct caller almost certainly breaks, a fourth-hop one probably does not.
fn impact(served: &Served, args: &Value) -> Result<Value, String> {
    let symbol = args["symbol"].as_str().ok_or("symbol must be a string")?;
    let depth = args["depth"].as_u64().unwrap_or(3).clamp(1, 10) as usize;
    let floor = parse_confidence(args["min_confidence"].as_str().unwrap_or("ambiguous"));
    let node = served.resolve_one(symbol)?;
    let levels = reverse_levels(served, &[node], depth, floor);
    let (offset, limit) = page(args, 200);
    let reached: Vec<NodeId> = levels.iter().flatten().map(|&(n, _)| n).collect();
    let elsewhere = served.unconnected_mentions(node, &reached);

    let total: usize = levels.iter().map(|l| l.len()).sum();
    let within = levels.len();
    // Nearest first across the page boundary.
    let mut levels_page: Vec<(usize, Vec<(NodeId, Confidence)>)> = Vec::new();
    let flat = levels
        .iter()
        .enumerate()
        .flat_map(|(i, level)| level.iter().map(move |&e| (i + 1, e)));
    for (hop, entry) in flat.skip(offset).take(limit) {
        match levels_page.last_mut() {
            Some((h, v)) if *h == hop => v.push(entry),
            _ => levels_page.push((hop, vec![entry])),
        }
    }
    let mut value = json!({
        "symbol": served.name(node),
        "dependents": total,
        "within": within,
        "named_elsewhere": elsewhere,
        "min_confidence": confidence_label(floor),
        // Grouped by hop because distance is the signal: a direct caller almost
        // certainly breaks, a fourth-hop one probably does not. Flattening the
        // levels would throw that away, so the nesting is the answer's shape
        // and not the prose's layout.
        "hops": levels_page
            .iter()
            .map(|(hop, level)| json!({
                "hop": hop,
                "symbols": level
                    .iter()
                    .map(|&(n, conf)| json!({
                        "symbol": served.name(n),
                        "confidence": confidence_label(conf),
                    }))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    });
    next_offset(&mut value, offset, limit, total);
    Ok(value)
}

/// `offset` and `limit` of a paged list; the limit is clamped like every other.
fn page(args: &Value, default: usize) -> (usize, usize) {
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let limit = args["limit"]
        .as_u64()
        .unwrap_or(default as u64)
        .clamp(1, 1000) as usize;
    (offset, limit)
}

fn next_offset(value: &mut Value, offset: usize, limit: usize, total: usize) {
    if offset.saturating_add(limit) < total {
        value["next_offset"] = json!(offset + limit);
    }
}

/// The line a paged answer ends with when more remain.
fn more_line(v: &Value, total: u64) -> String {
    v["next_offset"].as_u64().map_or(String::new(), |next| {
        format!(
            "\n… {} more: call again with `offset: {next}`\n",
            total.saturating_sub(next)
        )
    })
}

/// The direct callers of one symbol. `impact` sweeps k hops and groups by
/// distance, which is right for a blast radius and too much here. Resolution is
/// strict, as in `impact`.
fn find_callers(served: &Served, args: &Value) -> Result<Value, String> {
    let symbol = args["symbol"].as_str().ok_or("symbol must be a string")?;
    let floor = parse_confidence(args["min_confidence"].as_str().unwrap_or("ambiguous"));
    let node = served.resolve_one(symbol)?;

    let reverse = served.snap.reverse();
    let mut callers = direct_callers(served, reverse, node, floor);
    // A multigraph may join two nodes several times; the caller is one answer,
    // at its strongest confidence.
    callers.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    callers.dedup_by_key(|&mut (n, _)| n);
    let (offset, limit) = page(args, 200);
    let reached: Vec<NodeId> = reverse_levels(served, &[node], 2, floor)
        .iter()
        .flatten()
        .map(|&(n, _)| n)
        .collect();

    let mut value = json!({
        "symbol": served.name(node),
        "named_elsewhere": served.unconnected_mentions(node, &reached),
        "callers": callers
            .iter()
            .skip(offset)
            .take(limit)
            .map(|&(n, conf)| json!({
                "symbol": served.name(n),
                "file": served.name(n).split_once('#').map_or(String::new(), |(f, _)| f.to_string()),
                "confidence": confidence_label(conf),
            }))
            .collect::<Vec<_>>(),
        "count": callers.len(),
        "min_confidence": confidence_label(floor),
    });
    next_offset(&mut value, offset, limit, callers.len());
    Ok(value)
}

fn render_find_callers(v: &Value) -> String {
    let sym = v["symbol"].as_str().unwrap_or("");
    let callers = v["callers"].as_array().map_or(&[][..], Vec::as_slice);
    if v["count"].as_u64().unwrap_or(0) == 0 {
        if has_elsewhere(v) {
            return format!(
                "{sym}\n\nnothing in the graph calls this{}",
                render_elsewhere(v, ", but")
            );
        }
        return format!(
            "{sym}\n\nnothing in the graph calls this. Callers in code the graph \
does not cover are not ruled out.\n"
        );
    }
    let count = v["count"].as_u64().unwrap_or(callers.len() as u64);
    let mut out = format!("{sym}\n\n{count} caller(s):\n");
    for c in callers {
        out.push_str(&format!(
            "  [{}]  {}\n",
            c["confidence"].as_str().unwrap_or(""),
            c["symbol"].as_str().unwrap_or("")
        ));
    }
    out.push_str(&more_line(v, count));
    if has_elsewhere(v) {
        out.push_str(&render_elsewhere(v, "\nbeyond these,"));
    }
    out
}

/// A git diff mapped to the symbols the changed files define, then to what
/// depends on those. It reads the working tree and re-uses the served
/// snapshot rather than re-analysing.
///
/// A file the graph does not carry yet is named rather than skipped: "defines
/// nothing" and "not indexed" are different answers, and only one of them means
/// nothing depends on it.
fn detect_changes(served: &Served, args: &Value) -> Result<Value, String> {
    let depth = args["depth"].as_u64().unwrap_or(3).clamp(1, 10) as usize;
    let diff = changed_symbols(served, args["rev"].as_str().unwrap_or(""))?;
    let seeds: Vec<NodeId> = diff.touched.iter().map(|&(_, n)| n).collect();
    let levels = reverse_levels(served, &seeds, depth, Confidence::Ambiguous);
    let dependents: usize = levels.iter().map(Vec::len).sum();
    let risk = assess(served, &diff, dependents);

    Ok(json!({
        "risk": risk,
        "changed_files": diff.files.len(),
        "changed_symbols": diff.touched.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(),
        // Named rather than counted: a file the graph does not carry is the
        // case where "nothing depends on this" is a wrong answer, so the
        // caller has to see which files those are.
        "files_without_known_symbols": diff.unknown,
        "dependents": dependents,
        "hops": levels
            .iter()
            .enumerate()
            .map(|(i, level)| json!({
                "hop": i + 1,
                "symbols": level.iter().map(|&(n, _)| served.name(n)).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    }))
}

/// How often a file must change with another, and in what share of its own
/// commits, before leaving the other out of a change is worth a warning.
const PARTNER_MIN: usize = 3;
const PARTNER_SHARE: f64 = 0.5;
/// Changed symbols checked for a reaching test; a sweeping diff stops here.
const RISK_SYMBOL_LIMIT: usize = 200;

/// Why a change is risky, as reasons rather than a score. A number says
/// "0.7"; a reason says which function has callers and no test, which is what
/// someone can act on. Three signals no graph alone has together: callers,
/// the tests that reach them, and the files history says usually change too.
fn assess(served: &Served, diff: &Diff, dependents: usize) -> Value {
    let mut untested = Vec::new();
    let candidates: Vec<&(String, NodeId)> = diff
        .touched
        .iter()
        .filter(|(s, _)| !is_test_symbol(s))
        .collect();
    for &&(ref symbol, node) in candidates.iter().take(RISK_SYMBOL_LIMIT) {
        let has_callers = reverse_levels(served, &[node], 2, Confidence::Ambiguous)
            .iter()
            .flatten()
            .any(|(n, _)| served.defined.contains(n));
        if has_callers && !reaches_test(served, node) {
            untested.push(symbol.clone());
        }
    }

    let mut missed: Vec<Value> = Vec::new();
    if let Some(root) = served.root
        && let Ok(commits) = crate::history::commits(root, 500)
    {
        let mut named: std::collections::HashSet<String> = Default::default();
        for file in &diff.files {
            let (own, pairs) = crate::history::coupled(&commits, file);
            for (partner, together) in pairs {
                if together >= PARTNER_MIN
                    && together as f64 >= PARTNER_SHARE * own as f64
                    && !diff.files.contains(&partner)
                    && root.join(&partner).exists()
                    && named.insert(partner.clone())
                {
                    missed.push(json!({
                        "file": file,
                        "partner": partner,
                        "together": together,
                        "commits": own,
                    }));
                }
            }
        }
    }

    let level = if !untested.is_empty() || !missed.is_empty() {
        "high"
    } else if dependents > 0 {
        "medium"
    } else {
        "low"
    };
    json!({
        "level": level,
        "untested": untested,
        "evaluated": candidates.len().min(RISK_SYMBOL_LIMIT),
        "missed_partners": missed,
    })
}

/// Whether any test reaches `node`, stopping at the first one found.
fn reaches_test(served: &Served, node: NodeId) -> bool {
    let reverse = served.snap.reverse();
    let mut seen = std::collections::HashSet::from([node]);
    let mut frontier = vec![node];
    for _ in 0..TEST_DEPTH {
        let mut next = Vec::new();
        for &n in &frontier {
            for (caller, _) in reverse.callers(n) {
                if !seen.insert(caller) {
                    continue;
                }
                if served.defined.contains(&caller) && is_test_symbol(&served.name(caller)) {
                    return true;
                }
                next.push(caller);
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }
    false
}

fn render_risk(v: &Value) -> String {
    let level = v["level"].as_str().unwrap_or("low");
    let mut out = format!("risk: {level}\n");
    let untested = v["untested"].as_array().map_or(&[][..], Vec::as_slice);
    if !untested.is_empty() {
        out.push_str(&format!(
            "  {} changed symbol(s) have callers and no test reaches them:\n",
            untested.len()
        ));
        for s in untested.iter().take(CHANGE_LIST_LIMIT) {
            out.push_str(&format!("    {}\n", s.as_str().unwrap_or("")));
        }
        if untested.len() > CHANGE_LIST_LIMIT {
            out.push_str(&format!(
                "    … and {} more\n",
                untested.len() - CHANGE_LIST_LIMIT
            ));
        }
    }
    for m in v["missed_partners"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
    {
        out.push_str(&format!(
            "  {} usually changes with {} ({} of {} commits), which this change leaves out\n",
            m["file"].as_str().unwrap_or(""),
            m["partner"].as_str().unwrap_or(""),
            m["together"].as_u64().unwrap_or(0),
            m["commits"].as_u64().unwrap_or(0)
        ));
    }
    out
}

/// A git diff read against the served graph.
struct Diff {
    files: Vec<String>,
    /// Symbols the changed files define, sorted.
    touched: Vec<(String, NodeId)>,
    /// Changed files that define nothing the graph knows.
    unknown: Vec<String>,
}

/// The working tree against HEAD for an empty `rev`, or `rev` against HEAD.
fn changed_symbols(served: &Served, rev: &str) -> Result<Diff, String> {
    let rev = rev.trim();
    // Same default as the CLI: the working tree against HEAD is what someone
    // asking mid-change means.
    let working = rev.is_empty() || rev == ".";
    let against: Vec<&str> = if working {
        vec!["HEAD"]
    } else {
        vec![rev, "HEAD"]
    };
    let root = served
        .root
        .ok_or("this server has no tree on disk to diff")?;
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
    };
    let out = git(&[&["diff", "--name-only"][..], &against].concat())
        .map_err(|e| format!("git could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not diff: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let lines_of = |bytes: &[u8]| -> Vec<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .map(str::trim)
            // What Glasir writes beside the tree is never part of a change.
            .filter(|l| !l.is_empty() && !l.rsplit('/').next().unwrap_or(l).starts_with(".glasir-"))
            .map(str::to_string)
            .collect()
    };
    let mut files = lines_of(&out.stdout);
    // `git diff HEAD` leaves out a file that was never added, so a new module
    // went unreported until it was committed.
    if working && let Ok(new) = git(&["ls-files", "--others", "--exclude-standard"]) {
        files.extend(lines_of(&new.stdout));
    }

    // Per changed file, the definitions the diff actually touches. `None`
    // falls back to every symbol of the file: a new, deleted or unparsable one.
    let mut narrowed: std::collections::HashMap<&str, std::collections::HashSet<String>> =
        Default::default();
    for file in &files {
        let new_src = if working {
            std::fs::read_to_string(root.join(file)).ok()
        } else {
            git(&["show", &format!("HEAD:{file}")])
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        let hunks = git(&[&["diff", "-U0", "--no-color"][..], &against, &["--", file]].concat())
            .ok()
            .map(|o| hunk_lines(&String::from_utf8_lossy(&o.stdout)));
        if let (Some(src), Some(hunks)) = (new_src, hunks)
            && let Some(names) = touched_names(served, file, &src, &hunks)
        {
            narrowed.insert(file.as_str(), names);
        }
    }

    // Symbols defined in a changed file. The file is the name's prefix, so no
    // file-to-node map is needed — the property incremental indexing relies on.
    // A file is known to the graph whether or not its change touched a
    // definition: a comment edit is a change to a known file.
    let mut touched: Vec<(String, NodeId)> = Vec::new();
    let mut known: std::collections::HashSet<&str> = Default::default();
    for (symbol, &node) in served.registry.entries() {
        if let Some((file, name)) = symbol.split_once('#')
            && let Some(f) = files.iter().find(|c| *c == file)
            && served.defined.contains(&node)
        {
            known.insert(f.as_str());
            if narrowed.get(file).is_none_or(|names| names.contains(name)) {
                touched.push((symbol.clone(), node));
            }
        }
    }
    touched.sort();
    let unknown = files
        .iter()
        .filter(|f| !known.contains(f.as_str()))
        .cloned()
        .collect();
    Ok(Diff {
        files,
        touched,
        unknown,
    })
}

/// New-side line ranges of `git diff -U0`, 1-based and inclusive. A pure
/// deletion has no new lines and is a point between two: it comes back as
/// `(line after, line before)`, start past end, and touches only a definition
/// that contains the point — not the one that happens to begin right there.
fn hunk_lines(diff: &str) -> Vec<(u32, u32)> {
    diff.lines()
        .filter_map(|l| l.strip_prefix("@@ "))
        .filter_map(|l| l.split_whitespace().find(|w| w.starts_with('+')))
        .filter_map(|w| {
            let (start, count) = w[1..].split_once(',').unwrap_or((&w[1..], "1"));
            let (start, count): (u32, u32) = (start.parse().ok()?, count.parse().ok()?);
            Some(if count == 0 {
                (start + 1, start)
            } else {
                (start, start + count - 1)
            })
        })
        .collect()
}

/// The names in one changed file the diff touches: definitions whose lines
/// changed, `<module>` when a change fell outside every definition, and every
/// definition the graph has for the file that the new content no longer
/// defines — a deleted or renamed function is the change whose callers break.
/// Parsed from the new content rather than read from the stored graph, which
/// may describe an older version of the file. `None` when the file's language
/// is unknown here, which falls back to the whole file.
fn touched_names(
    served: &Served,
    file: &str,
    src: &str,
    hunks: &[(u32, u32)],
) -> Option<std::collections::HashSet<String>> {
    let path = std::path::Path::new(file);
    use crate::parse_ast::LangExt;
    let lang = crate::parse_ast::Lang::from_path(path)?;
    let facts = crate::parse_ast::parse_file(path, src, lang)?;
    let mut line_start = vec![0u32];
    line_start.extend(
        src.bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'\n')
            .map(|(i, _)| i as u32 + 1),
    );
    let end = src.len() as u32;
    let bytes = |(a, b): (u32, u32)| {
        let at = |line: u32| line_start.get(line as usize - 1).copied().unwrap_or(end);
        (at(a.max(1)), at(b + 1))
    };
    let mut names = std::collections::HashSet::new();
    for &(a, b) in hunks {
        let (hs, he) = if a > b {
            let at = bytes((a, a)).0;
            (at, at)
        } else {
            bytes((a, b))
        };
        let mut inside = false;
        for (name, (s, e)) in &facts.ranges {
            let hit = if hs == he {
                *s < hs && hs < *e
            } else {
                *s < he && hs < *e
            };
            if hit {
                names.insert(name.clone());
                inside = true;
            }
        }
        if !inside {
            names.insert("<module>".to_string());
        }
    }
    let now: std::collections::HashSet<&str> = facts.defines.iter().map(String::as_str).collect();
    let prefix = format!("{file}#");
    for (symbol, _) in served.registry.entries() {
        if let Some(name) = symbol.strip_prefix(&prefix)
            && !now.contains(name)
            && name != "<module>"
        {
            names.insert(name.to_string());
        }
    }
    Some(names)
}

/// The symbols calling `node`, looking through the placeholders tier 3 linked
/// onto it: a placeholder is the name a call was written with, not a caller,
/// and counting it as one put every cross-file caller a hop further away than
/// it is and left `find_callers` naming only the name.
/// The definitions that use a symbol without calling it — a type in a
/// signature, a constant read, a base class — for every name the tree defines
/// exactly once. Resolved here rather than stored as edges, so what a question
/// expands through and how the tree partitions are untouched.
#[derive(Default)]
pub struct References {
    by_target: std::collections::HashMap<NodeId, Vec<NodeId>>,
}

impl Served<'_> {
    fn references<'r>(&'r self, fallback: &'r std::sync::OnceLock<References>) -> &'r References {
        self.references.unwrap_or(fallback).get_or_init(|| {
            let mut defs: std::collections::HashMap<&str, Vec<(&str, NodeId)>> = Default::default();
            for (symbol, &node) in self.registry.entries() {
                if let Some((file, name)) = symbol.split_once('#')
                    && self.defined.contains(&node)
                    && !crate::docs::is_markdown(std::path::Path::new(file))
                {
                    defs.entry(name).or_default().push((file, node));
                }
            }
            let resolve = |name: &str| -> Option<NodeId> {
                let (bare, target) = match crate::imports::unscope(name) {
                    Some((bare, target)) => (bare, Some(target)),
                    None => (name, None),
                };
                let candidates = defs.get(bare)?;
                let chosen: Vec<NodeId> = candidates
                    .iter()
                    .filter(|(file, _)| match target {
                        Some(dir) if dir.ends_with('/') => {
                            file.rsplit_once('/').map_or("", |(d, _)| d)
                                == dir.trim_end_matches('/')
                        }
                        Some(path) => *file == path,
                        None => true,
                    })
                    .map(|&(_, n)| n)
                    .collect();
                match chosen[..] {
                    [one] => Some(one),
                    _ => None,
                }
            };
            let mut by_target: std::collections::HashMap<NodeId, Vec<NodeId>> = Default::default();
            for refs in self.registry.refs().values() {
                for (from, name) in refs {
                    if let Some(target) = resolve(name)
                        && target != *from
                    {
                        by_target.entry(target).or_default().push(*from);
                    }
                }
            }
            for sources in by_target.values_mut() {
                sources.sort_unstable();
                sources.dedup();
            }
            References { by_target }
        })
    }
}

fn direct_callers(
    served: &Served,
    reverse: &crate::graph::Reverse,
    node: NodeId,
    floor: Confidence,
) -> Vec<(NodeId, Confidence)> {
    direct_callers_with(served, reverse, node, floor, true)
}

/// `uses`: whether names count beside calls. `reverse_levels` takes them at
/// the first hop only: a user of a type is affected by it, but what calls that
/// user is affected through the call. Carried further, a type pulled in every
/// method whose receiver names it and then all of their callers — same recall
/// on the foreign sets, 2.5-4x the answer.
fn direct_callers_with(
    served: &Served,
    reverse: &crate::graph::Reverse,
    node: NodeId,
    floor: Confidence,
    uses: bool,
) -> Vec<(NodeId, Confidence)> {
    let mut out = Vec::new();

    for (caller, edge) in reverse.callers(node) {
        if edge.confidence < floor {
            continue;
        }
        if served.defined.contains(&caller) {
            out.push((caller, edge.confidence));
            continue;
        }
        for (through, call) in reverse.callers(caller) {
            if call.confidence >= floor && through != node {
                out.push((through, edge.confidence.min(call.confidence)));
            }
        }
    }
    // After the calls, so a caller that also names the symbol keeps its
    // call's confidence. A use by name is matched by name, the weakest tier.
    if uses && Confidence::Ambiguous >= floor {
        let local = std::sync::OnceLock::new();
        for &from in served
            .references(&local)
            .by_target
            .get(&node)
            .into_iter()
            .flatten()
        {
            out.push((from, Confidence::Ambiguous));
        }
    }
    out
}

/// Everything that reaches `seeds` backwards, one level per hop. Breadth-first,
/// so a node appears at its shortest distance — the hop count worth reporting,
/// since a direct caller almost certainly breaks and a fourth-hop one probably
/// does not. One sweep from all seeds at once: a diff overlaps heavily, and a
/// caller reached from two changed symbols belongs in the answer once.
fn reverse_levels(
    served: &Served,
    seeds: &[NodeId],
    depth: usize,
    floor: Confidence,
) -> Vec<Vec<(NodeId, Confidence)>> {
    let reverse = served.snap.reverse();
    let mut seen: std::collections::HashSet<NodeId> = seeds.iter().copied().collect();
    let mut frontier: Vec<NodeId> = seen.iter().copied().collect();
    frontier.sort_unstable();
    let mut levels = Vec::new();
    for hop in 0..depth {
        let mut next: Vec<(NodeId, Confidence)> = Vec::new();
        for &n in &frontier {
            for (caller, confidence) in direct_callers_with(served, reverse, n, floor, hop == 0) {
                if seen.insert(caller) {
                    next.push((caller, confidence));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        // Deterministic: the reverse index is built in node order, but a
        // frontier merges several sources.
        next.sort_by_key(|&(n, _)| n);
        frontier = next.iter().map(|&(n, _)| n).collect();
        levels.push(next);
    }
    levels
}

fn render_detect_changes(v: &Value) -> String {
    let files = v["changed_files"].as_u64().unwrap_or(0);
    if files == 0 {
        return "no files changed\n".to_string();
    }
    let syms = v["changed_symbols"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let unknown = v["files_without_known_symbols"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let mut out = format!(
        "{files} file(s) changed, {} symbol(s) in them\n",
        syms.len()
    );
    out.push_str(&render_risk(&v["risk"]));
    if !unknown.is_empty() {
        out.push_str(&format!(
            "\n{} changed file(s) define nothing this graph knows — a file added \
since the last analysis is not in it yet:\n",
            unknown.len()
        ));
        for f in unknown {
            out.push_str(&format!("  {}\n", f.as_str().unwrap_or("")));
        }
    }
    if syms.is_empty() {
        return out;
    }
    out.push_str("\nchanged:\n");
    for s in syms.iter().take(CHANGE_LIST_LIMIT) {
        out.push_str(&format!("  {}\n", s.as_str().unwrap_or("")));
    }
    if syms.len() > CHANGE_LIST_LIMIT {
        out.push_str(&format!(
            "  … and {} more (all of them in structuredContent)\n",
            syms.len() - CHANGE_LIST_LIMIT
        ));
    }
    let total = v["dependents"].as_u64().unwrap_or(0);
    if total == 0 {
        out.push_str("\nnothing in the graph depends on what changed.\n");
        return out;
    }
    out.push_str(&format!("\n{total} symbol(s) depend on the change:\n"));
    for level in v["hops"].as_array().map_or(&[][..], Vec::as_slice) {
        let ss = level["symbols"].as_array().map_or(&[][..], Vec::as_slice);
        out.push_str(&format!(
            "\n{} hop ({}):\n",
            level["hop"].as_u64().unwrap_or(0),
            ss.len()
        ));
        for sm in ss.iter().take(CHANGE_LIST_LIMIT) {
            out.push_str(&format!("  {}\n", sm.as_str().unwrap_or("")));
        }
        if ss.len() > CHANGE_LIST_LIMIT {
            out.push_str(&format!("  … and {} more\n", ss.len() - CHANGE_LIST_LIMIT));
        }
    }
    out
}

/// Whether a symbol is a test, by where it lives or how it is named. Broader
/// than `resolve::is_test_path`, which only keeps test definitions from
/// capturing production calls: selecting tests has to recognise the
/// conventions of every language that names them — `tests/`, `spec/`,
/// `__tests__/`, `test_x.py`, `x_test.go`, `x.test.ts`, `x.spec.js`,
/// `x_spec.rb`, `FooTest.java`, `FooTests.cs`, and `test_x`, `TestX`, `testX`.
/// A production function named `test_connection` is misread; the cost is one
/// extra test in the list, never a missing one.
pub fn is_test_symbol(symbol: &str) -> bool {
    let Some((file, name)) = symbol.split_once('#') else {
        return false;
    };
    let lower = file.to_ascii_lowercase();
    let in_test_dir = [
        "test/",
        "tests/",
        "spec/",
        "specs/",
        "__tests__/",
        "testing/",
    ]
    .iter()
    .any(|d| lower.starts_with(d) || lower.contains(&format!("/{d}")));
    let base = file.rsplit('/').next().unwrap_or(file);
    let stem = base.split('.').next().unwrap_or(base);
    let lower_base = base.to_ascii_lowercase();
    let test_file = lower_base.starts_with("test_")
        || ["_test.", ".test.", ".spec.", "_spec.", "-test.", "-spec."]
            .iter()
            .any(|m| lower_base.contains(m))
        || stem.ends_with("Test")
        || stem.ends_with("Tests");
    let after = |p: &str| {
        name.strip_prefix(p)
            .and_then(|r| r.chars().next())
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
    };
    let test_name = name.starts_with("test_") || after("Test") || after("test");
    in_test_dir || test_file || test_name
}

/// Hops a test may sit from what it covers. A test reaches code through
/// placeholders and helpers, each a hop, so the blast-radius default of 3
/// misses most of them.
const TEST_DEPTH: u64 = 12;

/// Which tests reach a symbol or a diff: the answer to "what do I run before
/// I commit this". `impact` lists every dependent, and an agent then has to
/// guess which of them are tests.
fn affected_tests(served: &Served, args: &Value) -> Result<Value, String> {
    let depth = args["depth"].as_u64().unwrap_or(TEST_DEPTH).clamp(1, 30) as usize;
    let floor = parse_confidence(args["min_confidence"].as_str().unwrap_or("ambiguous"));
    let mut single = None;
    let (target, seeds, unknown) = match args["symbol"].as_str() {
        Some(symbol) => {
            let node = served.resolve_one(symbol)?;
            single = Some(node);
            (served.name(node), vec![node], Vec::new())
        }
        None => {
            let diff = changed_symbols(served, args["rev"].as_str().unwrap_or(""))?;
            let target = format!("{} changed file(s)", diff.files.len());
            (
                target,
                diff.touched.iter().map(|&(_, n)| n).collect(),
                diff.unknown,
            )
        }
    };

    // A changed test is a test to run, at distance zero.
    let mut found: std::collections::BTreeMap<String, usize> = Default::default();
    let hop0 = seeds.iter().map(|&n| (n, 0));
    let reached = reverse_levels(served, &seeds, depth, floor)
        .into_iter()
        .enumerate()
        .flat_map(|(i, level)| level.into_iter().map(move |(n, _)| (n, i + 1)));
    for (node, hop) in hop0.chain(reached) {
        let name = served.name(node);
        if served.defined.contains(&node) && is_test_symbol(&name) {
            found.entry(name).or_insert(hop);
        }
    }
    let mut tests: Vec<(String, usize)> = found.into_iter().collect();
    tests.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut files: Vec<String> = tests
        .iter()
        .filter_map(|(t, _)| t.split_once('#').map(|(f, _)| f.to_string()))
        .collect();
    files.sort();
    files.dedup();
    // Tests naming it without an edge — a type in a fixture, a constant
    // compared against.
    let naming: Vec<String> = single.map_or_else(Vec::new, |node| {
        served
            .unconnected_mentions(node, &[])
            .into_iter()
            .filter(|f| is_test_symbol(&format!("{f}#x")) && !files.contains(f))
            .collect()
    });

    Ok(json!({
        "target": target,
        "count": tests.len(),
        "named_elsewhere": naming,
        "files": files,
        "tests": tests
            .iter()
            .map(|(t, hop)| json!({
                "symbol": t,
                "file": t.split_once('#').map_or("", |(f, _)| f),
                "hop": hop,
            }))
            .collect::<Vec<_>>(),
        "depth": depth,
        "files_without_known_symbols": unknown,
    }))
}

fn render_affected_tests(v: &Value) -> String {
    let target = v["target"].as_str().unwrap_or("");
    let tests = v["tests"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = format!("{target}\n\n");
    let unknown = v["files_without_known_symbols"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    if !unknown.is_empty() {
        out.push_str("changed but unknown to the graph, so not traced:\n");
        for f in unknown {
            out.push_str(&format!("  {}\n", f.as_str().unwrap_or("")));
        }
        out.push('\n');
    }
    if tests.is_empty() {
        out.push_str(&format!(
            "no test in the graph reaches this within {} hops. That is not \
proof it is untested: a test in a file the graph does not index, or one that \
calls through dynamic dispatch, is invisible here.\n",
            v["depth"].as_u64().unwrap_or(0)
        ));
        let naming = v["named_elsewhere"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        if !naming.is_empty() {
            out.push_str(&format!(
                "\nbut {} test file(s) name it without an edge the graph records — \
run them:\n",
                naming.len()
            ));
            for f in naming {
                out.push_str(&format!("  {}\n", f.as_str().unwrap_or("")));
            }
        }
        return out;
    }
    let files = v["files"].as_array().map_or(0, Vec::len);
    out.push_str(&format!(
        "{} test(s) in {files} file(s), nearest first:\n",
        tests.len()
    ));
    for t in tests.iter().take(CHANGE_LIST_LIMIT) {
        out.push_str(&format!(
            "  {:>2} hop  {}\n",
            t["hop"].as_u64().unwrap_or(0),
            t["symbol"].as_str().unwrap_or("")
        ));
    }
    if tests.len() > CHANGE_LIST_LIMIT {
        out.push_str(&format!(
            "  … and {} more (all of them in structuredContent)\n",
            tests.len() - CHANGE_LIST_LIMIT
        ));
    }
    out.push_str("\nfiles to run:\n");
    for f in v["files"].as_array().map_or(&[][..], Vec::as_slice) {
        out.push_str(&format!("  {}\n", f.as_str().unwrap_or("")));
    }
    out
}

/// Files that change in the same commits as one file, with whether the graph
/// connects them at all. The ones it does not are the hidden coupling: a
/// template, a migration, a fixture in another language — what `impact`
/// cannot see because no code names it.
fn co_changes(served: &Served, args: &Value) -> Result<Value, String> {
    let root = served
        .root
        .ok_or("this server has no tree on disk to read history from")?;
    let file = match (args["file"].as_str(), args["symbol"].as_str()) {
        (Some(f), _) => f.trim_start_matches("./").to_string(),
        (None, Some(sym)) => {
            let node = served.resolve_one(sym)?;
            let name = served.name(node);
            name.split_once('#')
                .map(|(f, _)| f.to_string())
                .ok_or(format!("{name} is not defined in a file"))?
        }
        (None, None) => return Err("pass `file` or `symbol`".into()),
    };
    let window = args["commits"].as_u64().unwrap_or(500).clamp(10, 5000) as usize;
    let top = args["top"].as_u64().unwrap_or(10).clamp(1, 100) as usize;

    let commits = crate::history::commits(root, window)?;
    let (own, pairs) = crate::history::coupled(&commits, &file);

    let fallback = std::sync::OnceLock::new();
    let graph = served.file_graph(&fallback);
    let index = |f: &str| graph.files.binary_search_by(|x| x.as_str().cmp(f)).ok();
    let linked = |other: &str| match (index(&file), index(other)) {
        (Some(a), Some(b)) => {
            graph.edges.contains_key(&(a, b)) || graph.edges.contains_key(&(b, a))
        }
        _ => false,
    };

    Ok(json!({
        "file": file,
        "commits": own,
        "window": commits.len(),
        "coupled": pairs
            .iter()
            .take(top)
            .map(|(f, n)| json!({
                "file": f,
                "together": n,
                "share": round_to(*n as f64 / own.max(1) as f64, 100.0),
                "in_graph": linked(f),
            }))
            .collect::<Vec<_>>(),
    }))
}

fn render_co_changes(v: &Value) -> String {
    let file = v["file"].as_str().unwrap_or("");
    let own = v["commits"].as_u64().unwrap_or(0);
    let window = v["window"].as_u64().unwrap_or(0);
    if own == 0 {
        return format!("{file}\n\nno change to it in the last {window} commits.\n");
    }
    let coupled = v["coupled"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = format!("{file}\n\nchanged in {own} of the last {window} commits.");
    if coupled.is_empty() {
        out.push_str(&format!(
            " No other file changed with it {} times or more.\n",
            crate::history::MIN_TOGETHER
        ));
        return out;
    }
    out.push_str(" Files that changed with it:\n\n");
    for c in coupled {
        let share = c["share"].as_f64().unwrap_or(0.0) * 100.0;
        let note = if c["in_graph"].as_bool().unwrap_or(false) {
            ""
        } else {
            "   history only: no edge in the graph"
        };
        out.push_str(&format!(
            "  {:>3}x {:>4.0}%  {}{note}\n",
            c["together"].as_u64().unwrap_or(0),
            share,
            c["file"].as_str().unwrap_or("")
        ));
    }
    out
}

/// Files whose definitions are data or entry points rather than code a caller
/// reaches — keys, selectors, build targets, schema fields.
const NOT_CALLED: &[&str] = &[
    "json",
    "yaml",
    "yml",
    "xml",
    "svg",
    "css",
    "scss",
    "sass",
    "less",
    "html",
    "htm",
    "ini",
    "properties",
    "env",
    "cfg",
    "conf",
    "md",
    "markdown",
    "rst",
    "bib",
    "tex",
    "txt",
    "mk",
    "tf",
    "tfvars",
    "hcl",
    "proto",
    "graphql",
    "gql",
    "sql",
    "prisma",
    "toml",
    "mod",
    // A header declares an interface for other translation units, as a
    // `.d.ts` does: measured on ktor, a vendored curl.h was 310 reports.
    "h",
    "hh",
    "hpp",
    "hxx",
];
/// Build files named without an extension; their targets are entry points.
const NOT_CALLED_NAMES: &[&str] = &["Makefile", "Justfile", "Dockerfile", "Rakefile", "Gemfile"];

/// Definitions whose name nothing in the tree refers to. Strict on purpose,
/// because the reader acts on it by deleting: a definition counts as used if
/// anything else points at it, *or* if any call anywhere names it bare —
/// which covers `self.x()`, calls through an import alias, and names defined
/// in several places. What it cannot see is named in the answer.
fn find_unused(served: &Served, args: &Value) -> Result<Value, String> {
    let scope = args["path"].as_str().unwrap_or("").trim_start_matches("./");
    let (offset, limit) = page(args, 50);
    let reverse = served.snap.reverse();

    let called_bare: std::collections::HashSet<&str> = served
        .registry
        .entries()
        .filter(|(s, _)| !s.contains('#'))
        .filter(|&(_, &n)| reverse.callers(n).next().is_some())
        // A placeholder scoped by an import (`f from a.py`) is a call of `f`.
        .map(|(s, _)| crate::imports::unscope(s).map_or(s.as_str(), |(name, _)| name))
        .collect();

    let mut uncalled: Vec<(&str, &str)> = Vec::new();
    let mut checked = 0;
    for (symbol, &node) in served.registry.entries() {
        let Some((file, name)) = symbol.split_once('#') else {
            continue;
        };
        let base = file.rsplit('/').next().unwrap_or(file);
        let ext = base
            .rsplit_once('.')
            .map_or("", |(_, e)| e)
            .to_ascii_lowercase();
        if !served.defined.contains(&node)
            || !file.starts_with(scope)
            || NOT_CALLED.contains(&ext.as_str())
            || NOT_CALLED_NAMES.contains(&base)
            || name == "<module>"
            || name == "main"
            // A package or namespace declaration, which nothing calls.
            || name.contains('.')
            || ["testdata/", "fixtures/"].iter().any(|d| file.to_ascii_lowercase().contains(d))
            || (name.starts_with("__") && name.ends_with("__"))
            || is_test_symbol(symbol)
        {
            continue;
        }
        checked += 1;
        let pointed_at = reverse.callers(node).any(|(c, _)| {
            c != node
                && !served
                    .name(c)
                    .split_once('#')
                    .is_some_and(|(f, _)| crate::docs::is_markdown(std::path::Path::new(f)))
        });
        if !pointed_at && !called_bare.contains(name) {
            uncalled.push((file, name));
        }
    }

    // The graph records calls, not every reference: a type in a signature, a
    // constant read, a module declaration or a handler passed by name never
    // becomes an edge. Measured, that made 218 of 685 definitions here look
    // unused and every one of them was named elsewhere. So the source has the
    // last word — every text file in the tree, not only the indexed ones, since
    // a Godot scene or an HTML template connects a handler by name alone. A
    // name written more often than it is defined is in use.
    let root = served
        .root
        .ok_or("this server has no tree on disk to check the source against")?;
    let mut wanted: std::collections::HashMap<&str, (usize, usize)> = Default::default();
    for &(_, name) in &uncalled {
        wanted.entry(name).or_default().0 += 1;
    }
    let mut excused: std::collections::HashSet<(&str, &str)> = Default::default();
    // By file, once: filtering the whole list per file was quadratic, 14 s on
    // a 1M-line tree against 144 ms at 100k.
    let mut by_file: std::collections::HashMap<&str, Vec<&str>> = Default::default();
    for &(file, name) in &uncalled {
        by_file.entry(file).or_default().push(name);
    }
    // The counts come from the tree-wide pass `mentions` takes once per
    // served state; only the files holding a candidate are read here, for
    // what their own text says about it.
    let local = std::sync::OnceLock::new();
    let counts = &served.mentions(&local).counts;
    for (name, seen) in wanted.iter_mut() {
        seen.1 = counts.get(*name).copied().unwrap_or(0);
    }
    for (&file, names) in &by_file {
        let Ok(bytes) = std::fs::read(root.join(file)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        // By characters: a byte offset can land inside one and panic.
        let head: String = text.chars().take(1024).collect();
        let generated = head.contains("DO NOT EDIT") || head.contains("@generated");
        for &name in names {
            let annotated = served
                .registry
                .node_of(&format!("{file}#{name}"))
                .and_then(|n| served.registry.span(n))
                .is_some_and(|(start, _)| is_annotated(&text, start as usize));
            if generated || annotated || is_engine_callback(file, name) {
                excused.insert((file, name));
            }
        }
    }
    let mut unused: Vec<String> = uncalled
        .iter()
        .filter(|entry| !excused.contains(*entry))
        .filter(|(_, name)| wanted.get(name).is_some_and(|&(defs, seen)| seen <= defs))
        .map(|(file, name)| format!("{file}#{name}"))
        .collect();
    unused.sort();
    let mut value = json!({
        "path": scope,
        "checked": checked,
        "count": unused.len(),
        "unused": unused.iter().skip(offset).take(limit).collect::<Vec<_>>(),
    });
    next_offset(&mut value, offset, limit, unused.len());
    Ok(value)
}

/// Whether an attribute or decorator sits on the definition starting at
/// `start` — `#[test]`, `#[global_allocator]`, `@app.route`, `@Override`,
/// `[Fact]`. A framework calls such a definition; nothing in the tree does.
fn is_annotated(text: &str, start: usize) -> bool {
    let Some(before) = text.get(..start.min(text.len())) else {
        return false;
    };
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    // `override` (Kotlin, C#, Swift) is a call through an interface.
    let line_end = text[start.min(text.len())..]
        .find('\n')
        .map_or(text.len(), |i| start + i);
    let line = text.get(line_start..line_end).unwrap_or("");
    if before[line_start..].contains('@')
        || before[line_start..].contains("#[")
        || line.split_whitespace().any(|w| w == "override")
    {
        return true;
    }
    before[..line_start]
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .is_some_and(|l| l.starts_with('@') || l.starts_with("#[") || l.starts_with('['))
}

/// Methods a library or engine calls by a name it builds: a visitor's
/// `visit_<Node>`, an event handler's `on_<event>` or `On<Event>`, and Godot's
/// virtual callbacks — a per-language list, like Python's dunders, because
/// nothing in the source marks them. Measured on graphify, the visitor and
/// handler conventions were 16 of 43 reports.
fn is_engine_callback(file: &str, name: &str) -> bool {
    // JavaBeans accessors, which Spring, Jackson, JPA and every template
    // engine reach through the property name (`owner.address`), never the
    // method name.
    let bean = ["get", "set", "is"].iter().any(|p| {
        name.strip_prefix(p)
            .and_then(|r| r.chars().next())
            .is_some_and(|c| c.is_ascii_uppercase())
    });
    if bean && (file.ends_with(".java") || file.ends_with(".kt") || file.ends_with(".groovy")) {
        return true;
    }
    let handler = name
        .strip_prefix("On")
        .and_then(|r| r.chars().next())
        .is_some_and(|c| c.is_ascii_uppercase());
    if ["visit_", "depart_", "on_"]
        .iter()
        .any(|p| name.starts_with(p))
        || handler
    {
        return true;
    }
    const GODOT: &[&str] = &[
        "_ready",
        "_process",
        "_physics_process",
        "_input",
        "_unhandled_input",
        "_unhandled_key_input",
        "_shortcut_input",
        "_gui_input",
        "_notification",
        "_draw",
        "_enter_tree",
        "_exit_tree",
        "_init",
        "_initialize",
        "_finalize",
        "_to_string",
        "_get",
        "_set",
        "_get_property_list",
        "_validate_property",
        "_can_drop_data",
        "_drop_data",
        "_get_drag_data",
        "_make_custom_tooltip",
        "_has_point",
        "_get_minimum_size",
        "_integrate_forces",
        "_run",
        "_get_configuration_warnings",
    ];
    file.ends_with(".gd") && GODOT.contains(&name)
}

/// Every text file under `root` the indexer would not ignore, for a search the
/// graph cannot answer. Binary files and files over 1 MiB are skipped.
/// The files whose text names a symbol, for every name the tree defines exactly
/// once. The graph records calls; a type in a signature, a constant read or a
/// handler wired up in a scene is a reference it never sees, and `impact` then
/// said "changing it breaks no caller" about a symbol other files plainly use.
/// Measured on five foreign trees, the files it named held 0-25% of those
/// using a type. A name defined twice is left out: its mentions cannot be told
/// apart.
#[derive(Default)]
pub struct Mentions {
    files: Vec<String>,
    by_name: std::collections::HashMap<String, Vec<u32>>,
    /// How often each defined name is written anywhere in the tree, for
    /// `find_unused`: the same pass over every text file, taken once.
    counts: std::collections::HashMap<String, usize>,
}

impl Mentions {
    fn of(&self, name: &str) -> impl Iterator<Item = &str> {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .map(|&f| self.files[f as usize].as_str())
    }
}

impl Served<'_> {
    fn mentions<'m>(&'m self, fallback: &'m std::sync::OnceLock<Mentions>) -> &'m Mentions {
        self.mentions.unwrap_or(fallback).get_or_init(|| {
            let Some(root) = self.root else {
                return Mentions::default();
            };
            // One slot per defined name, looked up once per word and keyed by
            // the registry's own strings: a match is frequent, and a String
            // per match cost 1.6 s on OTP. A name is mentionable when the tree
            // defines it once and it reads as a word; every defined name is
            // counted, for `find_unused`. `$` splits words — a sigil or a
            // template (`$x` in PHP and Bash, `"big$size"` in Kotlin) — so a
            // definition spelled `$x` is counted when `x` is.
            #[derive(Default)]
            struct Slot {
                defs: usize,
                count: usize,
                // Some definition is spelled `$` + this name.
                sigil: bool,
                postings: Vec<u32>,
            }
            let mut slots: std::collections::HashMap<&str, Slot> = Default::default();
            for (symbol, &node) in self.registry.entries() {
                if let Some((_, name)) = symbol.split_once('#')
                    && self.defined.contains(&node)
                {
                    slots.entry(name).or_default().defs += 1;
                    if let Some(bare) = name.strip_prefix('$') {
                        slots.entry(bare).or_default().sigil = true;
                    }
                }
            }
            let mentionable = |name: &str, slot: &Slot| {
                slot.defs == 1
                    && name.len() > 2
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            };
            let mut out = Mentions::default();
            for path in text_files(root) {
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let file = out.files.len() as u32;
                let mut hit = false;
                for word in String::from_utf8_lossy(&bytes)
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                {
                    let Some((&name, slot)) = slots.get_key_value(word) else {
                        continue;
                    };
                    let mention = mentionable(name, slot);
                    let slot = slots.get_mut(word).expect("just found");
                    slot.count += 1;
                    if mention && slot.postings.last() != Some(&file) {
                        slot.postings.push(file);
                        hit = true;
                    }
                }
                if hit {
                    out.files.push(
                        path.strip_prefix(root)
                            .map(|r| r.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                    );
                }
            }
            for (name, slot) in slots {
                if slot.defs > 0 {
                    *out.counts.entry(name.to_string()).or_default() += slot.count;
                }
                // `x` written in the text also counts for a `$x` definition.
                if slot.sigil {
                    *out.counts.entry(format!("${name}")).or_default() += slot.count;
                }
                if !slot.postings.is_empty() {
                    out.by_name.insert(name.to_string(), slot.postings);
                }
            }
            out
        })
    }

    /// Files naming `node` that no edge in `reached` connects, sorted.
    fn unconnected_mentions(&self, node: NodeId, reached: &[NodeId]) -> Vec<String> {
        let own = self.name(node);
        let Some((home, name)) = own.split_once('#') else {
            return Vec::new();
        };
        let local = std::sync::OnceLock::new();
        let connected: std::collections::HashSet<String> = reached
            .iter()
            .filter_map(|&n| self.name(n).split_once('#').map(|(f, _)| f.to_string()))
            .collect();
        let mut files: Vec<String> = self
            .mentions(&local)
            .of(name)
            .filter(|f| *f != home && !connected.contains(*f))
            .map(str::to_string)
            .collect();
        files.sort();
        files
    }
}

fn text_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(kind) = e.file_type() else { continue };
            let own = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".glasir"));
            if own || kind.is_symlink() || crate::watcher::ignored_under(&p, Some(root)) {
                continue;
            }
            if kind.is_dir() {
                stack.push(p);
            } else if e.metadata().is_ok_and(|m| m.len() <= 1 << 20)
                && std::fs::read(&p).is_ok_and(|b| !b[..b.len().min(8192)].contains(&0))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn render_find_unused(v: &Value) -> String {
    let count = v["count"].as_u64().unwrap_or(0);
    let checked = v["checked"].as_u64().unwrap_or(0);
    let mut out =
        format!("{count} of {checked} definition(s) are not referenced anywhere in the graph");
    let unused = v["unused"].as_array().map_or(&[][..], Vec::as_slice);
    if unused.is_empty() {
        out.push_str(".\n");
        return out;
    }
    out.push_str(":\n");
    let mut last = "";
    for u in unused {
        let s = u.as_str().unwrap_or("");
        let (file, name) = s.split_once('#').unwrap_or(("", s));
        if file != last {
            out.push_str(&format!("\n{file}\n"));
            last = file;
        }
        out.push_str(&format!("  {name}\n"));
    }
    out.push_str(&more_line(v, count));
    out.push_str(
        "\nNot proof of dead code: a library's public API, a framework callback, a trait \
method called implicitly, or a name reached through reflection or a string is invisible \
here. Check before deleting.\n",
    );
    out
}

/// Where a tree keeps its architecture contract; `glasir guard` reads the same.
pub const RULES_FILE: &str = "glasir-rules.txt";

/// The architecture contract, checked against the served graph — the rules in
/// the tree's `glasir-rules.txt`, or ones passed in to test a boundary before
/// writing it down. Under `serve --watch` the graph follows the working tree,
/// so an agent can check a change before committing it, which is when a
/// violation is cheapest.
fn check_architecture(served: &Served, args: &Value) -> Result<Value, String> {
    let (text, source) = match args["rules"].as_str() {
        Some(r) => (r.to_string(), "argument"),
        None => {
            let root = served
                .root
                .ok_or("this server has no tree on disk; pass `rules` instead")?;
            let text = std::fs::read_to_string(root.join(RULES_FILE)).map_err(|_| {
                format!(
                    "no {RULES_FILE} in this tree. Pass `rules` to check one ad hoc, \
e.g. \"deny src/api -> src/db\" or \"no-cycles\"."
                )
            })?;
            (text, RULES_FILE)
        }
    };
    let rules = crate::guard::parse(&text)?;
    let violations = crate::guard::check(
        served.snap,
        served.registry,
        served.names,
        served.defined,
        &rules,
    );
    Ok(json!({
        "source": source,
        "rules": rules.len(),
        "holds": violations.is_empty(),
        "violations": violations
            .iter()
            .map(|v| json!({"rule": v.rule, "evidence": v.evidence}))
            .collect::<Vec<_>>(),
    }))
}

fn render_check_architecture(v: &Value) -> String {
    let rules = v["rules"].as_u64().unwrap_or(0);
    let source = v["source"].as_str().unwrap_or("");
    let violations = v["violations"].as_array().map_or(&[][..], Vec::as_slice);
    if violations.is_empty() {
        return format!("all {rules} rule(s) from {source} hold\n");
    }
    let mut out = format!(
        "{} of {rules} rule(s) from {source} broken:\n",
        violations.len()
    );
    for v in violations {
        out.push_str(&format!("\n{}\n", v["rule"].as_str().unwrap_or("")));
        let evidence = v["evidence"].as_array().map_or(&[][..], Vec::as_slice);
        for e in evidence.iter().take(CHANGE_LIST_LIMIT) {
            out.push_str(&format!("  {}\n", e.as_str().unwrap_or("")));
        }
        if evidence.len() > CHANGE_LIST_LIMIT {
            out.push_str(&format!(
                "  … and {} more\n",
                evidence.len() - CHANGE_LIST_LIMIT
            ));
        }
    }
    out
}

fn render_impact(v: &Value) -> String {
    let mut out = format!("{}\n\n", v["symbol"].as_str().unwrap_or(""));
    let levels = v["hops"].as_array().map_or(&[][..], Vec::as_slice);
    if v["dependents"].as_u64().unwrap_or(0) == 0 {
        if has_elsewhere(v) {
            out.push_str("nothing in the graph reaches this symbol");
            out.push_str(&render_elsewhere(v, ", but"));
        } else {
            out.push_str(
                "nothing in the graph reaches this symbol: changing it \
breaks no caller here. Callers in code the graph does not cover are not ruled out.\n",
            );
        }
        return out;
    }
    out.push_str(&format!(
        "{} symbols depend on this, within {} hops\n",
        v["dependents"].as_u64().unwrap_or(0),
        v["within"].as_u64().unwrap_or(0)
    ));
    for level in levels {
        let syms = level["symbols"].as_array().map_or(&[][..], Vec::as_slice);
        out.push_str(&format!(
            "\n{} hop ({}):\n",
            level["hop"].as_u64().unwrap_or(0),
            syms.len()
        ));
        for sm in syms {
            out.push_str(&format!(
                "  [{}]  {}\n",
                sm["confidence"].as_str().unwrap_or(""),
                sm["symbol"].as_str().unwrap_or("")
            ));
        }
    }
    out.push_str(&more_line(v, v["dependents"].as_u64().unwrap_or(0)));
    if has_elsewhere(v) {
        out.push_str(&render_elsewhere(v, "\nbeyond these,"));
    }
    out
}

fn has_elsewhere(v: &Value) -> bool {
    v["named_elsewhere"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
}

/// The files that name a symbol without an edge to it, after `lead`.
fn render_elsewhere(v: &Value, lead: &str) -> String {
    let files = v["named_elsewhere"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let mut out = format!(
        "{lead} its name appears in {} other file(s) no edge connects — a type, a \
constant, an import or a reference the graph does not record as a call. Read \
them before changing it:\n",
        files.len()
    );
    for f in files.iter().take(CHANGE_LIST_LIMIT) {
        out.push_str(&format!("  {}\n", f.as_str().unwrap_or("")));
    }
    if files.len() > CHANGE_LIST_LIMIT {
        out.push_str(&format!(
            "  … and {} more (all of them in structuredContent)\n",
            files.len() - CHANGE_LIST_LIMIT
        ));
    }
    out
}

/// Files that depend on each other in a loop, shortest cycle first.
///
/// Symbol edges are collapsed to the files their endpoints live in — the file
/// is already the prefix of every symbol name, so there is nothing to look up.
/// Only edges the tree defines on both ends count: a cycle through `HashMap::
/// new` is an artefact of two files using the standard library, not coupling.
///
/// Cycles are found by depth-first search with the recursion stack carried
/// explicitly, bounded by `max_len` — enumerating every elementary cycle is
/// exponential on a dense graph and the long ones are not actionable anyway.
///
/// **The confidence floor defaults to `extracted` here, unlike every other
/// tool.** A tier 2 callee is resolved by bare name, so two same-named
/// functions in different files collapse onto one placeholder and the graph
/// grows an edge between files that never reference each other. Measured on
/// this tree: at `ambiguous` the answer was 20 cycles, all but three of them
/// through such a collision (`main.rs -> mcp.rs -> main.rs`, where mcp.rs
/// names nothing in main.rs); at `extracted` only compiler-resolved edges
/// count and what remains is real. A cycle report is read as a verdict about
/// architecture, so a false one costs more than a missed one.
/// The first question asked about a tree nobody has read yet.
///
/// Not a search: no query, no seeds, no expansion. The partition already holds
/// this and `view` has drawn it since the map existed; no tool handed it over
/// as text. Without it a caller starts by guessing identifiers, which is the
/// vocabulary problem `query_graph` documents.
///
/// **Entry points are ranked by incoming edges, not by size.** What a reader
/// needs from a subsystem is the symbol everything else in it goes through, and
/// that is the one with callers. A long function calling thirty things is not
/// an entry point — the same distinction `community::hub_min_incoming` draws,
/// and for the same reason.
/// One subsystem while `overview` builds it: size, label, and its entry points
/// as (caller count, symbol) so the ranking never has to be parsed back out of
/// a rendered string.
type Group = (usize, String, Vec<(usize, String)>);

fn overview(served: &Served, args: &Value) -> Result<Value, String> {
    let limit = args["limit"].as_u64().unwrap_or(12).clamp(1, 60) as usize;
    let per = args["entry_points"].as_u64().unwrap_or(3).clamp(0, 10) as usize;

    // One pass to group, as `subsystem_names` does and for the same measured
    // reason: `members(cid)` per community is O(communities x nodes), which
    // took 13 seconds on a million-line tree.
    let mut by_community: std::collections::HashMap<u32, Vec<NodeId>> = Default::default();
    for (node, &cid) in served.communities.of_node.iter().enumerate() {
        let node = node as NodeId;
        if served.defined.contains(&node) {
            by_community.entry(cid).or_default().push(node);
        }
    }

    let reverse = served.snap.reverse();
    let mut groups: Vec<Group> = Vec::new();
    let mut cids: Vec<u32> = by_community.keys().copied().collect();
    cids.sort_unstable();
    for cid in cids {
        let members = &by_community[&cid];
        // Judged by defined symbols, not raw size: foreign names cluster with
        // each other and inflate a community while saying nothing about the
        // code. Same threshold the partition is reported by elsewhere.
        if members.len() < 2 {
            continue;
        }
        let mut counts: std::collections::HashMap<String, usize> = Default::default();
        for &m in members {
            if let Some((file, _)) = served.name(m).split_once('#') {
                // Markdown sections are graph nodes too, and a community of
                // them would be labelled `roadmap` — a document, offered as a
                // subsystem of the code. This tool answers "what is built
                // here", so a document is not one of the parts.
                if crate::docs::is_markdown(std::path::Path::new(file)) {
                    continue;
                }
                let stem = file.rsplit('/').next().unwrap_or(file);
                let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
                *counts.entry(stem.to_string()).or_insert(0) += 1;
            }
        }
        let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let Some((label, _)) = ranked.first() else {
            continue;
        };
        let label = label.clone();

        // **Entry points come from the labelled file, not from the whole
        // community.** The label is the majority file, so a community also
        // holding a few symbols from elsewhere would otherwise advertise them
        // as the way into it — the first version listed `csr.rs#add_node`
        // under `delta` and `graph.rs#Graph` under `embed`, which sends a
        // reader to the wrong file on the one question this tool exists to
        // answer.
        let mut byin: Vec<(usize, String)> = members
            .iter()
            .map(|&m| (reverse.callers(m).count(), served.name(m)))
            .filter(|(_, name)| {
                name.split_once('#').is_some_and(|(f, _)| {
                    let stem = f.rsplit('/').next().unwrap_or(f);
                    stem.rsplit_once('.').map_or(stem, |(s, _)| s) == label
                })
            })
            .collect();
        // Ties on name, so two runs of the same tree print the same list.
        byin.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let entries: Vec<(usize, String)> = byin
            .into_iter()
            // A symbol nothing calls is not an entry point, and printing one
            // as though it were is worse than printing fewer.
            .filter(|(n, _)| *n > 0)
            .take(per)
            .collect();
        groups.push((members.len(), label, entries));
    }
    groups.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    // Two communities can carry the same majority file — this tree has two
    // labelled `csr` and two `embed`. Printing a name twice reads as a
    // duplicate rather than as a real split, so they are merged: the sizes add
    // up and the strongest entry points survive.
    let mut merged: Vec<Group> = Vec::new();
    for (size, label, entries) in groups {
        match merged.iter_mut().find(|(_, l, _)| *l == label) {
            Some(slot) => {
                slot.0 += size;
                slot.2.extend(entries);
                // Re-rank the pooled list by caller count. This used to parse
                // the number back out of the rendered "name (n)" string, which
                // only worked because nothing else ever put a bracket there.
                slot.2.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                slot.2.truncate(per);
            }
            None => merged.push((size, label, entries)),
        }
    }
    merged.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let groups = merged;

    if groups.is_empty() {
        return Ok(json!({"symbols": 0, "files": 0, "subsystems": []}));
    }

    let symbols = served.defined.len();
    let files: std::collections::HashSet<&str> = served
        .registry
        .entries()
        .filter(|(_, n)| served.defined.contains(n))
        .filter_map(|(s, _)| s.split_once('#').map(|(f, _)| f))
        .collect();
    Ok(json!({
        "symbols": symbols,
        "files": files.len(),
        // Every subsystem, not only the `limit` shown: a caller reading the
        // JSON has no "... and N more" line to act on, and truncating the data
        // to match a display cap loses what the cap exists to keep readable.
        "subsystems": groups
            .iter()
            .map(|(size, label, entries)| json!({
                "name": label,
                "symbols": size,
                "entry_points": entries
                    .iter()
                    .map(|(callers, name)| json!({"symbol": name, "callers": callers}))
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
        "shown": limit.min(groups.len()),
    }))
}

fn render_overview(v: &Value) -> String {
    let groups = v["subsystems"].as_array().map_or(&[][..], Vec::as_slice);
    if groups.is_empty() {
        return "no subsystems: the tree has no partition yet, which means \
either no analysis has run or nothing in it is connected.\n"
            .to_string();
    }
    let limit = v["shown"].as_u64().unwrap_or(0) as usize;
    let mut out = format!(
        "{} symbols in {} files, {} subsystems\n\n",
        v["symbols"].as_u64().unwrap_or(0),
        v["files"].as_u64().unwrap_or(0),
        groups.len()
    );
    for g in groups.iter().take(limit) {
        out.push_str(&format!(
            "{}  ({} symbols)\n",
            g["name"].as_str().unwrap_or(""),
            g["symbols"].as_u64().unwrap_or(0)
        ));
        for e in g["entry_points"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "    {} ({})\n",
                e["symbol"].as_str().unwrap_or(""),
                e["callers"].as_u64().unwrap_or(0)
            ));
        }
    }
    if groups.len() > limit {
        out.push_str(&format!("\n... and {} more\n", groups.len() - limit));
    }
    out.push_str(
        "\nSubsystems are named after the file most of their symbols live in, \
and the number beside an entry point is how many symbols call it. \
Ask `query_graph` with one of these names to go deeper.\n",
    );
    out
}

impl Served<'_> {
    /// See `FileGraph`. `fallback` owns it where no cache exists; not a `Cow`,
    /// which wants `Clone` and invites copying what is built once.
    pub fn file_graph<'f>(&'f self, fallback: &'f std::sync::OnceLock<FileGraph>) -> &'f FileGraph {
        self.files.unwrap_or(fallback).get_or_init(|| {
            let mut file_of: std::collections::HashMap<NodeId, &str> =
                std::collections::HashMap::new();
            let mut placeholders: Vec<NodeId> = Vec::new();
            for (symbol, &node) in self.registry.entries() {
                match symbol.split_once('#') {
                    Some((file, _)) if self.defined.contains(&node) => {
                        file_of.insert(node, file);
                    }
                    // Tier 2 mints an unqualified placeholder for every callee and
                    // tier 3 links it onto the definition, so a cross-file call
                    // routes `a.rs#alpha -> beta -> b.rs#beta` and the middle hop
                    // belongs to no file. Reading only qualified names made every
                    // such cycle invisible — which is most of them, since a call
                    // within one file is not a cycle in the first place.
                    None => placeholders.push(node),
                    _ => {}
                }
            }
            // A placeholder stands in for whatever it resolves to.
            let mut stands_for: std::collections::HashMap<NodeId, &str> =
                std::collections::HashMap::new();
            // Only when a placeholder has exactly one file to stand for.
            //
            // The audit established that condition — it is what keeps
            // `deny mcp -> lsp` honest — but the code took the first
            // of however many there were. Measured, that invented
            // `src/ingest.rs -> src/parse_ast.rs`, a pair where neither file
            // names the other: some shared callee resolved to `parse_ast.rs`
            // for the file graph while `ingest.rs` was calling something else
            // of the same name. A cycle report is read as a verdict about
            // architecture, so a false one costs more than a missed one.
            for &p in &placeholders {
                let mut targets = self
                    .snap
                    .neighbors(p)
                    .filter_map(|e| file_of.get(&e.target).copied());
                if let Some(file) = targets.next()
                    && targets.all(|f| f == file)
                {
                    stands_for.insert(p, file);
                }
            }
            let mut files: Vec<&str> = file_of.values().copied().collect();
            files.sort_unstable();
            files.dedup();
            let index: std::collections::HashMap<&str, usize> =
                files.iter().enumerate().map(|(i, &f)| (f, i)).collect();

            let mut edges: std::collections::HashMap<(usize, usize), Confidence> =
                std::collections::HashMap::new();
            for (&node, &file) in &file_of {
                let from = index[file];
                for e in self.snap.neighbors(node) {
                    let target = file_of
                        .get(&e.target)
                        .or_else(|| stands_for.get(&e.target))
                        .copied();
                    if let Some(to_file) = target {
                        let to = index[to_file];
                        if to != from {
                            // The strongest edge decides whether a pair survives a
                            // floor: one compiler-resolved call is enough.
                            let slot = edges.entry((from, to)).or_insert(e.confidence);
                            if e.confidence > *slot {
                                *slot = e.confidence;
                            }
                        }
                    }
                }
            }
            FileGraph {
                files: files.into_iter().map(str::to_string).collect(),
                edges,
            }
        })
    }
}

fn cycles(served: &Served, args: &Value) -> Result<Value, String> {
    let max_len = args["max_len"].as_u64().unwrap_or(5).clamp(2, 12) as usize;
    let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 200) as usize;
    // Default to compiler-resolved edges when the tree has any, and fall back
    // to syntactic ones when it does not: a tree with no index would otherwise
    // report "no cycles" for want of a single edge strong enough to walk.
    let floor = match args["min_confidence"].as_str() {
        Some(named) => parse_confidence(named),
        None if has_extracted(served) => Confidence::Extracted,
        None => Confidence::Inferred,
    };

    let local = std::sync::OnceLock::new();
    let graph = served.file_graph(&local);
    let files = &graph.files;
    // Per call, against pairs rather than symbol edges.
    let mut adj: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); files.len()];
    for (&(from, to), &confidence) in &graph.edges {
        if confidence >= floor {
            adj[from].insert(to);
        }
    }

    // Each cycle is reported once, from its lowest-numbered file: a search
    // starting at `start` never steps onto a file below it, so a rotation
    // cannot be found twice and no separate dedup pass is needed.
    let mut found: Vec<Vec<usize>> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; files.len()];
    for start in 0..files.len() {
        if found.len() >= limit {
            break;
        }
        stack.push(start);
        on_stack[start] = true;
        walk(
            start,
            start,
            &adj,
            &mut stack,
            &mut on_stack,
            max_len,
            limit,
            &mut found,
        );
        on_stack[start] = false;
        stack.pop();
    }

    found.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
    found.truncate(limit);

    Ok(json!({
        "min_confidence": confidence_label(floor),
        "max_len": max_len,
        // The files in loop order, without repeating the first at the end: the
        // prose says "back to X" instead, and a consumer closing the loop
        // itself is better served than one that has to know the last entry is
        // a duplicate.
        "cycles": found
            .iter()
            .map(|cycle| json!({
                "files": cycle.iter().map(|&i| files[i].as_str()).collect::<Vec<_>>(),
                "length": cycle.len(),
            }))
            .collect::<Vec<_>>(),
    }))
}

fn render_cycles(v: &Value) -> String {
    let found = v["cycles"].as_array().map_or(&[][..], Vec::as_slice);
    if found.is_empty() {
        return format!(
            "no cycles up to {} files among {}-confidence edges: every \
dependency runs one way\n",
            v["max_len"].as_u64().unwrap_or(0),
            v["min_confidence"].as_str().unwrap_or("")
        );
    }
    let mut out = format!("{} cycle(s), tightest first\n\n", found.len());
    for c in found {
        let names: Vec<&str> = c["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        out.push_str(&format!(
            "{} files: {}\n",
            c["length"].as_u64().unwrap_or(0),
            names.join(" -> ")
        ));
        out.push_str(&format!("  back to {}\n", names.first().unwrap_or(&"")));
    }
    out
}

/// Whether any edge in the graph was compiler-resolved, which decides how much
/// a cycle report can afford to trust.
fn has_extracted(served: &Served) -> bool {
    (0..served.snap.width() as NodeId).any(|n| {
        served
            .snap
            .neighbors(n)
            .any(|e| e.confidence == Confidence::Extracted)
    })
}

/// One step of the cycle search. Split out because the recursion needs a name;
/// everything it touches is owned by `cycles`.
#[allow(clippy::too_many_arguments)]
fn walk(
    start: usize,
    at: usize,
    adj: &[std::collections::BTreeSet<usize>],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    max_len: usize,
    limit: usize,
    found: &mut Vec<Vec<usize>>,
) {
    if found.len() >= limit {
        return;
    }
    for &next in &adj[at] {
        if next == start {
            found.push(stack.clone());
            if found.len() >= limit {
                return;
            }
            continue;
        }
        // Below `start` means this cycle has a lower entry point and will be
        // (or was) found from there instead.
        if next < start || on_stack[next] {
            continue;
        }
        // The bound stops the *descent*, not the check above it: a cycle of
        // `max_len` files is found by walking `max_len` nodes and then seeing
        // the edge back to `start`. Testing before the loop returned one step
        // early, so `max_len: 2` reported "every dependency runs one way" on a
        // tree holding `audit.rs -> main.rs -> audit.rs`.
        if stack.len() >= max_len {
            continue;
        }
        stack.push(next);
        on_stack[next] = true;
        walk(start, next, adj, stack, on_stack, max_len, limit, found);
        on_stack[next] = false;
        stack.pop();
    }
}

/// Rounds to the precision the prose prints at.
///
/// The value is an `f32`, and widening one to `f64` carries its representation
/// error with it — `0.15f32 as f64` is 0.15000000596046448, so rounding after
/// the widening keeps every digit the text never showed. Rounding *in* `f64`
/// against the same scale the format string uses gives the number a reader
/// sees, which is the point of publishing it as data.
fn round_to(v: f64, scale: f64) -> f64 {
    (v * scale).round() / scale
}

/// The machine-readable half of a tool result, and the only place a tool
/// assembles its answer.
///
/// An agent parsing the prose is parsing a layout that exists to be read by a
/// person: `impact` groups by hop with indented headings, `cycles` writes
/// "back to" on its own line. Both are fine to read and awkward to consume,
/// and every change to the wording silently breaks whoever matched on it.
///
/// **The text is rendered from this, never assembled beside it.** Two
/// producers of the same facts drift — the prose gains a field the JSON never
/// learns about — which is the failure this exists to remove rather than
/// reproduce. So each tool builds a `Value` and a `render_*` turns it into the
/// string clients have always received; the wording is unchanged, and every
/// existing self-check and benchmark still asserts against it.
///
/// The spec (2025-06-18, "Structured Content") asks that the serialised JSON
/// also appear in a text block, for clients predating the field. That is not
/// what we send: a client that cannot read `structuredContent` is better
/// served by the prose these tools already write than by a wall of JSON it
/// would have to summarise, and the `outputSchema` tells a client that *can*
/// read it what shape to expect.
/// The source a symbol names, so a caller does not have to open the file.
///
/// Every other tool answers with names and lets the caller fetch the text
/// itself, which costs a round trip and a path the caller has to trust. The
/// range comes from the parse that built the graph, so it is the definition
/// the graph is talking about and not a guess from a line number.
///
/// Refuses rather than truncates a symbol the graph does not carry a range
/// for: a placeholder names a call site in someone else's crate, and there is
/// no text here to show for it.
fn get_code_snippet(served: &Served, args: &Value) -> Result<Value, String> {
    let symbol = args["symbol"].as_str().ok_or("symbol must be a string")?;
    let node = served.resolve_one(symbol)?;
    let name = served.name(node);
    let root = served
        .root
        .ok_or("this server has no tree on disk to read from")?;
    let (file, _) = name
        .split_once('#')
        .ok_or_else(|| format!("{name} is a placeholder, not a definition with source"))?;
    let (start, end) = served
        .registry
        .span(node)
        .ok_or_else(|| format!("no source range recorded for {name}"))?;

    let bytes = std::fs::read(root.join(file)).map_err(|e| format!("cannot read {file}: {e}"))?;
    // A range from an older parse can point past a file edited since; clamping
    // returns less rather than panicking on a slice.
    let (from, to) = (
        (start as usize).min(bytes.len()),
        (end as usize).min(bytes.len()),
    );
    if from >= to {
        return Err(format!("{file} has changed since {name} was indexed"));
    }
    // Latin-1 for the same reason the indexer uses it: a copyright header with
    // one accented byte must not cost the whole definition.
    let text = match std::str::from_utf8(&bytes[from..to]) {
        Ok(t) => t.to_string(),
        Err(_) => bytes[from..to].iter().map(|&b| b as char).collect(),
    };
    let line = 1 + bytes[..from].iter().filter(|&&b| b == b'\n').count();
    Ok(json!({
        "symbol": name,
        "file": file,
        "line": line,
        "bytes": [start, end],
        "source": text,
    }))
}

fn render_get_code_snippet(v: &Value) -> String {
    format!(
        "{}\n{}:{}\n\n{}\n",
        v["symbol"].as_str().unwrap_or(""),
        v["file"].as_str().unwrap_or(""),
        v["line"].as_u64().unwrap_or(0),
        v["source"].as_str().unwrap_or("")
    )
}

fn call_tool_json(served: &Served, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "get_code_snippet" => get_code_snippet(served, args),
        "query_graph" => query_graph(served, args),
        "overview" => overview(served, args),
        "shortest_path" => shortest_path(served, args),
        "explain_node" => explain_node(served, args),
        "impact" => impact(served, args),
        "cycles" => cycles(served, args),
        "find_callers" => find_callers(served, args),
        "detect_changes" => detect_changes(served, args),
        "affected_tests" => affected_tests(served, args),
        "co_changes" => co_changes(served, args),
        "check_architecture" => check_architecture(served, args),
        "find_unused" => find_unused(served, args),
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// The prose a client has always received, rendered from the structured answer.
fn render(name: &str, v: &Value) -> String {
    match name {
        "get_code_snippet" => render_get_code_snippet(v),
        "query_graph" => render_query_graph(v),
        "overview" => render_overview(v),
        "shortest_path" => render_shortest_path(v),
        "explain_node" => render_explain_node(v),
        "impact" => render_impact(v),
        "cycles" => render_cycles(v),
        "find_callers" => render_find_callers(v),
        "detect_changes" => render_detect_changes(v),
        "affected_tests" => render_affected_tests(v),
        "co_changes" => render_co_changes(v),
        "check_architecture" => render_check_architecture(v),
        "find_unused" => render_find_unused(v),
        _ => String::new(),
    }
}

fn error(id: Value, code: i64, message: String) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn capabilities() -> Value {
    // No listChanged: the served snapshot is fixed for the process, so the tool
    // list cannot change and promising notifications would be a lie.
    json!({"tools": {}})
}

fn server_info() -> Value {
    json!({"name": "glasir", "version": env!("CARGO_PKG_VERSION")})
}

/// Handles one request. `None` means the message was a notification and takes
/// no reply.
pub fn handle(served: &Served, msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str()?;
    let id = msg.get("id").cloned();

    // A notification has no id and must never be answered, not even on error.
    let id = id?;

    let response = match method {
        // Legacy handshake.
        "initialize" => {
            // Echo the client's version when we can speak it, otherwise name
            // ours — the client then decides whether to continue.
            let requested = msg["params"]["protocolVersion"].as_str();
            let version = match requested {
                Some(v) if v <= LEGACY_VERSION => v,
                _ => LEGACY_VERSION,
            };
            json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": version,
                "capabilities": capabilities(),
                "serverInfo": server_info(),
                "instructions": "A graph of this codebase. Prefer query_graph over \
            reading files when a question spans several symbols; each edge says whether it \
            was compiler-resolved, syntax-inferred or name-matched."
            }})
        }
        // Modern discovery: no handshake, version carried per request.
        "server/discover" => json!({"jsonrpc": "2.0", "id": id, "result": {
            "resultType": "complete",
            "supportedVersions": [MODERN_VERSION, LEGACY_VERSION],
            "capabilities": capabilities(),
            "_meta": {"io.modelcontextprotocol/serverInfo": server_info()},
        }}),
        "tools/list" => {
            json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tool_definitions()}})
        }
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = &msg["params"]["arguments"];
            match call_tool_json(served, name, args) {
                // Both halves of one answer: the prose a person or an older
                // client reads, and the same facts as data. The text is
                // rendered *from* the value, so they cannot disagree.
                Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": {
                    "content": [{"type": "text", "text": render(name, &value)}],
                    "structuredContent": value,
                    "isError": false
                }}),
                // A tool that fails on its inputs reports through the result,
                // not as a protocol error: the model should see and correct it.
                Err(e) => json!({"jsonrpc": "2.0", "id": id, "result": {
                    "content": [{"type": "text", "text": e}],
                    "isError": true
                }}),
            }
        }
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        // -32601 is JSON-RPC's "method not found".
        _ => error(id, -32601, format!("unknown method: {method}")),
    };
    Some(response)
}

/// Serves from a state that may be replaced under it, loading per request.
///
/// The stdio counterpart to the HTTP loop. A long-lived borrow would pin the
/// state a `--watch` server is trying to swap, and pin every superseded graph
/// in memory besides.
pub fn serve_swappable(state: &crate::published::Published<ServedState>) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&state.load().as_served(), &msg),
            Err(e) => Some(error(Value::Null, -32700, format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Serves until stdin closes, which is how a client signals shutdown.
pub fn serve(served: &Served) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(served, &msg),
            // -32700 is JSON-RPC's parse error. Null id: the request was
            // unreadable, so its id is unknown.
            Err(e) => Some(error(Value::Null, -32700, format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            // Line-buffered stdout would strand a reply until the next write,
            // and the client is waiting on it.
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Exposed for the self-check: exercising the dispatcher directly is what makes
/// the message shapes testable without spawning a process.
pub fn handle_for_test(served: &Served, msg: &Value) -> Option<Value> {
    handle(served, msg)
}

/// Exposed for the self-check: seed resolution decides whether a question finds
/// anything at all, and its precedence rules are worth testing directly.
pub fn seeds_for_test(served: &Served, query: &str) -> Vec<(NodeId, f32)> {
    served.seeds(query)
}
