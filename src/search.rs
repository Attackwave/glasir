//! Seed discovery: turning a question into the nodes to expand from.
//!
//! Two stages, in order of precision. An exact symbol name wins outright — if
//! someone names a symbol, they mean that symbol. Otherwise the query is scored
//! lexically against every symbol name with BM25 over identifier words, so
//! "compaction worker" reaches `spawn_compaction` without either word matching
//! the whole name.
//!
//! Deliberately not vector search. The embeddings in `embed.rs` encode graph
//! *structure* — their starting signature is a hash of the symbol id, not of
//! its text — so cosine distance says "these have similar neighbourhoods", not
//! "these mean similar things". Searching them with a natural-language query
//! would return confident nonsense. What a name carries as meaning is its
//! words, and BM25 is what reads those; the embeddings then rank what the graph
//! expansion found, which is the job they can actually do.

use crate::csr::NodeId;
use crate::ingest::SymbolRegistry;
use std::collections::HashMap;

/// Splits an identifier into lowercase words: `spawn_compaction` and
/// `spawnCompaction` and `SpawnCompaction` all yield ["spawn", "compaction"].
/// Paths contribute their segments too, so `src/graph.rs#update` carries
/// "graph" as well as "update".
/// Words that carry no information about code, so a question may not be ranked
/// by them.
///
/// Not a general English stop list: these are the words a *question about code*
/// is built from. The problem they cause is specific to this index — BM25's IDF
/// suppresses a term that is common in the corpus, but the corpus here is
/// identifiers and prose, where "how" is genuinely rare and so scores high.
/// Measured before this list: `how` had IDF 4.2 against `graph`'s 2.3, so the
/// question word outweighed the subject.
const STOP: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "from",
    "into",
    "how",
    "what",
    "when",
    "where",
    "why",
    "who",
    "which",
    "does",
    "did",
    "are",
    "is",
    "was",
    "were",
    "be",
    "been",
    "it",
    "its",
    "of",
    "to",
    "in",
    "on",
    "at",
    "by",
    "as",
    "an",
    "or",
    "if",
    "then",
    "than",
    "so",
    "not",
    "no",
    "can",
    "will",
    "would",
    "should",
    "could",
    "may",
    "might",
    "must",
    "has",
    "have",
    "had",
    "do",
    "get",
    "one",
    "per",
    "out",
    "up",
    "down",
    "back",
    "about",
    "any",
    "all",
    "some",
    "you",
    "your",
    // German. The same argument as above, and on paper it bites harder: in a
    // corpus of English identifiers a German filler word is *rarer* than an
    // English one, so it earns even higher IDF. Before this list, "wie sollte
    // der Index aktuell gehalten werden, während jemand tippt?" tokenized to
    // ten terms of which eight were filler, and ranked the roadmap sections
    // carrying the most German prose rather than anything about the subject.
    //
    // **Re-measured in audit step 8c, and it now carries 12 points:** without
    // the German half that set scores 44% against 56%. An earlier note here
    // recorded "changes no answer at all", measured before the step 4 and 5
    // repairs reshuffled the ranking; it is no longer true. Ablating the list
    // entirely gives 33/88/25/56, so the English half is worth 21 points on
    // code questions and 25 on English documentation.
    //
    // It is still not what stands between a German question and its answer.
    // What does is vocabulary: the question says "gespeichert", "Lesen",
    // "zerlegen" while the section says "Persistenz", "Memory-Mapping",
    // "Deserialisierungszeiten". This is a structural vocabulary gap inside
    // one language, not a defect in token normalization.
    //
    // Industry codebases are where this matters at all: measured across ten of
    // them, half carry 50-90% non-English comments. See C.4 in the roadmap.
    //
    // Deliberately omitted, each because it is an English content word this
    // index must keep: "war" and "bald", "kind" (edge_kind, EventKind), "so"
    // and "not" (already here for English), "um" and "an". "es" and "wo" are
    // omitted for a different reason — two letters, so `tokenize` drops them
    // before this list is ever consulted.
    "der",
    "die",
    "das",
    "den",
    "dem",
    "des",
    "ein",
    "eine",
    "einer",
    "eines",
    "einem",
    "einen",
    "und",
    "oder",
    "nicht",
    "kein",
    "keine",
    "wie",
    "wieso",
    "warum",
    "weshalb",
    "wann",
    "wer",
    "welche",
    "welcher",
    "welches",
    "wird",
    "werden",
    "wurde",
    "wurden",
    "sind",
    "waren",
    "ist",
    "hat",
    "haben",
    "hatte",
    "kann",
    "koennen",
    "können",
    "soll",
    "sollen",
    "sollte",
    "muss",
    "muessen",
    "müssen",
    "sich",
    "man",
    "jemand",
    "etwas",
    "bei",
    "mit",
    "von",
    "vom",
    "beim",
    "fuer",
    "für",
    "ueber",
    "über",
    "nach",
    "aus",
    "auf",
    "zum",
    "zur",
    "durch",
    "gegen",
    "ohne",
    "wenn",
    "dass",
    "damit",
    "weil",
    "diese",
    "dieser",
    "dieses",
    "auch",
    "schon",
    "noch",
    "nur",
    "wieder",
    "gibt",
    "sowie",
    "dabei",
    "dann",
    "also",
    "wobei",
    // Adverbs that qualify a question without narrowing it. `actually` is the
    // measured one: it is the only word separating "what does an agent reach
    // this system through" from the same question without it, and its stem
    // `actua` reaches `actual` in three unrelated doc comments — measured, that
    // one word cost 12 points of English documentation recall the moment
    // prose-only terms were allowed to widen.
    "actually",
    "really",
    "simply",
    "just",
    "exactly",
    "eigentlich",
    "wirklich",
    "genau",
    "einfach",
    "tatsaechlich",
    "tatsächlich",
];

