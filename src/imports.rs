//! What a name in a file refers to elsewhere in the tree, read from its imports.
//!
//! A call through a module — `cli.main()` in Python, `store.New()` in
//! Go, `utils.parse()` in TypeScript — reached a bare placeholder, and tier 3
//! refuses a name defined in several places, so the call linked to nothing.
//! The import line names the file, which makes the target decidable rather
//! than guessed. Python, Go and JavaScript/TypeScript write a module-qualified
//! call as `alias.name()`, and their import syntax is regular enough to read
//! line by line. Elixir's `alias` reads the same way, and Java's declarations
//! do: a variable's declared type names the class a call through it reaches.

use crate::parse_ast::Lang;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One import statement, as written. Resolved against the tree by `resolve`.
#[derive(Debug, Clone, PartialEq)]
pub enum Import {
    /// `import a.b as x`, `import * as x from './y'`, `import x "p"`:
    /// `alias.f()` means `f` in `spec`.
    Module { alias: String, spec: String },
    /// `from a import f as g`, `import { f as g } from './y'`: `g()` means
    /// `f` in `spec`.
    Name {
        local: String,
        spec: String,
        name: String,
    },
    /// `class A extends B`, a mixin, Elixir's `import B`: a name the file does
    /// not define may come from the module `B`, for the definition around the
    /// byte `at`.
    Base { module: String, at: u32 },
    /// `interface A`, `protocol A`: the type only declares its methods.
    Contract { name: String },
    /// `trait A`, `module A`, `mixin A`: its methods reach into the class using
    /// it, so its own chain does not bound what `self` holds.
    Mixin { name: String },
    /// `import a.b.C`: the class `C` lives in a package ending in `b`. A `*`
    /// class says the file reaches names no statement lists.
    Qualified { class: String, package: String },
    /// `func (r *T) name()`: a Go method, which its type's body does not hold.
    Method {
        class: String,
        name: String,
        receiver: String,
        at: u32,
    },
    /// `extension A`: more of a type defined elsewhere, not a definition of it.
    Extension { name: String },
}

/// The import statements of one file.
pub fn read(lang: Lang, src: &str) -> Vec<Import> {
    match lang {
        Lang::Python => [python(src), python_bases(src)].concat(),
        Lang::Go => [go(src), go_classes(src)].concat(),
        Lang::TypeScript | Lang::JavaScript => [
            javascript(src),
            heads(src, &["class"], &["extends"], &["implements"]),
        ]
        .concat(),
        Lang::Elixir => elixir(src),
        Lang::Java => [
            java(src),
            contracts(src, "interface"),
            qualified(src),
            without_outsiders(
                src,
                heads(
                    src,
                    &["class", "interface", "enum", "record"],
                    &["extends", "implements"],
                    &[],
                ),
            ),
        ]
        .concat(),
        Lang::Kotlin => [
            kotlin(src),
            contracts(src, "interface"),
            qualified(src),
            kotlin_actuals(src),
            kotlin_receivers(src),
            without_outsiders(src, colon_heads(src, &["class", "interface", "object"])),
        ]
        .concat(),
        Lang::CSharp => [
            colon_heads(src, &["class", "struct", "interface", "record"]),
            contracts(src, "interface"),
        ]
        .concat(),
        Lang::Rust => [
            rust_uses(src),
            rust_traits(src),
            rust_methods(src),
            rust_types(src),
        ]
        .concat(),
        Lang::Swift => [
            colon_heads(src, &["class", "struct", "enum", "actor", "extension"]),
            contracts(src, "protocol"),
            swift_extensions(src),
            class_heads(src, &["extension"])
                .into_iter()
                .filter_map(|(head, len)| {
                    let name = head[len..].split_whitespace().next()?;
                    Some(Import::Extension {
                        name: name.split(':').next()?.to_string(),
                    })
                })
                .collect(),
        ]
        .concat(),
        Lang::Cpp | Lang::C => colon_heads(src, &["class", "struct"]),
        Lang::Scala => heads(
            src,
            &["class", "object", "trait"],
            &["extends", "with"],
            &[],
        ),
        Lang::Php => [
            without_outsiders(src, php(src)),
            mixins(src, "trait"),
            qualified(src),
        ]
        .concat(),
        Lang::Ruby => [ruby(src), mixins(src, "module")].concat(),
        Lang::Dart => [dart(src), mixins(src, "mixin")].concat(),
        _ => Vec::new(),
    }
}

/// Where `slice`, a part of `src`, begins.
fn offset(src: &str, slice: &str) -> u32 {
    (slice.as_ptr() as usize).saturating_sub(src.as_ptr() as usize) as u32
}

/// The last segment of a class named in code: `\\A\\B`, `A::B`, `A.B`, `B<T>`,
/// `B(x)` and `B[T]` are `B`.
fn base(src: &str, written: &str) -> Option<Import> {
    let name = written.trim().split(['<', '(', '[']).next()?.trim();
    let name = name.rsplit(['\\', ':', '.']).next()?;
    let valid = name.starts_with(char::is_alphabetic)
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    valid.then(|| Import::Base {
        module: name.to_string(),
        at: offset(src, written),
    })
}

/// How far a statement or a class head is followed: a file with an opener on
/// every line must not make each of them read to its end.
const STATEMENT_LINES: usize = 64;

/// `first`, a line of `src`, with the lines that continue it: after a trailing
/// comma or inside an open `[`.
fn statement<'a>(src: &'a str, first: &'a str) -> &'a str {
    let start = offset(src, first) as usize;
    let mut end = start;
    let mut brackets = 0i64;
    for line in src
        .get(start..)
        .unwrap_or("")
        .split_inclusive('\n')
        .take(STATEMENT_LINES)
    {
        end += line.len();
        brackets += line.matches('[').count() as i64 - line.matches(']').count() as i64;
        if brackets <= 0 && !line.trim_end().ends_with(',') {
            return src.get(start..end).map_or(first, str::trim_end);
        }
    }
    first
}

/// The bases a class head names, from the `class` at `head` to its `{` or `;`:
/// what follows a word of `open` until a word of `close`. A bound inside
/// `<…>`, as in `A<T extends B>`, is not one.
fn head_bases(src: &str, head: &str, open: &[&str], close: &[&str]) -> Vec<Import> {
    let start = offset(src, head) as usize;
    let rest = src.get(start..).unwrap_or("");
    let lines = rest.split_inclusive('\n').take(STATEMENT_LINES);
    let rest = &rest[..lines.map(str::len).sum()];
    let rest = &rest[..rest.find(['{', ';']).unwrap_or(rest.len())];
    let mut out = Vec::new();
    let mut angles = 0i32;
    let mut reading = false;
    for (k, word) in rest.split_whitespace().enumerate() {
        // Heads of languages without a `{` here end at the next declaration.
        const DECLARATIONS: [&str; 7] = ["class", "object", "trait", "def", "val", "var", "case"];
        if k > 0 && angles == 0 && DECLARATIONS.contains(&word) {
            break;
        }
        if angles == 0 {
            if open.contains(&word) {
                reading = true;
                continue;
            }
            if close.contains(&word) {
                reading = false;
            }
            if reading {
                out.extend(word.split(',').filter_map(|name| base(src, name)));
            }
        }
        angles += word.matches('<').count() as i32 - word.matches('>').count() as i32;
    }
    out
}

