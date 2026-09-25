//! HTTP routes and the requests that reach them, read from source.
//!
//! A service's handler is called by a URL, not by a name, so no call edge ever
//! connects a client to it: `impact` on a handler found nothing in the web app
//! calling it. Both halves are in the source as strings. A server registers a
//! path — `@GetMapping("/{id}")` under `@RequestMapping("/api/accounts")`,
//! `api.MapGet("/items/{id:int}", GetItemById)` on `MapGroup("api/catalog")`,
//! `router.get("/users/:id", show)`, `@app.get("/users/{id}")` — and a client
//! sends one — `http.get(\`${this.resourceUrl}/${id}\`)`,
//! `GetFromJsonAsync(uri)` after `var uri = $"{baseUrl}items/{id}"`.
//!
//! Each side is reduced to a verb and path segments: string constants of the
//! same file are substituted, anything else interpolated is a parameter, a
//! leading unknown is a base URL and dropped, a query string is not part of a
//! route, and case does not matter. A request reaches a route when every
//! segment agrees and the verbs do. Everything is per file and syntactic —
//! a prefix set in another file (Express's `app.use("/api", router)`) or a URL
//! built by a helper function is not followed.

use native_parsers::lexer::{Token, TokenKind};

use crate::parse_ast::Lang;

/// The top level of a file, as a definition index.
pub const MODULE: u32 = u32::MAX;

/// An HTTP method, or any when the source does not say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Verb {
    Any,
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Verb {
    fn from_name(name: &str) -> Option<Verb> {
        let lower = name.to_ascii_lowercase();
        let has = |w: &str| lower.starts_with(w) || lower.starts_with(&format!("map{w}"));
        Some(if has("get") {
            Verb::Get
        } else if has("post") {
            Verb::Post
        } else if has("put") {
            Verb::Put
        } else if has("patch") {
            Verb::Patch
        } else if has("delete") {
            Verb::Delete
        } else {
            return None;
        })
    }

    fn agrees(self, other: Verb) -> bool {
        self == Verb::Any || other == Verb::Any || self == other
    }
}

/// What one file declares and sends. `def` indexes the file's ranges;
/// `MODULE` is the top level.
#[derive(Debug, Default, PartialEq)]
pub struct Http {
    /// (handler definition, verb, path). A handler named by reference in
    /// another file is carried by name instead.
    pub routes: Vec<(Handler, Verb, String)>,
    /// (sending definition, verb, path).
    pub requests: Vec<(u32, Verb, String)>,
}

#[derive(Debug, PartialEq)]
pub enum Handler {
    Def(u32),
    Name(String),
}

/// Annotations and decorators that declare a route on the next definition.
const ANNOTATIONS: &[&str] = &[
    "GetMapping",
    "PostMapping",
    "PutMapping",
    "PatchMapping",
    "DeleteMapping",
    "RequestMapping",
    "Get",
    "Post",
    "Put",
    "Patch",
    "Delete",
    "HttpGet",
    "HttpPost",
    "HttpPut",
    "HttpPatch",
    "HttpDelete",
    "Route",
    "Controller",
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "route",
    "api_route",
];

/// Calls that register a route with a handler argument.
const REGISTRATIONS: &[&str] = &[
    "MapGet",
    "MapPost",
    "MapPut",
    "MapPatch",
    "MapDelete",
    "Map",
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "GET",
    "POST",
    "PUT",
    "PATCH",
    "DELETE",
    "HandleFunc",
    "Handle",
    "all",
];

/// Calls that group routes under a prefix held in a variable.
const GROUPS: &[&str] = &["MapGroup", "Group", "group"];