/// Turns a question in plain words into the terms a symbol name is
/// searched by, dropping the words a question is built from.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    tokenize_into(text, &mut out);
    out
}

/// `tokenize` writing into a caller-owned buffer.
///
/// One allocation per call, 200k of them at 1M lines, for a result read at
/// once and dropped. With `stop_set`, 811 ms -> 480 ms.
pub fn tokenize_into(text: &str, out: &mut Vec<String>) {
    out.clear();
    let mut word = String::new();
    let mut prev_lower = false;

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            // A lower-to-upper transition starts a new word in camelCase, but
            // a run of capitals (HTTPServer) is one word until the last.
            if ch.is_uppercase() && prev_lower && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            word.push(ch.to_ascii_lowercase());
            prev_lower = ch.is_lowercase() || ch.is_numeric();
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
            prev_lower = false;
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    // Single letters are noise (a loop variable, a file extension's tail), and
    // so are the words a question is phrased with rather than about.
    // A set, not a linear scan of 166 entries per token. Measured at 1M lines:
    // see `tokenize_into`.
    let stop = stop_set();
    out.retain(|w| w.len() > 1 && !stop.contains(w.as_str()));
}

/// `STOP` as a set, built once per process.
fn stop_set() -> &'static std::collections::HashSet<&'static str> {
    static SET: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| STOP.iter().copied().collect())
}

/// How a question in plain words reaches a symbol name: a lexical index
/// over the words a name is built from.
pub struct SearchIndex {
    /// Word -> (node, times the word occurs in that node's name).
    postings: HashMap<String, Vec<(NodeId, u32)>>,
    /// Words reaching no code symbol at all — neither through a name nor
    /// through documentation attached to one.
    ///
    /// Such a word names nothing here however often it is written, so the stem
    /// fallback treats it as absent rather than as common. That is what lets a
    /// question in one language reach a symbol named in another: the German
    /// form names nothing, so its stem is allowed to look for one that does.
    /// `STEM_FALLBACK_DF` asks whether the evidence is thin, and on a
    /// bilingual tree the posting count answers the wrong question.
    ///
    /// Two earlier attempts failed and both are worth keeping. Counting symbol
    /// *names* cost 22 points across three sets, because a word like the one
    /// in "buffered writes" reaches its symbol through that symbol's
    /// documentation — so nearly every English prose word counted as naming
    /// nothing and widened its own query. Counting code reachability fixed
    /// that and still cost 12, because an adverb qualifying a question is
    /// prose-only too and its stem drags in whatever shares five letters. Such
    /// adverbs are in `STOP` now, which is the general form of the fix: a word
    /// that narrows nothing does not belong in the query at all.
    prose_only: std::collections::HashSet<String>,
    /// Word count per node, for BM25's length normalisation.
    lengths: HashMap<NodeId, u32>,
    average_length: f32,
    documents: usize,
}

