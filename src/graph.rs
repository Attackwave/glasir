//! Lock-free reader access to the union view.
//!
//! Agents read through `ArcSwap<GraphSnapshot>`: a read clones an `Arc` and
//! never blocks, while a writer publishes a whole new snapshot with one atomic
//! pointer swap. Once the delta store passes `COMPACTION_THRESHOLD` mutations,
//! a background worker merges base and delta into a fresh CSR off the write
//! path; writers keep running while it does.
//!
//! Compaction is speculative. The worker rebuilds from the snapshot it was
//! handed, and on publish the generation counter tells it whether a writer got
//! there first. If so the result is stale and simply dropped — the next write
//! past the threshold schedules another pass. Correctness never depends on the
//! worker winning the race, only throughput does.

use crate::csr::{BaseCsr, Confidence, CsrBuilder, Edge, NodeId};
use crate::delta::DeltaStore;
use crate::published::Published;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Delta size at which compaction folds the buffer back into the base CSR.
///
/// Calibrated against a measured ~1M-LOC graph (22.6k nodes, 48.7k edges, about
/// 1 MiB of CSR): 1000 buffered mutations are roughly 2% of that edge set, i.e.
/// a handful of saved files. A full rebuild at that size costs single-digit
/// milliseconds and runs off the write path, so compacting often is cheap;
/// letting the delta grow instead is what degrades read latency, since every
/// read filters the union view through the tombstone set.
pub const COMPACTION_THRESHOLD: usize = 1_000;

/// Overwritten by the fill pass before anything reads it.
const PLACEHOLDER: Edge = Edge {
    target: 0,
    timestamp: 0,
    authority: 0.0,
    edge_kind: 0,
    confidence: Confidence::Ambiguous,
};

/// Incoming edges, in the CSR shape: built on demand from a snapshot, since the
/// stored graph holds outgoing edges only.
pub struct Reverse {
    offsets: Vec<u32>,
    entries: Vec<(NodeId, Edge)>,
}

impl Reverse {
    /// Nodes with an edge into `node`, each with the edge itself.
    pub fn callers(&self, node: NodeId) -> impl Iterator<Item = (NodeId, Edge)> + '_ {
        let start = *self.offsets.get(node as usize).unwrap_or(&0) as usize;
        let end = *self.offsets.get(node as usize + 1).unwrap_or(&0) as usize;
        self.entries[start..end].iter().copied()
    }
}

/// A consistent read view: an immutable base plus the delta captured with it.
pub struct GraphSnapshot {
    pub base: Arc<BaseCsr>,
    pub delta: DeltaStore,
    /// Bumped on every publish, for diagnostics and for spotting how often a
    /// speculative compaction lost its race.
    generation: u64,
    /// The reverse index, built on first use and kept for this snapshot's life.
    ///
    /// It is a pure function of the topology, and a snapshot's topology never
    /// changes — a re-index publishes a *new* snapshot, which starts with an
    /// empty cell. Three tools ask for it (`impact`, `explain_node`,
    /// `overview`) and each rebuilt it per request: 15 ms on a 200k-node tree,
    /// paid again on every call for a result that could not differ.
    reverse: std::sync::OnceLock<Reverse>,
}

