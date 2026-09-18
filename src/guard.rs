//! Rules in a versioned file, checked against the graph, breaking the build
//! when broken. `bench/baseline.txt` guards recall the same way.
//!
//! Rules name paths. The first version named communities instead, which reads
//! better and is wrong: a community is *named* after the file holding most of
//! its members but contains others too, so `mcp.rs#name` sits in the one
//! called `lsp` and `deny mcp -> lsp` fired on an edge from mcp.rs to itself.
//! A path is unambiguous — every symbol carries it as a prefix.

use crate::csr::{Confidence, NodeId};
use crate::graph::GraphSnapshot;
use crate::ingest::SymbolRegistry;
use std::collections::{BTreeSet, HashMap, HashSet};

/// One line of the contract.
#[derive(Debug, PartialEq, Clone)]
pub enum Rule {
    /// `deny a -> b [confidence]`: no call from one path into another.
    Deny {
        from: String,
        to: String,
        floor: Confidence,
    },
    /// `no-cycles [confidence]`: no file may depend on itself, however
    /// indirectly.
    NoCycles(Confidence),
}

/// A rule that is broken, with enough detail to fix it.
pub struct Violation {
    pub rule: String,
    /// The concrete edges that break it — the point of the report. "mcp must
    /// not call lsp" is a verdict; "mcp.rs#handle calls lsp.rs#start" is
    /// something someone can act on.
    pub evidence: Vec<String>,
}

/// Reads the contract. An unparseable line is an error, not a skipped rule:
/// skipping one means it silently stops being enforced, and the gate then
/// passes while the code drifts — the one failure this must not have.
pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let rule = match f.as_slice() {
            // `inferred` by default for the reason `cycles` defaults higher:
            // tier 2 resolves by bare name, so two same-named functions
            // collapse onto one node. A false verdict costs more than a miss.
            ["deny", from, "->", to] => deny(from, to, Confidence::Inferred),
            ["deny", from, "->", to, c] => deny(from, to, confidence_from(c, n)?),
            // Without spaces around the arrow, because someone will write it
            // that way.
            ["deny", pair] if pair.contains("->") => {
                let (from, to) = pair.split_once("->").unwrap();
                deny(from.trim(), to.trim(), Confidence::Inferred)
            }
            ["no-cycles"] => Rule::NoCycles(Confidence::Extracted),
            ["no-cycles", c] => Rule::NoCycles(confidence_from(c, n)?),
            _ => {
                return Err(format!(
                    "line {}: cannot read `{line}`\n\
                     expected `deny <path> -> <path> [confidence]` \
                     or `no-cycles [confidence]`",
                    n + 1
                ));
            }
        };
        rules.push(rule);
    }
    Ok(rules)
}

fn deny(from: &str, to: &str, floor: Confidence) -> Rule {
    Rule::Deny {
        from: from.to_string(),
        to: to.to_string(),
        floor,
    }
}

fn confidence_from(word: &str, line: usize) -> Result<Confidence, String> {
    match word {
        "extracted" => Ok(Confidence::Extracted),
        "inferred" => Ok(Confidence::Inferred),
        "ambiguous" => Ok(Confidence::Ambiguous),
        other => Err(format!(
            "line {}: unknown confidence `{other}` (extracted, inferred or ambiguous)",
            line + 1
        )),
    }
}

/// Whether a rule's path covers a file: a prefix, so `src/graph` takes a whole
/// directory. The boundary check is why it is not a bare `starts_with` —
/// `src/graph` must not quietly take `src/graph_extra.rs`.
fn covers(pattern: &str, file: &str) -> bool {
    file == pattern
        || file
            .strip_prefix(pattern)
            .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('.'))
}

/// The file each defined symbol lives in.
fn files_of(names: &[String], defined: &HashSet<NodeId>) -> HashMap<NodeId, String> {
    let mut out = HashMap::new();
    for &node in defined {
        if let Some(name) = names.get(node as usize)
            && let Some((file, _)) = name.split_once('#')
        {
            out.insert(node, file.to_string());
        }
    }
    out
}

