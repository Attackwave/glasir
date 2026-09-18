//! Community detection: partition the graph into coherent subsystems.
//!
//! The full three-phase Leiden procedure: local movement, refinement, then
//! aggregation into a coarser graph, repeated until a level stops improving.
//!
//! Refinement is what distinguishes Leiden from Louvain. Local movement can
//! leave a community internally disconnected — a node follows its neighbours
//! out and the halves it bridged have nothing else in common — so each
//! community is re-derived from singletons within its own bounds before the
//! level is aggregated. `is_connected` makes the guarantee checkable.
//!
//! Communities give a retrieved subgraph a frame: "this function lives in the
//! payment subsystem" is more useful to a model than a bare list of neighbours.
//!
//! Super-hubs are excluded first. A logger called from everywhere links
//! otherwise unrelated modules and, left in, merges the whole graph into one
//! community — the failure mode this parameter exists to prevent. Exclusion is
//! by neighbour cohesion, not by degree alone: what makes a hub harmful is that
//! its neighbours have nothing to do with each other, and direction is
//! irrelevant since modularity is undirected.
//!
//! Measured on this repository, largest community by iteration: 6 with local
//! movement alone, 12 once aggregation existed. The number that matters is not
//! size but **purity** — the share of a community's symbols living in the file
//! it is named after, which is what `overview` prints — and it went 36% -> 89%
//! once *every* undefined reference stopped being partitioned, not just the
//! weakly attached ones. See `drop_undefined` and `bench::overview_scale`.
//!
//! Refinement changes nothing on this particular graph: its communities are
//! small enough that none is internally disconnected in the first place. It is
//! still worth having — it is what makes "a community is a subsystem" a
//! guarantee rather than an observation — and is tested directly against a
//! deliberately disconnected partition.

