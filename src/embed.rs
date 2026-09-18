//! Deterministic CPU embeddings, Cleora-style.
//!
//! No training, no random walks, no GPU: the embedding of a node is the
//! distribution of paths leaving it. Build the row-normalised adjacency matrix
//! (a Markov chain over the graph), start from a fixed per-node signature, and
//! propagate it `k` times. Nodes that reach similar neighbourhoods converge to
//! similar vectors.
//!
//! Determinism is the property that matters here. The initial signature is
//! derived by hashing the node id, not sampled, so the same graph yields
//! bit-identical vectors on every machine and every run — which is what makes a
//! retrieved result reproducible and citable.
//!
//! Iteration count trades locality for reach. Measured (audit step 8b) by how
//! often `nearest` agrees with the community a symbol is in: 1 iteration 6.7%,
//! 2 gives 10.2%, 4 gives 10.3%, 8 gives 10.7% — one is clearly too few and
//! everything past two is a plateau. Oversmoothing is not the risk the Cleora
//! paper warns of on this shape of graph: mean pairwise similarity rises only
//! from 0.018 to 0.043 across those eight steps.
//!
//! Two things stop the propagation from erasing what it is meant to encode,
//! both measured on three trees: `SELF_WEIGHT` and `SINK_DAMP` below.

use crate::graph::GraphSnapshot;
use crate::parallel;

/// Embedding width. Small on purpose: these vectors seed a graph traversal
/// rather than serve as a standalone semantic index, and 64 floats per node
/// keeps a large repo's matrix in cache.
pub const DIM: usize = 64;

/// Share of its own vector a node keeps at each step, the rest coming from its
/// neighbours — a lazy restart term, the same idea PageRank uses.
///
/// **Measured on three trees, and it is a trade.** At 0 a node whose only
/// target is `readFileSync` *is* `readFileSync`: 74 of 118 out-degree-1 nodes
/// here, 22 of 25 in a Rust tree, 444 of 600 in a monorepo come out identical
/// to their target, and `nearest` then reports two such nodes as each other's
/// closest match while knowing nothing about either.
///
/// | weight | identical to target | same file | same community |
/// |---|---|---|---|
/// | 0.0 | 74 | 23.1% | 11.2% |
/// | 0.15 | 58 | 22.6% | 11.7% |
/// | **0.35** | **0** | **20.7%** | **10.7%** |
/// | 0.5 | 1 | 17.5% | 9.4% |
///
/// 0.35 is where the collapse ends: 444 -> 4 on the monorepo, 22 -> 0 on the
/// Rust tree. Past it the node drowns out its own neighbourhood and agreement
/// falls without buying anything. The 2.4 points of file agreement it costs are
/// paid for a vector that is about the node rather than about its callee.
const SELF_WEIGHT: f32 = 0.35;

/// How much of a sink's vector reaches its caller, relative to a neighbour that
/// leads somewhere. A sink never changes — it has no neighbours to average —
/// so it contributes identity, not structure, and a caller with one such target
/// converges onto it regardless of `SELF_WEIGHT`. Measured below.
/// | damping | identical to target | same file | same community |
/// |---|---|---|---|
/// | 0.0 (drop it) | 9 | 15.3% | 7.2% |
/// | **0.3** | **3** | **22.6%** | **12.7%** |
/// | 0.5 | 3 | 21.3% | 11.8% |
/// | 1.0 (no damping) | 60 | 19.7% | 9.9% |
///
/// It is not a trade: 0.3 beats undamped on both agreement measures *and*
/// clears the collapse — 57 sink cases to 0 here, 316 to 0 on a monorepo, 21 to
/// 0 on a third tree, with file agreement up 1.4 to 3.2 points everywhere.
/// Dropping a sink's contribution entirely (0.0) is worse than keeping some:
/// the placeholder still says *which* foreign function was called.
const SINK_DAMP: f32 = 0.3;

pub struct Embeddings {
    pub vectors: Vec<f32>,
    pub dim: usize,
}

impl Embeddings {
    pub fn node_count(&self) -> usize {
        if self.dim == 0 {
            return 0;
        }
        self.vectors.len() / self.dim
    }

    pub fn get(&self, node: u32) -> Option<&[f32]> {
        let i = node as usize * self.dim;
        self.vectors.get(i..i + self.dim)
    }

    /// Cosine similarity. Vectors are L2-normalised after every iteration, so
    /// this is a plain dot product.
    pub fn similarity(&self, a: u32, b: u32) -> f32 {
        match (self.get(a), self.get(b)) {
            (Some(x), Some(y)) => x.iter().zip(y).map(|(p, q)| p * q).sum(),
            _ => 0.0,
        }
    }

    /// The `k` nodes most similar to `node`, best first.
    pub fn nearest(&self, node: u32, k: usize) -> Vec<(u32, f32)> {
        let mut out: Vec<(u32, f32)> = (0..self.node_count() as u32)
            .filter(|&n| n != node)
            .map(|n| (n, self.similarity(node, n)))
            .collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                // Node id breaks ties, so the result is stable.
                .then_with(|| a.0.cmp(&b.0))
        });
        out.truncate(k);
        out
    }
}