/// Where a line declares one of `keywords`: the word itself and what follows,
/// taking the last on the line, past `template <class T>`.
fn class_heads<'a>(src: &'a str, keywords: &[&str]) -> Vec<(&'a str, usize)> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        if line.starts_with(['*', '/', '#']) {
            continue;
        }
        let found = keywords.iter().filter_map(|word| {
            let mut last = None;
            for (at, _) in line.match_indices(word) {
                let before = line[..at].chars().next_back();
                let after = line[at + word.len()..].chars().next();
                if before.is_none_or(|c| c.is_whitespace())
                    && after.is_some_and(char::is_whitespace)
                {
                    last = Some((at, word.len()));
                }
            }
            last
        });
        if let Some((at, len)) = found.max() {
            out.push((&line[at..], len));
        }
    }
    out
}

/// `class A extends B implements C` in every class head of the file.
fn heads(src: &str, keywords: &[&str], open: &[&str], close: &[&str]) -> Vec<Import> {
    class_heads(src, keywords)
        .into_iter()
        .flat_map(|(head, _)| head_bases(src, head, open, close))
        .collect()
}

/// The classes a list names, split at its top level: `B(x), C<D, E>, public F`
/// names `B`, `C` and `F`.
fn listed_bases(src: &str, list: &str) -> Vec<Import> {
    let mut items = Vec::new();
    let (mut depth, mut from) = (0i32, 0);
    let mut before = ' ';
    for (at, c) in list.char_indices() {
        match c {
            '(' | '<' | '[' => depth += 1,
            '>' if before == '-' => {}
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                items.push(&list[from..at]);
                from = at + 1;
            }
            _ => {}
        }
        before = c;
    }
    items.push(&list[from..]);
    const QUALIFIERS: [&str; 5] = ["public", "private", "protected", "virtual", "final"];
    items
        .into_iter()
        .filter_map(|item| {
            let word = item
                .split_whitespace()
                .find(|w| !QUALIFIERS.contains(w) && !w.starts_with('@'))?;
            base(src, word)
        })
        .collect()
}

/// `class A(x: T) : B(x), C where T : D`: the list after the colon outside any
/// brackets. Without a `{` the head ends with a line nothing continues.
fn colon_heads(src: &str, keywords: &[&str]) -> Vec<Import> {
    let mut out = Vec::new();
    for (head, keyword) in class_heads(src, keywords) {
        let after = &head[keyword..];
        let first = after.split_whitespace().next().unwrap_or("");
        if first.starts_with(':') || matches!(first, "func" | "var" | "let" | "fun") {
            continue;
        }
        let start = offset(src, after) as usize;
        let rest = src.get(start..).unwrap_or("");
        let mut lines = rest.split_inclusive('\n').take(STATEMENT_LINES).peekable();
        let (mut end, mut depth) = (0, 0i32);
        while let Some(line) = lines.next() {
            end += line.len();
            depth +=
                line.matches(['(', '<']).count() as i32 - line.matches([')', '>']).count() as i32;
            let text = line.trim();
            let next = lines.peek().map_or("", |l| l.trim());
            if depth <= 0
                && !text.ends_with([',', ':'])
                && !next.starts_with([':', ','])
                && !next.starts_with("where ")
            {
                break;
            }
        }
        let text = &rest[..end];
        let text = &text[..text.find(['{', ';']).unwrap_or(text.len())];
        let (mut depth, mut before, mut colon) = (0i32, ' ', None);
        for (at, c) in text.char_indices() {
            match c {
                '(' | '<' | '[' => depth += 1,
                '>' if before == '-' => {}
                ')' | '>' | ']' => depth -= 1,
                // `struct a *b = c ? d : e;` declares a variable.
                '=' | '*' if depth == 0 => break,
                ':' if depth == 0 && before != ':' && !text[at + 1..].starts_with(':') => {
                    colon = Some(at);
                    break;
                }
                _ => {}
            }
            before = c;
        }
        if let Some(colon) = colon {
            let list = &text[colon + 1..];
            let list = list.split("where").next().unwrap_or(list);
            out.extend(listed_bases(src, list));
        }
    }
    out
}

/// The types declared with `keyword`.
fn contracts(src: &str, keyword: &str) -> Vec<Import> {
    class_heads(src, &[keyword])
        .into_iter()
        .filter_map(|(head, len)| {
            let name = head[len..].split_whitespace().next()?;
            let name = name.split(['<', ':', '{', '(']).next()?;
            (!name.is_empty()).then(|| Import::Contract {
                name: name.to_string(),
            })
        })
        .collect()
}

/// Drops bases the file says come from the platform, whatever a class of the
/// same name in the tree: PHP's `use Exception;` and `\\Exception`, and a
/// `java.` or `kotlin.` import.
fn without_outsiders(src: &str, bases: Vec<Import>) -> Vec<Import> {
    let mut outside = HashSet::new();
    for line in src.lines() {
        let php = line.strip_prefix("use ").filter(|n| !n.contains('\\'));
        let name = php.or_else(|| {
            let spec = line.strip_prefix("import ")?;
            let platform = ["java.", "javax.", "kotlin.", "kotlinx."];
            platform
                .iter()
                .any(|p| spec.starts_with(p))
                .then(|| spec.rsplit('.').next().unwrap_or(spec))
        });
        if let Some(name) = name {
            outside.insert(name.trim().trim_end_matches(';'));
        }
    }
    bases
        .into_iter()
        .filter(|import| match import {
            Import::Base { module, at } => {
                !outside.contains(module.as_str())
                    && src.as_bytes().get(*at as usize) != Some(&b'\\')
            }
            _ => true,
        })
        .collect()
}

/// The mixins declared with `keyword`.
fn mixins(src: &str, keyword: &str) -> Vec<Import> {
    contracts(src, keyword)
        .into_iter()
        .filter_map(|c| match c {
            Import::Contract { name } => Some(Import::Mixin { name }),
            _ => None,
        })
        .collect()
}

/// `fun Foo<T>.bar()` runs with a `Foo` as its implicit receiver.
fn kotlin_receivers(src: &str) -> Vec<Import> {
    class_heads(src, &["fun"])
        .into_iter()
        .filter_map(|(head, len)| {
            let rest = head[len..].trim_start();
            let rest = match rest.strip_prefix('<') {
                Some(after) => {
                    let mut depth = 1;
                    let end = after.char_indices().find_map(|(at, c)| {
                        match c {
                            '<' => depth += 1,
                            '>' => depth -= 1,
                            _ => {}
                        }
                        (depth == 0).then_some(at)
                    })?;
                    after[end + 1..].trim_start()
                }
                None => rest,
            };
            let name = &rest[..rest.find('(')?];
            let (receiver, _) = name.rsplit_once('.')?;
            base(src, receiver)
        })
        .collect()
}

/// `extension Foo` holds methods of `Foo`.
fn swift_extensions(src: &str) -> Vec<Import> {
    class_heads(src, &["extension"])
        .into_iter()
        .filter_map(|(head, len)| {
            let name = head[len..].split_whitespace().next()?;
            base(src, name.split(':').next()?)
        })
        .collect()
}

/// `impl<T> Walk<T> for Kid` reaches the default methods of `Walk`.
fn rust_traits(src: &str) -> Vec<Import> {
    class_heads(src, &["impl"])
        .into_iter()
        .filter_map(|(head, _)| {
            let (before, _) = head.split_once(" for ")?;
            base(src, before.split_whitespace().next_back()?)
        })
        .collect()
}

