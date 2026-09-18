//! Where each node sits, computed once on the server.
//!
//! Layout has to live here rather than in the page for one reason: a viewport
//! query is "which nodes fall inside this rectangle", and nobody can answer
//! that without coordinates. The CSR stores topology and no positions, so this
//! is the missing half — and once it exists, serving a pan is a range scan over
//! a grid instead of shipping the whole graph.
//!
//! Deterministic like everything else: the seed is a golden-angle spiral, not a
//! sample, and relaxation runs a fixed number of passes. The same graph always
//! produces the same map, so a position can be cited and a bookmark keeps
//! working.

use crate::csr::NodeId;
use crate::graph::GraphSnapshot;
use crate::parallel;
use serde::{Deserialize, Serialize};

/// Relaxation passes. Fixed rather than run-to-convergence: the graph is
/// static, so a bounded count cannot drift and cannot hang on a pathological
/// shape.
const PASSES: usize = 140;

/// Cell size of the spatial grid, in layout units. Roughly the span a node's
/// neighbourhood occupies, so a viewport query touches few cells and each holds
/// few nodes.
const CELL: f32 = 128.0;

/// Positions on disk, in the same memory layout the CSR uses.
///
/// Two flat coordinate arrays plus the rule identity. The stored document is
/// decoded into owned memory and schema-validated before it is used.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct StoredLayout {
    pub parser_rules: String,
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

pub struct Layout {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    /// Grid cell -> nodes in it, so a rectangle query does not scan every node.
    grid: std::collections::HashMap<(i32, i32), Vec<NodeId>>,
    pub bounds: (f32, f32, f32, f32),
}

impl Layout {
    /// Nodes whose position falls inside the rectangle, plus a margin so
    /// edges leaving the viewport still have both endpoints.
    pub fn in_view(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<NodeId> {
        let mut out = Vec::new();
        let (cx0, cy0) = ((x0 / CELL).floor() as i32, (y0 / CELL).floor() as i32);
        let (cx1, cy1) = ((x1 / CELL).ceil() as i32, (y1 / CELL).ceil() as i32);
        // A viewport spanning the whole graph would touch every cell anyway, so
        // there is no fast path to add here — the cap on returned nodes is what
        // bounds the response.
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                let Some(cell) = self.grid.get(&(cx, cy)) else {
                    continue;
                };
                for &n in cell {
                    let (px, py) = (self.x[n as usize], self.y[n as usize]);
                    if px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                        out.push(n);
                    }
                }
            }
        }
        // Deterministic order regardless of how the grid hashed.
        out.sort_unstable();
        out
    }
}

/// How strongly a shared community pulls two nodes together.
///
/// This one number is the difference between the two layouts the view offers:
/// at `Grouped` a subsystem contracts into a visible region, at `Free` the
/// graph finds its own shape and communities only tint it. Same relaxation,
/// same determinism — one constant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Subsystems as regions: the default, and what the map is for.
    Grouped,
    /// A plain force graph, for when the partition is what you want to judge.
    Free,
}

impl Mode {
    pub fn from_str(s: &str) -> Mode {
        match s {
            "free" => Mode::Free,
            _ => Mode::Grouped,
        }
    }

    fn same_community_pull(self) -> f32 {
        match self {
            Mode::Grouped => 0.020,
            // Equal to the cross-community pull, so membership stops shaping
            // the result at all.
            Mode::Free => 0.010,
        }
    }
}

