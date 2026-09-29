//! Language-specific syntax; lexical and call rules come from the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// POD — any line opening with `=word` at column 0 through `=cut` — and
/// everything after `__END__` or `__DATA__` is documentation or data. Blanked
/// rather than removed, so every byte offset stays where it was; read as code,
/// DBI's `=head1` prose became definitions named `if` and `for`.
fn blank_pod(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut pod = false;
    let mut data = false;
    for line in src.split_inclusive('\n') {
        let starts_pod =
            line.starts_with('=') && line[1..].starts_with(|c: char| c.is_ascii_alphabetic());
        if !data && (line.starts_with("__END__") || line.starts_with("__DATA__")) {
            data = true;
        }
        if starts_pod {
            pod = true;
        }
        if pod || data {
            // One space per byte, so a multi-byte character keeps its width.
            let body = line.trim_end_matches('\n');
            out.extend(std::iter::repeat_n(' ', body.len()));
            out.push_str(&line[body.len()..]);
        } else {
            out.push_str(line);
        }
        if pod && line.starts_with("=cut") {
            pod = false;
        }
    }
    out
}

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let blanked = blank_pod(src);
    let src = blanked.as_str();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                scope.on_open_delimiter();
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("->") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => match *ident {
                "sub" | "method" => {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                            scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                            scope.on_word(fn_name);
                            i += 2;
                            continue;
                        }
                    }
                }
                "package" | "class" => {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(pkg_name) = tokens[i + 1].kind {
                            scope.open_definition(pkg_name, start_byte as usize, true, &mut facts);
                            scope.on_word(pkg_name);
                            i += 2;
                            continue;
                        }
                    }
                }
                _ => {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len()
                        && tokens[j].kind == TokenKind::Symbol('(')
                        && calls.allows(ident)
                    {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                }
            },
            TokenKind::Number(num) => {
                scope.on_word(num);
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