/// `import a.b.C;`, `import a.b.C as D`, and PHP's `use A\\B\\C;` at file level,
/// each with its package as a path. `a.b.*` and `A\\B\\{C, D}` read as class `*`.
fn qualified(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines() {
        let own = line
            .strip_prefix("package ")
            .or_else(|| line.strip_prefix("namespace "));
        if let Some(own) = own {
            out.push(Import::Qualified {
                class: ".".to_string(),
                package: own
                    .trim()
                    .trim_end_matches([';', '{'])
                    .trim()
                    .replace(['.', '\\'], "/"),
            });
            continue;
        }
        let path = line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("use "))
            .filter(|p| !p.starts_with("static "));
        let Some(path) = path.map(|p| p.trim().trim_end_matches(';')) else {
            continue;
        };
        let (path, alias) = match path.split_once(" as ") {
            Some((path, alias)) => (path.trim(), Some(alias.trim())),
            None => (path, None),
        };
        let (path, listed) = match path.find(['{', '*']) {
            Some(end) => (path[..end].trim_end_matches(['.', '\\']), true),
            None => (path, false),
        };
        let mut segments: Vec<&str> = path.split(['.', '\\']).collect();
        let class = if listed {
            "*"
        } else {
            segments.pop().unwrap_or("")
        };
        out.push(Import::Qualified {
            class: alias.unwrap_or(class).to_string(),
            package: segments.join("/"),
        });
    }
    out
}

/// `actual class A` carries out `expect class A`, the one a caller reads.
fn kotlin_actuals(src: &str) -> Vec<Import> {
    src.lines()
        .filter(|line| line.trim_start().contains("actual "))
        .flat_map(|line| class_heads(line, &["class", "interface", "object"]))
        .filter_map(|(head, len)| {
            let name = head[len..].split_whitespace().next()?;
            Some(Import::Extension {
                name: name.split(['<', '(', ':']).next()?.to_string(),
            })
        })
        .collect()
}

/// Go's embedded fields, and its methods with their receivers.
fn go_classes(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut in_struct = false;
    for line in src.lines() {
        let text = line.split("//").next().unwrap_or("").trim();
        if in_struct {
            if text.starts_with('}') {
                in_struct = false;
            } else if !text.is_empty() {
                let field = text.split('`').next().unwrap_or("").trim();
                if !field.contains(char::is_whitespace) {
                    out.extend(base(src, field.trim_start_matches('*')));
                }
            }
        } else if text.starts_with("type ") && text.ends_with("struct {") {
            in_struct = true;
        } else if let Some(rest) = text.strip_prefix("func (") {
            let Some((receiver, after)) = rest.split_once(')') else {
                continue;
            };
            let mut words = receiver.split_whitespace();
            let (Some(variable), Some(class)) = (words.next(), words.next()) else {
                continue;
            };
            let class = class.trim_start_matches('*');
            let class = class.split('[').next().unwrap_or(class);
            let name = after.trim().split(['(', '[']).next().unwrap_or("");
            if !name.is_empty() {
                out.push(Import::Method {
                    class: class.to_string(),
                    name: name.to_string(),
                    receiver: variable.to_string(),
                    at: offset(src, text),
                });
            }
        }
    }
    out
}

/// The methods of each `impl`, which the type's own body does not hold.
fn rust_methods(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut class = None;
    for line in src.lines() {
        if line.starts_with('}') {
            class = None;
        } else if line.starts_with("impl") {
            let head = line.split('{').next().unwrap_or(line);
            let head = head.rsplit(" for ").next().unwrap_or(head);
            let mut depth = 0;
            let plain: String = head
                .strip_prefix("impl")
                .unwrap_or(head)
                .chars()
                .filter(|c| {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    depth == 0 && *c != '>'
                })
                .collect();
            class = plain
                .split_whitespace()
                .next_back()
                .and_then(|w| w.rsplit("::").next())
                .map(str::to_string);
        } else if let Some(class) = &class {
            let mut words = line.split_whitespace().skip_while(|w| *w != "fn");
            let Some(name) = words.nth(1) else {
                continue;
            };
            let name = name.split(['(', '<']).next().unwrap_or(name);
            if line.trim_start().starts_with("//") || name.is_empty() {
                continue;
            }
            out.push(Import::Method {
                class: class.clone(),
                name: name.to_string(),
                receiver: String::new(),
                at: offset(src, line),
            });
        }
    }
    out
}

/// The type a Rust declaration states: `&mut Box<a::B<T>>` is `B`.
fn rust_type(written: &str) -> Option<&str> {
    const WRAPPERS: [&str; 7] = ["Box", "Arc", "Rc", "Option", "RefCell", "Mutex", "RwLock"];
    let mut text = written.trim().trim_start_matches('&').trim_start();
    if text.starts_with('\'') {
        text = text.split_once(' ')?.1;
    }
    text = text.strip_prefix("mut ").unwrap_or(text).trim_start();
    loop {
        let (head, rest) = text.split_once('<').unwrap_or((text, ""));
        let name = head.rsplit("::").next()?.trim();
        if !WRAPPERS.contains(&name) || rest.is_empty() {
            let valid = name.starts_with(char::is_uppercase)
                && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            return valid.then_some(name);
        }
        text = rest;
    }
}

/// The type a constructor call builds: `Type::new(…)`, and only the names that
/// by convention return their type.
fn constructed(value: &str) -> Option<&str> {
    let (ty, call) = value.split_once("::")?;
    let name = call.split('(').next()?;
    let builds = matches!(name, "new" | "default") || name.starts_with("with_");
    if builds && !call.contains(").") {
        rust_type(ty)
    } else {
        None
    }
}

/// `use crate::a::b;`, `use super::{c, d as e};`: names this crate defines.
/// A glob, `self` in a list and another crate's path are left out.
fn rust_uses(src: &str) -> Vec<Import> {
    fn expand(prefix: &str, body: &str, out: &mut Vec<Import>) {
        let body = body.trim();
        if let Some(open) = body.find('{') {
            let head = format!("{prefix}{}", &body[..open]);
            let inner = body[open + 1..]
                .strip_suffix('}')
                .unwrap_or(&body[open + 1..]);
            let (mut depth, mut start) = (0, 0);
            for (at, c) in inner.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 0 => {
                        expand(&head, &inner[start..at], out);
                        start = at + 1;
                    }
                    _ => {}
                }
            }
            expand(&head, &inner[start..], out);
            return;
        }
        let (path, alias) = match body.split_once(" as ") {
            Some((p, a)) => (p.trim(), Some(a.trim())),
            None => (body, None),
        };
        let full = format!("{prefix}{path}");
        let Some((spec, name)) = full.rsplit_once("::") else {
            return;
        };
        let ours = ["crate", "self", "super"].contains(&spec.split("::").next().unwrap_or(""));
        if ours && !matches!(name, "*" | "self" | "") && alias != Some("_") {
            out.push(Import::Name {
                local: alias.unwrap_or(name).to_string(),
                spec: spec.to_string(),
                name: name.to_string(),
            });
        }
    }
    let mut out = Vec::new();
    for line in src.lines() {
        let text = line.trim_start();
        let text = text.strip_prefix("pub ").unwrap_or(text);
        let text = match text.strip_prefix("pub(") {
            Some(rest) => rest.split_once(')').map_or(text, |(_, r)| r.trim_start()),
            None => text,
        };
        let Some(rest) = text.strip_prefix("use ") else {
            continue;
        };
        let start = offset(src, rest) as usize;
        let statement = src.get(start..).unwrap_or("");
        let lines = statement.split_inclusive('\n').take(STATEMENT_LINES);
        let statement = &statement[..lines.map(str::len).sum()];
        if let Some((body, _)) = statement.split_once(';') {
            let body: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
            expand(
                "",
                &body.replace(":: ", "::").replace(" ::", "::"),
                &mut out,
            );
        }
    }
    out
}

