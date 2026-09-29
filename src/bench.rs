//! Does the filtering make an answer better, or only smaller?
//!
//! The honest version of that question needs a ground truth, so `bench/
//! questions.txt` pairs each question with the symbols an answer must reach.
//! Recall is the headline: a smaller context that misses what you needed is
//! worse, not better. Token cost is the second column and means nothing on its
//! own — a tool that returns one node is maximally "efficient" and useless.
//!
//! **The baseline has to be the strong version of grep, or this measures
//! nothing.** It gets the same stemmed identifier words the graph's own search
//! uses, searches every source file, and ranks files by how many query words
//! they contain — an agent piping a question into `grep -r` verbatim would do
//! far worse, and beating that would prove only that the strawman was weak.
//! What it cannot do is follow an edge: that is the difference under test.
//!
//! Tokens are counted as chars/4, the standard approximation. It is rough, but
//! it is applied identically to both sides, and the ratio is what is claimed —
//! never the absolute number.

use crate::mcp::Served;
use crate::physics;
use crate::search::tokenize;
use std::collections::HashSet;
use std::path::Path;

/// Chars per token. Approximate by construction; both sides pay it equally.
const CHARS_PER_TOKEN: usize = 4;

/// How many files a grep-driven agent opens before it stops. Measured against
/// the alternative of reading everything: at 3 the baseline finds 6 of 24
/// symbols, at 5 it finds 12, at 8 it finds 15 and costs 4x the tokens. Five is
/// the point where a real agent's patience and its context budget meet.
const BASELINE_FILES: usize = 5;

#[derive(Clone)]
pub struct Question {
    pub text: String,
    pub expected: Vec<String>,
}

/// Parses the question file: a question line, then indented expected symbols.
pub fn load_questions(path: &Path) -> std::io::Result<Vec<Question>> {
    let raw = std::fs::read_to_string(path)?;
    let mut out: Vec<Question> = Vec::new();
    for line in raw.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(q) = out.last_mut() {
                q.expected.push(line.trim().to_string());
            }
        } else {
            out.push(Question {
                text: line.trim().to_string(),
                expected: Vec::new(),
            });
        }
    }
    Ok(out)
}

/// What one method returned for one question.
pub struct Answer {
    pub found: usize,
    pub returned: usize,
    pub tokens: usize,
}

impl Answer {
    fn recall(&self, expected: usize) -> f32 {
        if expected == 0 {
            return 0.0;
        }
        self.found as f32 / expected as f32
    }
    fn precision(&self) -> f32 {
        if self.returned == 0 {
            return 0.0;
        }
        self.found as f32 / self.returned as f32
    }
}

/// The graph's own answer: seed discovery, then filtered expansion.
///
/// Token cost is what `query_graph` actually puts in front of a model — one
/// line per node — not a hypothetical minimum.
pub fn graph_answer(served: &Served, q: &Question) -> Answer {
    let kept = physics::expand(
        served.snap,
        &served.seeds(&q.text),
        served.now,
        // The same budget `query_graph` uses, not the Physics default: this
        // must measure what a client actually receives.
        &physics::Physics {
            max_nodes: crate::mcp::DEFAULT_MAX_NODES,
            max_hops: crate::mcp::DEFAULT_MAX_HOPS,
            ..served.physics
        },
        Some(served.defined),
    );
    let names: Vec<String> = kept.iter().map(|s| served.name(s.node)).collect();
    let found = q
        .expected
        .iter()
        .filter(|e| names.iter().any(|n| n == *e))
        .count();
    let tokens = names.iter().map(|n| n.len() + 24).sum::<usize>() / CHARS_PER_TOKEN;
    Answer {
        found,
        returned: names.len(),
        tokens,
    }
}

