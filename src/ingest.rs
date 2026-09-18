//! Wires the parser to the graph: file change -> facts -> delta edges.
//!
//! The base CSR is immutable, so a symbol first seen at runtime cannot get a
//! node there. `SymbolRegistry` hands out ids past `base.node_count()`; those
//! nodes live only in the delta until a compaction folds them into a new base.
//! That is why lookups go through the registry rather than through the CSR.

use crate::arena::SymbolArena;
use crate::csr::{Edge, NodeId};
use crate::delta::DeltaStore;
use crate::graph::{Graph, GraphSnapshot};
use crate::parse_ast::{self, Lang, LangExt};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Canonical graph paths are root-relative and always use `/`.
///
/// Symbol identities, snapshot metadata and contracts are persisted across
/// operating systems. Keeping this conversion at the ingestion boundary
/// prevents Windows separators from creating a second identity for a file.
fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Maps a qualified symbol name to a node id, minting ids for symbols the base
/// graph has never seen.
pub struct SymbolRegistry {
    by_name: HashMap<String, NodeId>,
    /// Documentation per node, when the parser found any. Kept here rather
    /// than threaded through `ingest_file`'s return value because it belongs
    /// to the symbol, exactly like its name — and the search index is built
    /// from the registry.
    docs: HashMap<NodeId, String>,
    /// Byte range per node, for the tool that returns source rather than a
    /// name. Only definitions have one — a placeholder names a call site in
    /// somebody else's crate and has no text here to point at.
    spans: HashMap<NodeId, (u32, u32)>,
    /// The language a placeholder was minted from, when it was minted from one.
    ///
    /// A placeholder is a bare name with no file, so nothing about it says
    /// which language its call site was written in — and tier 3 links by name
    /// alone. Measured on this tree, `console.error` in a `.js` file reached
    /// `src/mcp.rs#error` that way, and 18 of the 20 cycles `cycles` reported
    /// ran through that one edge. SCIP solves the same problem by putting the
    /// scheme in the symbol itself; this keeps it beside the symbol instead, so
    /// the name a caller sees is unchanged.
    ///
    /// A name reached from two languages has none: it is then shared, and the
    /// safe reading is to link it nowhere rather than to guess a side.
    placeholder_lang: HashMap<NodeId, Option<Lang>>,
    next: NodeId,
}

impl SymbolRegistry {
    /// Starts allocating past the base graph's nodes, so ids never collide with
    /// the ones the CSR already owns.
    pub fn new(base_node_count: usize) -> Self {
        Self {
            by_name: HashMap::new(),
            docs: HashMap::new(),
            spans: HashMap::new(),
            placeholder_lang: HashMap::new(),
            next: base_node_count as NodeId,
        }
    }

    /// Pre-registers the symbols an imported graph already contains.
    pub fn insert(&mut self, name: String, node: NodeId) {
        self.by_name.insert(name, node);
    }