/// Calls that send a request, with the argument position of the URL.
const REQUESTS: &[(&str, usize)] = &[
    ("get", 0),
    ("post", 0),
    ("put", 0),
    ("patch", 0),
    ("delete", 0),
    ("request", 0),
    ("fetch", 0),
    ("GetAsync", 0),
    ("GetFromJsonAsync", 0),
    ("GetStringAsync", 0),
    ("GetStreamAsync", 0),
    ("GetByteArrayAsync", 0),
    ("PostAsync", 0),
    ("PostAsJsonAsync", 0),
    ("PutAsync", 0),
    ("PutAsJsonAsync", 0),
    ("PatchAsync", 0),
    ("PatchAsJsonAsync", 0),
    ("DeleteAsync", 0),
    ("DeleteFromJsonAsync", 0),
    ("HttpRequestMessage", 1),
    ("getForObject", 0),
    ("getForEntity", 0),
    ("postForObject", 0),
    ("postForEntity", 0),
    ("exchange", 0),
    ("NewRequest", 1),
    ("Get", 0),
    ("Post", 0),
];

pub fn extract(lang: Lang, src: &str, ranges: &[(String, (u32, u32))]) -> Http {
    let tokens = native_parsers::rules::active().tokens(lang, src);
    if tokens.is_empty() {
        return Http::default();
    }
    let enclosing = |at: u32| -> u32 {
        ranges
            .iter()
            .enumerate()
            .filter(|(_, (_, (s, e)))| *s <= at && at < *e)
            .min_by_key(|(_, (_, (s, e)))| e - s)
            .map_or(MODULE, |(k, _)| k as u32)
    };
    let contains = |outer: u32, inner: u32| -> bool {
        outer == MODULE
            || ranges
                .get(outer as usize)
                .zip(ranges.get(inner as usize))
                .is_some_and(|((_, (os, oe)), (_, (is, ie)))| os <= is && ie <= oe)
    };

    // Strings assigned to a name: `resourceUrl = \`...\``, `var uri = $"..."`,
    // `private const string ApiUrlBase = "api/catalog"`. Kept with the
    // definition they are assigned in, so each method's `uri` is its own.
    let mut consts: Vec<(&str, u32, &str)> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let TokenKind::StringLit(value) = t.kind else {
            continue;
        };
        if i >= 2
            && matches!(
                tokens[i - 1].kind,
                TokenKind::Symbol('=') | TokenKind::DoubleSymbol(":=")
            )
            && let TokenKind::Ident(name) = tokens[i - 2].kind
        {
            consts.push((name, enclosing(t.start), value));
        }
    }
    let resolve = |n: &str, d: u32, depth: usize| lookup_inner(&consts, &contains, n, d, depth);

    // Prefixes held in variables: `var api = app.MapGroup("api/catalog")`.
    let mut groups: Vec<(&str, String)> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let TokenKind::Ident(name) = t.kind else {
            continue;
        };
        if !GROUPS.contains(&name)
            || !matches!(next_kind(&tokens, i + 1), Some(TokenKind::Symbol('(')))
        {
            continue;
        }
        let Some(first) = argument(&tokens, i + 1, 0) else {
            continue;
        };
        let Some(TokenKind::StringLit(raw)) = tokens.get(first.0).map(|t| t.kind.clone()) else {
            continue;
        };
        // The variable is the name before `=` on the same statement.
        let mut k = i;
        while k > 0
            && !matches!(
                tokens[k].kind,
                TokenKind::Symbol('=') | TokenKind::DoubleSymbol(":=")
            )
        {
            if matches!(tokens[k].kind, TokenKind::Symbol(';' | '{' | '}')) {
                break;
            }
            k -= 1;
        }
        if k > 0
            && matches!(
                tokens[k].kind,
                TokenKind::Symbol('=') | TokenKind::DoubleSymbol(":=")
            )
            && let TokenKind::Ident(var) = tokens[k - 1].kind
        {
            let base = groups
                .iter()
                .find(|(g, _)| receiver(&tokens, i).is_some_and(|r| r == *g))
                .map(|(_, p)| p.clone())
                .unwrap_or_default();
            if let Some(path) = template(raw, enclosing(t.start), 0, &resolve) {
                groups.push((var, join(&base, &path)));
            }
        }
    }

    let mut out = Http::default();
    // Annotations first: a route declared on a type is the prefix of the
    // routes declared on its members.
    let mut declared: Vec<(u32, Verb, String)> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let TokenKind::Ident(name) = t.kind else {
            continue;
        };
        if !annotated_at(&tokens, i, name) || !ANNOTATIONS.contains(&name) {
            continue;
        }
        let path = match annotation_path(&tokens, i) {
            Some(raw) => template(raw, enclosing(t.start), 0, &resolve),
            None => Some(String::new()),
        };
        let Some(path) = path else { continue };
        // The next definition to start after the annotation is what it
        // annotates.
        let Some(def) = ranges
            .iter()
            .enumerate()
            .filter(|(_, (_, (s, _)))| *s >= t.end)
            .min_by_key(|(_, (_, (s, _)))| *s)
            .map(|(k, _)| k as u32)
        else {
            continue;
        };
        let verb = Verb::from_name(name.trim_start_matches("Http")).unwrap_or(Verb::Any);
        declared.push((def, verb, path));
    }
    for (def, verb, path) in &declared {
        // A declaration whose definition holds other declared definitions is
        // a prefix, not a route of its own.
        let holds_routes = declared
            .iter()
            .any(|(d, _, _)| d != def && contains(*def, *d));
        if holds_routes {
            continue;
        }
        let prefix = declared
            .iter()
            .filter(|(d, _, _)| d != def && contains(*d, *def))
            .map(|(_, _, p)| p.as_str())
            .collect::<Vec<_>>()
            .join("/");
        out.routes
            .push((Handler::Def(*def), *verb, join(&prefix, path)));
    }

    for (i, t) in tokens.iter().enumerate() {
        let TokenKind::Ident(name) = t.kind else {
            continue;
        };
        let Some(open) = call_paren(&tokens, i) else {
            continue;
        };
        let here = enclosing(t.start);
        let annotated = annotated_at(&tokens, i, name);

        // A registration: a path literal, then the handler.
        if !annotated && REGISTRATIONS.contains(&name) && receiver(&tokens, i).is_some() {
            let always = name.starts_with("Map") || name.starts_with("Handle");
            let second = argument(&tokens, open, 1).map(|(s, e)| &tokens[s..e]);
            // `router.get("/x", show)` and `http.post(url, body)` read alike;
            // a registration is told by its handler: a function of this
            // file, a path to one elsewhere, or a function written inline.
            let handler_arg = second.is_some_and(|arg| {
                matches!(
                    arg.first().map(|t| &t.kind),
                    Some(TokenKind::Ident("function" | "async"))
                ) || arg.iter().any(|t| t.kind == TokenKind::DoubleSymbol("=>"))
                    || handler_name(arg).is_some_and(|n| {
                        ranges.iter().any(|(d, _)| d == n)
                            || (arg.len() > 1
                                && !matches!(arg[0].kind, TokenKind::Ident("this" | "self")))
                    })
            });
            if let Some((first, _)) = argument(&tokens, open, 0)
                && let TokenKind::StringLit(raw) = tokens[first].kind
                && let Some(path) = template(raw, here, 0, &resolve)
                && (always || (path_like(raw) && handler_arg))
            {
                let prefix = receiver(&tokens, i)
                    .and_then(|r| groups.iter().find(|(g, _)| *g == r))
                    .map(|(_, p)| p.clone())
                    .unwrap_or_default();
                let handler = argument(&tokens, open, 1)
                    .and_then(|(s, e)| handler_name(&tokens[s..e]))
                    .map_or(Handler::Def(here), |n| {
                        ranges
                            .iter()
                            .position(|(d, _)| d == n)
                            .map_or(Handler::Name(n.to_string()), |k| Handler::Def(k as u32))
                    });
                let verb = Verb::from_name(name).unwrap_or(Verb::Any);
                out.routes.push((handler, verb, join(&prefix, &path)));
                continue;
            }
        }

        // A request: a URL in the known argument position.
        if annotated {
            continue;
        }
        let Some(&(_, position)) = REQUESTS.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        let Some((s, e)) = argument(&tokens, open, position) else {
            continue;
        };
        let url = match &tokens[s..e] {
            [
                Token {
                    kind: TokenKind::StringLit(raw),
                    ..
                },
            ] => template(raw, here, 0, &resolve),
            [
                Token {
                    kind: TokenKind::Ident(n),
                    ..
                },
            ] => resolve(n, here, 0),
            [
                Token {
                    kind: TokenKind::Ident("this" | "self"),
                    ..
                },
                Token {
                    kind: TokenKind::Symbol('.'),
                    ..
                },
                Token {
                    kind: TokenKind::Ident(n),
                    ..
                },
            ] => resolve(n, here, 0),
            _ => None,
        };
        // A generic verb (`get`, `delete`) is a URL only when the argument
        // reads as a path; `cache.get("user")` is not a request.
        let generic = name.chars().next().is_some_and(|c| c.is_lowercase());
        let Some(url) = url.filter(|u| !segments(u).is_empty() && (u.contains('/') || !generic))
        else {
            continue;
        };
        let verb = match name {
            "HttpRequestMessage" => argument(&tokens, open, 0)
                .and_then(|(s, e)| tokens[s..e].last().map(|t| t.kind.clone()))
                .and_then(|k| match k {
                    TokenKind::Ident(v) => Verb::from_name(v),
                    _ => None,
                })
                .unwrap_or(Verb::Any),
            "NewRequest" => argument(&tokens, open, 0)
                .and_then(|(s, _)| match tokens[s].kind {
                    TokenKind::StringLit(v) => Verb::from_name(v.trim_matches('"')),
                    _ => None,
                })
                .unwrap_or(Verb::Any),
            "fetch" => method_option(&tokens, open).unwrap_or(Verb::Get),
            "request" | "exchange" => Verb::Any,
            _ => Verb::from_name(name).unwrap_or(Verb::Any),
        };
        out.requests.push((here, verb, url));
    }
    out
}

