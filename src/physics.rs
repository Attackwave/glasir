//! Semantic physics: relevance filtering before token generation.
//!
//! A k-hop neighbourhood grows exponentially, so handing one to a model whole
//! is the context-gluttony anti-pattern this layer exists to prevent. Every
//! node reached during expansion carries a score, and anything below the
//! threshold is dropped here rather than costing tokens later.
//!
//! Three independent factors, multiplied:
//!
//! - **Temporal decay** `e^(-lambda*dt)`, so a function touched today outranks
//!   one last touched two years ago. Half-life driven, not a raw timestamp
//!   comparison, so the curve is tunable in units a human can reason about.
//! - **Authority**, from the edge's provenance: compiler-extracted outranks
//!   AST-inferred outranks a name-matched guess.
//! - **Distance decay**, so a node three hops out does not rank alongside a
//!   direct neighbour.
//!
//! All deterministic: no sampling, no learned weights. The same graph and
//! query always yield the same subgraph, which is what makes a result citable.

use crate::csr::{Confidence, NodeId};
use crate::graph::GraphSnapshot;
use std::collections::{BinaryHeap, HashMap};

/// What decides which nodes are worth returning for a query: the thresholds
/// and budget the relevance model reads.
///
/// Defaults are a starting point, not a
/// measured optimum — the values that matter depend on the repository.
#[derive(Debug, Clone, Copy)]
pub struct Physics {
    /// Time for a node's temporal weight to halve, in seconds.
    pub half_life_secs: f32,
    /// Score multiplier per hop away from a seed.
    pub hop_decay: f32,
    /// Nodes scoring below this are dropped. This is the subtraction in
    /// "scale by subtraction".
    pub threshold: f32,
    /// Hard cap on returned nodes, so a hub cannot blow the budget even if
    /// everything scores above the threshold.
    pub max_nodes: usize,
    /// Maximum hops from a seed.
    pub max_hops: usize,
    /// Score multiplier for a node not defined in the indexed tree.
    ///
    /// A reference into another crate (`push`, `collect`, `Some`) is a real
    /// edge and worth traversing *through* — two functions that both call it
    /// are related — but it is never itself the answer to "what should I read".
    /// Without this, foreign names crowd out every local symbol, because there
    /// are simply more of them.
    pub foreign_penalty: f32,
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            // Two weeks: recent work dominates without erasing last month's.
            half_life_secs: 14.0 * 86_400.0,
            hop_decay: 0.5,
            threshold: 0.01,
            max_nodes: 200,
            max_hops: 3,
            // Enough to sink a foreign name below any local symbol at the same
            // distance, without cutting the path that runs through it.
            foreign_penalty: 0.05,
        }
    }
}

/// How much a *source* is worth, independent of how well an edge was resolved
/// within it. The plan's second axis, and the one that only starts to matter
/// once the graph holds more than code: a design document can name a symbol
/// exactly and still be out of date, which is a different claim from "this
/// edge was guessed".
///
/// Multiplied with `authority(confidence)` in `expand`, so the two stay
/// independent — a documentation edge is not demoted to a guess, it is a
/// precise statement from a weaker source.
pub const SOURCE_CODE: f32 = 1.0;
/// A document describes intent; the compiler describes fact.
pub const SOURCE_DOC: f32 = 0.7;

/// Authority by provenance. The ordering is the point: a compiler-resolved edge
/// must outrank a guessed one, so a wrong guess cannot outweigh a fact.
pub fn authority(confidence: Confidence) -> f32 {
    match confidence {
        Confidence::Extracted => 1.0,
        Confidence::Inferred => 0.7,
        Confidence::Ambiguous => 0.3,
    }
}

/// `e^(-lambda*dt)` with `lambda = ln(2)/half_life`, so `dt == half_life`
/// yields exactly 0.5.
pub fn temporal_weight(age_secs: f32, half_life_secs: f32) -> f32 {
    if half_life_secs <= 0.0 {
        return 1.0;
    }
    (-std::f32::consts::LN_2 * age_secs.max(0.0) / half_life_secs).exp()
}

/// A node that survived filtering, with why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scored {
    pub node: NodeId,
    pub score: f32,
    pub hops: usize,
}

