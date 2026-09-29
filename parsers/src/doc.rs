//! Robust multi-language documentation comment cleaner and normalizer.
//! Exactly matches glasir's clean_doc logic and strips all doc markers,
//! collapsing runs of whitespace into single space separated words.

/// Strips comment syntax and markers so only clean prose words remain.
pub fn clean_doc(raw: &str) -> String {
    let raw = raw
        .replace("/**", " ")
        .replace("*/", " ")
        .replace("/*", " ")
        .replace("///", " ")
        .replace("//!", " ")
        .replace("//", " ")
        .replace("{-|", " ")
        .replace("{-", " ")
        .replace("-}", " ")
        .replace("(*", " ")
        .replace("*)", " ")
        .replace("--[[", " ")
        .replace("]]", " ")
        .replace("-- |", " ")
        .replace("-- ^", " ")
        .replace("--", " ")
        .replace("#[", " ")
        .replace("]#", " ")
        .replace("#=", " ")
        .replace("=#", " ")
        .replace("<!--", " ")
        .replace("-->", " ")
        .replace("=pod", " ")
        .replace("=cut", " ")
        .replace("~S\"\"\"", " ")
        .replace("\"\"\"", " ")
        .replace("'''", " ")
        .replace(";;", " ")
        .replace("#'", " ");

    let mut out = String::new();
    for line in raw.lines() {
        let line = line.trim();
        let line = line
            .trim_start_matches('#')
            .trim_start_matches('*')
            .trim_start_matches('-')
            .trim_start_matches(';')
            .trim_start_matches('%')
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        // Collapse runs of whitespace left by removing markers.
        let mut spaced = false;
        for ch in line.chars() {
            if ch.is_whitespace() {
                spaced = true;
            } else {
                if spaced && !out.is_empty() {
                    out.push(' ');
                }
                spaced = false;
                out.push(ch);
            }
        }
    }
    out
}

pub fn clean_comment(raw: &str) -> String {
    clean_doc(raw)
}

#[derive(Debug, Default, Clone)]
pub struct DocTracker {
    pending: Vec<String>,
}

impl DocTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_comment(&mut self, text: &str, _end_byte: u32) {
        self.pending.push(text.to_string());
    }

    pub fn take_doc(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            let joined = self.pending.join(" ");
            self.pending.clear();
            let cleaned = clean_doc(&joined);
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned)
            }
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}
