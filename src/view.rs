//! The graph as a map, for a person rather than an agent.
//!
//! The MCP tools answer a question you already have. This answers the one you
//! have when you open an unfamiliar repository: what is in here, and what talks
//! to what.
//!
//! The layout is built around communities as regions rather than a plain force
//! graph, because that is what this graph knows that a generic viewer does not:
//! which symbols form a subsystem, which are connectors between them, and how
//! each edge was resolved. A cloud of dots would throw all three away.

use crate::csr::{Confidence, NodeId};
use crate::layout::Layout;
use crate::mcp::Served;
use serde_json::{Value, json};

/// How many nodes the page is sent. Beyond this the view shows the most
/// connected part of the graph instead of all of it.
///
/// Set from a measurement, not a guess: the WebGL renderer draws every node in
/// one call and swaps to per-subsystem clumps as you zoom out, so 24,880 nodes
/// and 143,240 edges — the shape of a 1M-LOC repository — lay out and render in
/// under half a second. The cap sits above that with room to spare.
///
/// What still scales past it is the CSR, which is memory-mapped: a
/// Linux-kernel-sized tree is ~40 MiB and a two-billion-line monorepo ~2 GiB,
/// faulting in only the pages a traversal touches. The remaining limit is the
/// JSON payload — the page is handed the whole graph up front.
///
/// Only `graph_json` is bounded by this. The page itself streams by viewport
/// (`viewport_json` plus a stored layout), so what it holds is bounded by the
/// screen rather than by the repository.
pub const MAX_VIEW_NODES: usize = 60_000;

/// Name a community after the file most of its symbols live in — the honest
/// label, and one nobody has to trust an LLM for. Ties break on name, so the
/// label is stable across runs.
fn community_label(cid: u32, members: &[NodeId], names: &[String]) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for &m in members {
        if let Some((file, _)) = names[m as usize].split_once('#') {
            *counts.entry(file).or_insert(0) += 1;
        }
    }
    let mut ranked: Vec<(&&str, &usize)> = counts.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    ranked
        .first()
        .map(|(f, _)| {
            let base = f.rsplit('/').next().unwrap_or(f);
            base.rsplit_once('.')
                .map_or(base, |(stem, _)| stem)
                .to_string()
        })
        .unwrap_or_else(|| format!("community {cid}"))
}

/// The whole graph in the shape the page draws from, or its most connected part
/// when the graph is too large to draw.
pub fn graph_json(served: &Served) -> Value {
    let width = served.snap.width();

    // Symbol names, once, so edges can reference nodes by index.
    let mut names: Vec<String> = vec![String::new(); width];
    for (symbol, &node) in served.registry.entries() {
        if let Some(slot) = names.get_mut(node as usize) {
            *slot = symbol.clone();
        }
    }

    // When the graph exceeds what a canvas can draw, keep the most connected
    // nodes: they are the ones a map is for. Ranking by degree with the node id
    // breaking ties keeps the choice deterministic.
    let mut candidates: Vec<NodeId> = (0..width as NodeId)
        .filter(|&id| !names[id as usize].is_empty())
        .collect();
    let truncated = candidates.len() > MAX_VIEW_NODES;
    if truncated {
        candidates.sort_by_key(|&id| (std::cmp::Reverse(served.snap.neighbors(id).count()), id));
        candidates.truncate(MAX_VIEW_NODES);
        candidates.sort_unstable();
    }
    let shown: std::collections::HashSet<NodeId> = candidates.iter().copied().collect();

    let hubs: std::collections::HashSet<NodeId> = served.communities.hubs.iter().copied().collect();
    let mut nodes = Vec::with_capacity(shown.len());
    for &id in &candidates {
        let name = &names[id as usize];
        // Split the qualified name once here rather than in the page: the file
        // is what groups symbols visually, the tail is what gets drawn.
        let (file, label) = name.split_once('#').unwrap_or(("", name.as_str()));
        nodes.push(json!({
            "id": id,
            "label": label,
            "file": file,
            "community": served.communities.of_node.get(id as usize).copied().unwrap_or(0),
            "degree": served.snap.neighbors(id).count(),
            "hub": hubs.contains(&id),
            // A symbol the tree does not define is a reference into another
            // crate — drawn faintly, since it is context rather than content.
            "local": served.defined.contains(&id),
        }));
    }

    let mut edges = Vec::new();
    for &source in &candidates {
        for edge in served.snap.neighbors(source) {
            if !shown.contains(&edge.target) {
                continue;
            }
            edges.push(json!({
                "s": source,
                "t": edge.target,
                "c": match edge.confidence {
                    Confidence::Extracted => "extracted",
                    Confidence::Inferred => "inferred",
                    Confidence::Ambiguous => "ambiguous",
                },
            }));
        }
    }

    // Communities worth naming: one holding a single symbol is not a subsystem.
    let mut communities = Vec::new();
    for cid in 0..served.communities.count() as u32 {
        let members: Vec<NodeId> = served
            .communities
            .members(cid)
            .into_iter()
            .filter(|n| served.defined.contains(n) && shown.contains(n))
            .collect();
        if members.len() < 2 {
            continue;
        }
        let label = community_label(cid, &members, &names);

        // Edge weight between subsystems, so a zoomed-out view can draw the
        // connections between clumps rather than nothing at all.
        communities.push(json!({
            "id": cid,
            "label": label,
            "size": members.len(),
            "members": members,
        }));
    }
    // Largest first: the page draws them in order and the biggest regions
    // should claim the centre.
    communities.sort_by_key(|c| std::cmp::Reverse(c["size"].as_u64().unwrap_or(0)));

    // Aggregate edges between subsystems, computed here rather than in the
    // page: at the zoomed-out level these are what there is to see, and the
    // server already has the adjacency.
    let of_node = &served.communities.of_node;
    let mut links: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    let kept: std::collections::HashSet<u32> = communities
        .iter()
        .filter_map(|c| c["id"].as_u64().map(|v| v as u32))
        .collect();
    for e in &edges {
        let (Some(s), Some(t)) = (e["s"].as_u64(), e["t"].as_u64()) else {
            continue;
        };
        let (Some(&a), Some(&b)) = (of_node.get(s as usize), of_node.get(t as usize)) else {
            continue;
        };
        if a == b || !kept.contains(&a) || !kept.contains(&b) {
            continue;
        }
        // Undirected at this level: "these two subsystems talk" is the fact,
        // not which way round.
        let key = if a < b { (a, b) } else { (b, a) };
        *links.entry(key).or_insert(0) += 1;
    }
    let mut community_links: Vec<Value> = links
        .into_iter()
        .map(|((a, b), w)| json!({"a": a, "b": b, "w": w}))
        .collect();
    community_links.sort_by_key(|l| (l["a"].as_u64().unwrap_or(0), l["b"].as_u64().unwrap_or(0)));

    json!({
        "nodes": nodes,
        "edges": edges,
        "communities": communities,
        "communityLinks": community_links,
        "stats": {
            "nodes": nodes.len(),
            "edges": edges.len(),
            "communities": communities.len(),
            "hubs": served.communities.hubs.len(),
            // The page says so rather than quietly showing a partial graph as
            // if it were the whole one.
            "total": width,
            "truncated": truncated,
        }
    })
}