    /// Every (symbol, node) pair. Tier 3 needs the whole table to match
    /// placeholders against definitions.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &NodeId)> {
        self.by_name.iter()
    }

    /// Attaches documentation to a symbol. A re-parse overwrites it, so an
    /// edited comment does not leave the old text behind.
    pub fn set_doc(&mut self, node: NodeId, doc: String) {
        self.docs.insert(node, doc);
    }

    /// Documentation per node, for the search index to fold in.
    pub fn docs(&self) -> &HashMap<NodeId, String> {
        &self.docs
    }

    pub fn set_span(&mut self, node: NodeId, span: (u32, u32)) {
        self.spans.insert(node, span);
    }

    /// Byte range of a definition, when the parser recorded one.
    pub fn span(&self, node: NodeId) -> Option<(u32, u32)> {
        self.spans.get(&node).copied()
    }

    /// Every recorded span, for the snapshot to store.
    pub fn spans(&self) -> &HashMap<NodeId, (u32, u32)> {
        &self.spans
    }

    /// Node for an exact symbol, without minting one if it is absent.
    pub fn node_of(&self, name: &str) -> Option<NodeId> {
        self.by_name.get(name).copied()
    }

    /// Records which language a placeholder was reached from.
    ///
    /// Called once per call site, so a name reached from two languages ends up
    /// with `None` — shared, and not safely linkable to either side.
    pub fn note_placeholder_lang(&mut self, node: NodeId, lang: Lang) {
        match self.placeholder_lang.get(&node) {
            Some(Some(seen)) if *seen == lang => {}
            Some(_) => {
                self.placeholder_lang.insert(node, None);
            }
            None => {
                self.placeholder_lang.insert(node, Some(lang));
            }
        }
    }

    /// The language a placeholder was reached from, if exactly one.
    pub fn placeholder_lang(&self, node: NodeId) -> Option<Lang> {
        self.placeholder_lang.get(&node).copied().flatten()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn get_or_mint(&mut self, name: &str) -> NodeId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.next;
        self.next += 1;
        self.by_name.insert(name.to_owned(), id);
        id
    }

    /// (unqualified, qualified) symbol counts. An unqualified name is a
    /// reference we could not attach to a definition.
    pub fn split_counts(&self) -> (usize, usize) {
        let bare = self.by_name.keys().filter(|k| !k.contains('#')).count();
        (bare, self.by_name.len() - bare)
    }

    /// Drops every symbol a vanished file defined.
    ///
    /// **A moved file is a whole new set of nodes, and the old set used to
    /// stay.** The path is part of the key, so `src/old/x.rs#f` and
    /// `src/new/x.rs#f` are different symbols; the incremental path invalidated
    /// the old file's *edges* and kept its names, on the reasoning that
    /// something might still reference them. Nothing can: a qualified name
    /// names a path that is no longer there. Measured on a 60-file fixture,
    /// moving five files per round: 120 nodes grew to 180 over six rounds while
    /// the tree still held 120 symbols, and `src/mod3.rs#work3` — moved away in
    /// round one — seeded at 1.000 *above* its own replacement.
    ///
    /// The id is not reused, only the name is dropped: ids are handed out
    /// monotonically and a recycled one would attach to whatever edges the old
    /// node still had.
    pub fn forget_file(&mut self, file: &str) {
        let prefix = format!("{file}#");
        let gone: Vec<NodeId> = self
            .by_name
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, &n)| n)
            .collect();
        self.by_name.retain(|k, _| !k.starts_with(&prefix));
        for n in gone {
            self.docs.remove(&n);
            // A span into a file that is gone points at bytes that are not
            // there; the same reasoning as the docs beside it.
            self.spans.remove(&n);
        }
    }

    /// Drops bare placeholders nothing references any more.
    ///
    /// `forget_file` drops a vanished file's `file#name` keys, but the *bare*
    /// nodes that file alone minted survive it: a placeholder's key carries no
    /// path, so nothing connects it to the file that caused it. Measured on a
    /// 60-file fixture deleting five files per round, the leak is linear —
    /// five orphans per round, thirty after six, while the tree held 61
    /// symbols and the registry 91. They are not inert: `unique_call_1`
    /// through `_9` all seeded a search for their own name, every one of them
    /// naming a call site that no longer exists.
    ///
    /// Only a placeholder with **no edge in either direction** goes. One that
    /// is still called, or that tier 3 resolved onto a definition, is live.
    /// Ids are not reused, exactly as in `forget_file`.
    pub fn forget_orphans(&mut self, referenced: &std::collections::HashSet<NodeId>) -> usize {
        let before = self.by_name.len();
        self.by_name
            .retain(|k, n| k.contains('#') || referenced.contains(n));
        before - self.by_name.len()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Qualified symbol name. Definitions are file-scoped so two files may each
/// define `main` without colliding.
fn qualify(file: &str, name: &str) -> String {
    format!("{file}#{name}")
}

/// Re-parses one file and replaces its edges in the delta.
///
/// A callee is resolved against the registry by qualified name first (same
/// file), then by bare name (already seen anywhere). An unresolved callee still
/// gets a node: it is a real reference to something we have not parsed yet, and
/// dropping it would silently lose the edge. Tier 3 is what later merges those
/// placeholders onto real definitions.
/// A Markdown document as graph nodes: one per section, edged to the code it
/// names.
///
/// Sections rather than whole files, because a document answers several
/// questions and a reader needs the paragraph that answers theirs. The
/// section's prose becomes its documentation, which is what makes it findable —
/// measured on code, prose is what carries a plain-language question.
///
/// **A mention only becomes an edge when the name already exists in the graph.**
/// A document naming `HashMap` must not mint a node for it, and one naming a
/// symbol this tree does not define is describing something else. Ordering
/// therefore matters: markdown is ingested after the code, so the symbols are
/// there to be found.
pub fn ingest_markdown(
    graph: &Arc<Graph>,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    source: &str,
    now: u64,
) -> usize {
    let file = relative_path(path, root);
    let stem = file
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map_or(file.as_str(), |(s, _)| s)
        .to_string();

    // Bare name -> the one symbol defining it, built once per document rather
    // than scanned per mention. The scan was O(mentions × symbols) and cost
    // 2.4 s of a 5 s cold start on a 153k-line tree — invisible on this repo,
    // where both numbers are small.
    let mut by_tail: HashMap<&str, Option<NodeId>> = HashMap::new();
    for (symbol, &node) in registry.entries() {
        if let Some((_, tail)) = symbol.split_once('#') {
            // `None` marks a name defined more than once: ambiguity is refused
            // for the reason tier 3 refuses it, a wrong edge being worse than a
            // missing one.
            by_tail
                .entry(tail)
                .and_modify(|slot| *slot = None)
                .or_insert(Some(node));
        }
    }
    let by_tail: HashMap<String, NodeId> = by_tail
        .into_iter()
        .filter_map(|(k, v)| v.map(|n| (k.to_string(), n)))
        .collect();

    let mut nodes = Vec::new();
    let mut edges: Vec<(NodeId, Edge)> = Vec::new();
    for section in crate::docs::sections(source, &stem) {
        let node = registry.get_or_mint(&qualify(&file, &section.title));
        nodes.push(node);
        registry.set_doc(node, section.prose.clone());

        for name in &section.mentions {
            let Some(target) = registry
                .node_of(name)
                .or_else(|| by_tail.get(name.as_str()).copied())
            else {
                continue;
            };
            if target == node {
                continue;
            }
            edges.push((
                node,
                Edge {
                    target,
                    timestamp: now,
                    // Precise — the document named the symbol outright — but
                    // from a source that describes intent rather than fact.
                    authority: crate::physics::SOURCE_DOC,
                    edge_kind: 0,
                    confidence: crate::csr::Confidence::Inferred,
                },
            ));
        }
    }

    let file_key = arena.intern(&file);
    let count = edges.len();
    graph.update(|snap: &GraphSnapshot, d: &mut DeltaStore| {
        let targets: HashMap<NodeId, Vec<NodeId>> = d
            .file_nodes(file_key)
            .iter()
            .map(|&n| (n, snap.base_targets(n)))
            .collect();
        d.replace_file_edges(
            file_key,
            |n| targets.get(&n).cloned().unwrap_or_default(),
            edges.clone(),
            &nodes,
        );
    });
    count
}

/// Attaches documentation to symbols a higher tier already defined.
///
/// Tier 1 resolves edges through the compiler and skips tier 2 for those files,
/// but a SCIP index carries no prose. This parses the file for documentation
/// only, touching no edges: the symbols already exist, so it looks them up
/// rather than minting, and a name the index did not define is left alone.
pub fn ingest_docs(registry: &mut SymbolRegistry, path: &Path, root: &Path, source: &str) {
    let facts = Lang::from_path(path).and_then(|lang| parse_ast::parse_file(path, source, lang));
    ingest_docs_from(registry, path, root, facts);
}

/// `ingest_docs` for facts someone else parsed — see `apply_facts`.
pub fn ingest_docs_from(
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: Option<parse_ast::FileFacts>,
) {
    let Some(facts) = facts else {
        return;
    };
    let file = relative_path(path, root);
    for (name, doc) in &facts.docs {
        if let Some(node) = registry.node_of(&qualify(&file, name)) {
            registry.set_doc(node, doc.clone());
        }
    }
    // Tier 1 supersedes tier 2 for edges but carries no byte ranges, so an
    // indexed tree would have no source to show without this — the same gap
    // the documentation had, for the same reason.
    for (name, span) in &facts.ranges {
        if let Some(node) = registry.node_of(&qualify(&file, name)) {
            registry.set_span(node, *span);
        }
    }
}

/// Links a call to the function it calls, and turns one changed file into
/// the edges that replace its previous ones.
pub fn ingest_file(
    graph: &Arc<Graph>,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    source: &str,
    now: u64,
) -> Option<usize> {
    let lang = Lang::from_path(path)?;
    let facts = parse_ast::parse_file(path, source, lang)?;
    Some(apply_facts(graph, arena, registry, path, root, facts, now))
}

/// The graph half of `ingest_file`, taking facts someone else parsed.
///
/// Split out so a whole tree can be parsed in parallel and folded in
/// afterwards: parsing is stateless per file and takes 63% of a cold start,
/// while this half touches the registry, the arena and the delta, all of which
/// are shared. Keeping the split at exactly this line is what lets the parallel
/// path stay bit-identical to the serial one — the order facts arrive here is
/// still `walk()`'s order, so every minted id is the same.
/// Everything `apply_facts` does before touching the delta: qualify the file's
/// symbols, attach their documentation, and resolve each callee.
///
/// Split out so the batch path can reuse it verbatim — the two must mint the
/// same ids in the same order, or a full build and a re-parse would disagree.
fn prepare_facts(
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) -> (u32, Vec<(NodeId, Edge)>, Vec<NodeId>) {
    // Relative to the indexed root: an absolute path bloats every symbol name
    // in a served answer and leaks where the tree happens to live.
    let file = relative_path(path, root);

    // Definitions first, so a call inside the file resolves to the local
    // definition rather than minting a placeholder for it.
    let mut nodes = Vec::with_capacity(facts.defines.len());
    for name in &facts.defines {
        nodes.push(registry.get_or_mint(&qualify(&file, name)));
    }
    for (name, doc) in &facts.docs {
        let node = registry.get_or_mint(&qualify(&file, name));
        registry.set_doc(node, doc.clone());
    }
    for (name, span) in &facts.ranges {
        let node = registry.get_or_mint(&qualify(&file, name));
        registry.set_span(node, *span);
    }

    let mut edges: Vec<(NodeId, Edge)> = Vec::with_capacity(facts.calls.len());
    for (caller, callee, has_receiver) in &facts.calls {
        let source_node = registry.get_or_mint(&qualify(&file, caller));
        let local = qualify(&file, callee);
        let target = if !*has_receiver && registry.contains(&local) {
            registry.get_or_mint(&local)
        } else {
            // Not defined here: a bare name, shared across files, so a later
            // file defining it lands on the same node. Which language reached
            // it is recorded beside the node, because tier 3 links by name
            // alone and a name means different things in different languages.
            let node = registry.get_or_mint(callee);
            if let Some(lang) = Lang::from_path(path) {
                registry.note_placeholder_lang(node, lang);
            }
            node
        };
        edges.push((
            source_node,
            Edge {
                target,
                timestamp: now,
                authority: crate::physics::SOURCE_CODE,
                edge_kind: 0,
                confidence: parse_ast::TIER_CONFIDENCE,
            },
        ));
    }

    let file_key = arena.intern(&file);
    (file_key, edges, nodes)
}

/// The delta mutation one parsed file makes: drop what the file owned, then
/// add what it now defines and calls.
///
/// Separate so a whole-tree build can run thousands of these against one
/// delta — see `Graph::update_batch`. The logic is identical either way; only
/// how often the snapshot is republished differs.
pub fn scope_file(
    snap: &GraphSnapshot,
    d: &mut DeltaStore,
    file_key: u32,
    edges: &[(NodeId, Edge)],
    nodes: &[NodeId],
) {
    let targets: HashMap<NodeId, Vec<NodeId>> = d
        .file_nodes(file_key)
        .iter()
        .map(|&n| (n, snap.base_targets(n)))
        .collect();
    d.replace_file_edges(
        file_key,
        |n| targets.get(&n).cloned().unwrap_or_default(),
        edges.to_vec(),
        nodes,
    );
}

/// `apply_facts` without publishing: the caller supplies the delta.
///
/// Used by `build_graph`, where nothing can read the graph yet, so one copy
/// for the whole tree replaces one per file.
#[allow(clippy::too_many_arguments)]
pub fn apply_facts_into(
    snap: &GraphSnapshot,
    d: &mut DeltaStore,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) {
    let (file_key, edges, nodes) = prepare_facts(arena, registry, path, root, facts, now);
    scope_file(snap, d, file_key, &edges, &nodes);
}

pub fn apply_facts(
    graph: &Arc<Graph>,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) -> usize {
    let (file_key, edges, nodes) = prepare_facts(arena, registry, path, root, facts, now);
    let count = edges.len();
    graph.update(|snap, d| scope_file(snap, d, file_key, &edges, &nodes));
    count
}