/// BM25 constants. `k1` bounds how much a repeated word can help; `b` is how
/// strongly a long name is penalised.
///
/// `K1` is the standard default and was swept: 0.5 and 1.2 tie, and from 2
/// upward it loses on three sets.
///
/// **`B` is 0 — no length normalisation at all — and that is measured on two
/// shapes of tree rather than assumed.** Step 8c found it gained 8 points on
/// identifier-phrased questions and appeared to lose 8 on prose ones, and left
/// it at 0.75 because one tree could not decide the trade. With the coverage
/// exponent fixed in that same step and a deep-tree ground truth to measure
/// against, the trade is gone:
///
/// | `B` | questions | identifier | docs | docs-de | **deep** |
/// |---|---|---|---|---|---|
/// | **0.0** | 54% | **93%** | 50% | 56% | **78%** |
/// | 0.3 / 0.5 | 54% | 85% | 50% | 56% | 72% |
/// | 0.75 (was) | 54% | 88% | 50% | 56% | 72% |
///
/// It wins on both shapes and loses on none. The reason it should: a symbol's
/// "length" here is its name plus its documentation, and documentation length
/// says nothing about relevance — penalising it is penalising a symbol for
/// being thorough. On a deep tree it is worse than useless, because the path
/// makes every name long in the same way.
const K1: f32 = 1.2;
/// How much a half of a split compound counts against the whole term.
const SPLIT_WEIGHT: f32 = 0.5;

/// How many postings a word may have and still be treated as too thin to be
/// the answer's home, so the prefix stem is consulted beside it. A word naming
/// something in this tree appears in that thing and in its callers; a word
/// appearing in three scattered comments names nothing, and the noun form of
/// a question's verb is routinely that case while the verb names the symbol.
const STEM_FALLBACK_DF: usize = 3;
const B: f32 = 0.0;

/// Term counts are integers, so a fractional doc weight needs a scale to
/// survive rounding: 0.03 of one occurrence is 0 without it.
///
/// 100, not more: at 1000 recall drops (43% -> 39% on prose, 93% -> 89% on
/// identifiers), because the coarseness is doing useful work. A word appearing
/// once in prose rounds to 3 rather than 30, so weak incidental matches stay
/// weak instead of accumulating into a rank.
const DOC_SCALE: u32 = 100;

/// How much a word found in documentation counts against one in the name.
///
/// Measured across the range, on both question sets, after body comments were
/// folded in — and the curve has a clear peak that is nowhere near where
/// intuition put it:
///
/// | weight | prose recall | identifier recall |
/// |--------|--------------|-------------------|
/// | 0.01   | 35%          | 93%               |
/// | 0.03   | **43%**      | **93%**           |
/// | 0.05   | 39%          | 93%               |
/// | 0.15   | 35%          | 86%               |
/// | 0.35   | 35%          | 82%               |
///
/// The first version of this used 0.35, reasoned from "prose must not swamp
/// the name". That is true but far too weak a bound: a definition's comments
/// run to hundreds of words against an identifier's two or three, so even a
/// third of a word each buries the name. At 1.0 the failure is visible in
/// `demo_doc_ranking` — a function merely *mentioning* `parse` outranks the
/// one called `parse`. What documentation is good for is separating candidates
/// the name cannot; giving it any more say costs on both axes at once.
const DOC_WEIGHT: f32 = 0.03;

/// The first `n` *characters* of a word, or `None` if it is shorter.
///
/// Characters, not bytes. `&word[..5]` panics when the fifth byte lands
/// inside a multi-byte character, which is the normal case for German —
/// `schlüsselverwaltung`, `ausführung`, `größenordnung` all break there, and
/// the panic took the MCP server down with the question. Found by an audit
/// after this session added German stop words and thereby invited exactly
/// those questions. `split_compound` below already did this correctly; the
/// lesson was learned in one place and not applied in the other.
fn prefix(word: &str, n: usize) -> Option<&str> {
    let end = word.char_indices().nth(n).map(|(i, _)| i)?;
    Some(&word[..end])
}