/// The file a Rust path inside the crate names: `crate::a::b` from any file,
/// `super::c` and `self::d` from `file`'s own module. `None` outside `src/`.
pub fn rust_path_target(file: &str, spec: &str, root: &Path) -> Option<String> {
    let path = Path::new(file);
    let crate_root = path
        .parent()?
        .ancestors()
        .find(|a| root.join(a).join("Cargo.toml").is_file())?;
    let src = crate_root.join("src");
    let mut module: Vec<String> = path
        .strip_prefix(&src)
        .ok()?
        .with_extension("")
        .iter()
        .map(|c| c.to_string_lossy().into_owned())
        .collect();
    if module.last().is_some_and(|m| m == "mod") || module == ["lib"] || module == ["main"] {
        module.pop();
    }
    for segment in spec.split("::") {
        match segment {
            "crate" => module.clear(),
            "self" => {}
            "super" => {
                module.pop()?;
            }
            name => module.push(name.to_string()),
        }
    }
    let candidates = if module.is_empty() {
        vec![src.join("lib.rs"), src.join("main.rs")]
    } else {
        let joined = module.join("/");
        vec![
            src.join(format!("{joined}.rs")),
            src.join(joined).join("mod.rs"),
        ]
    };
    candidates
        .into_iter()
        .find(|p| root.join(p).is_file())
        .map(|p| slash(&p))
}