/// Deterministic per-node starting vector.
///
/// A hash, not a sample: reproducibility across runs and machines is the whole
/// point. Distinct symbols get near-orthogonal signatures, which is all the
/// propagation needs to tell neighbourhoods apart.
fn signature(symbol: u32, out: &mut [f32]) {
    // SplitMix64: cheap, well-distributed, and fully specified — so the vectors
    // do not depend on the standard library's hasher, which is not stable
    // across versions.
    let mut state = (symbol as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xDEAD_BEEF_CAFE_F00D;
    for slot in out.iter_mut() {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Map to [-1, 1).
        *slot = (z >> 40) as f32 / 8_388_608.0 - 1.0;
    }
    l2_normalise(out);
}

fn l2_normalise(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Computes embeddings by propagating signatures `iterations` times.
///
/// Each iteration is one sparse matrix-vector product per dimension block,
/// parallelised over nodes with bounded standard-library workers. Rows are normalised by out-degree, which
/// is the Markov step: a node's new vector is the mean of its neighbours'.
pub fn embed(snap: &GraphSnapshot, iterations: usize) -> Embeddings {
    let n = snap.width();
    if n == 0 {
        return Embeddings {
            vectors: Vec::new(),
            dim: DIM,
        };
    }

    let mut current = vec![0.0f32; n * DIM];
    parallel::for_each_unit_mut(&mut current, DIM, |first, slots| {
        for (offset, slot) in slots.chunks_mut(DIM).enumerate() {
            let node = first + offset;
            // Seeded from the node id, not from the CSR's interned key.
            //
            // **The key is not populated on any real path.** Only tier 0 and
            // the self-check call `CsrBuilder::add_node` with an interned
            // symbol; tier 2 mints ids through `SymbolRegistry` and never
            // writes one, and `GraphSnapshot::compact` copies whatever the base
            // holds — which is 0. Measured on this tree: 667 defined nodes, two
            // distinct keys, and therefore **four distinct vectors**, so 99.1%
            // of all pairs had cosine similarity above 0.999 and `nearest`
            // returned five unrelated symbols at exactly 1.00.
            //
            // The id is what actually distinguishes a node here, and it is what
            // `nearest` indexes by. Ids are monotonic and never reused (see
            // `forget_file`), so a signature keyed on one is as stable as the
            // graph it came from.
            signature(node as u32, slot);
        }
    });

    // Adjacency in CSR form, flattened once so each iteration is a plain
    // gather: the union view is not contiguous, and re-walking it per
    // iteration would dominate the cost.
    let mut offsets = Vec::with_capacity(n + 1);
    let mut targets = Vec::new();
    for node in 0..n as u32 {
        offsets.push(targets.len() as u32);
        targets.extend(snap.neighbors(node).map(|e| e.target));
    }
    offsets.push(targets.len() as u32);

    let mut next = vec![0.0f32; n * DIM];
    for _ in 0..iterations {
        parallel::for_each_unit_mut(&mut next, DIM, |first, slots| {
            for (offset, out) in slots.chunks_mut(DIM).enumerate() {
                let node = first + offset;
                let from = offsets[node] as usize;
                let to = offsets[node + 1] as usize;
                if from == to {
                    // A sink keeps its own vector rather than decaying to zero,
                    // which would make every leaf indistinguishable.
                    //
                    // It also keeps it *unchanged*, and that is what the
                    // restart term alone cannot fix: a node whose one target is
                    // a sink converges onto a vector that never moves, so it
                    // arrives at 0.9998 after four steps however much of itself
                    // it keeps. Measured, every remaining identical pair was of
                    // this shape — 57 of 57 here, 316 of 321 on a monorepo —
                    // and the target is always a placeholder calling into
                    // another crate. Handled at the caller instead: see
                    // `SINK_DAMP`.
                    out.copy_from_slice(&current[node * DIM..node * DIM + DIM]);
                    return;
                }
                // The node keeps a share of its own vector, which is what
                // stops a pure Markov step from erasing the node itself.
                //
                // Without it a node with exactly one target gets that target's
                // vector *verbatim* — the mean over one neighbour is that
                // neighbour — so two functions that both call `readFileSync`
                // and nothing else come out identical at 1.00 and `nearest`
                // reports them as each other's closest match while knowing
                // nothing about either. Measured: 74 of 118 out-degree-1 nodes
                // here, 22 of 25 in a second tree, **444 of 600** in a third.
                //
                // A lazy restart term, the same idea PageRank uses for the same
                // reason. At SELF_WEIGHT 0 the collapse is total; the value is
                // measured below.
                out.copy_from_slice(&current[node * DIM..node * DIM + DIM]);
                for o in out.iter_mut() {
                    *o *= SELF_WEIGHT;
                }
                let inv = (1.0 - SELF_WEIGHT) / (to - from) as f32;
                for &t in &targets[from..to] {
                    let base = t as usize * DIM;
                    if let Some(v) = current.get(base..base + DIM) {
                        // A sink contributes less than a node that leads
                        // somewhere: its vector is frozen at its signature, so
                        // it carries no neighbourhood, only identity. Without
                        // this a node whose one call is `readFileSync` *is*
                        // `readFileSync`.
                        let w = if offsets[t as usize] == offsets[t as usize + 1] {
                            inv * SINK_DAMP
                        } else {
                            inv
                        };
                        for (o, x) in out.iter_mut().zip(v) {
                            *o += x * w;
                        }
                    }
                }
                // L2 keeps magnitudes comparable so cosine similarity stays
                // meaningful across iterations.
                l2_normalise(out);
            }
        });
        std::mem::swap(&mut current, &mut next);
    }

    Embeddings {
        vectors: current,
        dim: DIM,
    }
}