/// What an agent with grep and read gets, done as well as grep can do it.
///
/// Ranks every source file by how many of the question's stemmed words appear
/// in it, opens the top `BASELINE_FILES`, and pays for their full text —
/// because reading a file is how that agent turns a grep hit into an answer.
/// Credit is given for every expected symbol defined in an opened file, which
/// is generous: it assumes the agent never misses a definition in a file it
/// read.
pub fn grep_answer(files: &[(String, String)], q: &Question) -> Answer {
    let words: Vec<String> = tokenize(&q.text);
    let mut ranked: Vec<(usize, &String, &String)> = files
        .iter()
        .map(|(path, body)| {
            let lower = body.to_lowercase();
            // Prefix matching, the same concession the graph's search makes:
            // "compacted" has to reach "compaction" on both sides or the
            // comparison is about stemming rather than about structure.
            let hits = words
                .iter()
                .filter(|w| {
                    let stem = &w[..w.len().min(5)];
                    lower.contains(stem)
                })
                .count();
            (hits, path, body)
        })
        .filter(|(hits, _, _)| *hits > 0)
        .collect();
    // Ties break on path, so the result does not depend on directory order.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    ranked.truncate(BASELINE_FILES);

    let opened: HashSet<&str> = ranked.iter().map(|(_, p, _)| p.as_str()).collect();
    let found = q
        .expected
        .iter()
        .filter(|e| {
            e.split_once('#')
                .is_some_and(|(file, _)| opened.contains(file))
        })
        .count();
    let tokens = ranked.iter().map(|(_, _, b)| b.len()).sum::<usize>() / CHARS_PER_TOKEN;
    Answer {
        found,
        // A read file is not a pointed answer: the agent still has to find the
        // symbol in it. Counting each opened file as one returned item is the
        // most generous reading available.
        returned: ranked.len(),
        tokens,
    }
}

/// Runs every question through both methods and prints the comparison.
/// Runs one question set and returns the graph's recall in percent.
///
/// The number is returned rather than only printed so `--check` can guard it:
/// a benchmark nobody compares against is decoration, and two silent
/// regressions in one session are what made that concrete.
/// The structural sets score through `structural_answer`; everything else
/// through `graph_answer`. One flag rather than two entry points, because the
/// printing, the baseline and the totals are identical either way.
pub fn run_structural(served: &Served, questions: &[Question], files: &[(String, String)]) -> f32 {
    run_with(served, questions, files, true)
}

pub fn run(served: &Served, questions: &[Question], files: &[(String, String)]) -> f32 {
    run_with(served, questions, files, false)
}

fn run_with(
    served: &Served,
    questions: &[Question],
    files: &[(String, String)],
    structural: bool,
) -> f32 {
    println!("{:<62} {:>11} {:>11}", "question", "glasir", "grep+read");
    println!("{}", "-".repeat(86));

    let (mut g_recall, mut b_recall) = (0.0f32, 0.0f32);
    let (mut g_prec, mut b_prec) = (0.0f32, 0.0f32);
    let (mut g_tok, mut b_tok) = (0usize, 0usize);
    let (mut g_ret, mut b_ret) = (0usize, 0usize);

    for q in questions {
        let g = if structural {
            structural_answer(served, q)
        } else {
            graph_answer(served, q)
        };
        let b = grep_answer(files, q);
        let n = q.expected.len();
        g_recall += g.recall(n);
        b_recall += b.recall(n);
        g_prec += g.precision();
        b_prec += b.precision();
        g_tok += g.tokens;
        b_tok += b.tokens;
        g_ret += g.returned;
        b_ret += b.returned;

        let short: String = q.text.chars().take(60).collect();
        println!(
            "{:<62} {:>4}/{:<6} {:>4}/{:<6}",
            short, g.found, n, b.found, n
        );
    }

    let k = questions.len() as f32;
    println!("{}", "-".repeat(86));
    println!(
        "recall        {:>28.0}% {:>10.0}%",
        100.0 * g_recall / k,
        100.0 * b_recall / k
    );
    println!(
        "nodes/question{:>28} {:>11}",
        g_ret / questions.len(),
        b_ret / questions.len()
    );
    println!(
        "precision     {:>28.0}% {:>10.0}%",
        100.0 * g_prec / k,
        100.0 * b_prec / k
    );
    println!(
        "tokens/question {:>26} {:>11}",
        g_tok / questions.len(),
        b_tok / questions.len()
    );
    if g_tok > 0 {
        println!("\n{:.1}x fewer tokens", b_tok as f32 / g_tok as f32);
    }
    100.0 * g_recall / k
}

