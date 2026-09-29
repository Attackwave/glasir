//! Tier 0 of the cascade parser: import a pre-built graph from node-link JSON.
//!
//! This is scaffolding, not a permanent path. It yields a real, large graph
//! before our own parsers exist, which is what the storage and delta layers
//! need in order to be exercised under realistic load instead of against a toy
//! fixture. Once tier 2 feeds the `CsrBuilder` directly, a normal start parses
//! no JSON at all and this becomes an import tool for foreign graphs only —
//! so it is not worth optimising.
//!
//! The format is the widely used node-link encoding: a `nodes` array plus an
//! edge array spelled either `links` or `edges` depending on the producer, so
//! both are accepted. Parallel edges between the same pair of nodes are legal
//! and preserved — the source graph is a directed multigraph.

use crate::arena::SymbolArena;
use crate::csr::{BaseCsr, Confidence, CsrBuilder, Edge, NodeId};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
struct GraphJson {
    #[serde(default)]
    nodes: Vec<NodeJson>,
    /// node-link spelling.
    #[serde(default)]
    links: Vec<EdgeJson>,
    /// extraction-format spelling.
    #[serde(default)]
    edges: Vec<EdgeJson>,
}

#[derive(Deserialize)]
struct NodeJson {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    source_file: String,
}

#[derive(Deserialize)]
struct EdgeJson {
    source: String,
    target: String,
    #[serde(default)]
    relation: String,
    #[serde(default)]
    confidence: String,
    /// Present on INFERRED edges; EXTRACTED edges are always 1.0.
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    source_file: String,
}

/// An imported graph, plus the lookup tables the delta store needs to scope a
/// re-parse to one file.
pub struct Imported {
    pub csr: BaseCsr,
    /// Interned file path -> nodes defined in that file.
    pub file_nodes: HashMap<u32, Vec<NodeId>>,
    /// External node id -> our node id, for resolving later updates.
    pub by_external_id: HashMap<String, NodeId>,
    /// Edge relations, interned in first-seen order and indexed by `edge_kind`.
    pub relations: Vec<String>,
    pub skipped_edges: usize,
}

/// Maps the source graph's confidence tags onto ours. Unrecognised tags are
/// treated as the weakest tier rather than dropped, so an unknown tag degrades
/// the edge instead of losing it.
fn confidence_from(tag: &str) -> Confidence {
    match tag {
        "EXTRACTED" => Confidence::Extracted,
        "INFERRED" => Confidence::Inferred,
        _ => Confidence::Ambiguous,
    }
}

/// Authority weight of an edge. Producers typically emit a numeric score only
/// for inferred edges and leave extracted ones implicit at 1.0, so a missing
/// weight falls back to the tier's nominal authority rather than to zero — zero
/// would make the edge invisible to every downstream relevance filter.
fn authority_from(weight: Option<f64>, confidence: Confidence) -> f32 {
    weight.map(|w| w as f32).unwrap_or(match confidence {
        Confidence::Extracted => 1.0,
        Confidence::Inferred => 0.7,
        Confidence::Ambiguous => 0.3,
    })
}

pub fn import(path: &Path, arena: &mut SymbolArena, now: u64) -> std::io::Result<Imported> {
    // Bytes, not a String: serde_json validates UTF-8 as it parses, so
    // read_to_string's separate validation pass over the whole file is wasted.
    let raw = std::fs::read(path)?;
    let doc: GraphJson =
        serde_json::from_slice(&raw).map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut b = CsrBuilder::new();
    let mut by_external_id = HashMap::with_capacity(doc.nodes.len());
    let mut qualified = String::new();
    let mut file_nodes: HashMap<u32, Vec<NodeId>> = HashMap::new();

    for n in &doc.nodes {
        // The label is the human-readable name; the id is what edges reference.
        // Interning the qualified form keeps distinct symbols distinct. Built
        // in a reused buffer: a format! per node allocates once per node for
        // nothing.
        qualified.clear();
        qualified.push_str(&n.source_file);
        qualified.push('#');
        qualified.push_str(&n.label);
        let symbol = arena.intern(&qualified);
        let node = b.add_node(symbol);
        by_external_id.insert(n.id.clone(), node);
        if !n.source_file.is_empty() {
            file_nodes
                .entry(arena.intern(&n.source_file))
                .or_default()
                .push(node);
        }
    }

    let mut relations: Vec<String> = Vec::new();
    let mut skipped_edges = 0;
    // node-link uses `links`, the extraction format uses `edges`; a file has one
    // or the other, so chaining them costs nothing and accepts both.
    for e in doc.links.iter().chain(doc.edges.iter()) {
        let (Some(&source), Some(&target)) =
            (by_external_id.get(&e.source), by_external_id.get(&e.target))
        else {
            // An edge naming a node the file never declared. Dangling ids are
            // a known hazard of exported graphs, so drop it and report the
            // count rather than trust it.
            skipped_edges += 1;
            continue;
        };
        let edge_kind = match relations.iter().position(|r| r == &e.relation) {
            Some(i) => i as u16,
            None => {
                relations.push(e.relation.clone());
                (relations.len() - 1) as u16
            }
        };
        let confidence = confidence_from(&e.confidence);
        b.add_edge(
            source,
            Edge {
                target,
                // The format carries no per-edge timestamp, so everything
                // imported shares the import time. Temporal decay can only
                // start discriminating once live updates land.
                timestamp: now,
                authority: authority_from(e.weight, confidence),
                edge_kind,
                confidence,
            },
        );
        let _ = &e.source_file;
    }

    Ok(Imported {
        csr: b.build(),
        file_nodes,
        by_external_id,
        relations,
        skipped_edges,
    })
}