use crate::csr::NodeId;
use crate::graph::GraphSnapshot;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Detach references to symbols defined outside the tree.
    ///
    /// A tier-2/3 graph is mostly references into other crates — `Ok`,
    /// `collect`, `assert_eq` — which have no definition here and never will.
    /// Each forms its own community, so they dominate the count while carrying
    /// none of this codebase's structure.
    ///
    /// **Only undefined symbols, and all of them.** The first half is
    /// necessary: a type definition makes no calls, so a degree test alone
    /// would discard `BaseCsr` and `DeltaStore` along with the foreign names.
    ///
    /// The second half — keeping a *heavily used* foreign name on the grounds
    /// that two callers of `HashMap::new` are related — was measured and is
    /// wrong. It is exactly backwards: a shared foreign name is what glues
    /// unrelated files into one lump, and the frequent ones do it hardest.
    /// `new` has degree 334 across 25 files here, `map` 169 across 26, and 160
    /// undefined nodes are reached from three or more files, each one a claim
    /// that those files belong together. Measured on three trees, keeping them
    /// (degree <= 2) against detaching all:
    ///
    /// | tree | purity | largest | covered |
    /// |---|---|---|---|
    /// | glasir | 36% -> **89%** | 56 -> **24** | 271 -> 252 |
    /// | epoch-engine | 57% -> **100%** | 38 -> **7** | 97 -> 86 |
    /// | office4u | 32% -> **100%** | **1265 -> 97** | 1650 -> 1324 |
    ///
    /// Purity is the share of a community's symbols living in the file it is
    /// named after — which is what `overview` prints, so it is the property
    /// that tool's usefulness rests on. The old justification ("121 nodes to
    /// 42, and communities left internally disconnected") came from a smaller
    /// tree and holds on none of the three: coverage falls 8-20%, and
    /// `is_connected` reports **zero** disconnected communities over the whole
    /// `pendant_degree` range.
    ///
    /// Detached nodes stay in the graph for retrieval; they are excluded only
    /// from the partition. Recall is unchanged (54/85/50/56) because
    /// `query_graph` never reads the partition — which is why the benchmark
    /// could not see any of this. See `docs/audit/research.md`, step 8a.
    pub drop_undefined: bool,
    /// Higher values yield more, smaller communities.
    pub resolution: f32,
    /// Degree above this multiple of the median makes a node a hub *candidate*.
    /// The median rather than the mean: a hub inflates the mean it would be
    /// measured against, which is worst exactly when there are few to find.
    pub hub_factor: f32,
    /// A candidate is only excluded if at most this fraction of its neighbour
    /// pairs are themselves connected. A dense cluster's centre has a high
    /// ratio and stays; a logger's callers do not know each other, so its ratio
    /// is near zero and it goes.
    ///
    /// Degree alone is the wrong test: what makes a hub harmful is joining
    /// parts that are otherwise unrelated, not being popular.
    pub hub_cohesion: f32,
    /// Minimum incoming edges before a node can be called a hub.
    ///
    /// A hub is reached *from* many unrelated places. A long function calling
    /// thirty things is not a hub — it is just long — and cohesion alone cannot
    /// tell the two apart, since modularity is undirected.
    ///
    /// Measured on this repository under a tier-1 graph: at 1, twenty
    /// communities over 131 nodes with sizes 30/14/12/12; at 2, four
    /// communities and a 138-node blob; at 3, a single 205-node clump. One
    /// incoming edge is all the condition needs — a node reached by nothing
    /// cannot route between anything.
    pub hub_min_incoming: usize,
    /// Distinct neighbour files required before a node counts as a hub.
    ///
    /// **A hub joins unrelated *modules*, and cohesion cannot see that.** It
    /// measures whether a node's neighbours know each other, which a long
    /// function fails just as a shared logger does. Measured across three trees
    /// before this existed, half of what was excluded touched only one file —
    /// 11 of 18 here, 1 of 1 in a Rust tree, **211 of 211 in a monorepo**:
    /// `run_install`, `now`, `walk`, `qualify`, `community.rs#detect` join
    /// nothing at all, they are merely long or often called.
    ///
    /// At 2 the seven that remain here are the ones that do join things —
    /// `analyse`, `build_graph`, `run_watch`, `query_graph`, `shortest_path`,
    /// `impact` — and the partition scale goes 48% -> 52% with no set of
    /// questions moving.
    ///
    /// **The largest community grows and that is correct, not a regression.**
    /// It goes 25 -> 69 here, and 68 of those 69 symbols are `main.rs`, which
    /// is 7,343 of this repository's 16,181 lines: one file really is one
    /// thing. Checked on all three trees — every largest community is
    /// attributable to a single file.
    pub hub_files: usize,
    /// Cap on local-movement sweeps per level, so a pathological graph cannot
    /// spin.
    pub max_passes: usize,
    /// Cap on aggregation levels. Each level collapses the graph, so this is
    /// reached only by a graph that keeps coarsening.
    pub max_levels: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            drop_undefined: true,
            resolution: 1.0,
            // A node wired to twice as much as a typical one is already
            // structural rather than functional; real hubs sit far above this.
            hub_factor: 2.0,
            // Measured separation on a two-cluster graph joined by a logger:
            // cluster members sit at 0.83-1.0, the logger at 0.36. Half is
            // comfortably inside that gap.
            hub_cohesion: 0.5,
            hub_files: 2,
            hub_min_incoming: 1,
            max_passes: 10,
            max_levels: 10,
        }
    }
}

pub struct Communities {
    /// Community id per node. Hubs, pendants and isolated nodes keep their own
    /// id.
    pub of_node: Vec<u32>,
    pub hubs: Vec<NodeId>,
    /// Nodes excluded as pendants: reached once, calling nothing back. Mostly
    /// references into other crates.
    pub pendants: Vec<NodeId>,
}

impl Communities {
    pub fn count(&self) -> usize {
        let mut seen: Vec<u32> = self.of_node.clone();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }

    pub fn members(&self, community: u32) -> Vec<NodeId> {
        self.of_node
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c == community)
            .map(|(n, _)| n as NodeId)
            .collect()
    }
}

