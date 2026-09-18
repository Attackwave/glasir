//! Tier 1 of the cascade parser: SCIP index import.
//!
//! The compiler-grade tier. A SCIP index is produced by a real indexer
//! (`rust-analyzer`, `scip-python`, `scip-typescript`), so its symbols are
//! globally unique and its occurrences are resolved — no name guessing. Edges
//! from here carry `Confidence::Extracted`.
//!
//! This is the batch path, read once at load time.
//!
//! Tier 1's other half is the live client in `lsp.rs`: a save is parsed
//! syntactically first, then re-queried through the language server, which
//! replaces its own edges wholesale. So a saved file no longer degrades to
//! tier 2 until the next index build.
//!
//! Only the fields actually read are decoded — see `proto/scip_min.proto`.
//! Protobuf identifies fields by number, so a full index decodes correctly
//! against that subset and unknown fields cost nothing.

use crate::csr::{Confidence, Edge, NodeId};
use crate::ingest::SymbolRegistry;
use std::collections::HashMap;
use std::path::Path;

/// `SymbolRole.Definition` — bit 0 of `symbol_roles`. Everything else is a
/// reference.
const ROLE_DEFINITION: i32 = 0x1;

pub struct ScipImport {
    /// Call edges, already resolved to node ids.
    pub edges: Vec<(NodeId, Edge)>,
    /// Interned file path -> the nodes it defines, for delta scoping.
    pub file_nodes: HashMap<String, Vec<NodeId>>,
    pub definitions: usize,
    pub references: usize,
    /// References whose symbol is defined outside this index (another crate).
    /// Counted rather than linked: the definition genuinely is not here.
    pub external: usize,
    /// Files the index covers that have been edited since it was written. Their
    /// facts are stale, so tier 2 must re-parse them despite the index
    /// nominally covering them.
    pub stale_files: Vec<String>,
}

impl ScipImport {
    /// Files whose index data is still current.
    ///
    /// Freshness is per file, not per index. Editing one file does not
    /// invalidate what the index knows about the other two hundred, and
    /// treating it as all-or-nothing means either trusting stale facts or
    /// throwing away good ones.
    pub fn fresh_files(&self) -> impl Iterator<Item = &String> {
        self.file_nodes
            .keys()
            .filter(|f| !self.stale_files.contains(f))
    }
}

/// Indexers that can produce a SCIP index, by the marker file that says the
/// tree is theirs. Adding one is a row.
const INDEXERS: &[(&str, &[&str], &str)] = &[
    (
        "Cargo.toml",
        &["rust-analyzer", "scip", "."],
        "rust-analyzer",
    ),
    ("go.mod", &["scip-go"], "scip-go"),
    (
        "tsconfig.json",
        &["scip-typescript", "index"],
        "scip-typescript",
    ),
];

/// The indexer that applies to this tree, by its marker file.
pub fn indexer_for(
    root: &Path,
) -> Option<&'static (&'static str, &'static [&'static str], &'static str)> {
    INDEXERS
        .iter()
        .find(|(marker, _, _)| root.join(marker).exists())
}

/// Rebuilds the index for `root` in a detached process, if an indexer for this
/// tree is installed.
///
/// Detached on purpose: indexing takes seconds (~6 s for this repository) and
/// the watcher must keep serving edits meanwhile. The new index is picked up on
/// the next start; nothing waits on it.
///
/// Rebuilds an index that has gone out of date, without blocking the watcher.
///
/// Returns the command it started, or `None` when no indexer applies.
pub fn rebuild_in_background(root: &Path) -> Option<&'static str> {
    let (_, argv, name) = indexer_for(root)?;
    std::process::Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    Some(name)
}

/// Notices that a compiler index has gone out of date, per file: which ones
/// were edited since the index was built, or are not in this tree at all.
///
/// mtimes rather than hashes: reading every covered file to hash it costs more
/// than re-parsing the handful that changed, and a false positive only means
/// one file goes through tier 2 unnecessarily.
///
/// **An unreadable file is stale, not fresh.** The test used to be "is newer
/// than the index", which a missing file fails — so an index built for a
/// different tree reported every one of its documents as current. Measured on
/// a two-line fixture given this repository's index: 30 files claimed fresh,
/// 2,923 `Extracted` edges about code that is not there, and tier 2 skipped
/// the one real source file because the index claimed to cover it.
fn stale_since(index: &Path, root: &Path, files: impl Iterator<Item = String>) -> Vec<String> {
    let Ok(index_time) = std::fs::metadata(index).and_then(|m| m.modified()) else {
        // No mtime to compare against: treat everything as stale rather than
        // silently serving facts that may be months old.
        return files.collect();
    };
    let mut stale: Vec<String> = files
        .filter(|rel| {
            std::fs::metadata(root.join(rel))
                .and_then(|m| m.modified())
                .ok()
                // Not readable — absent, or outside this tree — so there is
                // nothing the index's claim can be checked against.
                .is_none_or(|t| t > index_time)
        })
        .collect();
    stale.sort();
    stale
}

