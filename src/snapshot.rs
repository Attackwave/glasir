//! The whole analysed graph on disk, so a start maps it instead of rebuilding.
//!
//! `store.rs` persists the CSR, but a CSR alone is not enough to serve: nodes
//! carry an interned key, and the strings those keys resolve to live in the
//! arena, which was never written. Opening a stored CSR therefore gave a
//! nameless graph — which is why every command still rebuilt from source.
//!
//! This stores what a reader actually needs: topology, the symbol names, and
//! the partition. What it deliberately does not store is anything derivable in
//! microseconds — embeddings and the search index are rebuilt on load, because
//! they cost less to compute than to validate.

use crate::csr::{BaseCsr, NodeId};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Bumped when any stored field changes meaning. A snapshot from an older
/// build is discarded rather than misread — the alternative is a graph that
/// looks right and is not.
///
/// **"Changes meaning" includes a fix to what produces a field, not only a
/// change to its type**, and that distinction is what made this stale for
/// sixty-one commits. The audit repaired edge construction (steps 2-5) and the
/// partition (8a) without touching this number, so a binary carrying every fix
/// happily read a snapshot built before them. Measured on this tree, a new
/// build inheriting a pre-step-8a snapshot: **27.5% partition purity against
/// 88.5%, largest community 199 against 24** — `overview` announced
/// "http (201 symbols)" where a fresh build says "main (40)". Inheriting a
/// pre-step-5 snapshot costs recall too: 50/85 against 54/88.
///
/// The rule that follows, and it belongs in the same commit as any such fix:
/// **if a change alters the edges, names, documentation or partition a rebuild
/// would produce, bump this.** A stored graph is not a cache of the source, it
/// is a cache of *this code's reading* of the source.
///
/// 3: the step 2-5 and 8a audit repairs.
/// 4: definition byte ranges, so a tool can return source and not only a name.
const FORMAT_VERSION: u32 = 23;

#[derive(Serialize, Deserialize, Debug)]
pub struct Snapshot {
    version: u32,
    parser_rules: String,
    pub csr: BaseCsr,
    /// Qualified symbol per node, indexed by node id. The CSR's interned keys
    /// are meaningless without this.
    pub names: Vec<String>,
    /// Community id per node, so the partition survives too — it costs
    /// milliseconds to recompute but must match the stored layout exactly.
    pub community: Vec<u32>,
    pub hubs: Vec<NodeId>,
    /// Documentation per node, sparse: only symbols the parser found prose for.
    /// Stored rather than rebuilt, unlike the embeddings and the search index —
    /// those are derived from what is here, but documentation comes from
    /// re-reading and re-parsing every source file, which is the cost the
    /// snapshot exists to avoid. Leaving it out silently halved recall: the
    /// first run after an analysis was good and every later one fell back to
    /// names alone.
    pub docs: Vec<(NodeId, String)>,
    /// Byte range per definition, sparse and stored for the same reason as
    /// `docs`: recomputing it means re-parsing every file, which is the cost
    /// this format exists to avoid. Without it `get_code_snippet` works on the
    /// run that analysed and never again.
    pub spans: Vec<(NodeId, (u32, u32))>,
    /// Source mtimes at write time, keyed by path relative to the root, in
    /// nanoseconds. A snapshot is only usable while the tree it describes has
    /// not moved on. Seconds were too coarse: an edit landing in the same
    /// second as the run that recorded it kept the stored value and stayed
    /// invisible, measured at 20 misses out of 20.
    pub sources: Vec<(String, u64)>,
    /// Explicit cross-repository contracts observed for this exact snapshot.
    pub contracts: serde_json::Value,
}