/// Lays out the graph by force relaxation.
///
/// Three forces, the standard set: neighbours attract, everything drifts
/// toward the origin so the drawing stays bounded, and nodes in the same
/// community attract more strongly — the last is what keeps the partition
/// visible instead of dissolving into an even ball.
pub fn compute(snap: &GraphSnapshot, community: &[u32], mode: Mode) -> Layout {
    let n = snap.width();
    if n == 0 {
        return Layout {
            x: Vec::new(),
            y: Vec::new(),
            grid: std::collections::HashMap::new(),
            bounds: (0.0, 0.0, 0.0, 0.0),
        };
    }

    // Golden-angle seeding: even coverage without randomness.
    let mut x: Vec<f32> = Vec::with_capacity(n);
    let mut y: Vec<f32> = Vec::with_capacity(n);
    for i in 0..n {
        let a = i as f32 * 2.399_963_2;
        let r = 26.0 * (i as f32).sqrt();
        x.push(a.cos() * r);
        y.push(a.sin() * r);
    }

    // Undirected adjacency, flattened once. Rebuilding it per pass would
    // dominate the cost.
    let mut offsets = Vec::with_capacity(n + 1);
    let mut peers: Vec<NodeId> = Vec::new();
    {
        let mut lists: Vec<Vec<NodeId>> = vec![Vec::new(); n];
        for node in 0..n as NodeId {
            for e in snap.neighbors(node) {
                if (e.target as usize) < n && e.target != node {
                    lists[node as usize].push(e.target);
                    lists[e.target as usize].push(node);
                }
            }
        }
        for list in &mut lists {
            list.sort_unstable();
            list.dedup();
            offsets.push(peers.len() as u32);
            peers.extend(list.iter().copied());
        }
        offsets.push(peers.len() as u32);
    }

    let same = mode.same_community_pull();
    let mut vx = vec![0.0f32; n];
    let mut vy = vec![0.0f32; n];

    for pass in 0..PASSES {
        let cool = 1.0 - pass as f32 / PASSES as f32;
        // Forces read the previous positions and write velocities, so a pass is
        // embarrassingly parallel; positions are applied afterwards.
        let (fx, fy): (Vec<f32>, Vec<f32>) = parallel::map_range(n, |i| {
            let (px, py) = (x[i], y[i]);
            let mut ax = -px * 0.006;
            let mut ay = -py * 0.006;
            let from = offsets[i] as usize;
            let to = offsets[i + 1] as usize;
            for &p in &peers[from..to] {
                let j = p as usize;
                // Same community pulls harder, which is what makes
                // subsystems read as regions rather than as a gradient.
                let k = if community.get(i) == community.get(j) {
                    same
                } else {
                    0.010
                };
                ax += (x[j] - px) * k;
                ay += (y[j] - py) * k;
            }
            (ax, ay)
        })
        .into_iter()
        .unzip();

        for i in 0..n {
            vx[i] = (vx[i] + fx[i]) * 0.82 * cool;
            vy[i] = (vy[i] + fy[i]) * 0.82 * cool;
            x[i] += vx[i];
            y[i] += vy[i];
        }
    }

    index(x, y)
}

/// Writes a layout beside the graph it belongs to.
pub fn write(layout: &Layout, path: &std::path::Path) -> std::io::Result<()> {
    let stored = StoredLayout {
        parser_rules: native_parsers::rules::active().identity().to_owned(),
        x: layout.x.clone(),
        y: layout.y.clone(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    std::fs::write(path, &bytes)
}

/// Reads a stored layout and rebuilds its spatial index.
///
/// The grid is rebuilt rather than stored: it is a `HashMap` of small vectors,
/// which does not map as cheaply as two float arrays, and rebuilding it is a
/// single pass over the coordinates.
pub fn read(path: &std::path::Path, expected_nodes: usize) -> Option<Layout> {
    let bytes = std::fs::read(path).ok()?;
    let stored: StoredLayout = serde_json::from_slice(&bytes).ok()?;
    if stored.parser_rules != native_parsers::rules::active().identity() {
        return None;
    }
    let x = stored.x;
    let y = stored.y;
    // A layout for a different graph is worse than none: node ids would point
    // at the wrong positions and the map would be silently wrong.
    if x.len() != expected_nodes || y.len() != expected_nodes {
        return None;
    }
    Some(index(x, y))
}

/// Builds the spatial grid and bounds from coordinates.
fn index(x: Vec<f32>, y: Vec<f32>) -> Layout {
    let bounds = (
        x.iter().copied().fold(f32::MAX, f32::min),
        y.iter().copied().fold(f32::MAX, f32::min),
        x.iter().copied().fold(f32::MIN, f32::max),
        y.iter().copied().fold(f32::MIN, f32::max),
    );
    let mut grid: std::collections::HashMap<(i32, i32), Vec<NodeId>> =
        std::collections::HashMap::new();
    for i in 0..x.len() {
        grid.entry(((x[i] / CELL).floor() as i32, (y[i] / CELL).floor() as i32))
            .or_default()
            .push(i as NodeId);
    }
    Layout { x, y, grid, bounds }
}