fn lookup_inner(
    consts: &[(&str, u32, &str)],
    contains: &dyn Fn(u32, u32) -> bool,
    name: &str,
    at_def: u32,
    depth: usize,
) -> Option<String> {
    if depth > 3 {
        return None;
    }
    let own = consts
        .iter()
        .rev()
        .find(|(n, d, _)| *n == name && *d == at_def);
    let outer = consts
        .iter()
        .rev()
        .find(|(n, d, _)| *n == name && contains(*d, at_def));
    // A field of a neighbouring class, when the file assigns the name once.
    let mut named = consts.iter().filter(|(n, _, _)| *n == name);
    let unique = match (named.next(), named.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    };
    let (_, def, raw) = own.or(outer).or(unique)?;
    template(raw, *def, depth + 1, &|n, d, depth| {
        lookup_inner(consts, contains, n, d, depth)
    })
}

/// A string literal or template as a path: quotes and prefixes gone, the
/// query, scheme and host cut off. In a template (`` `...` ``, `$"..."`,
/// `f"..."`) known names are substituted, a leading unknown is dropped as a
/// base URL and any other unknown becomes a parameter `{}`; in a plain
/// literal `{id}` is already the server's own parameter syntax.
fn template(
    raw: &str,
    def: u32,
    depth: usize,
    resolve: &dyn Fn(&str, u32, usize) -> Option<String>,
) -> Option<String> {
    if depth > 3 {
        return None;
    }
    let backtick = raw.starts_with('`');
    let interpolated = backtick
        || raw.starts_with("$\"")
        || raw.starts_with("$@\"")
        || raw.starts_with("@$\"")
        || raw.starts_with("f\"")
        || raw.starts_with("f'");
    let body = raw.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '$' || c == '@');
    let body = body.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let mut out = String::new();
    if !interpolated {
        out.push_str(body);
    } else {
        let bytes = body.as_bytes();
        let mut i = 0;
        while i < body.len() {
            let opens = if backtick {
                body[i..].starts_with("${")
            } else {
                bytes[i] == b'{' && !body[i..].starts_with("{{")
            };
            if !opens {
                let ch = body[i..].chars().next().expect("in bounds");
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }
            let start = i + if backtick { 2 } else { 1 };
            let mut level = 1;
            let mut end = start;
            for (j, ch) in body[start..].char_indices() {
                match ch {
                    '{' => level += 1,
                    '}' => {
                        level -= 1;
                        if level == 0 {
                            end = start + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if level != 0 {
                break;
            }
            let expr = body[start..end].trim();
            let name = expr.trim_start_matches("this.").trim_start_matches("self.");
            let known = name
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_')
                .then(|| resolve(name, def, depth))
                .flatten();
            match known {
                Some(value) => out.push_str(&value),
                None if out.is_empty() => {}
                None => out.push_str("{}"),
            }
            i = end + 1;
        }
    }
    let mut path = out.as_str();
    if let Some((_, rest)) = path.split_once("://") {
        path = rest.split_once('/').map_or("", |(_, p)| p);
    }
    // A query ends the path, but `{brandId?}` is an optional parameter.
    let mut level = 0;
    let end = path
        .char_indices()
        .find(|&(_, c)| {
            match c {
                '{' => level += 1,
                '}' => level -= 1,
                _ => {}
            }
            level == 0 && (c == '?' || c == '#')
        })
        .map_or(path.len(), |(i, _)| i);
    Some(path[..end].to_string())
}

fn path_like(raw: &str) -> bool {
    raw.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '$' || c == '@')
        .trim_start_matches(['"', '\'', '`'])
        .starts_with('/')
}

fn join(prefix: &str, path: &str) -> String {
    format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Path segments for matching: case folded, parameters as `*`.
pub fn segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let s = s.to_ascii_lowercase();
            if s.contains('{') || s.starts_with(':') || s.starts_with('<') || s == "*" {
                "*".to_string()
            } else {
                s
            }
        })
        .collect()
}