/// The recall floors, read from `bench/baseline.txt`.
///
/// Scores the *structural* tools by calling them, not by expanding seeds.
///
/// `impact` and `shortest_path` resolve their own arguments and shape their own
/// answers, so `graph_answer` above — which is the `query_graph` path — cannot
/// score them. Nothing did, which meant the audit's five repairs to edge
/// construction were only ever measured through their side effect on
/// retrieval.
///
/// The question line is `<tool> <argument...>`; the answer is the tool's own
/// text, and a symbol counts as found when it appears in it. That is what a
/// client actually receives, including the confidence labels and the hop
/// grouping.
pub fn structural_answer(served: &Served, q: &Question) -> Answer {
    let mut parts = q.text.split_whitespace();
    let tool = parts.next().unwrap_or_default();
    let args: Vec<&str> = parts.collect();
    let arguments = match (tool, args.as_slice()) {
        ("impact", [symbol])
        | ("explain_node", [symbol])
        | ("find_callers", [symbol])
        | ("affected_tests", [symbol]) => serde_json::json!({ "symbol": symbol }),
        ("shortest_path", [from, to]) => serde_json::json!({ "from": from, "to": to }),
        _ => {
            return Answer {
                found: 0,
                returned: 0,
                tokens: 0,
            };
        }
    };
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let text = crate::mcp::handle_for_test(served, &msg)
        .and_then(|v| {
            v.get("result")?
                .get("content")?
                .get(0)?
                .get("text")?
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default();
    // An expectation may name a *section* of the answer with `in:`, because a
    // symbol often appears in several. `explain_node src/auth.rs#current` lists
    // `src/auth.rs#read` both as a subsystem sibling and as a callee, so a
    // plain substring test cannot tell whether the embeddings put it in
    // "structurally similar" — which is the only thing that reads them, and the
    // only place step 8b's repairs are visible.
    let section_of = |name: &str| -> &str {
        let Some(start) = text.find(name) else {
            return "";
        };
        let after = &text[start + name.len()..];
        let end = after
            .find("\n\n")
            .map_or(text.len(), |i| start + name.len() + i);
        &text[start..end]
    };
    // A tool error is a legitimate answer — `impact` refuses an ambiguous name
    // on purpose — and scores zero rather than crashing the run.
    let found = q
        .expected
        .iter()
        .filter(|e| match e.split_once(' ') {
            Some(("in:", rest)) => match rest.split_once(' ') {
                Some((section, symbol)) => section_of(&section.replace('_', " ")).contains(symbol),
                None => false,
            },
            _ => text.contains(e.as_str()),
        })
        .count();
    Answer {
        found,
        returned: text.lines().filter(|l| l.contains('#')).count(),
        tokens: text.len() / CHARS_PER_TOKEN,
    }
}

/// How well the partition answers the question `overview` exists for: can a
/// symbol be found through the subsystem named after its own file?
///
/// **The partition had no ground truth at all, and that is why a five-session
/// regression stayed invisible** — `query_graph` never reads it, so all four
/// recall sets are flat across every partition parameter. This reuses the
/// symbol ground truth that does exist: for each expected answer symbol, look
/// up the community it landed in, take that community's label the way
/// `overview` does (majority file), and ask whether the symbol's own file is
/// that label. A mislabelled symbol is the concrete failure — a reader sent to
/// the wrong file on the one question the tool is for.
///
/// Measured on this tree: 4/24 before the step 8a fix with 9 mislabelled,
/// 15/24 after with none.
pub fn overview_scale(served: &Served, questions: &[Question]) -> (usize, usize, usize) {
    let stem = |f: &str| {
        let s = f.rsplit('/').next().unwrap_or(f);
        s.rsplit_once('.').map_or(s, |(x, _)| x).to_string()
    };
    let file_of = |n: crate::csr::NodeId| -> Option<String> {
        served.name(n).split_once('#').map(|(f, _)| f.to_string())
    };
    // Group once, as `overview` does — per-community scans are quadratic.
    let mut by_c: std::collections::HashMap<u32, Vec<crate::csr::NodeId>> = Default::default();
    for (node, &cid) in served.communities.of_node.iter().enumerate() {
        let node = node as crate::csr::NodeId;
        if served.defined.contains(&node) {
            by_c.entry(cid).or_default().push(node);
        }
    }
    let label_of = |cid: u32| -> Option<String> {
        let members = by_c.get(&cid)?;
        if members.len() < 2 {
            return None;
        }
        let mut counts: std::collections::HashMap<String, usize> = Default::default();
        for &n in members {
            let Some(f) = file_of(n) else { continue };
            // Markdown clusters are not subsystems, exactly as in `overview`.
            if crate::docs::is_markdown(std::path::Path::new(&f)) {
                continue;
            }
            *counts.entry(stem(&f)).or_insert(0) += 1;
        }
        // Ties on name, so the label does not depend on hash order.
        counts
            .into_iter()
            .max_by_key(|(k, v)| (*v, std::cmp::Reverse(k.clone())))
            .map(|(k, _)| k)
    };

    // Code only: a Markdown section is never a subsystem member, so counting a
    // documentation answer here moved the floor with every question added to
    // a documentation set — two English and ten German ones took it 47% -> 44%
    // with the partition unchanged.
    let mut want: Vec<&str> = questions
        .iter()
        .flat_map(|q| q.expected.iter().map(String::as_str))
        .filter(|w| {
            let file = w.split_once('#').map_or(*w, |(f, _)| f);
            !crate::docs::is_markdown(std::path::Path::new(file))
        })
        .collect();
    want.sort_unstable();
    want.dedup();

    // `GLASIR_PARTITION_DEBUG` names the symbols behind the score. This floor
    // moves in whole symbols — 33 of them, so one is three points — and the
    // number alone cannot say which. It is what found `src/parse_ast.rs#parse`
    // as the single difference the native scanners make here.
    let (mut placed, mut wrong, mut absent) = (0, 0, 0);
    for w in want {
        let found = served.registry.node_of(w).and_then(|n| {
            let cid = *served.communities.of_node.get(n as usize)?;
            Some((n, label_of(cid)?))
        });
        match found {
            Some((n, label)) => {
                if file_of(n).map(|f| stem(&f)).as_deref() == Some(label.as_str()) {
                    placed += 1;
                } else {
                    if std::env::var("GLASIR_PARTITION_DEBUG").is_ok() {
                        eprintln!(
                            "  misplaced: {w} lives in {:?} but its subsystem is {label:?}",
                            file_of(n).map(|f| stem(&f))
                        );
                    }
                    wrong += 1;
                }
            }
            None => {
                if std::env::var("GLASIR_PARTITION_DEBUG").is_ok() {
                    eprintln!("  no subsystem: {w}");
                }
                absent += 1;
            }
        }
    }
    (placed, wrong, absent)
}

/// Kept in a file rather than in the code: a floor is a measured value, and
/// changing one should be a visible line in a diff with a reason beside it.
pub struct Baseline {
    pub floors: Vec<(String, f32)>,
    pub tolerance: f32,
}

pub fn load_baseline(path: &std::path::Path) -> std::io::Result<Baseline> {
    let text = std::fs::read_to_string(path)?;
    let mut floors = Vec::new();
    let mut tolerance = 3.0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(v) = value.trim().parse::<f32>() else {
            continue;
        };
        if key == "tolerance" {
            tolerance = v;
        } else {
            floors.push((key.to_string(), v));
        }
    }
    Ok(Baseline { floors, tolerance })
}
