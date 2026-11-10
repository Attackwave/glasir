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
use std::collections::{HashMap, HashSet, VecDeque};

/// One placeholder resolved onto a definition.
#[derive(Debug, PartialEq)]
pub struct Link {
    pub placeholder: NodeId,
    pub definition: NodeId,
    /// Definitions that also carry this name. 1 means unique.
    pub candidates: usize,
    /// Named by an import rather than matched by name alone.
    pub exact: bool,
}

/// The language a definition's file is written in.
fn lang_of(file_of: &HashMap<NodeId, &str>, node: NodeId) -> Option<crate::parse_ast::Lang> {
    use crate::parse_ast::LangExt;
    crate::parse_ast::Lang::from_path(std::path::Path::new(file_of.get(&node)?))
}

/// Files whose definitions should not be considered. Test files define names
/// that shadow production ones (`main`, `run`, `setup`), and linking a real
/// call onto a test definition is worse than leaving it unlinked.
///
/// Judged by path components: a substring test took `latest_x.rs` for a
/// test and missed `tests/` at the root.
fn is_test_path(file: &str) -> bool {
    let (dirs, name) = file.rsplit_once('/').unwrap_or(("", file));
    dirs.split('/').any(|d| {
        matches!(
            d,
            "test" | "tests" | "testing" | "testdata" | "__tests__" | "spec"
        )
    }) || name.starts_with("test_")
        || ["_test.", ".test.", ".spec."]
            .iter()
            .any(|m| name.contains(m))
}

/// Links a call to the function it calls when the parser could not tell:
/// an unqualified placeholder is matched to a definition of the same name.
///
/// A name defined in exactly one place is an unambiguous win. A name defined in
/// several is reported with its candidate count and left for the caller to
/// decide — see `link_edges`, which drops those rather than guessing.
/// The definition whose directory best ends in one of `packages`, `|/` joined.
/// `io/ktor/http` lies under `common/src/`, and an equal match decides nothing.
fn in_package(defs: &[NodeId], file_of: &HashMap<NodeId, &str>, packages: &str) -> Option<NodeId> {
    let matched = |d: NodeId| -> usize {
        let file = file_of.get(&d).copied().unwrap_or("");
        let parent = file.rsplit_once('/').map_or("", |(p, _)| p);
        packages
            .split("|/")
            .map(|package| {
                if package == parent {
                    return usize::MAX;
                }
                package
                    .rsplit('/')
                    .zip(parent.rsplit('/'))
                    .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
                    .count()
            })
            .max()
            .unwrap_or(0)
    };
    let best = defs.iter().map(|&d| matched(d)).max().filter(|&m| m > 0)?;
    let mut found = defs.iter().copied().filter(|&d| matched(d) == best);
    match (found.next(), found.next()) {
        (Some(d), None) => Some(d),
        _ => None,
    }
}

/// The one definition of a module. `A::BcD`, defined several times, is the one
/// in the file the convention names: `a/bc_d.rb`.
fn defining_module(
    by_name: &HashMap<&str, Vec<NodeId>>,
    file_of: &HashMap<NodeId, &str>,
    module: &str,
) -> Option<NodeId> {
    let (module, beside) = match module.split_once('^') {
        Some((module, dir)) => (module, Some(dir)),
        None => (module, None),
    };
    let last = module.rsplit("::").next()?;
    // A heading in the documentation names a class it is not.
    let code = |d: &NodeId| {
        file_of
            .get(d)
            .is_some_and(|f| !crate::docs::is_markdown(std::path::Path::new(f)))
    };
    let defs: Vec<NodeId> = by_name.get(last)?.iter().copied().filter(code).collect();
    let defs = &defs;
    if let Some(packages) = beside.and_then(|b| b.strip_prefix('/')) {
        return in_package(defs, file_of, packages);
    }
    if let [only] = defs[..] {
        return Some(only);
    }
    if let Some(dir) = beside {
        let in_dir = |d: &&NodeId| {
            file_of
                .get(d)
                .and_then(|f| f.rsplit_once('/'))
                .is_some_and(|(parent, _)| parent == dir)
        };
        let mut found = defs.iter().filter(in_dir);
        return match (found.next(), found.next()) {
            (Some(&d), None) => Some(d),
            _ => None,
        };
    }
    let mut tail = String::new();
    for segment in module.split("::") {
        tail.push('/');
        for (j, c) in segment.chars().enumerate() {
            if c.is_uppercase() && j > 0 {
                tail.push('_');
            }
            tail.extend(c.to_lowercase());
        }
    }
    tail.push_str(".rb");
    let mut found = defs
        .iter()
        .copied()
        .filter(|d| file_of.get(d).is_some_and(|f| f.ends_with(&tail)));
    match (module.contains("::"), found.next(), found.next()) {
        (true, Some(d), None) => Some(d),
        _ => None,
    }
}