/// Human-readable name from a SCIP symbol.
///
/// A symbol is `<scheme> <package> <descriptor>+`, where each descriptor ends
/// in a suffix marking its kind: `/` namespace, `#` type, `.` term, `().`
/// method, `!` macro. The last descriptor is the thing itself, so the tail
/// after the final separator is its name.
///
/// **A receiver in brackets is dropped, and that is what makes the cascade a
/// cascade.** rust-analyzer writes an inherent method as
/// `graph/impl#[GraphSnapshot]compact().`, and `#` is a descriptor separator
/// while `[` is not — so the tail was `[GraphSnapshot]compact`, which tier 2
/// never produces. The two tiers then named the same function differently and
/// built *separate nodes* instead of one superseding the other: measured on
/// this repository, **65 of 67 tier 1 methods existed twice**, and
/// `graph.rs#compact` three times over (`[GraphSnapshot]compact` with 0 edges,
/// `compact` with 8, a bare placeholder with 3).
///
/// It also poisoned search. The bracket tokenizes, so every method of
/// `DeltaStore` carried the words "delta" and "store": the question
/// "compaction of the delta store" scored twelve `[DeltaStore]` methods at an
/// identical 54.5 and spent every seed slot on them, while `graph.rs#compact`
/// — the answer — was not in the result at all. A fresh index cost 18 points
/// of `questions` recall against having no index whatsoever.
fn symbol_name(symbol: &str) -> Option<String> {
    // `local 3` and friends are file-local and not worth a graph node.
    if symbol.starts_with("local ") {
        return None;
    }
    let descriptors = symbol.rsplit(' ').next()?;
    // A descriptor chain ending in `/` is a *namespace* — a module path, not
    // something anyone calls. `crate/`, `fs/`, `io/`, `collections/` are the
    // common ones, and importing them minted nodes named after modules that
    // every file in the tree points at: measured, `src/main.rs#crate` collected
    // **51 incoming edges** and the index carried 2,339 such occurrences over
    // 82 distinct module names.
    //
    // It is not merely noise. `crate::auth::now()` in `audit.rs` became an edge
    // `audit.rs#write_line -> main.rs#crate`, which reads as a dependency from
    // the audit log into the binary's root — and `no-cycles` reported
    // `audit.rs -> main.rs -> audit.rs` on the strength of it. A false cycle is
    // read as a verdict about architecture, which is the one thing that check
    // must not get wrong.
    if descriptors.ends_with('/') {
        return None;
    }
    let trimmed = descriptors.trim_end_matches(['.', '#', '/', '!', ':', ')', '(']);
    let name = trimmed
        .rsplit(['/', '#', '.', '!', ':'])
        .next()?
        // A method descriptor is `name().`, so drop an argument list.
        .split('(')
        .next()?
        // `[Receiver]method` -> `method`, so tier 1 and tier 2 agree on the
        // name and land on one node.
        .rsplit(']')
        .next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Reads a SCIP index and resolves its occurrences into edges.
///
/// Two passes: definitions first, so a reference in the first document to a
/// symbol defined in the last still resolves. That is the property tier 3
/// cannot have — SCIP symbols are globally unique, so nothing is guessed here.
/// `root` is the tree the index describes, used to check whether its files have
/// been edited since it was built.
pub fn import(
    path: &Path,
    root: &Path,
    registry: &mut SymbolRegistry,
) -> std::io::Result<ScipImport> {
    let bytes = std::fs::read(path)?;
    let index = crate::scip_wire::decode_index(&bytes)
        .map_err(|e| std::io::Error::other(format!("scip: {e}")))?;

    // Pass 1: every definition, keyed by its globally unique SCIP symbol.
    let mut symbol_to_node: HashMap<&str, NodeId> = HashMap::new();
    // Per document, the definitions that can enclose a reference, as
    // (start_line, node). A local variable is a definition too, so attributing
    // a call to "the most recent definition seen" lands it on whatever was
    // declared last rather than on the function containing it.
    let mut enclosing: HashMap<&str, Vec<(i32, NodeId)>> = HashMap::new();
    let mut file_nodes: HashMap<String, Vec<NodeId>> = HashMap::new();
    let mut definitions = 0;

    for doc in &index.documents {
        for occ in &doc.occurrences {
            if occ.symbol_roles & ROLE_DEFINITION == 0 {
                continue;
            }
            let Some(name) = symbol_name(&occ.symbol) else {
                continue;
            };
            // Qualified the same way tier 2 does, so both tiers name the same
            // definition identically and land on one node.
            let node = registry.get_or_mint(&format!("{}#{}", doc.relative_path, name));
            symbol_to_node.insert(&occ.symbol, node);
            // Only a callable can enclose a call. A type or a term cannot, and
            // treating one as a scope swallows every call after it.
            if occ.symbol.ends_with("().") || occ.symbol.ends_with('!') {
                enclosing
                    .entry(&doc.relative_path)
                    .or_default()
                    .push((occ.range.first().copied().unwrap_or(0), node));
            }
            file_nodes
                .entry(doc.relative_path.clone())
                .or_default()
                .push(node);
            definitions += 1;
        }
    }

    // Pass 2: references, attributed to the definition enclosing them. The
    // document is carried alongside so a stale file's edges can be dropped
    // after `stale_since` has run.
    let mut edges: Vec<(&str, NodeId, Edge)> = Vec::new();
    let mut references = 0;
    let mut external = 0;

    for doc in &index.documents {
        // Functions in this document, by start line, so a reference can be
        // attributed to the last function beginning above it.
        let mut scopes = enclosing
            .remove(doc.relative_path.as_str())
            .unwrap_or_default();
        scopes.sort_unstable();

        for occ in &doc.occurrences {
            if occ.symbol_roles & ROLE_DEFINITION != 0 {
                continue;
            }
            if symbol_name(&occ.symbol).is_none() {
                continue;
            }
            references += 1;
            let Some(&target) = symbol_to_node.get(occ.symbol.as_str()) else {
                // Defined in another package; the index says so explicitly, so
                // there is nothing to guess at.
                external += 1;
                continue;
            };
            let line = occ.range.first().copied().unwrap_or(0);
            let Some(&(_, source)) = scopes.iter().rev().find(|&&(start, _)| start <= line) else {
                // A reference above the first function: a use statement or an
                // attribute, belonging to no callable.
                continue;
            };
            if source == target {
                continue;
            }
            edges.push((
                doc.relative_path.as_str(),
                source,
                Edge {
                    target,
                    timestamp: 0, // stamped by the caller, which knows the index mtime
                    authority: crate::physics::SOURCE_CODE,
                    edge_kind: 0,
                    confidence: Confidence::Extracted,
                },
            ));
        }
    }

    let stale_files = stale_since(path, root, file_nodes.keys().cloned());

    // An edge from a file the index no longer describes is dropped, not served.
    //
    // `stale_files` used to steer only which files tier 2 re-parses, while
    // every edge the index produced went into the graph regardless — so a
    // deleted function kept a live `Extracted` edge pointing at it, and the
    // freshness check that had already identified the file did nothing about
    // it. Measured on this repository with ten of thirty indexed files touched:
    // **610 of 2,923 edges (21%)** claimed compiler provenance for code that
    // had moved on. Tier 2 re-parses exactly these files, so dropping them
    // loses nothing it does not immediately replace.
    let stale: std::collections::HashSet<&str> = stale_files.iter().map(String::as_str).collect();
    let edges = edges
        .into_iter()
        .filter(|(file, _, _)| !stale.contains(file))
        .map(|(_, source, edge)| (source, edge))
        .collect();

    Ok(ScipImport {
        edges,
        file_nodes,
        definitions,
        references,
        external,
        stale_files,
    })
}

/// Exposed for the self-check: the symbol grammar is the part most likely to be
/// wrong, and it is worth pinning directly.
pub fn symbol_name_for_test(symbol: &str) -> Option<String> {
    symbol_name(symbol)
}

/// One occurrence in a test fixture: symbol, whether it is a definition, and
/// the line it sits on.
pub type TestOccurrence = crate::scip_wire::TestOccurrence;

/// Builds an index from (path, occurrences) for the self-check. Lines matter: a reference is attributed to the last function
/// starting above it.
/// Encoding it through the same generated types the importer decodes means the
/// fixture cannot drift from the schema.
pub fn encode_index_for_test(docs: &[crate::scip_wire::TestDocument<'_>]) -> Vec<u8> {
    crate::scip_wire::encode_index_for_test(docs)
}
