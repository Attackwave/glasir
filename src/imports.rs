//! What a name in a file refers to elsewhere in the tree, read from its imports.
//!
//! A call through a module — `mainmod.main()` in Python, `binding.New()` in
//! Go, `utils.parse()` in TypeScript — reached a bare placeholder, and tier 3
//! refuses a name defined in several places, so the call linked to nothing.
//! The import line names the file, which makes the target decidable rather
//! than guessed. Python, Go and JavaScript/TypeScript write a module-qualified
//! call as `alias.name()`, and their import syntax is regular enough to read
//! line by line. Elixir's `alias` reads the same way, and Java's declarations
//! do: a variable's declared type names the class a call through it reaches.

use crate::parse_ast::Lang;
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
}

/// The import statements of one file.
pub fn read(lang: Lang, src: &str) -> Vec<Import> {
    match lang {
        Lang::Python => python(src),
        Lang::Go => go(src),
        Lang::TypeScript | Lang::JavaScript => javascript(src),
        Lang::Elixir => elixir(src),
        Lang::Java => java(src),
        _ => Vec::new(),
    }
}

/// `alias Shop.Cart` makes `Cart` mean `Shop.Cart`; `, as: C` names it `C`.
fn elixir(src: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim().strip_prefix("alias ") else {
            continue;
        };
        let (spec, alias) = match rest.split_once(", as:") {
            Some((spec, alias)) => (spec.trim(), alias.trim()),
            None => (rest.trim(), rest.trim().rsplit('.').next().unwrap_or("")),
        };
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
    }
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
        let lhs = ["const ", "let ", "var "]
            .iter()
            .find_map(|k| lhs.strip_prefix(k))
            .unwrap_or("");
        let braces = lhs.replace(':', " as ");
        (braces, quoted(&stmt[at..]))
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
            spec,
        });
    }
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
        Lang::Java => Some(format!("@{spec}")),
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
        _ => None,
    }
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