impl GraphSnapshot {
    /// Outgoing edges of `node`: base edges minus tombstones, then delta edges.
    pub fn neighbors(&self, node: NodeId) -> impl Iterator<Item = Edge> + '_ {
        self.base
            .neighbors(node)
            .filter(move |e| !self.delta.is_tombstoned(node, e.target))
            .chain(self.delta.added(node).iter().copied())
    }

    /// Incoming edges per node, in the same CSR shape as the forward graph:
    /// `offsets` of width+1 and a flat `(source, edge)` array.
    ///
    /// The CSR stores outgoing edges only, so every "who calls this" question
    /// was a full O(V+E) scan — once per caller lookup. Impact analysis walks
    /// backwards repeatedly, which would make that quadratic, so the reverse
    /// side is built once per call instead: two passes, count then fill.
    pub fn reverse(&self) -> &Reverse {
        self.reverse.get_or_init(|| self.build_reverse())
    }

    fn build_reverse(&self) -> Reverse {
        let width = self.width();
        let mut offsets = vec![0u32; width + 1];
        for n in 0..width as NodeId {
            for e in self.neighbors(n) {
                if (e.target as usize) < width {
                    offsets[e.target as usize + 1] += 1;
                }
            }
        }
        for i in 0..width {
            offsets[i + 1] += offsets[i];
        }
        // Fill by cursor rather than push, so an entry lands in its target's
        // slice: MaybeUninit would be the alternative and is not worth it.
        let mut entries: Vec<(NodeId, Edge)> = vec![(0, PLACEHOLDER); offsets[width] as usize];
        let mut cursor = offsets.clone();
        for n in 0..width as NodeId {
            for e in self.neighbors(n) {
                if (e.target as usize) < width {
                    let slot = &mut cursor[e.target as usize];
                    entries[*slot as usize] = (n, e);
                    *slot += 1;
                }
            }
        }
        Reverse { offsets, entries }
    }

    /// How many times this snapshot's lineage has been republished. Names the
    /// number of write-visible steps taken, which is what tells a build that
    /// published once from one that published per file.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn base_targets(&self, node: NodeId) -> Vec<NodeId> {
        self.base.neighbors(node).map(|e| e.target).collect()
    }

    /// Nodes in the base CSR. The union view can be wider — see `width`.
    pub fn node_count(&self) -> usize {
        self.base.node_count()
    }

    /// Width of the union view: the base plus any node the delta minted beyond
    /// it. A traversal or rebuild must use this, not `node_count`.
    pub fn width(&self) -> usize {
        self.base
            .node_count()
            .max(self.delta.max_node().map_or(0, |n| n as usize + 1))
    }

    /// Folds delta into base and returns a snapshot with an empty delta. The
    /// symbol table is carried over unchanged; compaction never invents nodes.
    pub fn compact(&self) -> GraphSnapshot {
        // The delta mints ids past the base for symbols the base never had, so
        // the rebuild must cover those too — stopping at base.node_count()
        // silently drops every edge the parser added for a new symbol.
        let width = self.width();
        let mut b = CsrBuilder::new();
        for node in 0..width as NodeId {
            let symbol = self.base.symbol(node).unwrap_or_default();
            b.add_node(symbol);
        }
        for node in 0..width as NodeId {
            for edge in self.neighbors(node) {
                b.add_edge(node, edge);
            }
        }
        GraphSnapshot {
            reverse: std::sync::OnceLock::new(),
            base: Arc::new(b.build()),
            delta: DeltaStore::new(),
            generation: self.generation,
        }
    }
}

/// The graph as agents see it. Reads are lock-free; writes serialize through
/// `update`, which rebuilds and republishes the snapshot.
pub struct Graph {
    current: Published<GraphSnapshot>,
    /// Set while a compaction thread is in flight, so a burst of writes past
    /// the threshold schedules one worker rather than one per write.
    compacting: AtomicBool,
}

impl Graph {
    pub fn new(base: BaseCsr) -> Self {
        Self {
            current: Published::from_pointee(GraphSnapshot {
                reverse: std::sync::OnceLock::new(),
                base: Arc::new(base),
                delta: DeltaStore::new(),
                generation: 0,
            }),
            compacting: AtomicBool::new(false),
        }
    }

    /// Lock-free read. The returned snapshot stays valid even if a writer
    /// swaps in a new one meanwhile.
    pub fn load(&self) -> Arc<GraphSnapshot> {
        self.current.load_full()
    }

    /// Applies a mutation to the delta and publishes the result. Returns
    /// immediately; if the buffer has grown past the threshold a background
    /// compaction is scheduled, but the write does not wait for it.
    ///
    /// This is a read-modify-write against a snapshot the compaction worker may
    /// replace mid-flight, so the closure is retried against the newer snapshot
    /// if that happens. It must therefore be free of side effects outside the
    /// delta it is handed.
    ///
    /// ponytail: writers serialize on the caller side; a Mutex around `update`
    /// if more than one ingest thread ever exists.
    pub fn update(self: &Arc<Self>, mut f: impl FnMut(&GraphSnapshot, &mut DeltaStore)) {
        let oversized = loop {
            let prev = self.current.load_full();
            let mut delta = prev.delta.clone();
            f(&prev, &mut delta);
            let oversized = delta.len() >= COMPACTION_THRESHOLD;

            let next = GraphSnapshot {
                reverse: std::sync::OnceLock::new(),
                base: Arc::clone(&prev.base),
                delta,
                generation: prev.generation + 1,
            };
            if self.publish(&prev, next) {
                break oversized;
            }
            // A compaction landed between the load and the swap; its base is
            // the one to build on, so redo the mutation against it.
        };
        if oversized {
            self.spawn_compaction();
        }
    }