/// Undirected weighted adjacency, which is what modularity is defined over.
/// Call direction does not matter for "these belong together".
fn undirected(snap: &GraphSnapshot, n: usize) -> Vec<HashMap<usize, f32>> {
    let mut adj = vec![HashMap::new(); n];
    for node in 0..n as NodeId {
        for edge in snap.neighbors(node) {
            let (a, b) = (node as usize, edge.target as usize);
            if a == b || b >= n {
                continue;
            }
            // Parallel edges reinforce the link rather than being deduplicated:
            // calling something twice is a stronger association.
            *adj[a].entry(b).or_insert(0.0) += 1.0;
            *adj[b].entry(a).or_insert(0.0) += 1.0;
        }
    }
    adj
}

/// How many distinct files a node's neighbours live in.
///
/// A hub joins *unrelated modules*; a node whose neighbours all sit in its own
/// file joins nothing and is merely long or often called.
fn spans_files(adj: &[HashMap<usize, f32>], file_of: &[u32], node: usize, need: usize) -> bool {
    // No file information — a caller without a registry — so the test cannot
    // apply and must not exclude anything on a guess.
    if need <= 1 || file_of.is_empty() {
        return true;
    }
    let mut seen: Vec<u32> = adj[node]
        .keys()
        .filter_map(|&p| file_of.get(p).copied())
        .filter(|&f| f != u32::MAX)
        .collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len() >= need
}

/// Fraction of a node's neighbour pairs that are themselves connected.
///
/// Near 1 means its neighbours form a cluster and the node belongs to it; near
/// 0 means it is the only thing they have in common, which is what a
/// cross-cutting utility looks like.
fn neighbour_cohesion(adj: &[HashMap<usize, f32>], node: usize) -> f32 {
    let mut peers: Vec<usize> = adj[node].keys().copied().collect();
    if peers.len() < 2 {
        return 1.0;
    }
    // Sorted, so the sample below is the same set on every run. Rust's HashMap
    // does not promise an iteration order, and every other ranking in this
    // codebase fixes one for that reason; relying on it here would make the
    // partition depend on an implementation detail that is free to change.
    peers.sort_unstable();

    // Every pair is quadratic in the degree, and the nodes this is asked about
    // are by definition the high-degree ones: on a 1M-line tree the candidates
    // included several with degree 10,000, and the full test came to 1.1
    // billion pair checks — 11 s, against 2 ms for the Leiden phases it
    // guards. Past `COHESION_SAMPLE` peers a fixed evenly-spaced subset is
    // used instead.
    //
    // A sample is enough because the answer is a comparison against
    // `hub_cohesion`, not a value anyone reads: a hub's neighbours are
    // unrelated to each other and score near zero, a cluster's centre near
    // one. Deterministic by construction — a fixed stride over a sorted list,
    // no sampling in the statistical sense.
    let sampled: Vec<usize> = if peers.len() > COHESION_SAMPLE {
        let stride = peers.len() / COHESION_SAMPLE;
        peers
            .iter()
            .step_by(stride)
            .take(COHESION_SAMPLE)
            .copied()
            .collect()
    } else {
        peers
    };

    let mut linked = 0usize;
    let mut pairs = 0usize;
    for (i, &a) in sampled.iter().enumerate() {
        for &b in &sampled[i + 1..] {
            pairs += 1;
            if adj[a].contains_key(&b) {
                linked += 1;
            }
        }
    }
    if pairs == 0 {
        return 1.0;
    }
    linked as f32 / pairs as f32
}

/// How many neighbours the cohesion test looks at. 64 gives 2,016 pairs, which
/// is bounded work per node whatever its degree.
const COHESION_SAMPLE: usize = 64;

/// A weighted undirected graph, used for both the original and every
/// aggregated level.
struct Level {
    adj: Vec<HashMap<usize, f32>>,
    /// Self-loop weight per node, carrying the edges collapsed inside an
    /// aggregated community. Without it, aggregation forgets internal density
    /// and every level looks sparser than the last.
    self_loops: Vec<f32>,
    degrees: Vec<f32>,
    total: f32,
}