impl Snapshot {
    /// The version is stamped here rather than by the caller: it describes the
    /// format, and a caller that could set it could write a lie.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        csr: BaseCsr,
        names: Vec<String>,
        community: Vec<u32>,
        hubs: Vec<NodeId>,
        docs: Vec<(NodeId, String)>,
        spans: Vec<(NodeId, (u32, u32))>,
        sources: Vec<(String, u64)>,
    ) -> Snapshot {
        Snapshot {
            version: FORMAT_VERSION,
            parser_rules: native_parsers::rules::active().identity().to_owned(),
            csr,
            names,
            community,
            hubs,
            docs,
            spans,
            sources,
            contracts: serde_json::Value::Null,
        }
    }
}

/// Stores the analysed graph between runs, so a later start can load it
/// without parsing source files again. The snapshot is decoded into owned
/// memory rather than memory-mapped, so corruption is rejected before a graph
/// can be queried.
pub fn write(snapshot: &Snapshot, path: &Path) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
    std::fs::write(path, &bytes)
}

/// Loads the analysed graph kept between runs, or `None` if it is missing,
/// from another version, or describes a tree that has since changed. This
/// avoids another complete analysis — parsing every source file again — when
/// the source times have not changed.
///
/// Staleness is checked against the recorded mtimes rather than a hash: reading
/// every source to hash it would cost more than the rebuild the check exists to
/// avoid.
pub fn read(path: &Path, root: &Path) -> Option<Snapshot> {
    match read_partial(path, root) {
        Some((snapshot, stale)) if stale.is_empty() => Some(snapshot),
        _ => None,
    }
}

/// Reads a snapshot together with the files that have moved on since it was
/// written, rather than discarding it whole.
///
/// The strict `read` above is this with an empty staleness list required.
/// Separating them is what makes an incremental start possible: one edited file
/// in a large tree used to mean re-parsing the tree, because the only answer to
/// "is this snapshot usable" was yes or no.
///
/// Staleness is still checked against mtimes rather than a hash: reading every
/// source to hash it would cost more than the rebuild the check exists to
/// avoid. `None` is returned only when the file is missing or unreadable as a
/// snapshot — that is a different failure from a tree that has changed.
pub fn read_partial(path: &Path, root: &Path) -> Option<(Snapshot, Vec<String>)> {
    let bytes = std::fs::read(path).ok()?;
    // A missing file and a stale version are ordinary and silent; a file that
    // is *there* and cannot be read as a snapshot is not. It rebuilds either
    // way, so nothing is wrong with the answer — but an operator watching a
    // start that got slower has no way to tell the two apart, and a snapshot
    // corrupted by a full disk or a killed write would look like normal
    // operation forever.
    let snapshot: Snapshot = match serde_json::from_slice(&bytes) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            eprintln!(
                "glasir: {} is not readable as a snapshot ({e}) — rebuilding",
                path.display()
            );
            return None;
        }
    };
    if snapshot.version != FORMAT_VERSION {
        return None;
    }

    if snapshot.parser_rules != native_parsers::rules::active().identity() {
        return None;
    }
    let mut stale = Vec::new();
    for (rel, stored) in &snapshot.sources {
        let current = std::fs::metadata(root.join(rel))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64);
        // Changed, vanished or unreadable — all three mean the stored facts for
        // this file describe code that is no longer there.
        if current != Some(*stored) {
            stale.push(rel.clone());
        }
    }
    Some((snapshot, stale))
}

/// Byte offset of the stored version stamp, for the self-check.
///
/// Finds the version digit in the canonical JSON object for the self-check.
pub fn version_offset_for_test(bytes: &[u8]) -> Option<usize> {
    let prefix = format!("\"version\":{FORMAT_VERSION}");
    bytes
        .windows(prefix.len())
        .position(|part| part == prefix.as_bytes())
        .map(|i| i + 10)
}

/// Source times (mtimes) for the files a kept graph covers, so a later run can
/// tell whether what it stored still describes the tree.
pub fn source_times(root: &Path, files: &[std::path::PathBuf]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = files
        .iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            let nanos = std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos() as u64;
            Some((rel, nanos))
        })
        .collect();
    // Sorted so the stored form is stable for the same tree.
    out.sort();
    out
}
