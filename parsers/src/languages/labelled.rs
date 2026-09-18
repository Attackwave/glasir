//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Four formats that declare a *label* and refer to it by name elsewhere:
//! reStructuredText's `.. _charge:` and `:ref:`charge``, BibTeX's
//! `@article{charge2024}` and `crossref`, GN's `source_set("charge")` and
//! `deps`, Kconfig's `config CHARGE` and `select`. The marker differs, the
//! shape does not — a declaration, then references to it by the same name.
//!
//! Measured before this existed: a `.bib` file read as LaTeX yielded **zero**
//! definitions and counted as empty, because `@article{…}` is not a control
//! sequence.

use crate::facts::FileFacts;

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

        // **The label check runs before the comment check, not after.** RST
        // marks a comment with `..` and a label with `.. _name:`, so the
        // shorter prefix swallowed every declaration: the fixture measured
        // zero definitions and counted as empty. Same ordering problem R's
        // rule file records for `#'` against `#`.
        let is_declaration = trimmed.starts_with(".. _")
            || trimmed.starts_with('@')
            || trimmed.starts_with("config ")
            || trimmed.starts_with("menuconfig ");
        if !is_declaration {
            if let Some(text) = style
                .line_comment_prefix
                .iter()
                .find_map(|p| trimmed.strip_prefix(*p))
            {
                pending.push(text.trim().to_string());
                continue;
            }
        }
        if trimmed.is_empty() {
            continue;
        }

        let doc = |pending: &mut Vec<String>| {
            if pending.is_empty() {
                None
            } else {
                let d = pending.join(" ");
                pending.clear();
                Some(d)
            }
        };
        let range = (start as u32, (start + line.len()) as u32);

        // BibTeX: `@article{charge2024,`
        if let Some(rest) = trimmed.strip_prefix('@') {
            if let Some((_, key)) = rest.split_once('{') {
                let key = key.trim().trim_end_matches(',').trim();
                if !key.is_empty() {
                    facts.add_definition(key, range, doc(&mut pending));
                    current = Some(key.to_string());
                }
            }
            continue;
        }

        // reStructuredText: `.. _charge:` declares, `.. include:: other`
        // refers.
        if let Some(rest) = trimmed.strip_prefix(".. ") {
            if let Some(label) = rest.strip_prefix('_').and_then(|r| r.strip_suffix(':')) {
                let label = label.trim();
                if !label.is_empty() {
                    facts.add_definition(label, range, doc(&mut pending));
                    current = Some(label.to_string());
                }
                continue;
            }
            if let Some((_, target)) = rest.split_once(":: ") {
                let target = target.trim();
                if !target.is_empty() {
                    let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
                    facts.add_call(owner, target.to_string(), false);
                }
                continue;
            }
        }

        // `:ref:`charge`` in RST and `crossref = {refuse2024}` in BibTeX:
        // both name another declaration in the middle of a line, which the
        // line-prefix branches above cannot see. Without them RST and BibTeX
        // each measured every declaration and **zero** edges.
        if let Some(owner) = current.clone() {
            let mut rest = trimmed;
            while let Some(pos) = rest.find(":ref:`") {
                let after = &rest[pos + 6..];
                if let Some((target, tail)) = after.split_once('`') {
                    let target = target.trim();
                    if !target.is_empty() {
                        facts.add_call(owner.clone(), target.to_string(), false);
                    }
                    rest = tail;
                } else {
                    break;
                }
            }
            if let Some(after) = trimmed.strip_prefix("crossref") {
                if let Some((_, value)) = after.split_once('{') {
                    let target = value.trim_end_matches([',', '}', ' ']).trim_end_matches('}');
                    let target = target.trim();
                    if !target.is_empty() {
                        facts.add_call(owner, target.to_string(), false);
                    }
                }
            }
        }

        // Kconfig: `config CHARGE` declares; `select`/`depends on` refer.
        // GN: `source_set("charge")` declares; a `":refuse"` in a list refers.
        let mut words = trimmed.split_whitespace();
        match words.next() {
            Some("config") | Some("menuconfig") => {
                if let Some(name) = words.next() {
                    facts.add_definition(name, range, doc(&mut pending));
                    current = Some(name.to_string());
                }
                continue;
            }
            Some("select") | Some("imply") => {
                if let Some(name) = words.next() {
                    let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
                    facts.add_call(owner, name.trim_end_matches(',').to_string(), false);
                }
                continue;
            }
            Some("depends") => {
                // `depends on COMMIT_ENTRY`
                if words.next() == Some("on") {
                    if let Some(name) = words.next() {
                        let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
                        facts.add_call(owner, name.to_string(), false);
                    }
                }
                continue;
            }
            _ => {}
        }

        // GN's declaration is `target_type("name") {`.
        if let Some((_, rest)) = trimmed.split_once("(\"") {
            if let Some((name, _)) = rest.split_once('"') {
                if trimmed.ends_with('{') && !name.is_empty() {
                    facts.add_definition(name, range, doc(&mut pending));
                    current = Some(name.to_string());
                    continue;
                }
            }
        }

        // A quoted `":refuse"` or `"//path:target"` inside a list is a
        // reference to another target; a bare quoted file name is not.
        for piece in trimmed.split('"').skip(1).step_by(2) {
            if let Some((_, target)) = piece.rsplit_once(':') {
                let target = target.trim();
                if !target.is_empty() && !target.contains('.') {
                    let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
                    facts.add_call(owner, target.to_string(), false);
                }
            }
        }

        // A requirements file names one dependency per line, and `-r other`
        // pulls in another file.
        if let Some(other) = trimmed.strip_prefix("-r ") {
            let owner = current.clone().unwrap_or_else(|| "<module>".to_string());
            facts.add_call(owner, other.trim().to_string(), false);
            continue;
        }
        if let Some(name) = trimmed
            .split(['=', '>', '<', '~', '!', ';', '['])
            .next()
            .map(str::trim)
        {
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
                && trimmed.contains(['=', '>', '<', '~'])
            {
                facts.add_definition(name, range, doc(&mut pending));
            }
        }
    }

    facts
}