/// The directories a Rust call in `file` can reach by a bare name: its own
/// crate and the crates its `Cargo.toml` names by `path`, directly or through
/// `[workspace.dependencies]`, `|` joined and relative to `root`. `None`
/// outside a crate.
pub fn rust_crate_scope(root: &Path, file: &str) -> Option<String> {
    let crate_dir = Path::new(file)
        .parent()?
        .ancestors()
        .find(|a| {
            std::fs::read_to_string(root.join(a).join("Cargo.toml"))
                .is_ok_and(|t| t.lines().any(|l| l.trim() == "[package]"))
        })?
        .to_path_buf();
    // `name = { path = "…" }` per dependency section; `name.workspace = true`
    // and `name = { workspace = true }` defer to the workspace's table.
    fn paths(toml: &str, sections: impl Fn(&str) -> bool) -> Vec<(String, Option<String>)> {
        let mut out = Vec::new();
        let mut inside = false;
        for line in toml.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = sections(line);
                continue;
            }
            let Some((name, value)) = line.split_once('=').filter(|_| inside) else {
                continue;
            };
            let name = name
                .trim()
                .trim_end_matches(".workspace")
                .trim()
                .to_string();
            let path = value
                .split_once("path")
                .and_then(|(_, rest)| rest.split('"').nth(1))
                .map(str::to_string);
            let deferred = line.contains("workspace");
            if path.is_some() || deferred {
                out.push((name, path));
            }
        }
        out
    }
    let own = std::fs::read_to_string(root.join(&crate_dir).join("Cargo.toml")).ok()?;
    let mut dirs = vec![crate_dir.clone()];
    let workspace = crate_dir
        .ancestors()
        .skip(1)
        .find(|a| {
            std::fs::read_to_string(root.join(a).join("Cargo.toml"))
                .is_ok_and(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        })
        .map(Path::to_path_buf);
    let shared = workspace
        .as_ref()
        .and_then(|w| std::fs::read_to_string(root.join(w).join("Cargo.toml")).ok())
        .map(|t| paths(&t, |h| h == "[workspace.dependencies]"))
        .unwrap_or_default();
    for (name, path) in paths(&own, |h| h.ends_with("dependencies]")) {
        let dir = match path {
            Some(p) => Some(crate_dir.join(p)),
            None => shared
                .iter()
                .find(|(n, _)| *n == name)
                .and_then(|(_, p)| Some(workspace.as_ref()?.join(p.as_ref()?))),
        };
        if let Some(dir) = dir {
            // `crates/a/../b` as `crates/b`, without touching the disk.
            let mut clean = std::path::PathBuf::new();
            for part in dir.components() {
                match part {
                    std::path::Component::ParentDir => {
                        clean.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => clean.push(other),
                }
            }
            dirs.push(clean);
        }
    }
    // The tree's root as `.`: an empty list means no crate at all.
    let named = |d: &std::path::PathBuf| match slash(d) {
        root if root.is_empty() => ".".to_string(),
        dir => dir,
    };
    Some(dirs.iter().map(named).collect::<Vec<_>>().join("|"))
}

/// The Rust files among `files` no crate builds: no `mod` from a crate's
/// roots (`src/lib.rs`, `src/main.rs`, `src/bin/`, `tests/`, `examples/`,
/// `benches/`, `build.rs`, a target's `path`) reaches them. `#[path]`,
/// inline `mod x { mod y; }` and `include!("…")` are followed; a module named
/// by a macro is not, so such a file shows here although it is built.
pub fn rust_outside_modules(root: &Path, files: &[&str]) -> Vec<String> {
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    let mut crates: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut crate_of: std::collections::HashMap<String, Option<String>> = Default::default();
    for &file in files.iter().filter(|f| f.ends_with(".rs")) {
        let dir = file.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let found = crate_of
            .entry(dir.clone())
            .or_insert_with(|| {
                Path::new(&dir)
                    .ancestors()
                    .find(|a| {
                        std::fs::read_to_string(root.join(a).join("Cargo.toml"))
                            .is_ok_and(|t| t.lines().any(|l| l.trim() == "[package]"))
                    })
                    .map(slash)
            })
            .clone();
        if let Some(c) = found {
            crates.entry(c).or_default().push(file);
        }
    }
    let mut outside = Vec::new();
    for (krate, members) in crates {
        let at = |rel: &str| {
            if krate.is_empty() {
                rel.to_string()
            } else {
                format!("{krate}/{rel}")
            }
        };
        let known: HashSet<&str> = members.iter().copied().collect();
        let mut roots: BTreeSet<String> = members
            .iter()
            .filter(|f| {
                let rel = f.strip_prefix(&at("")).unwrap_or(f);
                let parts: Vec<&str> = rel.split('/').collect();
                matches!(
                    parts[..],
                    ["src", "lib.rs" | "main.rs"]
                        | ["build.rs"]
                        | ["src", "bin", _]
                        | ["src", "bin", _, "main.rs"]
                        | ["tests" | "examples" | "benches", _]
                        | ["examples" | "benches" | "tests", _, "main.rs"]
                )
            })
            .map(|f| f.to_string())
            .collect();
        // A target's own `path` in `[lib]`, `[[bin]]`, `[[test]]`, ….
        if let Ok(toml) = std::fs::read_to_string(root.join(&krate).join("Cargo.toml")) {
            let mut target = false;
            for line in toml.lines().map(str::trim) {
                if line.starts_with('[') {
                    target = matches!(
                        line,
                        "[lib]" | "[[bin]]" | "[[test]]" | "[[example]]" | "[[bench]]"
                    );
                } else if target
                    && let Some(value) = line
                        .strip_prefix("path")
                        .and_then(|r| r.trim_start().strip_prefix('='))
                    && let Some(path) = value.split('"').nth(1)
                {
                    roots.insert(at(path));
                }
            }
        }
        let mut reached: HashSet<String> = HashSet::new();
        let mut waiting: Vec<(String, bool)> = roots.into_iter().map(|r| (r, true)).collect();
        while let Some((file, is_root)) = waiting.pop() {
            // `include!` and `#[path]` name any path, and the tree chooses it:
            // only one inside the tree, a regular file of source size, is read.
            let Some(file) = within(&file) else {
                continue;
            };
            if !reached.insert(file.clone()) {
                continue;
            }
            let readable = std::fs::metadata(root.join(&file))
                .is_ok_and(|m| m.is_file() && m.len() <= 4 << 20);
            let Some(text) = readable
                .then(|| std::fs::read_to_string(root.join(&file)).ok())
                .flatten()
            else {
                continue;
            };
            let path = Path::new(&file);
            let parent = path.parent().unwrap_or(Path::new(""));
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // Where `mod x;` looks: beside a root or `mod.rs`, else in `stem/`.
            let base = if is_root || stem == "mod" {
                parent.to_path_buf()
            } else {
                parent.join(stem)
            };
            let mut inline: Vec<(String, i32)> = Vec::new();
            let mut depth = 0i32;
            let mut pinned: Option<String> = None;
            for line in text.lines() {
                let code = line.split("//").next().unwrap_or("").trim();
                if let Some(rest) = code.strip_prefix("#[path") {
                    pinned = rest.split('"').nth(1).map(str::to_string);
                }
                if let Some(at_include) = code.find("include!(\"")
                    && let Some(name) = code[at_include + 10..].split('"').next()
                {
                    waiting.push((slash(&parent.join(name)), false));
                }
                let words: Vec<&str> = code.split_whitespace().collect();
                let at_mod = words.iter().position(|w| *w == "mod");
                if let Some(i) = at_mod.filter(|&i| {
                    words[..i]
                        .iter()
                        .all(|w| w.starts_with("pub") || w.starts_with("#["))
                }) {
                    let name = words
                        .get(i + 1)
                        .map_or("", |w| w.trim_end_matches([';', '{']));
                    let opens = code.ends_with('{') || words.get(i + 2) == Some(&"{");
                    if opens {
                        inline.push((name.to_string(), depth + 1));
                    } else if code.ends_with(';') && !name.is_empty() {
                        let mut dir = base.clone();
                        for (n, _) in &inline {
                            dir.push(n);
                        }
                        let candidates = match pinned.take() {
                            Some(p) => vec![parent.join(p)],
                            None => vec![
                                dir.join(format!("{name}.rs")),
                                dir.join(name).join("mod.rs"),
                            ],
                        };
                        for c in candidates {
                            let c = slash(&c);
                            if known.contains(c.as_str()) || root.join(&c).is_file() {
                                waiting.push((c, false));
                            }
                        }
                    }
                }
                depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
                while inline.last().is_some_and(|(_, d)| depth < *d) {
                    inline.pop();
                }
            }
        }
        outside.extend(
            members
                .iter()
                .filter(|f| !reached.contains(**f))
                .map(|f| f.to_string()),
        );
    }
    outside.sort();
    outside
}

/// The plain name a value refers to: `&mut self.data`, `&a`, `b`.
fn named(value: &str) -> Option<&str> {
    let text = value.trim().trim_start_matches('&').trim_start();
    let text = text.strip_prefix("mut ").unwrap_or(text).trim();
    let text = text.strip_prefix("self.").unwrap_or(text);
    let plain = !text.is_empty()
        && text.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    plain.then_some(text)
}

/// `name: Type` in fields, parameters and `let`, and `let name = Type::new()`.
/// A name declared with two types is left out, as for Java.
fn rust_types(src: &str) -> Vec<Import> {
    type Seen<'a> = Vec<(&'a str, Option<&'a str>)>;
    let mut seen: Seen = Vec::new();
    // Names written with a type that is no receiver (`s: &str`): an alias of
    // or to such a name elsewhere in the file stays untyped.
    let mut written: Vec<&str> = Vec::new();
    // A name bound to other names, typed once those are.
    let mut aliases: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut in_struct = false;
    for line in src.lines() {
        let text = line.split("//").next().unwrap_or("").trim();
        if in_struct {
            if text.starts_with('}') {
                in_struct = false;
                continue;
            }
            let field = match text.strip_prefix("pub(") {
                Some(rest) => rest.split_once(')').map_or(text, |(_, f)| f.trim_start()),
                None => text.strip_prefix("pub ").unwrap_or(text),
            };
            if let Some((name, ty)) = field.split_once(':') {
                match rust_type(ty.trim_end_matches(',')) {
                    Some(t) => seen.push((name.trim(), Some(t))),
                    None => written.push(name.trim()),
                }
            }
        } else if text.split_whitespace().any(|w| w == "struct") && text.ends_with('{') {
            in_struct = true;
        } else if let Some(rest) = text.strip_prefix("let ") {
            let rest = rest.strip_prefix("mut ").unwrap_or(rest);
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            // A written type counts even when it is no receiver (`n: u32`);
            // an unwritten one only when the value names it.
            let written_type = name.split_once(':');
            let (name, ty) = match written_type {
                Some((name, ty)) => (name.trim(), rust_type(ty)),
                None => (name.trim(), constructed(value)),
            };
            // An alias only when the statement ends here: `let set = b` may
            // go on with `.build()` on the next line.
            let alias = value.trim().strip_suffix(';').and_then(named);
            match (ty, alias) {
                (Some(t), _) => seen.push((name, Some(t))),
                (None, Some(source)) if written_type.is_none() && named(name).is_some() => {
                    aliases.push((name, vec![source]))
                }
                _ if written_type.is_some() => written.push(name),
                _ => {}
            }
        } else if let Some((pattern, list)) =
            text.strip_prefix("for ").and_then(|r| r.split_once(" in "))
        {
            // `for bank in [&mut self.data, &mut self.inst]`.
            let list = list.trim_end_matches('{').trim();
            let items = list
                .strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
                .and_then(|items| items.split(',').map(named).collect::<Option<Vec<_>>>());
            let name = pattern.trim().trim_start_matches("mut ");
            if let Some(items) = items.filter(|_| named(name).is_some()) {
                aliases.push((name, items));
            }
        } else if let Some(at) = text.find("fn ") {
            let list = text[at..].split_once('(').map_or("", |(_, l)| l);
            let list = list.rsplit_once(')').map_or(list, |(l, _)| l);
            for parameter in list.split(',') {
                if let Some((name, ty)) = parameter.split_once(':') {
                    let name = name.trim().trim_start_matches("mut ");
                    match rust_type(ty) {
                        Some(t) => seen.push((name, Some(t))),
                        None => written.push(name),
                    }
                }
            }
        }
    }
    fn merge<'a>(types: &mut std::collections::BTreeMap<&'a str, Option<&'a str>>, seen: Seen<'a>) {
        for (name, ty) in seen {
            let known = types.entry(name).or_insert(ty);
            if *known != ty {
                *known = None;
            }
        }
    }
    let mut types = std::collections::BTreeMap::new();
    merge(&mut types, seen);
    // Typed only where every source has the same known type.
    let typed: Seen = aliases
        .iter()
        .filter(|(name, sources)| {
            !written.contains(name) && !sources.iter().any(|s| written.contains(s))
        })
        .filter_map(|(name, sources)| {
            let mut tys = sources.iter().map(|s| types.get(s).copied().flatten());
            let first = tys.next().flatten()?;
            tys.all(|t| t == Some(first))
                .then_some((*name, Some(first)))
        })
        .collect();
    merge(&mut types, typed);
    types
        .into_iter()
        .filter_map(|(alias, ty)| {
            ty.map(|ty| Import::Module {
                alias: alias.to_string(),
                spec: ty.to_string(),
            })
        })
        .collect()
}

