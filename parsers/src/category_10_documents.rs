//! Category 10: Documents, Markup & Typesetting
//! Parsers for: Markdown, Typst

use crate::doc::clean_doc;
use crate::facts::FileFacts;

pub fn parse_markdown(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else if trimmed.starts_with('#') {
            let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
            if hash_count <= 6 {
                let title = trimmed[hash_count..].trim();
                if !title.is_empty() {
                    let sym = format!("h{}:{}", hash_count, title);
                    let start_byte = byte_offset + (line.find('#').unwrap_or(0));
                    let end_byte = start_byte + line.trim_end().len();
                    let doc = if !pending_comments.is_empty() {
                        let d = pending_comments.join(" ");
                        pending_comments.clear();
                        let cleaned = clean_doc(&d);
                        if cleaned.is_empty() { None } else { Some(cleaned) }
                    } else {
                        None
                    };
                    facts.add_definition(sym, (start_byte as u32, end_byte as u32), doc);
                }
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

pub fn parse_typst(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else if trimmed.starts_with('=') {
            let eq_count = trimmed.chars().take_while(|&c| c == '=').count();
            if eq_count <= 6 {
                let title = trimmed[eq_count..].trim();
                if !title.is_empty() {
                    let sym = format!("h{}:{}", eq_count, title);
                    let start_byte = byte_offset + (line.find('=').unwrap_or(0));
                    let end_byte = start_byte + line.trim_end().len();
                    let doc = if !pending_comments.is_empty() {
                        let d = pending_comments.join(" ");
                        pending_comments.clear();
                        let cleaned = clean_doc(&d);
                        if cleaned.is_empty() { None } else { Some(cleaned) }
                    } else {
                        None
                    };
                    facts.add_definition(sym, (start_byte as u32, end_byte as u32), doc);
                }
            }
        } else if let Some(stripped) = trimmed.strip_prefix("#let ") {
            let rest = stripped.trim();
            if let Some(eq_pos) = rest.find('=') {
                let var_name = rest[..eq_pos].trim().trim_end_matches('(');
                let start_byte = byte_offset + (line.find(var_name).unwrap_or(0));
                let end_byte = start_byte + var_name.len();
                let doc = if !pending_comments.is_empty() {
                    let d = pending_comments.join(" ");
                    pending_comments.clear();
                    let cleaned = clean_doc(&d);
                    if cleaned.is_empty() { None } else { Some(cleaned) }
                } else {
                    None
                };
                facts.add_definition(var_name, (start_byte as u32, end_byte as u32), doc);
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

/// The control sequences that *declare* rather than call. `\section` names a
/// section, `\newcommand` names a macro; neither is a use of something else.
const DECLARING: [&str; 16] = [
    "part", "chapter", "section", "subsection", "subsubsection", "paragraph",
    "subparagraph", "newcommand", "renewcommand", "def", "DeclareMathOperator",
    "newenvironment", "bibitem", "label", "begin", "end",
];

pub fn parse_latex(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut current: Option<String> = None;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('%') {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else {
            let mut detected_sym: Option<String> = None;
            for prefix in &[
                "\\part{",
                "\\chapter{",
                "\\section{",
                "\\subsection{",
                "\\subsubsection{",
                "\\paragraph{",
                "\\subparagraph{",
                "\\newcommand{\\",
                "\\newcommand*{\\",
                "\\renewcommand{\\",
                "\\def\\",
                "\\DeclareMathOperator{\\",
                "\\newenvironment{",
                "\\bibitem{",
                "\\label{",
            ] {
                if let Some(pos) = line.find(prefix) {
                    let after = &line[pos + prefix.len()..];
                    let end_delim = if *prefix == "\\def\\" {
                        after
                            .find(|c: char| !c.is_alphanumeric() && c != '_')
                            .unwrap_or(after.len())
                    } else {
                        after.find('}').unwrap_or(after.len())
                    };
                    let raw_name = after[..end_delim].trim();
                    if !raw_name.is_empty() {
                        let sym = if *prefix == "\\section{" {
                            format!("section:{}", raw_name)
                        } else if *prefix == "\\subsection{" {
                            format!("subsection:{}", raw_name)
                        } else if *prefix == "\\chapter{" {
                            format!("chapter:{}", raw_name)
                        } else {
                            raw_name.to_string()
                        };
                        detected_sym = Some(sym);
                        break;
                    }
                }
            }

            if let Some(sym) = detected_sym {
                let start_byte = byte_offset + (line.find('\\').unwrap_or(0));
                let end_byte = start_byte + line.trim_end().len();
                let doc = if !pending_comments.is_empty() {
                    let d = pending_comments.join(" ");
                    pending_comments.clear();
                    let cleaned = clean_doc(&d);
                    if cleaned.is_empty() {
                        None
                    } else {
                        Some(cleaned)
                    }
                } else {
                    None
                };
                facts.add_definition(sym.clone(), (start_byte as u32, end_byte as u32), doc);
                current = Some(sym);
            } else if !trimmed.is_empty() {
                pending_comments.clear();
            }

            // Every `\name` that is not one of the declaring forms above is a
            // use of a macro, which is this language's only kind of call.
            // Without this the scanner measured 750 definitions per 1,000
            // lines and **zero** edges: every macro looked defined and nothing
            // reached anything.
            let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
            let bytes = line.as_bytes();
            let mut at = 0usize;
            while let Some(rel) = line.get(at..).and_then(|r| r.find('\\')) {
                let start = at + rel + 1;
                // A trailing backslash puts `start` past the end, and a
                // multi-byte character can put it off a boundary: both panic
                // on a bare slice. Same class as the eleven crashes the corpus
                // sweep found, so the same rule applies — `get`, never an
                // index.
                let Some(rest) = line.get(start..) else { break };
                let end = start
                    + rest
                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                        .unwrap_or(rest.len());
                let Some(name) = line.get(start..end) else { break };
                at = end.max(start + 1);
                if name.is_empty() || DECLARING.contains(&name) {
                    continue;
                }
                // A declaration names itself; the macro it declares is not a
                // call to it.
                if bytes.get(start.saturating_sub(2)) == Some(&b'{') {
                    continue;
                }
                facts.add_call(owner.clone(), name.to_string(), false);
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}