impl Level {
    fn new(adj: Vec<HashMap<usize, f32>>, self_loops: Vec<f32>) -> Self {
        let degrees: Vec<f32> = adj
            .iter()
            .zip(&self_loops)
            .map(|(m, &l)| m.values().sum::<f32>() + 2.0 * l)
            .collect();
        let total = degrees.iter().sum::<f32>() / 2.0;
        Self {
            adj,
            self_loops,
            degrees,
            total,
        }
    }

    fn len(&self) -> usize {
        self.adj.len()
    }
}

/// Phase 1: move each node to the neighbouring community with the best
/// modularity gain, repeating until nothing improves.
///
/// `constraint`, when given, restricts a node to communities within its own
/// partition — that is what turns this into Leiden's refinement rather than
/// plain local movement.
fn local_movement(
    level: &Level,
    community: &mut [u32],
    params: &Params,
    constraint: Option<&[u32]>,
) -> bool {
    if level.total <= 0.0 {
        return false;
    }
    let m2 = 2.0 * level.total;
    let mut any_moved = false;

    for _ in 0..params.max_passes {
        let mut moved = false;
        let mut comm_degree: HashMap<u32, f32> = HashMap::new();
        for (&c, &d) in community.iter().zip(&level.degrees) {
            *comm_degree.entry(c).or_insert(0.0) += d;
        }

        for node in 0..level.len() {
            if level.adj[node].is_empty() {
                continue;
            }
            let own = community[node];
            let k = level.degrees[node];
            *comm_degree.get_mut(&own).unwrap() -= k;

            let mut links: HashMap<u32, f32> = HashMap::new();
            for (&peer, &w) in &level.adj[node] {
                // Refinement only considers peers from the same outer
                // community, which is what keeps a refined part inside it.
                if let Some(c) = constraint
                    && c[peer] != c[node]
                {
                    continue;
                }
                *links.entry(community[peer]).or_insert(0.0) += w;
            }

            let gain = |c: u32, w: f32| {
                w - params.resolution * k * comm_degree.get(&c).copied().unwrap_or(0.0) / m2
            };
            let stay = gain(own, links.get(&own).copied().unwrap_or(0.0));
            let (best, best_gain) =
                links
                    .iter()
                    .map(|(&c, &w)| (c, gain(c, w)))
                    .fold((own, stay), |acc, x| {
                        // Ties go to the lower id, so the partition is stable.
                        if x.1 > acc.1 || (x.1 == acc.1 && x.0 < acc.0) {
                            x
                        } else {
                            acc
                        }
                    });

            if best != own && best_gain > stay {
                community[node] = best;
                moved = true;
                any_moved = true;
            }
            *comm_degree.entry(community[node]).or_insert(0.0) += k;
        }
        if !moved {
            break;
        }
    }
    any_moved
}

/// Phase 2 (Leiden): split each community into well-connected parts.
///
/// Louvain can leave a community internally disconnected, because a node may
/// follow its neighbours out and leave two unrelated halves behind. Refinement
/// re-runs local movement *within* each community, starting from singletons, so
/// a part that is not actually connected to the rest breaks off.
///
/// Returns the refined partition, which is finer than or equal to the input.
fn refine(level: &Level, community: &[u32], params: &Params) -> Vec<u32> {
    // Start from singletons: every node its own refined community.
    let mut refined: Vec<u32> = (0..level.len() as u32).collect();
    local_movement(level, &mut refined, params, Some(community));
    refined
}

/// Phase 3: collapse each community into one node of a smaller graph.
///
/// Edges within a community become that node's self-loop, edges between
/// communities become weighted links. Running phase 1 again on this graph is
/// what lets communities grow beyond what one pass of local movement can find —
/// the step whose absence kept the partition at a largest size of six.
fn aggregate(level: &Level, community: &[u32], count: usize) -> Level {
    let mut adj = vec![HashMap::new(); count];
    let mut self_loops = vec![0.0f32; count];

    for node in 0..level.len() {
        let a = community[node] as usize;
        // Carry the already-collapsed weight forward.
        self_loops[a] += level.self_loops[node];
        for (&peer, &w) in &level.adj[node] {
            let b = community[peer] as usize;
            if a == b {
                // Counted from both ends, so halve it.
                self_loops[a] += w / 2.0;
            } else {
                *adj[a].entry(b).or_insert(0.0) += w;
            }
        }
    }
    // Each inside edge was added once from each endpoint.
    for l in self_loops.iter_mut() {
        *l /= 2.0;
    }
    Level::new(adj, self_loops)
}