/// `class A(B, mod.C, metaclass=M):`, whose list may span lines.
fn python_bases(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("class ") else {
            continue;
        };
        let Some(open) = rest.find('(').filter(|o| !rest[..*o].contains(':')) else {
            continue;
        };
        // Python has no word for a mixin but the one its authors use.
        if rest[..open].ends_with("Mixin") {
            out.push(Import::Mixin {
                name: rest[..open].to_string(),
            });
        }
        let start = offset(src, rest) as usize + open + 1;
        let text = src.get(start..).unwrap_or("");
        let lines = text.split_inclusive('\n').take(STATEMENT_LINES);
        let mut depth = 1;
        let mut end = None;
        'text: for (at, c) in text[..lines.map(str::len).sum()].char_indices() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at);
                        break 'text;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            out.extend(listed_bases(src, &text[..end]));
        }
    }
    out
}

/// `class A extends B`, and `use C, D;` inside a class body.
fn php(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("use ") {
            if line.starts_with(char::is_whitespace) {
                let names = statement(src, rest).split([';', '{']).next().unwrap_or("");
                out.extend(names.split(',').filter_map(|n| base(src, n)));
            }
        } else if !trimmed.starts_with(['*', '/', '#'])
            && trimmed.split_whitespace().any(|w| w == "class")
        {
            let from = trimmed.find("class").unwrap_or(0);
            out.extend(head_bases(
                src,
                &trimmed[from..],
                &["extends"],
                &["implements"],
            ));
        }
    }
    out
}

/// `class A < B`, `include C, D`, `extend E`, `prepend F`.
fn ruby(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        if let Some((_, parent)) = line
            .strip_prefix("class ")
            .and_then(|rest| rest.split_once(" < "))
        {
            out.extend(base(src, parent.split_whitespace().next().unwrap_or("")));
        }
        for keyword in ["include ", "extend ", "prepend "] {
            if let Some(names) = line.strip_prefix(keyword) {
                let names = statement(src, names);
                out.extend(names.split(',').filter_map(|n| base(src, n)));
            }
        }
    }
    out
}

/// `class A extends B with C, D implements E`.
fn dart(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines().map(str::trim) {
        if line.starts_with(['*', '/']) {
            continue;
        }
        let head = line
            .find("class ")
            .or(line.starts_with("mixin ").then_some(0));
        if let Some(head) = head {
            out.extend(head_bases(
                src,
                &line[head..],
                &["extends", "with", "on"],
                &["implements"],
            ));
        }
    }
    out
}

/// `alias Shop.Cart` makes `Cart` mean `Shop.Cart`, `, as: C` names it `C`,
/// and `alias Shop.{Cart, Order}` reads as one alias each, over several lines.
fn elixir(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut push = |spec: &str, alias: &str| {
        if !spec.is_empty()
            && !alias.is_empty()
            && spec
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '_')
        {
            out.push(Import::Module {
                alias: alias.to_string(),
                spec: spec.to_string(),
            });
        }
    };
    let last = |spec: &'_ str| spec.rsplit('.').next().unwrap_or("").to_string();
    let mut lines = src.lines();
    let mut in_heredoc = false;
    let mut bases = Vec::new();
    let mut names_imported = Vec::new();
    while let Some(line) = lines.next() {
        // A `@doc` example shows an alias its module does not have.
        if line.matches("\"\"\"").count() % 2 == 1 {
            in_heredoc = !in_heredoc;
            continue;
        }
        if in_heredoc {
            continue;
        }
        if let Some(rest) = line.trim().strip_prefix("import ") {
            let rest = statement(src, rest);
            if let Some((prefix, names)) = rest.split_once(".{") {
                let names = names.split('}').next().unwrap_or("");
                for name in names.split(',').map(str::trim) {
                    bases.push(Import::Base {
                        module: format!("{}.{name}", prefix.trim()),
                        at: offset(src, name),
                    });
                }
                continue;
            }
            let (spec, options) = rest.split_once(',').unwrap_or((rest, ""));
            let spec = spec.trim();
            let plain = spec
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '_');
            let only = options.split_once("only: [").map(|(_, names)| names);
            match only {
                Some(names) if plain => {
                    let names = names.split(']').next().unwrap_or("");
                    for name in names.split(',').filter_map(|n| n.split(':').next()) {
                        let name = name.trim().to_string();
                        names_imported.push(Import::Name {
                            local: name.clone(),
                            spec: spec.to_string(),
                            name,
                        });
                    }
                }
                // Options are the reader's limit: what they exclude, or what
                // the next line says, is not known here.
                None if plain && !rest.contains(',') => bases.push(Import::Base {
                    module: spec.to_string(),
                    at: offset(src, spec),
                }),
                _ => {}
            }
            continue;
        }
        let Some(rest) = line.trim().strip_prefix("alias ") else {
            continue;
        };
        if let Some((base, names)) = rest.split_once(".{") {
            let mut names = names.to_string();
            while !names.contains('}') {
                let Some(next) = lines.next() else { break };
                names.push_str(next);
            }
            let names = names.split('}').next().unwrap_or("");
            for name in names.split(',').map(str::trim) {
                push(&format!("{}.{name}", base.trim()), &last(name));
            }
            continue;
        }
        let (spec, options) = rest.split_once(',').unwrap_or((rest, ""));
        let spec = spec.trim();
        let alias = match options.split_once("as:") {
            Some((_, alias)) => alias.split(',').next().unwrap_or("").trim().to_string(),
            None => last(spec),
        };
        push(spec, &alias);
    }
    out.append(&mut bases);
    out.append(&mut names_imported);
    out
}

/// Java states a variable's type where it declares it, so a call through a
/// variable names the method in the class the declaration gives. Fields, parameters, locals and loop variables are read alike, as
/// `Type name` followed by `=`, `;`, `,`, `)` or `:`. A name declared with two
/// types in one file is left out: without scopes, either could be meant.
fn java(src: &str) -> Vec<Import> {
    let words = java_tokens(src);
    let mut types: std::collections::BTreeMap<&str, Option<&str>> = Default::default();
    let mut i = 0;
    while i + 1 < words.len() {
        let ty = words[i];
        if !ty.starts_with(|c: char| c.is_ascii_uppercase()) {
            i += 1;
            continue;
        }
        // Skip type arguments and array brackets between the type and the name.
        let mut j = i + 1;
        let mut depth = 0;
        while j < words.len() {
            match words[j] {
                "<" => depth += 1,
                ">" => depth -= 1,
                "[" | "]" => {}
                _ if depth > 0 => {}
                _ => break,
            }
            j += 1;
        }
        let (Some(name), Some(next)) = (words.get(j), words.get(j + 1)) else {
            break;
        };
        if name.starts_with(|c: char| c.is_ascii_lowercase())
            && matches!(*next, "=" | ";" | "," | ")" | ":")
        {
            let seen = types.entry(name).or_insert(Some(ty));
            if *seen != Some(ty) {
                *seen = None;
            }
        }
        i += 1;
    }
    types
        .into_iter()
        .filter_map(|(alias, ty)| {
            ty.map(|ty| Import::Module {
                alias: alias.to_string(),
                spec: ty.to_string(),
            })
        })
        .collect()
}

