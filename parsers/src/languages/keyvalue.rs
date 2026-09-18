//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! A settings file declares by `key = value`, and its structure is the key
//! path: `ledger.charge.limit` says charge belongs to ledger. There are no
//! calls, so the edges are what the dotted path and the `[section]` header
//! express — which is the only thing in such a file worth asking about.
//!
//! One module for `.properties`, `.ini` and `.env`, since the three differ
//! only in whether a section header exists and which comment marker they use,
//! and both of those already come from the rule file.

use crate::facts::FileFacts;

pub(crate) fn parse(src: &str, style: crate::lexer::CommentStyle<'_>, _calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut section: Option<String> = None;
    let mut pending: Vec<String> = Vec::new();
    let mut offset = 0usize;

    for line in src.lines() {
        let trimmed = line.trim();
        let start = offset;
        offset += line.len() + 1;

        if trimmed.is_empty() {
            pending.clear();
            continue;
        }
        if let Some(text) = style
            .line_comment_prefix
            .iter()
            .find_map(|p| trimmed.strip_prefix(*p))
        {
            pending.push(text.trim().to_string());
            continue;
        }

        let doc = if pending.is_empty() {
            None
        } else {
            let joined = pending.join(" ");
            pending.clear();
            Some(joined)
        };

        // `[section]` opens a scope the keys below it belong to.
        if let Some(name) = trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            let name = name.trim();
            if !name.is_empty() {
                facts.add_definition(name, (start as u32, (start + line.len()) as u32), doc);
                section = Some(name.to_string());
            }
            continue;
        }

        let Some((key, _)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_end_matches(':');
        if key.is_empty() {
            continue;
        }
        facts.add_definition(key, (start as u32, (start + line.len()) as u32), doc);

        // **The structure is the edge.** `ledger.charge.limit` belongs under
        // `ledger.charge`, and a key under `[charge]` belongs to that section:
        // without either, a settings file is a flat list of names and a reader
        // cannot ask what a section holds.
        if let Some(parent) = key.rsplit_once('.').map(|(p, _)| p) {
            facts.add_call(parent.to_string(), key.to_string(), false);
        } else if let Some(sec) = &section {
            facts.add_call(sec.clone(), key.to_string(), false);
        }
    }

    facts
}