/// Renumbers a partition to dense ids 0..n, returning the count.
fn densify(community: &mut [u32]) -> usize {
    let mut remap: HashMap<u32, u32> = HashMap::new();
    for c in community.iter_mut() {
        let next = remap.len() as u32;
        *c = *remap.entry(*c).or_insert(next);
    }
    remap.len()
}

/// Detects communities with the full three-phase Leiden procedure: local
/// movement, refinement, aggregation, repeated until a level stops improving.
///
/// Refinement is what separates this from Louvain. Louvain moves a node to
/// whichever community its neighbours favour, which can leave the community it
/// left in two disconnected halves; refinement re-derives each community from
/// singletons within its own bounds, so a part that is not genuinely connected
/// breaks off before the level is aggregated.
/// `defined` lists the nodes that are definitions in the indexed tree, which
/// the caller knows from the symbol registry and the graph does not. Passing
/// `None` disables the filter.
/// What a caller knows about a node beyond the topology: whether the tree
/// defines it, and which file it lives in.
///
/// One argument rather than two because both come from the symbol registry,
/// and both are absent together — a caller building a bare CSR (the checks)
/// passes the default and gets a partition from topology alone.
#[derive(Default)]
pub struct Context<'a> {
    pub defined: Option<&'a std::collections::HashSet<NodeId>>,
    /// File id per node, `u32::MAX` where there is none — every bare
    /// placeholder. Compared only for equality, so the ids need no meaning.
    pub file_of: Vec<u32>,
}

impl<'a> Context<'a> {
    /// Both halves from a registry, where a qualified name carries its file as
    /// the prefix before `#`.
    pub fn from_names<S: AsRef<str>>(
        defined: Option<&'a std::collections::HashSet<NodeId>>,
        width: usize,
        names: impl Iterator<Item = (S, NodeId)>,
    ) -> Context<'a> {
        let mut ids: HashMap<String, u32> = HashMap::new();
        let mut file_of = vec![u32::MAX; width];
        for (symbol, node) in names {
            let Some((file, _)) = symbol.as_ref().split_once('#') else {
                continue;
            };
            let next = ids.len() as u32;
            let id = *ids.entry(file.to_string()).or_insert(next);
            if let Some(slot) = file_of.get_mut(node as usize) {
                *slot = id;
            }
        }
        Context { defined, file_of }
    }
}

