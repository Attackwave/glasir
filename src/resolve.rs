//! Tier 3 of the cascade parser: symbol heuristics.
//!
//! Tier 2 emits a call to a name it cannot see a definition for as a bare
//! placeholder node. Tier 3 links those placeholders to real definitions once
//! the whole tree has been parsed, because a call is routinely seen before the
//! file defining it is.
//!
//! Everything linked here is `Confidence::Ambiguous`: the match is by name, so
//! two unrelated functions called `run` are indistinguishable. That flag is
//! what lets retrieval prefer a compiler-resolved edge over a guessed one, and
//! it is why an ambiguous name is left unlinked rather than resolved by
//! coin flip.
//!
//! Deliberately exact-match only. Measured on this repo, 35 of 151 unresolved
//! references share a name with a definition; the remaining 116 are calls into
//! other crates (`push`, `unwrap`, `HashMap::new`) that have no definition here
//! and must not acquire one. Edit-distance matching would invent edges for
//! exactly those.

use crate::csr::{Confidence, Edge, NodeId};
use crate::ingest::SymbolRegistry;
use std::collections::HashMap;

/// One placeholder resolved onto a definition.
#[derive(Debug, PartialEq)]
pub struct Link {
    pub placeholder: NodeId,
    pub definition: NodeId,
    /// Definitions that also carry this name. 1 means unique.
    pub candidates: usize,
}

/// The language a definition's file is written in.
fn lang_of(registry: &SymbolRegistry, node: NodeId) -> Option<crate::parse_ast::Lang> {
    use crate::parse_ast::LangExt;
    let name = registry
        .entries()
        .find(|(_, n)| **n == node)
        .map(|(s, _)| s)?;
    let (file, _) = name.split_once('#')?;
    crate::parse_ast::Lang::from_path(std::path::Path::new(file))
}

/// Files whose definitions should not be considered. Test files define names
/// that shadow production ones (`main`, `run`, `setup`), and linking a real
/// call onto a test definition is worse than leaving it unlinked.
fn is_test_path(file: &str) -> bool {
    file.contains("/test") || file.contains("test_") || file.contains("_test.")
}

/// Links a call to the function it calls when the parser could not tell:
/// an unqualified placeholder is matched to a definition of the same name.
///
/// A name defined in exactly one place is an unambiguous win. A name defined in
/// several is reported with its candidate count and left for the caller to
/// decide — see `link_edges`, which drops those rather than guessing.
pub fn resolve(registry: &SymbolRegistry) -> Vec<Link> {
    // Definition name -> nodes defining it. A name may be defined many times.
    let mut by_name: HashMap<&str, Vec<NodeId>> = HashMap::new();
    for (symbol, &node) in registry.entries() {
        if let Some((file, name)) = symbol.split_once('#')
            && !is_test_path(file)
        {
            by_name.entry(name).or_default().push(node);
        }
    }

    let mut links = Vec::new();
    for (symbol, &placeholder) in registry.entries() {
        // Unqualified symbols are the placeholders tier 2 minted.
        if symbol.contains('#') {
            continue;
        }
        let Some(defs) = by_name.get(symbol.as_str()) else {
            continue;
        };
        // A definition never resolves onto itself, and never across languages.
        //
        // A placeholder is a bare name: `error` from `console.error` in a `.js`
        // file and `error` from a Rust function are the same string and nothing
        // in the name says otherwise. Measured on this tree, that one pairing
        // produced an edge from `assets/check_view.js` to `src/mcp.rs`, and 18
        // of the 20 cycles `cycles` reported ran through it. SCIP avoids the
        // question by carrying the scheme in the symbol; the registry carries
        // it beside the node instead, so the name a caller sees is unchanged.
        //
        // A placeholder reached from two languages has no language, and then
        // nothing is refused: it is genuinely shared, and the pre-existing
        // behaviour is the safe one.
        let want = registry.placeholder_lang(placeholder);
        if let Some(&definition) = defs.iter().find(|&&d| {
            d != placeholder && want.is_none_or(|w| lang_of(registry, d).is_none_or(|l| l == w))
        }) {
            links.push(Link {
                placeholder,
                definition,
                candidates: defs.len(),
            });
        }
    }
    // Stable output: the registry is a HashMap, so iteration order is not.
    links.sort_by_key(|l| (l.placeholder, l.definition));
    links
}

/// Turns unambiguous links into edges, at `Confidence::Ambiguous`. This links
/// a call to its function when the parser could not tell the target directly.
///
/// A name with several definitions is skipped: picking one would be a coin
/// flip, and a wrong edge is worse than a missing one for a retrieval layer
/// that traverses these paths. Tier 1 is what will resolve those properly.
pub fn link_edges(links: &[Link], now: u64) -> Vec<(NodeId, Edge)> {
    links
        .iter()
        .filter(|l| l.candidates == 1)
        .map(|l| {
            (
                l.placeholder,
                Edge {
                    target: l.definition,
                    timestamp: now,
                    authority: crate::physics::SOURCE_CODE,
                    edge_kind: RESOLVES_TO,
                    confidence: Confidence::Ambiguous,
                },
            )
        })
        .collect()
}

/// Edge kind for a heuristic link, distinct from a call so retrieval can tell
/// "this is what that name probably refers to" from "this calls that".
pub const RESOLVES_TO: u16 = 1;
