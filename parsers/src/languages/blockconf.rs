//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Two configuration formats whose structure is a *named block* plus
//! references between blocks: `Host charge` with `ProxyJump refuse`, and
//! `source = other.conf` pulling in a second file. Neither has a `key = value`
//! shape throughout — an SSH config uses whitespace, Hyprland uses `=` — so
//! the key-value module cannot read either, and a keyword table finds the
//! headers and none of the references.

use crate::facts::FileFacts;

/// Directives whose value names another block or file.
const REFERS: [&str; 5] = ["proxyjump", "include", "source", "match", "proxycommand"];

pub(crate) fn parse(
    src: &str,
    style: crate::lexer::CommentStyle<'_>,
    _calls: &crate::rules::Calls,
) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut current: Option<String> = None;
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

        let range = (start as u32, (start + line.len()) as u32);
        let doc = if pending.is_empty() {
            None
        } else {
            let d = pending.join(" ");
            pending.clear();
            Some(d)
        };

        // `Host charge` opens a block the directives below belong to.
        let mut words = trimmed.split_whitespace();
        let Some(first) = words.next() else { continue };
        let key = first.trim_end_matches('=').to_lowercase();

        if key == "host" || key == "match" && current.is_none() {
            if let Some(name) = words.next() {
                facts.add_definition(name, range, doc);
                current = Some(name.to_string());
                continue;
            }
        }

        // Hyprland opens a block with `device {` and names it inside, with
        // `name = charge`. The brace is what says a block began; the `name`
        // key is what it is called, so the definition is deferred until that
        // line rather than taken from the block type — otherwise every
        // `device` block would be one node called `device`.
        if trimmed.ends_with('{') {
            current = None;
            continue;
        }
        if trimmed == "}" {
            current = None;
            continue;
        }
        if key == "name" {
            if let Some((_, value)) = trimmed.split_once('=') {
                let name = value.trim();
                if !name.is_empty() {
                    facts.add_definition(name, range, doc);
                    current = Some(name.to_string());
                    continue;
                }
            }
        }

        // `source = other.conf` and `ProxyJump refuse` both name a target;
        // the separator differs, the meaning does not.
        if REFERS.contains(&key.as_str()) {
            let target = trimmed
                .split_once('=')
                .map(|(_, v)| v)
                .unwrap_or_else(|| trimmed[first.len()..].as_ref())
                .trim();
            if !target.is_empty() {
                let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
                facts.add_call(owner, target.to_string(), false);
            }
            continue;
        }

        // `$limit = 5000` is a variable, and Hyprland's only declaration
        // outside a block.
        if let Some(name) = trimmed.strip_prefix('$').and_then(|r| r.split('=').next()) {
            let name = name.trim();
            if !name.is_empty() {
                facts.add_definition(name, range, doc);
            }
        }
    }

    facts
}