/// Nodes returned by one viewport query. A screen cannot usefully show more,
/// and the cap is what makes the response size independent of the graph.
const VIEWPORT_NODES: usize = 3_000;

/// The slice of the graph inside a rectangle, for a page that streams as it
/// pans instead of holding everything.
///
/// This is what the stored layout buys: positions live in flat arrays beside
/// the CSR, so answering "what is on screen" is a grid lookup and a range read
/// rather than a serialisation of the whole graph. The cost is bounded by the
/// viewport, not by the repository.
pub fn viewport_json(
    served: &Served,
    layout: &Layout,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
) -> Value {
    let mut inside = layout.in_view(x0, y0, x1, y1);
    let clipped = inside.len() > VIEWPORT_NODES;
    if clipped {
        // Keep the most connected: at this density they are what carries the
        // structure, and dropping arbitrary nodes would make panning flicker.
        inside.sort_by_key(|&n| (std::cmp::Reverse(served.snap.neighbors(n).count()), n));
        inside.truncate(VIEWPORT_NODES);
        inside.sort_unstable();
    }
    let shown: std::collections::HashSet<NodeId> = inside.iter().copied().collect();

    let mut names: std::collections::HashMap<NodeId, &String> = std::collections::HashMap::new();
    for (symbol, node) in served.registry.entries() {
        if shown.contains(node) {
            names.insert(*node, symbol);
        }
    }

    let hubs: std::collections::HashSet<NodeId> = served.communities.hubs.iter().copied().collect();
    let nodes: Vec<Value> = inside
        .iter()
        .filter_map(|&id| {
            let name = names.get(&id)?;
            let (file, label) = name.split_once('#').unwrap_or(("", name.as_str()));
            Some(json!({
                "id": id,
                "label": label,
                "file": file,
                // Positions come from the server now, so the page never lays
                // out anything — it draws what it is given.
                "x": layout.x[id as usize],
                "y": layout.y[id as usize],
                "community": served.communities.of_node.get(id as usize).copied().unwrap_or(0),
                "degree": served.snap.neighbors(id).count(),
                "hub": hubs.contains(&id),
                "local": served.defined.contains(&id),
            }))
        })
        .collect();

    // Only edges with both ends on screen: one dangling into the dark would be
    // a line to nowhere.
    let mut edges = Vec::new();
    for &s in &inside {
        for e in served.snap.neighbors(s) {
            if shown.contains(&e.target) {
                edges.push(json!({
                    "s": s,
                    "t": e.target,
                    "c": match e.confidence {
                        Confidence::Extracted => "extracted",
                        Confidence::Inferred => "inferred",
                        Confidence::Ambiguous => "ambiguous",
                    },
                }));
            }
        }
    }

    json!({
        "nodes": nodes,
        "edges": edges,
        "clipped": clipped,
        "bounds": [layout.bounds.0, layout.bounds.1, layout.bounds.2, layout.bounds.3],
    })
}

/// Subsystem clumps for the whole graph, sent once.
///
/// Small enough to ship up front at any repository size — a monorepo has
/// thousands of subsystems, not millions — and it is what the page draws when
/// zoomed out, so it must never depend on a viewport query.
pub fn overview_json(served: &Served, layout: &Layout) -> Value {
    let mut names: Vec<String> = vec![String::new(); served.snap.width()];
    for (symbol, &node) in served.registry.entries() {
        if let Some(slot) = names.get_mut(node as usize) {
            *slot = symbol.clone();
        }
    }
    let mut out = Vec::new();
    for cid in 0..served.communities.count() as u32 {
        let members: Vec<NodeId> = served
            .communities
            .members(cid)
            .into_iter()
            .filter(|n| served.defined.contains(n))
            .collect();
        if members.len() < 2 {
            continue;
        }
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for &m in &members {
            cx += layout.x[m as usize];
            cy += layout.y[m as usize];
        }
        let k = members.len() as f32;
        out.push(json!({
            "id": cid,
            "label": community_label(cid, &members, &names),
            "size": members.len(),
            "cx": cx / k,
            "cy": cy / k,
        }));
    }
    json!(out)
}
