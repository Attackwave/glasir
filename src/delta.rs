//! Delta store over the immutable base CSR.
//!
//! The base CSR is rebuilt only by compaction, so live edits land here instead:
//! `added_edges` holds new outgoing edges per node, `tombstoned_edges` masks
//! base edges that no longer exist. A read is the union of both
//! (base + added - tombstoned), which is what `GraphSnapshot` serves.

use crate::csr::{Edge, NodeId};
use std::collections::{HashMap, HashSet};

/// Identifies a base edge for masking. Edge ids are not stable across a
/// compaction, so a tombstone names the (source, target) pair instead.
///
/// ponytail: the graph is a multigraph — two nodes may be joined by several
/// edges of different relations — so one tombstone masks every base edge
/// between the pair. That is exactly right for file-level scoping, which drops
/// all of a node's outgoing edges anyway, but too coarse to retract a single
/// relation. Key on (source, target, edge_kind) if edge-level retraction is
/// ever needed.
pub type EdgeKey = (NodeId, NodeId);

#[derive(Default, Debug, Clone)]
pub struct DeltaStore {
    added_edges: HashMap<NodeId, Vec<Edge>>,
    tombstoned_edges: HashSet<EdgeKey>,
    /// Which nodes a file owns, so saving a file can invalidate exactly its
    /// outgoing edges. The base CSR does not carry this mapping.
    file_nodes: HashMap<u32, Vec<NodeId>>,
}

impl DeltaStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of buffered mutations, used to trigger compaction.
    pub fn len(&self) -> usize {
        self.added_edges.values().map(Vec::len).sum::<usize>() + self.tombstoned_edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Registers which nodes belong to a file. Called by the parser when a file
    /// is first ingested and on every re-parse.
    pub fn set_file_nodes(&mut self, file: u32, nodes: Vec<NodeId>) {
        self.file_nodes.insert(file, nodes);
    }

    pub fn file_nodes(&self, file: u32) -> &[NodeId] {
        self.file_nodes.get(&file).map_or(&[], Vec::as_slice)
    }

    /// File-level scoping: drops every outgoing edge of the file's nodes and
    /// replaces them with the freshly parsed ones. Base edges are masked by
    /// tombstone, delta edges are simply dropped.
    ///
    /// `base_targets` supplies the current base targets per node so the caller
    /// stays free of a borrow on the snapshot. `defines` lists the nodes the
    /// file declares, which is not derivable from `parsed` alone.
    pub fn replace_file_edges(
        &mut self,
        file: u32,
        base_targets: impl Fn(NodeId) -> Vec<NodeId>,
        parsed: Vec<(NodeId, Edge)>,
        defines: &[NodeId],
    ) {
        for &node in self.file_nodes.get(&file).map_or(&[][..], Vec::as_slice) {
            self.added_edges.remove(&node);
            for target in base_targets(node) {
                self.tombstoned_edges.insert((node, target));
            }
        }

        // Seeded with the file's definitions: one that makes no calls has no
        // outgoing edge to be discovered through, and would otherwise drop out
        // of the file's ownership and never be invalidated again.
        let mut nodes: Vec<NodeId> = defines.to_vec();
        for (source, edge) in parsed {
            // The base counterpart stays tombstoned: the freshly parsed edge
            // supersedes it and carries the current timestamp and confidence.
            // Un-masking it here would surface the same edge twice.
            self.added_edges.entry(source).or_default().push(edge);
            if !nodes.contains(&source) {
                nodes.push(source);
            }
        }
        self.file_nodes.insert(file, nodes);
    }

    /// Appends a single edge. The parser goes through `replace_file_edges`;
    /// this is the unscoped path used by tests and by incremental LSP diffs.
    pub fn add_edge(&mut self, source: NodeId, edge: Edge) {
        self.added_edges.entry(source).or_default().push(edge);
    }

    /// Replaces every edge of one kind, wherever it starts.
    ///
    /// Tier 3's links are recomputed over the whole registry after each batch
    /// rather than per file, so they have no owning file to be scoped by;
    /// without this they accumulate a duplicate set on every batch.
    pub fn replace_edges_of_kind(&mut self, kind: u16, edges: Vec<(NodeId, Edge)>) {
        for es in self.added_edges.values_mut() {
            es.retain(|e| e.edge_kind != kind);
        }
        self.added_edges.retain(|_, es| !es.is_empty());
        for (source, edge) in edges {
            self.added_edges.entry(source).or_default().push(edge);
        }
    }

    /// Highest node id referenced by the delta, as a source or a target.
    /// Needed to size a rebuild: the delta may mint ids past the base.
    pub fn max_node(&self) -> Option<NodeId> {
        let sources = self.added_edges.keys().copied();
        let targets = self
            .added_edges
            .values()
            .flat_map(|es| es.iter().map(|e| e.target));
        let files = self.file_nodes.values().flat_map(|ns| ns.iter().copied());
        sources.chain(targets).chain(files).max()
    }

    pub fn added(&self, node: NodeId) -> &[Edge] {
        self.added_edges.get(&node).map_or(&[], Vec::as_slice)
    }

    pub fn is_tombstoned(&self, source: NodeId, target: NodeId) -> bool {
        self.tombstoned_edges.contains(&(source, target))
    }
}