/// Folds the spelling differences that separate a German word from the English
/// one it means: German `k` and `z` both map to `c`, and that is the whole
/// list. It applies to both sides of the comparison and never to a stored
/// posting, so the index does not change shape. The measurements, and why the
/// example words are not written here, are in CLAUDE.md under "Closing the
/// language gap".
///
/// **It runs only when the plain stem reaches nothing.** Unconditionally it
/// costs four points on `questions`, as demonstrated by the `prose_only`
/// regression cases.
fn fold_spelling(stem: &str) -> String {
    stem.chars()
        .map(|c| match c {
            'k' => 'c',
            'z' => 'c',
            _ => c,
        })
        .collect()
}

impl SearchIndex {
    /// Builds from names alone. `build_with_docs` is the same index with
    /// documentation folded in; this stays for callers that have none.
    pub fn build(registry: &SymbolRegistry) -> SearchIndex {
        SearchIndex::build_with_docs(registry, &HashMap::new())
    }

    /// Names plus the prose written above each definition.
    ///
    /// Measured: names alone recall 8% of the expected symbols on questions
    /// phrased as a person asks them (`bench/questions.txt`) — what a symbol is
    /// *for* is in its documentation, and a question is asked in those words.
    pub fn build_with_docs(
        registry: &SymbolRegistry,
        docs: &HashMap<NodeId, String>,
    ) -> SearchIndex {
        // Index construction mutates a few tightly coupled maps. Keeping this
        // fold serial avoids a general-purpose work-stealing dependency and
        // retains deterministic memory bounds; parsing and numerical passes
        // remain bounded-parallel where their work is naturally disjoint.
        let (postings, lengths, total, _, _, coded) = registry.entries().fold(
            (
                HashMap::<String, Vec<(NodeId, u32)>>::new(),
                HashMap::new(),
                0u64,
                Vec::<String>::new(),
                Vec::<String>::new(),
                std::collections::HashSet::<String>::new(),
            ),
            |(mut postings, mut lengths, mut total, mut words, mut doc_words, mut coded),
             (symbol, &node)| {
                tokenize_into(symbol, &mut words);
                if words.is_empty() {
                    return (postings, lengths, total, words, doc_words, coded);
                }
                let is_code = !symbol.contains(".md#") && !symbol.contains(".markdown#");
                let mut counts: HashMap<&str, u32> = HashMap::new();
                for w in &words {
                    *counts.entry(w.as_str()).or_insert(0) += 1;
                }
                match docs.get(&node) {
                    Some(d) => tokenize_into(d, &mut doc_words),
                    None => doc_words.clear(),
                }
                let mut doc_counts: HashMap<&str, u32> = HashMap::new();
                for w in &doc_words {
                    *doc_counts.entry(w.as_str()).or_insert(0) += 1;
                }
                for (word, n) in &counts {
                    postings
                        .entry((*word).to_string())
                        .or_default()
                        .push((node, n * DOC_SCALE));
                }
                for (word, n) in doc_counts {
                    // A word already in the name keeps its full weight
                    // rather than gaining from being repeated in the prose
                    // below it. `counts` is that set, and asking it is
                    // O(1) — scanning the posting list for the node
                    // instead was O(postings per word), and a common
                    // word's list is as long as the symbol table: 28 s to
                    // build the index for a 1M-line tree, against 0.4 s
                    // here.
                    if counts.contains_key(word) {
                        continue;
                    }
                    postings
                        .entry(word.to_string())
                        .or_default()
                        .push((node, (n as f32 * DOC_WEIGHT * DOC_SCALE as f32) as u32));
                }
                // Length normalisation counts documentation at its weight
                // too, or a well-documented symbol would look artificially
                // long and BM25 would penalise it for being thorough.
                //
                // Scaled by `DOC_SCALE` like the frequencies are: `len` and
                // `freq` meet in the same formula, so they must be in the
                // same unit. It changes no measured score today — `K1` is
                // 1.2 against frequencies in hundredths, so `tf` saturates
                // at `K1 + 1` regardless of length — and that saturation is
                // load-bearing: raising `K1` to match the scale was
                // measured and costs far more than it returns (prose
                // 50% -> 38%, identifier 93% -> 88%, docs 56% -> 69%).
                // Left consistent so the next person to touch `DOC_SCALE`
                // does not inherit a length a hundred times too small.
                let length =
                    (words.len() as f32 + doc_words.len() as f32 * DOC_WEIGHT) * DOC_SCALE as f32;
                lengths.insert(node, length as u32);
                total += length as u64;
                if is_code {
                    for w in words.iter().chain(doc_words.iter()) {
                        if !coded.contains(w.as_str()) {
                            coded.insert(w.clone());
                        }
                    }
                }
                (postings, lengths, total, words, doc_words, coded)
            },
        );
        let mut postings = postings;

        // Postings are built from a HashMap, so sort them: ranking must not
        // depend on iteration order.
        for list in postings.values_mut() {
            list.sort_unstable();
        }
        let documents = lengths.len();
        let prose_only = postings
            .keys()
            .filter(|w| !coded.contains(w.as_str()))
            .cloned()
            .collect();
        SearchIndex {
            postings,
            prose_only,
            lengths,
            average_length: if documents == 0 {
                0.0
            } else {
                total as f32 / documents as f32
            },
            documents,
        }
    }

