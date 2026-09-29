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

/// Lays out the graph. `file_of` is the file id per node, `u32::MAX` for a
/// name the tree does not define (see `community::Context`).
pub fn compute(snap: &GraphSnapshot, community: &[u32], file_of: &[u32], mode: Mode) -> Layout {
    match mode {
        Mode::Grouped => regions(snap, community, file_of),
        Mode::Free => relax(snap, community, mode),
    }
}

/// How far a node's push reaches in the free layout, and how hard it pushes.
const REPEL_RANGE: f32 = 60.0;
const REPEL: f32 = 60.0;

/// Distance between neighbouring nodes inside a region, in layout units.
const SPACING: f32 = 11.0;
/// Empty margin around each region, so two regions never read as one.
const GAP: f32 = 34.0;

/// Subsystems as separate discs: every region gets its own area, sized by its
/// member count and packed around the origin largest first, and inside it the
/// most connected symbols sit at the centre. Relaxation alone cannot do this —
/// with attraction only, every connected component falls onto one point, which
/// is what the map showed before.
///
/// A region is a file: the one most of a community's symbols live in, or a
/// loose symbol's own. A name the tree does not define sits on the rim of the
/// region that uses it most.
fn regions(snap: &GraphSnapshot, community: &[u32], file_of: &[u32]) -> Layout {
    use std::collections::HashMap;
    let n = snap.width();
    let local = |i: usize| file_of.get(i).is_some_and(|&f| f != u32::MAX);
    let comm = |i: usize| community.get(i).copied().unwrap_or(u32::MAX);

    let mut size: HashMap<u32, usize> = HashMap::new();
    for i in (0..n).filter(|&i| local(i)) {
        *size.entry(comm(i)).or_default() += 1;
    }
    // A community's region is the file most of its symbols live in, so
    // communities sharing that file share a disc, as they share a name.
    let mut counts: HashMap<u32, HashMap<u32, usize>> = HashMap::new();
    for i in (0..n).filter(|&i| local(i) && size[&comm(i)] >= 2) {
        *counts
            .entry(comm(i))
            .or_default()
            .entry(file_of[i])
            .or_default() += 1;
    }
    let home: HashMap<u32, u32> = counts
        .iter()
        .filter_map(|(c, files)| {
            let (f, _) = files
                .iter()
                .max_by_key(|(f, v)| (**v, std::cmp::Reverse(**f)))?;
            Some((*c, *f))
        })
        .collect();
    // Region key: (0, file) for the tree's own symbols — a symbol in no
    // community joins its own file's disc — and (2, 0) for names used by
    // nothing local.
    let mut region: Vec<Option<(u8, u32)>> = vec![None; n];
    for i in (0..n).filter(|&i| local(i)) {
        region[i] = Some((0, home.get(&comm(i)).copied().unwrap_or(file_of[i])));
    }
    // A foreign name goes where most of its users are.
    let mut users: Vec<HashMap<(u8, u32), usize>> = vec![HashMap::new(); n];
    for from in 0..n as NodeId {
        let Some(r) = region[from as usize] else {
            continue;
        };
        for e in snap.neighbors(from) {
            let t = e.target as usize;
            if t < n && region[t].is_none() {
                *users[t].entry(r).or_default() += 1;
            }
        }
    }
    let foreign: Vec<Option<(u8, u32)>> = (0..n)
        .map(|i| {
            if region[i].is_some() {
                return None;
            }
            users[i]
                .iter()
                .max_by_key(|(k, v)| (**v, std::cmp::Reverse(**k)))
                .map(|(k, _)| *k)
                .or(Some((2, 0)))
        })
        .collect();

    let degree: Vec<usize> = {
        let mut d = vec![0usize; n];
        for from in 0..n as NodeId {
            for e in snap.neighbors(from) {
                d[from as usize] += 1;
                if let Some(t) = d.get_mut(e.target as usize) {
                    *t += 1;
                }
            }
        }
        d
    };
    let mut members: HashMap<(u8, u32), Vec<usize>> = HashMap::new();
    for i in 0..n {
        if let Some(r) = region[i].or(foreign[i]) {
            members.entry(r).or_default().push(i);
        }
    }
    // Defined symbols first and the most connected innermost; foreign names
    // after them, so they land on the rim.
    for list in members.values_mut() {
        list.sort_by_key(|&i| (!local(i), std::cmp::Reverse(degree[i]), i));
    }
    let mut order: Vec<(u8, u32)> = members.keys().copied().collect();
    order.sort_by_key(|k| (std::cmp::Reverse(members[k].len()), *k));

    let (mut x, mut y) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut placed: Vec<(f32, f32, f32)> = Vec::with_capacity(order.len());
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    const PACK_CELL: f32 = 96.0;
    let cells = |cx: f32, cy: f32, r: f32| {
        let (x0, x1) = (
            ((cx - r) / PACK_CELL).floor() as i32,
            ((cx + r) / PACK_CELL).floor() as i32,
        );
        let (y0, y1) = (
            ((cy - r) / PACK_CELL).floor() as i32,
            ((cy + r) / PACK_CELL).floor() as i32,
        );
        (x0..=x1).flat_map(move |gx| (y0..=y1).map(move |gy| (gx, gy)))
    };
    let mut area = 0.0f32;
    for (k, key) in order.iter().enumerate() {
        let list = &members[key];
        let r = SPACING * (list.len() as f32 + 1.0).sqrt() + GAP / 2.0;
        area += r * r;
        // Golden-angle spiral by cumulative area, pushed outward until clear.
        let angle = k as f32 * 2.399_963_2;
        let (dx, dy) = (angle.cos(), angle.sin());
        let mut dist = if k == 0 { 0.0 } else { area.sqrt() * 0.9 };
        let (cx, cy) = loop {
            let (cx, cy) = (dx * dist, dy * dist);
            let clash = cells(cx, cy, r).any(|c| {
                grid.get(&c).is_some_and(|ids| {
                    ids.iter().any(|&j| {
                        let (px, py, pr) = placed[j];
                        (px - cx).hypot(py - cy) < pr + r
                    })
                })
            });
            if !clash {
                break (cx, cy);
            }
            dist += r * 0.25 + 1.0;
        };
        for c in cells(cx, cy, r) {
            grid.entry(c).or_default().push(placed.len());
        }
        placed.push((cx, cy, r));
        for (slot, &i) in list.iter().enumerate() {
            let a = slot as f32 * 2.399_963_2;
            let d = SPACING * (slot as f32 + 0.5).sqrt();
            x[i] = cx + a.cos() * d;
            y[i] = cy + a.sin() * d;
        }
    }
    index(x, y)
}