/// A class's file may define the name elsewhere: `LIMIT.freeze()` is no call
/// into the file-level `freeze` beside the constant.
fn contains(registry: &SymbolRegistry, outer: NodeId, inner: NodeId) -> bool {
    match (registry.span(outer), registry.span(inner)) {
        (Some(o), Some(i)) => o.0 <= i.0 && i.1 <= o.1,
        _ => false,
    }
}

/// What `name` means in the classes `modules`, `+` joined: each class's own
/// definition, or the nearest one up its bases. The flag says every class on
/// the way was known, so that nothing else can hold the name.
fn inherited(
    registry: &SymbolRegistry,
    by_name: &HashMap<&str, Vec<NodeId>>,
    file_of: &HashMap<NodeId, &str>,
    modules: &str,
    name: &str,
) -> (Vec<NodeId>, bool) {
    // Past what any hierarchy reaches, and a class may name itself.
    const WIDEST: usize = 64;
    let mut waiting: VecDeque<&str> = modules.split('+').collect();
    let mut seen: HashSet<&str> = waiting.iter().copied().collect();
    let (mut found, mut declared) = (Vec::new(), Vec::new());
    let mut known = true;
    while let Some(class) = waiting.pop_front() {
        // A class several files define has no one chain to follow.
        let Some(module) = defining_module(by_name, file_of, class) else {
            known = false;
            continue;
        };
        let Some(file) = file_of.get(&module) else {
            continue;
        };
        let own = class.split('^').next().unwrap_or(class);
        let own = own.rsplit("::").next().unwrap_or(own);
        let directory = file.rsplit_once('/').map_or("", |(d, _)| d);
        let beside: &[String] = registry.method_files(directory, own, name);
        let here = match beside {
            [only] => registry.node_of(&format!("{only}#{name}")),
            _ => registry
                .node_of(&format!("{file}#{name}"))
                .filter(|&d| contains(registry, module, d)),
        };
        let symbol = format!("{file}#{own}");
        known &= !registry.is_mixin(&symbol);
        if let Some(definition) = here {
            let list = if registry.is_contract(&symbol) {
                &mut declared
            } else {
                &mut found
            };
            list.push(definition);
            continue;
        }
        for base in registry.bases_of(&symbol) {
            if seen.len() < WIDEST && seen.insert(base) {
                waiting.push_back(base);
            }
        }
    }
    if found.is_empty() {
        found = declared;
    }
    found.sort_unstable();
    found.dedup();
    (found, known)
}