/// Whether a request reaches a route. A parameter on the route takes any
/// segment; a parameter in the request only meets a parameter, since an id
/// cannot be told from a literal path segment it happens to equal.
pub fn reaches(request: (Verb, &[String]), route: (Verb, &[String])) -> bool {
    request.0.agrees(route.0)
        && request.1.len() == route.1.len()
        && request
            .1
            .iter()
            .zip(route.1)
            .all(|(q, r)| r == "*" || (q != "*" && q == r))
}

fn next_kind<'a>(tokens: &'a [Token<'a>], i: usize) -> Option<&'a TokenKind<'a>> {
    tokens.get(i).map(|t| &t.kind)
}

fn prev_kind<'a>(tokens: &'a [Token<'a>], i: usize) -> Option<&'a TokenKind<'a>> {
    i.checked_sub(1)
        .and_then(|k| tokens.get(k))
        .map(|t| &t.kind)
}

/// Whether the name at `i` is used as an annotation or decorator: `@Get`,
/// `@app.get`, or C#'s `[HttpGet]` / `[Route]` — not any name after `[`,
/// which in most languages opens an array.
fn annotated_at(tokens: &[Token<'_>], i: usize, name: &str) -> bool {
    match prev_kind(tokens, i) {
        Some(TokenKind::Symbol('@')) => true,
        Some(TokenKind::Symbol('[')) => name.starts_with("Http") || name == "Route",
        _ => decorator_chain(tokens, i),
    }
}

/// `@app.get` / `@router.post`: a decorator reaching the name through dots.
fn decorator_chain(tokens: &[Token<'_>], i: usize) -> bool {
    let mut k = i;
    while k >= 2
        && tokens[k - 1].kind == TokenKind::Symbol('.')
        && matches!(tokens[k - 2].kind, TokenKind::Ident(_))
    {
        k -= 2;
    }
    k != i && k >= 1 && tokens[k - 1].kind == TokenKind::Symbol('@')
}

/// The identifier a call is made on: `api` in `api.MapGet(`.
fn receiver<'a>(tokens: &[Token<'a>], i: usize) -> Option<&'a str> {
    if i >= 2 && tokens[i - 1].kind == TokenKind::Symbol('.') {
        if let TokenKind::Ident(r) = tokens[i - 2].kind {
            return Some(r);
        }
        return Some("");
    }
    None
}