/// Splits the codebase into subsystems: groups of symbols that belong
/// together, each named after the file most of its members live in.
pub fn detect(snap: &GraphSnapshot, params: &Params, ctx: &Context<'_>) -> Communities {
    let defined = ctx.defined;
    let n = snap.width();
    if n == 0 {
        return Communities {
            of_node: Vec::new(),
            hubs: Vec::new(),
            pendants: Vec::new(),
        };
    }

    let mut adj = undirected(snap, n);

    let mut pendants = Vec::new();
    if let Some(defined) = defined.filter(|_| params.drop_undefined) {
        for node in 0..n {
            if defined.contains(&(node as NodeId)) {
                continue;
            }
            pendants.push(node as NodeId);
            let peers: Vec<usize> = adj[node].keys().copied().collect();
            for p in peers {
                adj[p].remove(&node);
            }
            adj[node].clear();
        }
    }

    // Exclude super-hubs before optimising: they bridge unrelated modules and
    // would collapse the partition into one community.
    let degrees: Vec<f32> = adj.iter().map(|m| m.values().sum()).collect();
    // Leaves (a call into another crate, with one edge and nothing out) are the
    // bulk of a real graph and drag the median to 1, which would make a 2x
    // cutoff mean "two edges". Median over the connective nodes only.
    let mut sorted: Vec<f32> = degrees.iter().copied().filter(|&d| d > 1.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let cutoff = median * params.hub_factor;
    // Incoming edges per node. Cohesion says a node's neighbours are unrelated;
    // this says the node is *reached* by them rather than reaching out, which
    // is what separates a shared utility from a merely long function.
    let mut incoming = vec![0usize; n];
    for node in 0..n as NodeId {
        for e in snap.neighbors(node) {
            if let Some(slot) = incoming.get_mut(e.target as usize) {
                *slot += 1;
            }
        }
    }

    let mut hubs = Vec::new();
    for node in 0..n {
        // A logger called from everywhere routes between modules through its
        // incoming edges, and modularity is undirected, so direction does not
        // enter the cohesion test. It does enter here: without the incoming
        // requirement, any long function looks like a hub.
        if median > 0.0
            && degrees[node] >= cutoff
            && incoming[node] >= params.hub_min_incoming
            && neighbour_cohesion(&adj, node) <= params.hub_cohesion
            && spans_files(&adj, &ctx.file_of, node, params.hub_files)
        {
            hubs.push(node as NodeId);
            let peers: Vec<usize> = adj[node].keys().copied().collect();
            for p in peers {
                adj[p].remove(&node);
            }
            adj[node].clear();
        }
    }

    let mut level = Level::new(adj, vec![0.0; n]);
    // Membership of each original node, rewritten as levels collapse.
    let mut of_node: Vec<u32> = (0..n as u32).collect();

    for _ in 0..params.max_levels {
        // Phase 1: local movement.
        let mut community: Vec<u32> = (0..level.len() as u32).collect();
        let moved = local_movement(&level, &mut community, params, None);
        if !moved {
            break;
        }
        densify(&mut community);

        // Phase 2: refinement, so no community is internally disconnected.
        let mut refined = refine(&level, &community, params);
        let refined_count = densify(&mut refined);

        // Map original nodes onto the refined partition of this level.
        for c in of_node.iter_mut() {
            *c = refined[*c as usize];
        }

        // Phase 3: aggregate and repeat on the coarser graph.
        let next = aggregate(&level, &refined, refined_count);
        if next.len() >= level.len() {
            // No coarsening happened, so another level cannot help.
            break;
        }
        level = next;
    }

    densify(&mut of_node);
    Communities {
        of_node,
        hubs,
        pendants,
    }
}

/// True if every member can reach every other through edges that stay inside
/// the set. Leiden's refinement guarantees this; Louvain does not, so it is
/// worth being able to check.
pub fn is_connected(snap: &GraphSnapshot, members: &[NodeId]) -> bool {
    if members.len() < 2 {
        return true;
    }
    let inside: std::collections::HashSet<NodeId> = members.iter().copied().collect();
    let mut seen = std::collections::HashSet::from([members[0]]);
    let mut stack = vec![members[0]];
    while let Some(node) = stack.pop() {
        // Undirected reachability: modularity does not care about direction,
        // so neither does connectivity within a community.
        let mut step = |peer: NodeId| {
            if inside.contains(&peer) && seen.insert(peer) {
                stack.push(peer);
            }
        };
        for e in snap.neighbors(node) {
            step(e.target);
        }
        for &m in members {
            if snap.neighbors(m).any(|e| e.target == node) {
                step(m);
            }
        }
    }
    seen.len() == members.len()
}

/// Exposed for the self-check: refinement is the phase that distinguishes this
/// from Louvain, and it is worth testing against a deliberately disconnected
/// partition rather than only through `detect`.
pub fn refine_for_test(snap: &GraphSnapshot, community: &[u32]) -> Vec<u32> {
    let n = snap.width();
    let level = Level::new(undirected(snap, n), vec![0.0; n]);
    let mut refined = refine(&level, community, &Params::default());
    densify(&mut refined);
    refined
}
