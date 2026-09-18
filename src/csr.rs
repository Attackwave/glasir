//! Compressed Sparse Row topology with parallel attribute arrays.
//!
//! Layout invariant: `row_offsets` has `|V| + 1` entries and is monotonically
//! non-decreasing; `col_indices` and every attribute array have `|E|` entries,
//! indexed by the same edge id. Metadata lives in separate flat arrays rather
//! than in a per-edge struct so a traversal that only reads timestamps does not
//! drag authority and confidence through the cache.

use serde::{Deserialize, Serialize};

pub type NodeId = u32;
pub type EdgeId = u32;

/// Provenance of an edge. The cascade parser degrades from `Extracted` down,
/// so a lower tier means the parser fell back, never that the edge is optional.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Confidence {
    /// Name-similarity heuristic across module boundaries.
    Ambiguous = 0,
    /// Native scanner, syntax-level only.
    Inferred = 1,
    /// SCIP / LSP, compiler-verified.
    Extracted = 2,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct BaseCsr {
    /// Length |V| + 1. `row_offsets[v]..row_offsets[v + 1]` are v's edge ids.
    row_offsets: Vec<EdgeId>,
    /// Length |E|. Target node of each edge.
    col_indices: Vec<NodeId>,
    /// Parallel attribute arrays, all length |E|.
    timestamp: Vec<u64>,
    authority: Vec<f32>,
    edge_kind: Vec<u16>,
    confidence: Vec<Confidence>,
    /// Interned key per node, resolved through the string arena.
    symbol: Vec<u32>,
}

/// One outgoing edge, gathered from the parallel arrays on demand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub target: NodeId,
    pub timestamp: u64,
    pub authority: f32,
    pub edge_kind: u16,
    pub confidence: Confidence,
}

/// Accessors over the owned, validated CSR representation.
macro_rules! impl_csr_accessors {
    ($t:ty) => {
        impl $t {
            pub fn node_count(&self) -> usize {
                // An empty graph has no offsets at all rather than a lone sentinel.
                self.row_offsets.len().saturating_sub(1)
            }

            pub fn edge_count(&self) -> usize {
                self.col_indices.len()
            }

            /// Edge id range of `node`'s outgoing edges, empty if the node is unknown.
            pub fn edge_range(&self, node: NodeId) -> std::ops::Range<usize> {
                let i = node as usize;
                if i + 1 >= self.row_offsets.len() {
                    return 0..0;
                }
                let (from, to): (EdgeId, EdgeId) = (self.row_offsets[i], self.row_offsets[i + 1]);
                from as usize..to as usize
            }

            pub fn edge(&self, e: EdgeId) -> Edge {
                let i = e as usize;
                Edge {
                    target: self.col_indices[i],
                    timestamp: self.timestamp[i],
                    authority: self.authority[i],
                    edge_kind: self.edge_kind[i],
                    confidence: self.confidence[i],
                }
            }

            pub fn neighbors(&self, node: NodeId) -> impl Iterator<Item = Edge> + '_ {
                self.edge_range(node).map(|e| self.edge(e as EdgeId))
            }

            pub fn symbol(&self, node: NodeId) -> Option<u32> {
                self.symbol.get(node as usize).copied()
            }
        }
    };
}

impl_csr_accessors!(BaseCsr);

/// Sorts edges by source and fills the parallel arrays. Building through this
/// type is the only way to get a `BaseCsr`, so the layout invariant holds by
/// construction.
#[derive(Default)]
pub struct CsrBuilder {
    edges: Vec<(NodeId, Edge)>,
    symbol: Vec<u32>,
}

impl CsrBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a node and returns its id. Nodes are dense and sequential.
    pub fn add_node(&mut self, symbol: u32) -> NodeId {
        self.symbol.push(symbol);
        (self.symbol.len() - 1) as NodeId
    }

    pub fn add_edge(&mut self, source: NodeId, edge: Edge) {
        self.edges.push((source, edge));
    }

    pub fn build(mut self) -> BaseCsr {
        let n = self.symbol.len();
        if n == 0 {
            return BaseCsr::default();
        }
        // Stable sort keeps insertion order within a node, so a file's edges
        // stay in parse order after a rebuild.
        self.edges.sort_by_key(|(src, _)| *src);

        let m = self.edges.len();
        let mut csr = BaseCsr {
            row_offsets: Vec::with_capacity(n + 1),
            col_indices: Vec::with_capacity(m),
            timestamp: Vec::with_capacity(m),
            authority: Vec::with_capacity(m),
            edge_kind: Vec::with_capacity(m),
            confidence: Vec::with_capacity(m),
            symbol: self.symbol,
        };

        let mut cursor = 0usize;
        for v in 0..n {
            csr.row_offsets.push(cursor as EdgeId);
            while cursor < m && self.edges[cursor].0 as usize == v {
                let e = self.edges[cursor].1;
                csr.col_indices.push(e.target);
                csr.timestamp.push(e.timestamp);
                csr.authority.push(e.authority);
                csr.edge_kind.push(e.edge_kind);
                csr.confidence.push(e.confidence);
                cursor += 1;
            }
        }
        csr.row_offsets.push(cursor as EdgeId);
        csr
    }
}