/// Kotlin states a type after the name, `name: Type`, for parameters and
/// properties, and infers the rest; a property built by a constructor call,
/// `val name = Type(`, is the inferred case a declaration shows. A name given
/// two types in one file is left out, as for Java.
fn kotlin(src: &str) -> Vec<Import> {
    let words = java_tokens(src);
    let mut types: std::collections::BTreeMap<&str, Option<&str>> = Default::default();
    let upper = |w: &str| w.starts_with(|c: char| c.is_ascii_uppercase());
    let lower = |w: &str| w.starts_with(|c: char| c.is_ascii_lowercase());
    for (i, w) in words.iter().enumerate() {
        let declared = match (words.get(i + 1), words.get(i + 2), words.get(i + 3)) {
            (Some(&":"), Some(ty), _) if lower(w) && upper(ty) => Some(*ty),
            (Some(&"="), Some(ty), Some(next))
                if lower(w)
                    && upper(ty)
                    && matches!(*next, "(" | "<")
                    && i > 0
                    && matches!(words[i - 1], "val" | "var") =>
            {
                Some(*ty)
            }
            _ => None,
        };
        if let Some(ty) = declared {
            let seen = types.entry(w).or_insert(Some(ty));
            if *seen != Some(ty) {
                *seen = None;
            }
        }
    }
    types
        .into_iter()
        .filter_map(|(alias, ty)| {
            ty.map(|ty| Import::Module {
                alias: alias.to_string(),
                spec: ty.to_string(),
            })
        })
        .collect()
}

/// Identifiers and single punctuation of a Java source, with comments and
/// string and character literals removed.
fn java_tokens(src: &str) -> Vec<&str> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] >= 0x80)
            {
                i += 1;
            }
            out.push(&src[start..i]);
        } else {
            if !c.is_ascii_whitespace() {
                out.push(&src[i..i + 1]);
            }
            i += 1;
        }
    }
    out
}

fn python(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    // A parenthesised `from x import (` list runs to its closing paren.
    let mut joined = String::new();
    let mut open = false;
    for line in src.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if open {
            joined.push(' ');
            joined.push_str(line);
            if line.contains(')') {
                open = false;
                python_statement(&joined, &mut out);
            }
            continue;
        }
        if line.starts_with("from ") && line.contains("import (") && !line.contains(')') {
            joined = line.to_string();
            open = true;
            continue;
        }
        python_statement(line, &mut out);
    }
    out
}

fn python_statement(line: &str, out: &mut Vec<Import>) {
    let line = line.trim_end_matches('\\').trim();
    if let Some(rest) = line.strip_prefix("import ") {
        for part in rest.split(',') {
            let mut words = part.split_whitespace();
            let Some(spec) = words.next() else { continue };
            let alias = match (words.next(), words.next()) {
                (Some("as"), Some(a)) => a.to_string(),
                // `import a.b.c` binds `a`, and a call reads `a.b.c.f()`: the
                // receiver the scanner sees is `c`, so both are recorded.
                _ => {
                    let first = spec.split('.').next().unwrap_or(spec);
                    if first != spec {
                        out.push(Import::Module {
                            alias: first.to_string(),
                            spec: first.to_string(),
                        });
                    }
                    spec.rsplit('.').next().unwrap_or(spec).to_string()
                }
            };
            out.push(Import::Module {
                alias,
                spec: spec.to_string(),
            });
        }
    } else if let Some(rest) = line.strip_prefix("from ")
        && let Some((spec, names)) = rest.split_once(" import ")
    {
        let spec = spec.trim();
        let names = names.trim().trim_start_matches('(').trim_end_matches(')');
        for part in names.split(',') {
            let mut words = part.split_whitespace();
            let Some(name) = words.next() else { continue };
            if name == "*" {
                continue;
            }
            let local = match (words.next(), words.next()) {
                (Some("as"), Some(a)) => a,
                _ => name,
            };
            // `from pkg import mod` imports a module as often as a name; which
            // one it is, only the tree can say, so both are offered and
            // `resolve` keeps whichever exists.
            let sep = if spec.ends_with('.') { "" } else { "." };
            out.push(Import::Module {
                alias: local.to_string(),
                spec: format!("{spec}{sep}{name}"),
            });
            out.push(Import::Name {
                local: local.to_string(),
                spec: spec.to_string(),
                name: name.to_string(),
            });
        }
    }
}

fn go(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    let mut block = false;
    for line in src.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let spec_line = if block {
            if line.starts_with(')') {
                block = false;
                continue;
            }
            line
        } else if let Some(rest) = line.strip_prefix("import") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                block = true;
                continue;
            }
            rest
        } else {
            continue;
        };
        let Some(q) = spec_line.find('"') else {
            continue;
        };
        let spec = spec_line[q + 1..].split('"').next().unwrap_or("");
        let named = spec_line[..q].trim();
        if named == "_" || named == "." || spec.is_empty() {
            continue;
        }
        let alias = if named.is_empty() {
            // The package name is the last path element, before a `/v2`
            // major-version suffix.
            let mut parts = spec.rsplit('/');
            let last = parts.next().unwrap_or(spec);
            if last.len() > 1
                && last.starts_with('v')
                && last[1..].chars().all(|c| c.is_ascii_digit())
            {
                parts.next().unwrap_or(last)
            } else {
                last
            }
        } else {
            named
        };
        out.push(Import::Module {
            alias: alias.to_string(),
            spec: spec.to_string(),
        });
    }
    out
}

fn javascript(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    // An import statement may span lines until its `from '…'`.
    let mut stmt = String::new();
    for line in src.lines() {
        let line = line.trim();
        if stmt.is_empty() && !(line.starts_with("import ") || line.contains("require(")) {
            continue;
        }
        stmt.push(' ');
        stmt.push_str(line);
        let done = line.contains(" from ")
            || line.contains("require(")
            || line.starts_with("import '")
            || line.starts_with("import \"")
            || line.ends_with(';');
        if !done {
            continue;
        }
        javascript_statement(stmt.trim(), &mut out);
        stmt.clear();
    }
    out
}

fn quoted(s: &str) -> Option<&str> {
    let start = s.find(['\'', '"', '`'])?;
    let q = s.as_bytes()[start] as char;
    s[start + 1..].split(q).next()
}

fn javascript_statement(stmt: &str, out: &mut Vec<Import>) {
    let (clause, spec) = if let Some(rest) = stmt.strip_prefix("import ") {
        let Some((clause, from)) = rest.rsplit_once(" from ") else {
            return;
        };
        (
            clause.trim().trim_start_matches("type ").to_string(),
            quoted(from),
        )
    } else if let Some(at) = stmt.find("require(") {
        // `const x = require('./y')`, `const { a, b: c } = require('./y')`.
        let lhs = stmt[..at].trim().trim_end_matches('=').trim();
        let lhs = ["const ", "let ", "var ", ", "]
            .iter()
            .find_map(|k| lhs.strip_prefix(k))
            .unwrap_or("");
        let spec = quoted(&stmt[at..]);
        if let (Some(spec), Some(name)) = (spec, require_member(&stmt[at..])) {
            if !lhs.is_empty() && !lhs.contains(['{', ' ']) {
                out.push(Import::Name {
                    local: lhs.to_string(),
                    spec: spec.to_string(),
                    name: name.to_string(),
                });
            }
            return;
        }
        (lhs.replace(':', " as "), spec)
    } else {
        return;
    };
    let Some(spec) = spec else { return };
    let spec = spec.to_string();
    let clause = clause.trim();
    if let Some(ns) = clause.strip_prefix("* as ") {
        out.push(Import::Module {
            alias: ns.trim().to_string(),
            spec,
        });
        return;
    }
    if let (Some(open), Some(close)) = (clause.find('{'), clause.rfind('}')) {
        for part in clause[open + 1..close].split(',') {
            let mut words = part.split_whitespace().filter(|w| *w != "type");
            let Some(name) = words.next() else { continue };
            let local = match (words.next(), words.next()) {
                (Some("as"), Some(a)) => a,
                _ => name,
            };
            out.push(Import::Name {
                local: local.to_string(),
                spec: spec.clone(),
                name: name.to_string(),
            });
        }
    } else if !clause.is_empty() && !clause.contains(['{', ',', ' ']) {
        // A default import or a whole `require`: `x.f()` means `f` in the
        // module when it exports an object of functions.
        out.push(Import::Module {
            alias: clause.to_string(),
            spec: spec.clone(),
        });
        // The module's own export is usually called what it is bound to.
        out.push(Import::Name {
            local: clause.to_string(),
            spec,
            name: clause.to_string(),
        });
    }
}