/// A plain force relaxation: neighbours attract, everything drifts toward the
/// origin, and a shared community pulls harder.
fn relax(snap: &GraphSnapshot, community: &[u32], mode: Mode) -> Layout {
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
        // Nearby nodes push apart, found through a grid rebuilt per pass so
        // the cost stays linear. Without it nothing opposes the attraction
        // and every connected node falls onto one point.
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
        for i in 0..n {
            cells
                .entry((
                    (x[i] / REPEL_RANGE).floor() as i32,
                    (y[i] / REPEL_RANGE).floor() as i32,
                ))
                .or_default()
                .push(i);
        }
        // Forces read the previous positions and write velocities, so a pass is
        // embarrassingly parallel; positions are applied afterwards.
        let (fx, fy): (Vec<f32>, Vec<f32>) = parallel::map_range(n, |i| {
            let (px, py) = (x[i], y[i]);
            let mut ax = -px * 0.006;
            let mut ay = -py * 0.006;
            let (gx, gy) = (
                (px / REPEL_RANGE).floor() as i32,
                (py / REPEL_RANGE).floor() as i32,
            );
            for cx in gx - 1..=gx + 1 {
                for cy in gy - 1..=gy + 1 {
                    for &j in cells.get(&(cx, cy)).map_or(&[][..], |v| v.as_slice()) {
                        if j == i {
                            continue;
                        }
                        let (dx, dy) = (px - x[j], py - y[j]);
                        let d2 = dx * dx + dy * dy;
                        if d2 < REPEL_RANGE * REPEL_RANGE {
                            // Coincident nodes are split by id, not by chance.
                            let (dx, dy, d2) = if d2 < 1e-4 {
                                let a = (i as f32 - j as f32) * 2.399_963_2;
                                (a.cos(), a.sin(), 1.0)
                            } else {
                                (dx, dy, d2)
                            };
                            ax += dx * REPEL / d2;
                            ay += dy * REPEL / d2;
                        }
                    }
                }
            }
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
        parser_rules: identity(),
        x: layout.x.clone(),
        y: layout.y.clone(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    std::fs::write(path, &bytes)
}

/// What a stored layout must match: the rules that produced the graph, and
/// this algorithm's version — a layout from an older algorithm has the right
/// length and the wrong picture.
fn identity() -> String {
    const LAYOUT_VERSION: u32 = 2;
    format!(
        "{} layout/{LAYOUT_VERSION}",
        native_parsers::rules::active().identity()
    )
}

/// Reads a stored layout and rebuilds its spatial index.
///
/// The grid is rebuilt rather than stored: it is a `HashMap` of small vectors,
/// which does not map as cheaply as two float arrays, and rebuilding it is a
/// single pass over the coordinates.
pub fn read(path: &std::path::Path, expected_nodes: usize) -> Option<Layout> {
    let bytes = std::fs::read(path).ok()?;
    let stored: StoredLayout = serde_json::from_slice(&bytes).ok()?;
    if stored.parser_rules != identity() {
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