    /// The posting lists, for the self-check that asserts they are sorted.
    pub fn postings_for_test(&self) -> impl Iterator<Item = (&String, &Vec<(NodeId, u32)>)> {
        self.postings.iter()
    }

    /// How many symbols carry a word. A term in half the tree distinguishes
    /// nothing, which is what makes it useless as a suggestion.
    pub fn document_frequency(&self, word: &str) -> usize {
        self.postings.get(word).map_or(0, |l| l.len())
    }

    /// Best-matching nodes for a query, highest score first.
    /// Splits a compound into halves that are themselves indexed terms.
    ///
    /// No dictionary: the corpus is the dictionary, which is what keeps this
    /// deterministic and dependency-free. Both halves must be known terms and
    /// at least `MIN_PART` long, so `Basisgraph` yields `basis` + `graph` while
    /// `builder` is left alone — `build` is a term but `er` is not.
    ///
    /// Only the first valid split is taken, longest prefix first: a compound
    /// with several readings is exactly where over-splitting starts.
    fn split_compound(&self, term: &str) -> Vec<String> {
        /// Shortest half worth producing. Below this a fragment matches too
        /// much to carry meaning.
        const MIN_PART: usize = 4;
        if term.len() < MIN_PART * 2 {
            return Vec::new();
        }
        let known = |w: &str| self.postings.contains_key(w);
        // Byte indices only at character boundaries, or a split inside a
        // multi-byte character panics — and German is exactly where those are.
        let bounds: Vec<usize> = term
            .char_indices()
            .map(|(i, _)| i)
            .filter(|&i| i >= MIN_PART && term.len() - i >= MIN_PART)
            .collect();
        for i in bounds.into_iter().rev() {
            let (a, b) = term.split_at(i);
            if known(a) && known(b) {
                return vec![a.to_string(), b.to_string()];
            }
        }
        Vec::new()
    }

    /// How many distinct *query terms* a node matched, which is the number the
    /// coverage factor multiplies by. Exposed because the count is what a
    /// per-list double-count changes: with the stem fallback one term reaches
    /// several posting lists, and scores alone can absorb the error without
    /// reordering anything on a small fixture.
    /// How many distinct query *terms* a node matched.
    ///
    /// Read by `vocabulary` to tell a partial miss from an answer: a question
    /// whose best hit covers one term out of several has not been answered,
    /// however confident that hit's score looks.
    pub fn coverage_of(&self, query: &str, node: NodeId) -> usize {
        self.search_scored(query, usize::MAX)
            .1
            .get(&node)
            .copied()
            .unwrap_or(0)
    }

    pub fn coverage_for_test(&self, query: &str, node: NodeId) -> usize {
        self.search_scored(query, usize::MAX)
            .1
            .get(&node)
            .copied()
            .unwrap_or(0)
    }