/// The `name` of `require('./y').name`, when the statement ends there.
fn require_member(call: &str) -> Option<&str> {
    let (_, rest) = call.split_once(')')?;
    let rest = rest.strip_prefix('.')?;
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(rest.len());
    let tail = rest[end..].trim_start();
    (end > 0 && (tail.is_empty() || tail.starts_with([';', ',']))).then(|| &rest[..end])
}

/// Where `spec`, imported from the root-relative `file`, lives in the tree:
/// a root-relative file, or for Go a package directory ending in `/`. `None`
/// for a module outside the tree — the standard library, a dependency.
pub fn resolve(lang: Lang, file: &str, spec: &str, root: &Path) -> Option<String> {
    let dir = Path::new(file).parent().unwrap_or(Path::new(""));
    match lang {
        Lang::Python => python_target(dir, spec, root),
        Lang::Go => go_target(dir, spec, root),
        Lang::TypeScript | Lang::JavaScript => js_target(dir, spec, root),
        Lang::Elixir => module_file(lang, spec),
        Lang::Java | Lang::Kotlin | Lang::Rust => Some(format!("@{spec}")),
        _ => None,
    }
}

fn python_target(dir: &Path, spec: &str, root: &Path) -> Option<String> {
    let dots = spec.len() - spec.trim_start_matches('.').len();
    let parts: Vec<&str> = spec[dots..].split('.').filter(|p| !p.is_empty()).collect();
    let candidates = |base: &Path| -> Vec<PathBuf> {
        let mut p = base.to_path_buf();
        for part in &parts {
            p.push(part);
        }
        if parts.is_empty() {
            vec![p.join("__init__.py")]
        } else {
            vec![p.with_extension("py"), p.join("__init__.py")]
        }
    };
    let bases: Vec<PathBuf> = if dots > 0 {
        let mut base = dir.to_path_buf();
        for _ in 1..dots {
            base.pop();
        }
        vec![base]
    } else {
        // An absolute import is rooted at whichever ancestor holds the
        // top-level package: the tree root, `src/`, or a service directory in
        // a monorepo. The nearest one that has it wins.
        dir.ancestors().map(Path::to_path_buf).collect()
    };
    bases
        .iter()
        .flat_map(|b| candidates(b))
        .find(|c| root.join(c).is_file())
        .map(|c| slash(&c))
}

fn go_target(dir: &Path, spec: &str, root: &Path) -> Option<String> {
    // The module path comes from the nearest go.mod above the file.
    for base in dir.ancestors() {
        let Ok(text) = std::fs::read_to_string(root.join(base).join("go.mod")) else {
            continue;
        };
        let module = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))?
            .trim()
            .trim_matches('"');
        let rest = spec.strip_prefix(module)?.trim_start_matches('/');
        let target = base.join(rest);
        return root.join(&target).is_dir().then(|| {
            let s = slash(&target);
            if s.is_empty() {
                String::new()
            } else {
                format!("{s}/")
            }
        });
    }
    None
}

fn js_target(dir: &Path, spec: &str, root: &Path) -> Option<String> {
    if !spec.starts_with('.') {
        return None;
    }
    let base = normalize(&dir.join(spec));
    let stem = base.to_string_lossy().trim_end_matches(".js").to_string();
    let mut candidates = vec![base.clone()];
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        // ESM TypeScript writes `./y.js` for a file that is `y.ts` on disk.
        candidates.push(PathBuf::from(format!("{stem}.{ext}")));
        candidates.push(base.join(format!("index.{ext}")));
    }
    candidates
        .into_iter()
        .find(|c| root.join(c).is_file())
        .map(|c| slash(&c))
}

/// `a/./b/../c` to `a/c`, without touching the file system.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` with `.` and `..` resolved, or `None` when it is absolute or leaves
/// the tree.
pub(crate) fn within(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in Path::new(path).components() {
        match c {
            std::path::Component::Normal(p) => parts.push(p.to_str()?),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Where a module-qualified call goes in a language whose module *is* a file
/// by name, wherever it lives: Erlang's `lists:map` is `map` in `lists.erl`,
/// which the compiler enforces. `*/` marks a target matched by file name;
/// tier 3 links it only when exactly one such file defines the name.
///
/// Elixir names a module by `defmodule`, not by its file: `@Shop.Cart` is the
/// one file defining that module, which tier 3 finds by the definition.
pub fn module_file(lang: Lang, module: &str) -> Option<String> {
    match lang {
        Lang::Erlang => Some(format!("*/{module}.erl")),
        Lang::Elixir => Some(format!("@{module}")),
        // `Billing.total()` goes to the one file defining `Billing`.
        Lang::Ruby | Lang::Php | Lang::Dart | Lang::Swift
            if module.starts_with(|c: char| c.is_uppercase()) =>
        {
            Some(format!("@{module}"))
        }
        _ => None,
    }
}

/// The file a call's module qualifier names, for a language whose modules
/// are files: `module_file` where the name alone decides, and for Rust the
/// path, since `commits::changes()` means `commits.rs` beside the caller or
/// `src/commits.rs` of the crate (`mod.rs` either way). Without it a
/// module-qualified call fell back to the bare name and stayed unlinked
/// whenever another file defined that name too.
pub fn module_target(lang: Lang, file: &str, module: &str, root: &Path) -> Option<String> {
    if lang != Lang::Rust {
        return module_file(lang, module);
    }
    // `crate::a::b`, `super::c`: the path from the crate root or this module.
    if ["crate", "self", "super"].contains(&module.split("::").next().unwrap_or("")) {
        return rust_path_target(file, module, root);
    }
    // Otherwise the last module, beside the caller or under `src/`.
    let module = module.rsplit("::").next().unwrap_or(module);
    let dir = Path::new(file).parent().unwrap_or(Path::new(""));
    let mut bases = vec![dir.to_path_buf()];
    if let Some(crate_root) = dir
        .ancestors()
        .find(|a| root.join(a).join("Cargo.toml").is_file())
    {
        bases.push(crate_root.join("src"));
    }
    bases
        .iter()
        .flat_map(|b| {
            [
                b.join(format!("{module}.rs")),
                b.join(module).join("mod.rs"),
            ]
        })
        .find(|p| root.join(p).is_file())
        .map(|p| slash(&p))
}

/// The placeholder for a name an import says lives at `target`: `f from
/// path/to/file.py`. A placeholder never contains a space, so the form cannot
/// collide with one written by any scanner, and it reads as what it is.
pub fn scoped(name: &str, target: &str) -> String {
    format!("{name} from {target}")
}

/// The name and target of a scoped placeholder, `None` for a bare one.
pub fn unscope(placeholder: &str) -> Option<(&str, &str)> {
    placeholder.split_once(" from ")
}