/// Checks every rule, returning what is broken. All of them run even after one
/// fails: stopping at the first turns a fix into a sequence of builds.
pub fn check(
    snap: &GraphSnapshot,
    registry: &SymbolRegistry,
    names: &[String],
    defined: &HashSet<NodeId>,
    rules: &[Rule],
) -> Vec<Violation> {
    let file = files_of(names, defined);
    // A call between two files routes through an unqualified middle node that
    // belongs to neither, so it stands for what it resolves to — but **only
    // when that is unambiguous**. A bare `name` or `stdin` is minted by every
    // caller writing it, so one node can stand for several definitions, and
    // choosing one invents a link that is not there.
    let mut stands_for: HashMap<NodeId, String> = HashMap::new();
    for (symbol, &node) in registry.entries() {
        if symbol.contains('#') || file.contains_key(&node) {
            continue;
        }
        let targets: BTreeSet<&String> = snap
            .neighbors(node)
            .filter_map(|e| file.get(&e.target))
            .collect();
        if let (1, Some(f)) = (targets.len(), targets.iter().next()) {
            stands_for.insert(node, (*f).clone());
        }
    }
    let name = |n: NodeId| names.get(n as usize).cloned().unwrap_or_default();

    let mut out = Vec::new();
    for rule in rules {
        match rule {
            Rule::Deny { from, to, floor } => {
                let mut evidence = Vec::new();
                for (&node, f) in &file {
                    if !covers(from, f) {
                        continue;
                    }
                    for e in snap.neighbors(node) {
                        // The call's own rating, not the resolving hop's.
                        // Taking the weaker of the two silenced every rule:
                        // `resolve.rs` rates *all* its links `Ambiguous`,
                        // including the ones it accepts as certain, so every
                        // link between two files fell under the floor. The
                        // single-target test above is what guards against a
                        // name collision, not this floor.
                        if e.confidence < *floor {
                            continue;
                        }
                        let Some(target_file) =
                            file.get(&e.target).or_else(|| stands_for.get(&e.target))
                        else {
                            continue;
                        };
                        // A rule whose two paths overlap must not fire on an
                        // edge that never leaves one file.
                        if !covers(to, target_file) || target_file == f {
                            continue;
                        }
                        let shown = name(e.target);
                        let shown = if shown.contains('#') {
                            shown
                        } else {
                            // Name the definition, not the node routed through:
                            // a bare `driver_start` says nothing about where.
                            snap.neighbors(e.target)
                                .map(|n| name(n.target))
                                .find(|t| t.contains('#'))
                                .unwrap_or(shown)
                        };
                        evidence.push(format!(
                            "{} -> {shown}  [{}]",
                            name(node),
                            confidence_name(e.confidence)
                        ));
                    }
                }
                if !evidence.is_empty() {
                    // Sorted and capped: hundreds of lines read as noise.
                    evidence.sort();
                    evidence.dedup();
                    evidence.truncate(10);
                    out.push(Violation {
                        rule: format!("deny {from} -> {to} {}", confidence_name(*floor)),
                        evidence,
                    });
                }
            }
            Rule::NoCycles(floor) => {
                if let Some(cycle) = first_cycle(snap, &file, &stands_for, *floor) {
                    out.push(Violation {
                        rule: format!("no-cycles {}", confidence_name(*floor)),
                        evidence: vec![cycle.join(" -> ")],
                    });
                }
            }
        }
    }
    out
}

fn confidence_name(c: Confidence) -> &'static str {
    match c {
        Confidence::Extracted => "extracted",
        Confidence::Inferred => "inferred",
        Confidence::Ambiguous => "ambiguous",
    }
}

/// One dependency cycle, or `None`. Only the first: a gate answers whether the
/// contract holds, and `cycles` is what lists them all, ranked.
fn first_cycle(
    snap: &GraphSnapshot,
    file: &HashMap<NodeId, String>,
    stands_for: &HashMap<NodeId, String>,
    floor: Confidence,
) -> Option<Vec<String>> {
    let mut files: Vec<&str> = file.values().map(String::as_str).collect();
    files.sort_unstable();
    files.dedup();
    let index: HashMap<&str, usize> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();

    let mut adj: Vec<BTreeSet<usize>> = vec![Default::default(); files.len()];
    for (&node, f) in file {
        let from = index[f.as_str()];
        for e in snap.neighbors(node) {
            if e.confidence < floor {
                continue;
            }
            if let Some(to_file) = file.get(&e.target).or_else(|| stands_for.get(&e.target))
                && let Some(&to) = index.get(to_file.as_str())
                && to != from
            {
                adj[from].insert(to);
            }
        }
    }

    // Never stepping onto a lower-numbered file, so a rotation cannot be found
    // twice and the first hit is stable rather than whichever came up first.
    let mut stack = Vec::new();
    let mut on_stack = vec![false; files.len()];
    for start in 0..files.len() {
        stack.push(start);
        on_stack[start] = true;
        if let Some(found) = dfs(start, start, &adj, &mut stack, &mut on_stack, 8) {
            return Some(found.iter().map(|&i| files[i].to_string()).collect());
        }
        on_stack[start] = false;
        stack.pop();
    }
    None
}

fn dfs(
    start: usize,
    node: usize,
    adj: &[BTreeSet<usize>],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    depth: usize,
) -> Option<Vec<usize>> {
    if stack.len() > depth {
        return None;
    }
    for &next in &adj[node] {
        if next == start {
            let mut cycle = stack.clone();
            cycle.push(start);
            return Some(cycle);
        }
        if next < start || on_stack[next] {
            continue;
        }
        stack.push(next);
        on_stack[next] = true;
        let found = dfs(start, next, adj, stack, on_stack, depth);
        on_stack[next] = false;
        stack.pop();
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Every file the graph knows, so a rule covering none can be reported: it
/// would otherwise match nothing and pass, which is the quietest way for a
/// contract to stop being one.
pub fn known_files(names: &[String], defined: &HashSet<NodeId>) -> BTreeSet<String> {
    files_of(names, defined).into_values().collect()
}

/// Whether any file the graph knows is covered by this path.
pub fn matches_any(pattern: &str, files: &BTreeSet<String>) -> bool {
    files.iter().any(|f| covers(pattern, f))
}