/// Ordering wrapper: `BinaryHeap` needs `Ord`, and scores are floats.
/// Best-first, with the node id breaking ties so expansion is deterministic.
#[derive(PartialEq)]
struct Candidate(f32, NodeId, usize);

impl Eq for Candidate {}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| other.1.cmp(&self.1))
    }
}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Expands outward from `seeds`, each weighted by how well it matched, and
/// returns the nodes worth spending tokens on, best first.
///
/// Best-first rather than breadth-first: a strong path four hops out beats a
/// weak one at two, and the cap should be spent on the former. A node keeps its
/// best score if reached by several paths.
/// `defined` lists nodes the indexed tree defines; anything outside it is
/// penalised by `foreign_penalty`. Pass `None` to score every node equally.
pub fn expand(
    snap: &GraphSnapshot,
    seeds: &[(NodeId, f32)],
    now: u64,
    cfg: &Physics,
    defined: Option<&std::collections::HashSet<NodeId>>,
) -> Vec<Scored> {
    let mut best: HashMap<NodeId, Scored> = HashMap::new();
    let mut queue: BinaryHeap<Candidate> = BinaryHeap::new();

    // Seeds enter at the confidence the search had in them, so a weak lexical
    // match does not rank alongside an exact symbol hit — and its neighbours
    // inherit that discount as the expansion proceeds.
    for &(seed, weight) in seeds {
        let weight = weight.clamp(0.0, 1.0);
        // A seed is penalised for being foreign exactly like any other node.
        // It was not, and the omission was visible: a question containing the
        // word "split" seeded on `split_once` and `split_whitespace` — standard
        // library names that outranked every local symbol, because the penalty
        // only applied to nodes the expansion *reached*. The path out of a
        // foreign seed keeps its undiscounted weight, same as elsewhere, so a
        // local symbol found through one is still reachable.
        let local = defined.is_none_or(|d| d.contains(&seed));
        let rank = if local {
            weight
        } else {
            weight * cfg.foreign_penalty
        };
        best.insert(
            seed,
            Scored {
                node: seed,
                score: rank,
                hops: 0,
            },
        );
        queue.push(Candidate(weight, seed, 0));
    }

    while let Some(Candidate(score, node, hops)) = queue.pop() {
        // A better path to this node was already processed.
        if best.get(&node).is_some_and(|s| s.score > score) {
            continue;
        }
        if hops >= cfg.max_hops {
            continue;
        }
        for edge in snap.neighbors(node) {
            let age = now.saturating_sub(edge.timestamp) as f32;
            // The penalty applies to the node's own rank, not to the path
            // continuing through it, so a local symbol reached via a shared
            // utility is still found.
            let local = defined.is_none_or(|d| d.contains(&edge.target));
            let path = score
                * authority(edge.confidence)
                * edge.authority.clamp(0.0, 1.0)
                * temporal_weight(age, cfg.half_life_secs)
                * cfg.hop_decay;
            let next = if local {
                path
            } else {
                path * cfg.foreign_penalty
            };

            // Scale by subtraction: below the threshold it is noise, and its
            // neighbours can only score lower, so the branch is cut entirely.
            if next < cfg.threshold && path < cfg.threshold {
                continue;
            }
            let improved = best.get(&edge.target).is_none_or(|prev| next > prev.score);
            if improved {
                best.insert(
                    edge.target,
                    Scored {
                        node: edge.target,
                        score: next,
                        hops: hops + 1,
                    },
                );
                // Queued at the unpenalised score: the penalty ranks the node,
                // it does not shorten the paths that run through it.
                queue.push(Candidate(path, edge.target, hops + 1));
            }
        }
    }

    let mut out: Vec<Scored> = best.into_values().collect();
    // Deterministic order: score first, node id to break ties.
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.cmp(&b.node))
    });
    out.truncate(cfg.max_nodes);
    out
}

/// How much of the graph a query avoided sending. The headline claim of this
/// layer is noise reduction, so it is measured rather than asserted.
pub fn reduction(kept: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    100.0 * (1.0 - kept as f32 / total as f32)
}