    /// Exposed for `glasir why`: what a term splits into is the first thing to
    /// look at when a split hurts rather than helps.
    pub fn split_for_test(&self, term: &str) -> Vec<String> {
        self.split_compound(term)
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<(NodeId, f32)> {
        self.search_scored(query, limit).0
    }

    /// The ranking plus the per-node query coverage behind it. One body rather
    /// than two: a copied scoring loop drifts from the one `search` runs, and
    /// then two things claim to measure the same property while only one does.
    fn search_scored(
        &self,
        query: &str,
        limit: usize,
    ) -> (Vec<(NodeId, f32)>, HashMap<NodeId, usize>) {
        let terms = tokenize(query);
        if terms.is_empty() || self.documents == 0 {
            return (Vec::new(), HashMap::new());
        }

        // Shared prefix length at which two identifier words are treated as
        // the same term. `compacted` must reach `compaction` and `parsed` must
        // reach `parse`, or a question phrased in the wrong tense finds
        // nothing — and code is full of such pairs. A real stemmer would be
        // more precise, but this needs no dictionary and no dependency.
        const STEM: usize = 5;

        // Compound splitting, against the index's own vocabulary rather than a
        // dictionary. German writes `Basisgraph` where the documentation says
        // `Graph`, and the prefix stemmer cannot reach it because the shared
        // part is at the *end*.
        //
        // **Measured: it changes no answer on any of the four question sets.**
        // The literature reports ~25% for German retrieval, and this tree does
        // not see it — of the German questions, only two carry a real compound
        // and both were already answered. Kept because the guards below make it
        // free and the case is real in a German codebase; not kept as a
        // measured win, because it is not one here.
        //
        // Three guards, each from a measurement rather than caution:
        //  - only terms the index does not already know: splitting `viewport`
        //    into `view` + `port` cost 4 points on the identifier set on its
        //    own, and it is the *correct* split — a real name must never be
        //    pulled apart.
        //  - both halves must themselves be indexed and at least `MIN_PART`
        //    long, since over-splitting manufactures sub-terms that lower IDF.
        //  - a half is worth `SPLIT_WEIGHT` and does not count towards query
        //    coverage: `hits * hits` below would otherwise turn one match into
        //    three. Damping the score alone did nothing — measured at three
        //    weights, all identical — because the coverage factor was carrying
        //    the damage.
        // A split half is weaker evidence than the term as written: `viewport`
        // means the viewport, `view` and `port` merely occur inside it.
        let mut weighted: Vec<(String, f32)> = terms.iter().map(|t| (t.clone(), 1.0)).collect();
        for t in &terms {
            // Only when the term as written is not itself indexed. `viewport`
            // is a real name here, so splitting it into `view` + `port` can
            // only pull the query away from what it asked for — measured, that
            // single split cost 4 points on the identifier set while the German
            // set gained nothing.
            if self.postings.contains_key(t.as_str()) {
                continue;
            }
            for part in self.split_compound(t) {
                weighted.push((part, SPLIT_WEIGHT));
            }
        }
        let terms: Vec<String> = weighted.iter().map(|(t, _)| t.clone()).collect();
        let term_weight: HashMap<&str, f32> =
            weighted.iter().map(|(t, w)| (t.as_str(), *w)).collect();

        let mut scores: HashMap<NodeId, f32> = HashMap::new();
        // How many distinct query terms each node matched. A name hitting two
        // of them is about the query; one hitting a single common word is
        // usually a coincidence, and BM25 alone cannot separate the two because
        // it sums independent per-term scores.
        let mut matched: HashMap<NodeId, usize> = HashMap::new();
        for term in &terms {
            // Exact match first, then anything sharing a long enough prefix.
            //
            // **An exact posting suppresses the stem only when it is big
            // enough to be a real home for the word.** `else if` alone cost a
            // whole answer: a question's noun form occurred in exactly three
            // comments here — a module header, a hub remark, a fixture note —
            // none attached to the function of that name, so the exact branch
            // matched those three, the stem never ran, and the symbol the
            // question is *about* scored nothing at all. It returned 0/1 while
            // the bare verb ranks it at 10.1.
            //
            // The example words are deliberately absent: naming them here
            // makes this comment a posting for them and moves the very count
            // the threshold tests. That is not hypothetical — it happened on
            // the first attempt, taking the word from three postings to four
            // and past the bound. Seventh instance of a comment displacing the
            // code it describes; the regression test protects this boundary.
            //
            // Widening every term instead is measured and worse: the stem
            // sweep changes each term's document frequency, so `origin
            // validation`, `resolve unresolved placeholders` and `search
            // tokenize identifier` each lose a hit — 85% -> 81% here and
            // 86% -> 79% on the prose set. Damping the relatives does not
            // rescue it, because the damage is in the IDF and not in the
            // scores: at every weight from 0.5 down to 0.05 the English
            // documentation set reads 62% against 75%, losing the
            // `plan.md#3.1` answer. The threshold is what keeps the widening
            // where the evidence is thin.
            let mut lists: Vec<&Vec<(NodeId, u32)>> = Vec::new();
            let exact = self.postings.get(term);
            if let Some(list) = exact {
                lists.push(list);
            }
            // A term naming nothing here may still widen: see `prose_only`.
            let prose_only = self.prose_only.contains(term.as_str());
            if (prose_only || exact.is_none_or(|l| l.len() <= STEM_FALLBACK_DF))
                && let Some(stem) = prefix(term, STEM)
            {
                let mut related: Vec<(&String, &Vec<(NodeId, u32)>)> = self
                    .postings
                    .iter()
                    // `starts_with` as before, not a prefix comparison: a
                    // posting shorter than the stem cannot start with it
                    // anyway, and comparing extracted prefixes silently
                    // narrowed the match — measured, 6 points off the English
                    // documentation set.
                    .filter(|(w, _)| w.starts_with(stem))
                    .collect();
                // Last resort only: see `fold_spelling`.
                if related.is_empty() {
                    let folded = fold_spelling(stem);
                    related = self
                        .postings
                        .iter()
                        .filter(|(w, _)| {
                            prefix(w, STEM).is_some_and(|p| fold_spelling(p) == folded)
                        })
                        .collect();
                }
                // The postings map is a HashMap, so fix an order.
                related.sort_by_key(|(w, _)| w.as_str());
                lists.extend(
                    related
                        .into_iter()
                        // Already pushed above; twice would double its term
                        // frequency and rank an exact hit above itself.
                        .filter(|(w, _)| w.as_str() != term.as_str())
                        .map(|(_, l)| l),
                );
            }

            // Query coverage is per *term*, not per posting list. Since the
            // stem fallback gives one term several lists, a node appearing in
            // two of them would otherwise count as having matched two query
            // words when it matched one — the same double-count the split-half
            // rule below exists to prevent.
            let mut counted: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
            for list in lists {
                // Inverse document frequency: a word in every name says nothing,
                // a rare one says a lot.
                let df = list.len() as f32;
                let idf = ((self.documents as f32 - df + 0.5) / (df + 0.5) + 1.0).ln();

                let tw = term_weight.get(term.as_str()).copied().unwrap_or(1.0);
                for &(node, freq) in list {
                    let len = self.lengths.get(&node).copied().unwrap_or(1) as f32;
                    let norm = 1.0 - B + B * len / self.average_length.max(1.0);
                    let tf = freq as f32 * (K1 + 1.0) / (freq as f32 + K1 * norm);
                    *scores.entry(node).or_insert(0.0) += idf * tf * tw;
                    // A split half does not count towards query coverage: it is
                    // the same term twice, and `hits * hits` below would turn
                    // one match into three and reward it ninefold. This is why
                    // damping the score alone did nothing — measured at three
                    // weights, all identical.
                    if tw >= 1.0 && counted.insert(node) {
                        *matched.entry(node).or_insert(0) += 1;
                    }
                }
            }
        }

        // Reward matching more of what was asked, linearly.
        //
        // **It was squared, reasoned rather than measured, and that is one
        // exponent too far:** 1 gives 54/88/50/56 against 2's 54/85/50/56, with
        // 0.5 to 1.5 a flat plateau and 3 losing on three sets. The damage at 2
        // is not a reordering but a *spread* — `seeds` normalises against the
        // best hit, so squaring pushes everything below it down; the four
        // `snapshot.rs` symbols entered at 0.34-0.42 instead of 0.51-0.63, in
        // the same order, and still fell outside the node budget.
        //
        // Removing the factor outright measures 50/88/**62**/56: it wins 12
        // points on the English prose set and loses 4 on the code one, which is
        // a trade rather than a win. Kept at 1.
        for (node, score) in scores.iter_mut() {
            let hits = matched.get(node).copied().unwrap_or(1) as f32;
            *score *= hits;
        }

        let mut out: Vec<(NodeId, f32)> = scores.into_iter().collect();
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                // Node id breaks ties so the same query always ranks the same.
                .then_with(|| a.0.cmp(&b.0))
        });
        out.truncate(limit);
        (out, matched)
    }
}
