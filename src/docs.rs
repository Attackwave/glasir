//! Markdown as a graph source: sections become nodes, the code they name
//! becomes edges.
//!
//! The plan calls for non-code sources with an authority of their own, and this
//! is the first one. It is worth having for a measured reason: seed discovery
//! lives on prose — indexing doc comments took recall on plain-language
//! questions from 8% to 58% — and a design document is nothing but prose about
//! code. What it adds over comments is the vocabulary a *reader* uses, written
//! by someone explaining the system rather than annotating a function.
//!
//! Deliberately not tree-sitter. A Markdown grammar exists, but the whole job
//! here is headings and backticks; a regex-free hand scan is a few dozen lines
//! against a grammar plus a tags query plus the four `Lang` match arms every
//! new language costs. Nothing else in this file needs to understand Markdown.

/// One section of a document: a heading, its prose, and the code it names.
#[derive(Default)]
pub struct Section {
    /// Heading text, used as the symbol name after the file path.
    pub title: String,
    /// Everything under the heading until the next one of the same or higher
    /// level. This is what the search index reads.
    pub prose: String,
    /// Identifiers named in backticks, in order of appearance.
    pub mentions: Vec<String>,
}

/// Splits a Markdown document into its sections.
///
/// Prose before the first heading belongs to a section named after the file, so
/// a document with no headings still contributes rather than being dropped.
/// Words after which a section is split at the next paragraph break.
const MAX_SECTION_WORDS: usize = 250;

pub fn sections(source: &str, file_stem: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut current = Section {
        title: file_stem.to_string(),
        prose: String::new(),
        mentions: Vec::new(),
    };
    let mut in_fence = false;
    // Running word count for the current part, and which part this is.
    let mut words = 0usize;
    let mut part = 0usize;
    let mut base_title = file_stem.to_string();

    for line in source.lines() {
        let trimmed = line.trim_start();
        // A fenced block is code, not prose: indexing it would put the whole
        // example into the section's terms and drown the explanation around it.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if let Some(title) = heading(trimmed) {
            if !current.prose.trim().is_empty() || !current.mentions.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            base_title = title.clone();
            current = Section {
                title,
                prose: String::new(),
                mentions: Vec::new(),
            };
            words = 0;
            part = 0;
            continue;
        }

        // Split an overlong section at a paragraph break, so each node covers
        // one thought rather than a whole chapter. At a blank line, so a split
        // never lands mid-sentence.
        //
        // **Each part is numbered**, and that is not cosmetic: the title
        // becomes the symbol name, so parts sharing one collide on a single
        // node and `set_doc` keeps only the last — measured at 87% of the
        // largest section's prose silently discarded, which turned the split
        // into the data loss it was meant to prevent.
        if line.trim().is_empty() && words >= MAX_SECTION_WORDS {
            part += 1;
            out.push(std::mem::take(&mut current));
            current = Section {
                title: format!("{base_title} ({part})"),
                prose: String::new(),
                mentions: Vec::new(),
            };
            words = 0;
            continue;
        }

        current.mentions.extend(backticked(line));
        // Counted as we go rather than recounted at every blank line: the
        // recount walks all the prose collected so far, so a document with many
        // paragraphs costs O(blank lines x prose) — invisible on this tree and
        // quadratic on a generated one, which is the shape of all four scaling
        // walls phase A found.
        words += line.split_whitespace().count();
        current.prose.push_str(line);
        current.prose.push(' ');
    }
    if !current.prose.trim().is_empty() || !current.mentions.is_empty() {
        out.push(current);
    }
    out
}

/// The text of an ATX heading (`## Title`), or `None` for an ordinary line.
fn heading(line: &str) -> Option<String> {
    let rest = line.strip_prefix('#')?;
    let level = 1 + rest.chars().take_while(|&c| c == '#').count();
    // `#####` with nothing after it is not a heading, and neither is `#tag`.
    let text = line[level..].strip_prefix(' ')?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

/// Identifiers inside backticks, cleaned of the punctuation prose wraps them in.
///
/// Only backticks count. Bare words in a sentence would match half the
/// dictionary against a symbol table, and a document that merely uses the word
/// "graph" must not acquire an edge to every `graph`-named symbol.
fn backticked(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let inner = &after[..end];
        rest = &after[end + 1..];
        // `fn parse(src)` names `parse`; a whole signature is not a symbol.
        let name = inner
            .split(['(', '<', ' ', ':'])
            .next()
            .unwrap_or(inner)
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.');
        if name.len() > 2 && name.chars().next().is_some_and(|c| c.is_alphabetic()) {
            out.push(name.to_string());
        }
    }
    out
}

/// Documents about the code, rather than about working on it.
///
/// A local instruction file describes this repository's own decisions and
/// measurements in the vocabulary a question is asked in, so it outranks the
/// code it describes on nearly every query — measured here, indexing them cost
/// 16 points of prose recall and 18 of identifier recall, while a `docs/` tree
/// alone cost 8. They are also not documentation *of* the system: they are
/// instructions to whoever edits it, and an agent asking "what does this code
/// do" is not asking for them.
///
/// The rule is a name, not a pattern under `docs/`: the rest of that tree is
/// documentation of the system and measured as worth indexing.
const NOT_DOCUMENTATION: &[&str] = &["CONTRIBUTING.md", "CHANGELOG.md", "CODE_OF_CONDUCT.md"];

/// Whether a path is a document this module can read.
pub fn is_markdown(path: &std::path::Path) -> bool {
    if path
        .file_name()
        .and_then(|f| f.to_str())
        .is_some_and(|f| NOT_DOCUMENTATION.contains(&f))
    {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}