pub fn resolve(registry: &SymbolRegistry) -> Vec<Link> {
    // Definition name -> nodes defining it. A name may be defined many times.
    let mut by_name: HashMap<&str, Vec<NodeId>> = HashMap::new();
    for (symbol, &node) in registry.entries() {
        // A workflow's jobs are named `build`, `test`, `publish`: reached by
        // `needs` in their own file, never by a bare name from code.
        if let Some((file, name)) = symbol.split_once('#')
            && !is_test_path(file)
            && !registry.is_extension(symbol)
            && !crate::workflow::is_workflow(std::path::Path::new(file))
        {
            by_name.entry(name).or_default().push(node);
        }
    }

    let file_of: HashMap<NodeId, &str> = registry
        .entries()
        .filter_map(|(s, &n)| s.split_once('#').map(|(f, _)| (n, f)))
        .collect();
    let impls = registry.methods_by_type();
    let mut links = Vec::new();
    for (symbol, &placeholder) in registry.entries() {
        // Unqualified symbols are the placeholders tier 2 minted.
        if symbol.contains('#') {
            continue;
        }
        // A placeholder an import scoped to a file or a Go package links to
        // the definition there. When the target does not define the name — a
        // re-export through an `__init__.py` or an `index.ts` — the bare-name
        // rule below still applies, with its refusal of an ambiguous name.
        // `=family` only says which callers share the node; the name rule
        // below decides, within that family.
        // `=go:dir` adds the package a Go call cannot leave; `=family.` is a
        // call through a value, which may reach a method anywhere.
        let family = crate::imports::unscope(symbol).and_then(|(_, t)| t.strip_prefix('='));
        let through_value = family.is_some_and(|t| t.ends_with('.') && !t.contains(':'));
        let package = family.and_then(|t| t.split_once(':')).map(|(_, dir)| dir);
        let (bare, target) = match crate::imports::unscope(symbol) {
            Some((name, target)) if target.starts_with('=') => (name, None),
            Some((name, target)) => (name, Some(target)),
            None => (symbol.as_str(), None),
        };
        // `~dirs`: a Rust name reaches only these crates, and an empty list
        // none. The crates only take away: a name defined once in the tree,
        // outside them, stays unlinked, and one defined twice stays refused
        // even when one of the two is out of reach — a local closure `run` is
        // no call of the crate's only `run`.
        if let Some(dirs) = target.and_then(|t| t.strip_prefix('~')) {
            // Rust definitions only: a crate reaches no Markdown heading or
            // YAML key, whatever its name.
            let defs: Vec<NodeId> = by_name
                .get(bare)
                .into_iter()
                .flatten()
                .copied()
                .filter(|d| *d != placeholder)
                .filter(|&d| lang_of(&file_of, d) == Some(crate::parse_ast::Lang::Rust))
                .collect();
            let found: Vec<NodeId> = defs
                .iter()
                .copied()
                .filter(|_| defs.len() == 1)
                .filter(|d| file_of.get(d).is_some_and(|file| in_crates(dirs, file)))
                .collect();
            if let [definition] = found[..] {
                links.push(Link {
                    placeholder,
                    definition,
                    candidates: 1,
                    exact: false,
                });
            }
            continue;
        }
        if let Some(target) = target {
            let mut chain_known = false;
            let found: Vec<NodeId> = if let Some(module) = target.strip_prefix('@') {
                let module = module.strip_prefix(['!', '?', '.']).unwrap_or(module);
                let (found, known) = inherited(registry, &by_name, &file_of, module, bare);
                chain_known = known;
                found
            } else if let Some(base) = target.strip_prefix("*/") {
                by_name
                    .get(bare)
                    .map(|defs| {
                        defs.iter()
                            .copied()
                            .filter(|&d| {
                                file_of
                                    .get(&d)
                                    .is_some_and(|file| file.rsplit('/').next() == Some(base))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else if let Some(dir) = target.strip_suffix('/') {
                by_name
                    .get(bare)
                    .map(|defs| {
                        defs.iter()
                            .copied()
                            .filter(|&d| {
                                file_of.get(&d).is_some_and(|file| {
                                    file.rsplit_once('/').map_or("", |(d, _)| d) == dir
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                registry
                    .node_of(&format!("{target}#{bare}"))
                    .into_iter()
                    .collect()
            };
            // The receiver's type has the method in an `impl` elsewhere.
            let found = match (found.is_empty(), target.strip_prefix("@.")) {
                (true, Some(types)) => types
                    .split('+')
                    .map(|t| t.split('^').next().unwrap_or(t))
                    .map(|t| t.rsplit("::").next().unwrap_or(t))
                    .flat_map(|t| impls.get(t).into_iter().flatten())
                    .flat_map(|names| names.get(bare).into_iter().flatten())
                    .filter_map(|file| registry.node_of(&format!("{file}#{bare}")))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                _ => found,
            };
            if let [definition] = found[..] {
                links.push(Link {
                    placeholder,
                    definition,
                    candidates: 1,
                    exact: true,
                });
                continue;
            }
            // `@.`, the receiver's type: strict as `@?` once its chain is known.
            if target.starts_with("@!")
                || ((target.starts_with("@?") || target.starts_with("@.")) && chain_known)
            {
                continue;
            }
            // `labels.get()` with `labels: HashMap`: the type is known and
            // none of it is in the tree, so no definition here is meant.
            if let Some(types) = target.strip_prefix("@.")
                && types.split('+').all(|t| {
                    let own = t.split('^').next().unwrap_or(t);
                    !by_name.contains_key(own.rsplit("::").next().unwrap_or(own))
                })
            {
                continue;
            }
        }
        let Some(defs) = by_name.get(bare) else {
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
        // A bare name is one placeholder per language family (`=family`), so
        // a call from Swift and one from a script never share an edge; within
        // a family, C calls C++. A definition in a file of no language (a
        // Markdown heading) is no call target.
        //
        // In Python, JavaScript and Go a call without a receiver reaches no
        // method, and a Go call no other package: those definitions are no
        // candidates at all, so a function beside a method of its name links.
        let want = registry.placeholder_langs(placeholder);
        // `@…` names a receiver's type: that call has one.
        let receiverless = !through_value
            && !target.is_some_and(|t| t.starts_with('@'))
            && !want.is_empty()
            && want.iter().all(|&w| crate::ingest::receiverless(w));
        let reachable = |d: NodeId| {
            let file = file_of.get(&d).copied().unwrap_or_default();
            !(receiverless && registry.is_member(&format!("{file}#{bare}")))
                && package.is_none_or(|dir| file.rsplit_once('/').map_or("", |(d, _)| d) == dir)
        };
        let unreachable = defs
            .iter()
            .filter(|&&d| d != placeholder && !reachable(d))
            .count();
        if let Some(&definition) = defs.iter().find(|&&d| {
            d != placeholder
                && reachable(d)
                && lang_of(&file_of, d).is_some_and(|l| {
                    let family = crate::ingest::family(l);
                    want.is_empty() || want.iter().any(|&w| crate::ingest::family(w) == family)
                })
        }) {
            // Assembly has no namespaces: a label is global to the unit the
            // assembler builds, and a unit defining it twice does not
            // assemble. One definition in the tree (`link_edges` keeps no
            // other) is therefore the label every call names, as decided as
            // an import would make it.
            // ponytail: tree-wide, not per INCLUDE unit; a call to an external
            // (`XREF`) label sharing a name with a local one is linked too.
            links.push(Link {
                placeholder,
                definition,
                candidates: defs.len() - unreachable,
                exact: want == [crate::parse_ast::Lang::Assembly],
            });
        }
    }
    // Stable output: the registry is a HashMap, so iteration order is not.
    links.sort_by_key(|l| (l.placeholder, l.definition));
    links
}

/// Turns the unambiguous links of unresolved placeholders into edges, at
/// `Confidence::Ambiguous`. This links a call to its function when the parser
/// could not tell the target directly.
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
                    // An import named the file: syntactic, like tier 2.
                    confidence: if l.exact {
                        Confidence::Inferred
                    } else {
                        Confidence::Ambiguous
                    },
                },
            )
        })
        .collect()
}

/// Edge kind for a heuristic link, distinct from a call so retrieval can tell
/// "this is what that name probably refers to" from "this calls that".
pub const RESOLVES_TO: u16 = 1;

/// Whether `file` lies in one of the `|`-separated crate dirs of a `~dirs`
/// placeholder.
pub(crate) fn in_crates(dirs: &str, file: &str) -> bool {
    dirs.split('|')
        .filter(|dir| !dir.is_empty())
        .any(|dir| dir == "." || file.strip_prefix(dir).is_some_and(|r| r.starts_with('/')))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_test_path_is_judged_by_its_components() {
        for test in [
            "tests/common/mod.rs",
            "src/test_checks.rs",
            "pkg/a_test.go",
            "web/a.spec.ts",
            "x/__tests__/a.js",
        ] {
            assert!(super::is_test_path(test), "{test}");
        }
        for code in ["src/latest_x.rs", "src/contest_entry.py", "src/attest.rs"] {
            assert!(!super::is_test_path(code), "{code}");
        }
    }
}