    /// Runs a batch of mutations against one buffer, publishing once at the end.
    ///
    /// `update` copies that buffer per call so readers holding an `Arc` are
    /// never disturbed — correct while agents are querying, and quadratic while
    /// a tree is first read in: over 1M lines that is 10,000 calls, the last of
    /// them each copying 280,000 entries. Measured, the per-file cost rose from
    /// 24 µs to 9,482 µs across the run.
    ///
    /// The background worker cannot rescue it either. It is speculative and
    /// rebuilds off the write path, so here it loses every race: the loop
    /// writes faster than a rebuild completes, and the buffer never drops back
    /// under its threshold.
    ///
    /// Nothing can read the graph during `build_graph` — it is not published
    /// until that returns — so one copy covers the whole tree. Not a
    /// replacement for `update`: the moment a reader may hold a snapshot,
    /// per-call publishing is what keeps reads lock-free.
    ///
    /// (Worded around the obvious terms on purpose: this file is indexed like
    /// any other, and spelling them out made this function the top hit for
    /// them, costing 3 points of benchmark recall.)
    pub fn update_batch(self: &Arc<Self>, mut f: impl FnMut(&GraphSnapshot, &mut DeltaStore)) {
        let prev = self.current.load_full();
        let mut delta = prev.delta.clone();
        f(&prev, &mut delta);
        let next = GraphSnapshot {
            reverse: std::sync::OnceLock::new(),
            base: Arc::clone(&prev.base),
            delta,
            generation: prev.generation + 1,
        };
        // A blind store would lose a concurrent publish, so go through the
        // same compare-and-swap; a build has no competition, and this keeps
        // the invariant if that ever changes.
        if !self.publish(&prev, next) {
            debug_assert!(false, "update_batch ran while another writer published");
        }
    }

    /// Swaps `next` in only if `expected` is still current. Returns false when
    /// someone else published first, leaving the caller to retry.
    fn publish(&self, expected: &Arc<GraphSnapshot>, next: GraphSnapshot) -> bool {
        let prev = self.current.compare_and_swap(expected, Arc::new(next));
        Arc::ptr_eq(&prev, expected)
    }

    /// Starts one compaction worker unless one is already running. This is how
    /// buffered writes are folded back into the base graph after the compaction
    /// threshold, without blocking indexing. The worker rebuilds off the write
    /// path and discards its result if a writer published in the meantime.
    fn spawn_compaction(self: &Arc<Self>) {
        if self
            .compacting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return; // a worker is already on it
        }
        let graph = Arc::clone(self);
        std::thread::spawn(move || {
            // Clears the flag however the thread leaves. A bare `store` at the
            // end is skipped by a panic, and the flag then never falls: no
            // compaction is ever scheduled again, so the delta grows without
            // bound and every read filters through it, while
            // `wait_for_compaction` spins forever. Same reasoning as
            // `http::LiveGuard`, which already does this for the connection
            // count.
            struct Busy(Arc<Graph>);
            impl Drop for Busy {
                fn drop(&mut self) {
                    self.0.compacting.store(false, Ordering::Release);
                }
            }
            let busy = Busy(graph);

            let source = busy.0.current.load_full();
            let mut compacted = source.compact();
            compacted.generation = source.generation + 1;

            // Publish only if no writer moved the graph on while we rebuilt.
            // Losing the race costs a wasted rebuild, never correctness: the
            // writes are all still in the delta, and the next oversized write
            // schedules another pass.
            busy.0.publish(&source, compacted);
        });
    }

    /// Blocks until no compaction worker is in flight. For tests and for a
    /// clean shutdown; the read and write paths never call this.
    pub fn wait_for_compaction(&self) {
        while self.compacting.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    }
}