/// The index of the `(` opening a call on the name at `i`, past type
/// arguments: `get<IBankAccount>(`.
fn call_paren(tokens: &[Token<'_>], i: usize) -> Option<usize> {
    let mut k = i + 1;
    if tokens.get(k)?.kind == TokenKind::Symbol('<') {
        let mut depth = 0i32;
        while let Some(t) = tokens.get(k) {
            match t.kind {
                TokenKind::Symbol('<') => depth += 1,
                TokenKind::Symbol('>') => depth -= 1,
                TokenKind::DoubleSymbol(">>") => depth -= 2,
                TokenKind::Ident(_) | TokenKind::Symbol('.' | ',' | '?' | '[' | ']') => {}
                _ => return None,
            }
            k += 1;
            if depth <= 0 {
                break;
            }
        }
    }
    (tokens.get(k)?.kind == TokenKind::Symbol('(')).then_some(k)
}

/// Token range of the `n`th argument of the call opening at `open`.
fn argument(tokens: &[Token<'_>], open: usize, n: usize) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut index = 0;
    let mut start = open + 1;
    for (k, t) in tokens.iter().enumerate().skip(open) {
        match t.kind {
            TokenKind::Symbol('(' | '[' | '{') => depth += 1,
            TokenKind::Symbol(')' | ']' | '}') => {
                depth -= 1;
                if depth == 0 {
                    return (index == n && k > start).then_some((start, k));
                }
            }
            TokenKind::Symbol(',') if depth == 1 => {
                if index == n {
                    return (k > start).then_some((start, k));
                }
                index += 1;
                start = k + 1;
            }
            _ => {}
        }
    }
    None
}

/// The path of an annotation: its first argument, or its `value = "..."` or
/// `path = "..."`.
fn annotation_path<'a>(tokens: &[Token<'a>], i: usize) -> Option<&'a str> {
    let open = call_paren(tokens, i)?;
    let (s, e) = argument(tokens, open, 0)?;
    match &tokens[s..e] {
        [
            Token {
                kind: TokenKind::StringLit(raw),
                ..
            },
        ] => Some(raw),
        [
            Token {
                kind: TokenKind::Ident("value" | "path"),
                ..
            },
            Token {
                kind: TokenKind::Symbol('='),
                ..
            },
            Token {
                kind: TokenKind::StringLit(raw),
                ..
            },
            ..,
        ] => Some(raw),
        _ => None,
    }
}

/// A handler passed by name: `GetItemById`, `handlers.show`.
fn handler_name<'a>(arg: &[Token<'a>]) -> Option<&'a str> {
    let mut last = None;
    for (k, t) in arg.iter().enumerate() {
        match t.kind {
            TokenKind::Ident(n) if k % 2 == 0 => last = Some(n),
            TokenKind::Symbol('.') if k % 2 == 1 => {}
            _ => return None,
        }
    }
    last
}

/// `fetch(url, { method: "POST" })`.
fn method_option(tokens: &[Token<'_>], open: usize) -> Option<Verb> {
    let (s, e) = argument(tokens, open, 1)?;
    tokens[s..e]
        .windows(3)
        .find_map(|w| match (&w[0].kind, &w[1].kind, &w[2].kind) {
            (TokenKind::Ident("method"), TokenKind::Symbol(':'), TokenKind::StringLit(v)) => {
                Verb::from_name(v.trim_matches(|c| c == '"' || c == '\'' || c == '`'))
            }
            _ => None,
        })
}
