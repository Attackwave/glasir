//! Versioned extraction rules, loaded explicitly and fixed for a process's life.

use crate::generic::{LangSpec, Opens};
use crate::lexer::{CommentStyle, TokenKind};
use crate::{FileFacts, Language};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;

pub const MAX_RULE_BYTES: usize = 64 * 1024;
// Bump when scanner semantics change, even if the rule schema does not.
const IMPLEMENTATION_VERSION: &str = "native-scanners/49";
/// Every language whose rules live in a file, with how it is scanned.
///
/// One table rather than three lists: the rule name, the `Language` it serves
/// and whether a `[generic]` table is required all have to agree, and keeping
/// them apart is how a language gets a file nothing reads. `native` names a
/// language whose syntax needs Rust — indentation, a two-word constant form,
/// position deciding which name is the call — and which therefore takes its
/// word lists from the file and nothing else.
struct Builtin {
    name: &'static str,
    language: Language,
    text: &'static str,
    native: bool,
}

macro_rules! builtins {
    ($(($name:literal, $lang:ident, $native:literal)),* $(,)?) => {
        [$(Builtin {
            name: $name,
            language: Language::$lang,
            text: include_str!(concat!("../languages/", $name, ".toml")),
            native: $native,
        }),*]
    };
}

const BUILTINS: [Builtin; 122] = builtins![
    // Native modules: syntax a description cannot carry.
    ("crystal", Crystal, false),
    ("cmake", Cmake, true),
    ("objc", ObjC, true),
    ("tcl", Tcl, true),
    ("janet", Janet, true),
    ("plsql", Plsql, true),
    ("apex", Apex, true),
    ("bicep", Bicep, false),
    ("puppet", Puppet, false),
    ("cuda", Cuda, true),
    ("makefile", Makefile, true),
    ("meson", Meson, true),
    ("jsonnet", Jsonnet, true),
    ("cfscript", Cfscript, false),
    ("smali", Smali, true),
    ("prisma", Prisma, false),
    ("soql", Soql, true),
    ("gotemplate", GoTemplate, true),
    ("liquid", Liquid, true),
    ("nasm", Nasm, true),
    ("assembly", Assembly, true),
    ("just", Just, true),
    ("hare", Hare, false),
    ("move", Move, false),
    ("squirrel", Squirrel, false),
    ("luau", Luau, true),
    ("teal", Teal, true),
    ("fennel", Fennel, true),
    ("jinja", Jinja, true),
    ("blade", Blade, true),
    ("rescript", ReScript, true),
    ("typst", Typst, true),
    ("objectscript", ObjectScript, false),
    ("qml", Qml, false),
    ("cairo", Cairo, false),
    ("llvm", LlvmIr, true),
    ("wolfram", Wolfram, true),
    ("cfml", Cfml, true),
    ("tlaplus", TlaPlus, true),
    ("arkts", ArkTs, true),
    ("templ", Templ, false),
    ("ispc", Ispc, true),
    ("chialisp", ChiaLisp, true),
    ("sosl", Sosl, true),
    ("agda", Agda, true),
    ("astro", Astro, true),
    ("slang", Slang, true),
    ("bitbake", BitBake, true),
    ("magma", Magma, true),
    ("pine", Pine, true),
    ("sway", Sway, false),
    ("smithy", Smithy, true),
    ("wit", Wit, true),
    ("mermaid", Mermaid, true),
    ("devicetree", DeviceTree, true),
    ("linkerscript", LinkerScript, true),
    ("gomod", GoMod, true),
    ("nickel", Nickel, true),
    ("pkl", Pkl, true),
    ("tablegen", TableGen, true),
    ("ron", Ron, true),
    ("beancount", Beancount, true),
    ("rst", Rst, true),
    ("bibtex", BibTeX, true),
    ("requirements", Requirements, true),
    ("gn", Gn, true),
    ("kconfig", Kconfig, true),
    ("properties", Properties, true),
    ("ini", Ini, true),
    ("dotenv", DotEnv, true),
    ("oracleforms", OracleForms, true),
    ("func", FunC, true),
    ("sshconfig", SshConfig, true),
    ("hyprlang", Hyprlang, true),
    ("awk", Awk, false),
    ("starlark", Starlark, false),
    ("pascal", Pascal, false),
    ("pony", Pony, false),
    ("vimscript", Vimscript, false),
    ("rust", Rust, true),
    ("ruby", Ruby, true),
    ("python", Python, true),
    ("java", Java, true),
    ("haskell", Haskell, true),
    ("ocaml", OCaml, true),
    ("scala", Scala, true),
    ("kotlin", Kotlin, true),
    ("julia", Julia, true),
    ("r", R, true),
    ("erlang", Erlang, true),
    ("nim", Nim, true),
    ("elixir", Elixir, true),
    ("gdscript", GdScript, true),
    ("typescript", TypeScript, true),
    ("javascript", JavaScript, true),
    ("csharp", CSharp, true),
    ("php", Php, true),
    ("vb", Vb, true),
    ("c", C, true),
    ("cpp", Cpp, true),
    ("swift", Swift, true),
    ("ada", Ada, true),
    ("d", D, true),
    ("clojure", Clojure, true),
    ("elm", Elm, true),
    ("purescript", PureScript, true),
    ("lean", Lean, true),
    ("dart", Dart, true),
    ("perl", Perl, true),
    ("lua", Lua, true),
    ("mojo", Mojo, true),
    ("fortran", Fortran, true),
    ("verilog", Verilog, true),
    // Driven entirely by their `[generic]` table.
    //
    // Absent on purpose: Bash, R, Nix, COBOL, Odin, Wat and Fish define by
    // *shape* rather than by a keyword — `name() {` and `name <- function(` —
    // which no list of words can express. They keep their hand-written
    // scanners; a rule file for them would be a description of something the
    // generic scanner cannot do. Lua is here too for a narrower reason: it
    // writes a constant as `local MAX = 3` and only at file scope, and a depth
    // condition is not something a word list carries — the self-check caught a
    // rule that would have made every function-local variable a definition.
    ("go", Go, false),
    ("zig", Zig, false),
    ("gleam", Gleam, false),
    ("graphql", GraphQL, false),
    ("thrift", Thrift, false),
    ("flatbuffers", FlatBuffers, false),
    ("capnproto", CapnProto, false),
    ("protobuf", Protobuf, false),
    ("solidity", Solidity, false),
];

fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    schema_version: u32,
    language: String,
    revision: u32,
    lexical: Lexical,
    calls: Calls,
    generic: Option<Generic>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lexical {
    line_comments: Vec<String>,
    doc_comments: Vec<String>,
    block_comment: Option<[String; 2]>,
    #[serde(default)]
    identifier_suffix_marks: bool,
    #[serde(default)]
    identifier_dashes: bool,
    /// See `CommentStyle::raw_escapes`.
    #[serde(default)]
    raw_string_escapes: bool,
    /// See `CommentStyle::quote_identifiers`.
    #[serde(default)]
    quote_identifiers: bool,
    /// Whether `name(` addresses memory rather than calls: assembly's
    /// `OFFSET(A2)` is a displacement, and taking it for a call hid every
    /// structure offset a routine reads from the uses a change breaks.
    #[serde(default)]
    parenthesis_is_operand: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Calls {
    exclude: Vec<String>,
}

impl Calls {
    pub(crate) fn allows(&self, word: &str) -> bool {
        !self.exclude.iter().any(|w| w == word)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    keyword: String,
    scope: Opens,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generic {
    definitions: Vec<Definition>,
    #[serde(default)]
    modifiers: Vec<String>,
    receivers: Vec<String>,
    #[serde(default)]
    block_open: Vec<String>,
    #[serde(default)]
    block_end: Vec<String>,
    braces: bool,
    #[serde(default)]
    bare_receiver_calls: bool,
    #[serde(default)]
    constant_by_case: bool,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// A comment marker, unlike a keyword, may hold a space: Haskell documents
/// with `-- |` and `-- ^`. Only control characters and a newline are refused —
/// a marker spanning lines would make the lexer's prefix test meaningless.
fn markers(name: &str, values: &[String]) -> io::Result<()> {
    if values.len() > 128 {
        return Err(invalid(format!("{name}: at most 128 entries are allowed")));
    }
    for value in values {
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(invalid(format!(
                "{name}: entries must be nonempty markers of at most 128 bytes"
            )));
        }
    }
    Ok(())
}

fn list(name: &str, values: &[String]) -> io::Result<()> {
    if values.len() > 128 {
        return Err(invalid(format!("{name}: at most 128 entries are allowed")));
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > 128
            || value.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(invalid(format!(
                "{name}: entries must be nonempty tokens of at most 128 bytes"
            )));
        }
        if !seen.insert(value) {
            return Err(invalid(format!("{name}: duplicate token {value:?}")));
        }
    }
    Ok(())
}

impl RuleFile {
    fn read(name: &str, text: &str) -> io::Result<Self> {
        let parse = || -> io::Result<Self> {
            if text.len() > MAX_RULE_BYTES {
                return Err(invalid("rule file exceeds 64 KiB"));
            }
            let rule: Self = toml::from_str(text).map_err(|e| invalid(e.to_string()))?;
            if rule.schema_version != 1 || rule.revision == 0 {
                return Err(invalid(
                    "schema_version must be 1 and revision must be positive",
                ));
            }
            let Some(entry) = builtin(name).filter(|_| rule.language == name) else {
                return Err(invalid(
                    "language must match the name of a supported rule file",
                ));
            };
            for (field, values) in [
                ("lexical.line_comments", &rule.lexical.line_comments),
                ("lexical.doc_comments", &rule.lexical.doc_comments),
            ] {
                markers(field, values)?;
            }
            list("calls.exclude", &rule.calls.exclude)?;
            if let Some(pair) = &rule.lexical.block_comment {
                markers("lexical.block_comment", pair)?;
            }
            match (entry.native, &rule.generic) {
                (false, Some(g)) => {
                    list(
                        "generic.definitions",
                        &g.definitions
                            .iter()
                            .map(|d| d.keyword.clone())
                            .collect::<Vec<_>>(),
                    )?;
                    if g.definitions.is_empty() {
                        return Err(invalid("generic.definitions must not be empty"));
                    }
                    for (field, values) in [
                        ("generic.modifiers", &g.modifiers),
                        ("generic.receivers", &g.receivers),
                        ("generic.block_open", &g.block_open),
                        ("generic.block_end", &g.block_end),
                    ] {
                        list(field, values)?;
                    }
                    if g.receivers
                        .iter()
                        .any(|v| ![".", "::", "->", "&.", "?->"].contains(&v.as_str()))
                    {
                        return Err(invalid(
                            "generic.receivers contains a token the lexer cannot emit",
                        ));
                    }
                    let mut roles = std::collections::HashSet::new();
                    for word in g
                        .definitions
                        .iter()
                        .map(|d| &d.keyword)
                        .chain(&g.modifiers)
                        .chain(&g.block_open)
                        .chain(&g.block_end)
                    {
                        if !roles.insert(word) {
                            return Err(invalid(format!(
                                "generic: conflicting roles for {word:?}"
                            )));
                        }
                    }
                }
                (false, None) => {
                    return Err(invalid("a generic language requires a [generic] table"))
                }
                (true, Some(_)) => {
                    return Err(invalid(
                        "this language uses a native syntax module, not a [generic] table",
                    ))
                }
                (true, None) => {}
            }
            Ok(rule)
        };
        parse().map_err(|e| invalid(format!("{name}.toml: {e}")))
    }

    fn comment_parts(&self) -> (Vec<&str>, Vec<&str>, Option<(&str, &str)>) {
        let line = self
            .lexical
            .line_comments
            .iter()
            .map(String::as_str)
            .collect();
        let docs = self
            .lexical
            .doc_comments
            .iter()
            .map(String::as_str)
            .collect();
        let block = self
            .lexical
            .block_comment
            .as_ref()
            .map(|p| (p[0].as_str(), p[1].as_str()));
        (line, docs, block)
    }

    /// Identifiers outside comments and strings that are not in call position,
    /// with their byte offsets and the name before a `.` that qualifies them.
    /// What a file names besides what it calls: a type in a signature, a
    /// constant read, a class it extends — and `util` in `util.Primitive`,
    /// which says through which import.
    fn identifiers<'a>(&self, src: &'a str) -> Vec<(u32, &'a str, Option<&'a str>)> {
        let (line, docs, block) = self.comment_parts();
        let style = CommentStyle {
            line_comment_prefix: &line,
            doc_comment_prefix: &docs,
            block_comment_start: block.map(|p| p.0),
            block_comment_end: block.map(|p| p.1),
            ident_suffix_marks: self.lexical.identifier_suffix_marks,
            ident_dashes: self.lexical.identifier_dashes,
            quote_identifiers: self.lexical.quote_identifiers,
            raw_escapes: self.lexical.raw_string_escapes,
        };
        let tokens = crate::lexer::Lexer::new(src, style).collect_all_tokens();
        let significant: Vec<&crate::lexer::Token> =
            tokens.iter().filter(|t| significant(&t.kind)).collect();
        let mut out = Vec::new();
        for (i, t) in significant.iter().enumerate() {
            let TokenKind::Ident(name) = t.kind else {
                continue;
            };
            let next = significant.get(i + 1).map(|t| &t.kind);
            if matches!(next, Some(TokenKind::Symbol('('))) && !self.lexical.parenthesis_is_operand
            {
                continue;
            }
            let qualifier = match (
                i.checked_sub(2).map(|k| &significant[k].kind),
                i.checked_sub(1).map(|k| &significant[k].kind),
            ) {
                (Some(TokenKind::Ident(q)), Some(TokenKind::Symbol('.'))) => Some(*q),
                _ => None,
            };
            out.push((t.start, name, qualifier));
        }
        out
    }

    fn tokens<'a>(&self, src: &'a str) -> Vec<crate::lexer::Token<'a>> {
        let (line, docs, block) = self.comment_parts();
        let style = CommentStyle {
            line_comment_prefix: &line,
            doc_comment_prefix: &docs,
            block_comment_start: block.map(|p| p.0),
            block_comment_end: block.map(|p| p.1),
            ident_suffix_marks: self.lexical.identifier_suffix_marks,
            ident_dashes: self.lexical.identifier_dashes,
            quote_identifiers: self.lexical.quote_identifiers,
            raw_escapes: self.lexical.raw_string_escapes,
        };
        crate::lexer::Lexer::new(src, style)
            .collect_all_tokens()
            .into_iter()
            .filter(|t| significant(&t.kind))
            .collect()
    }

    fn parse(&self, src: &str) -> FileFacts {
        let (line, docs, block) = self.comment_parts();
        let style = CommentStyle {
            line_comment_prefix: &line,
            doc_comment_prefix: &docs,
            block_comment_start: block.map(|p| p.0),
            block_comment_end: block.map(|p| p.1),
            ident_suffix_marks: self.lexical.identifier_suffix_marks,
            ident_dashes: self.lexical.identifier_dashes,
            quote_identifiers: self.lexical.quote_identifiers,
            raw_escapes: self.lexical.raw_string_escapes,
        };
        if let Some(facts) = crate::languages::parse(&self.language, src, style, &self.calls) {
            return facts;
        }
        let g = self
            .generic
            .as_ref()
            .expect("validation requires a [generic] table for a non-native language");
        let definitions: Vec<_> = g
            .definitions
            .iter()
            .map(|d| (d.keyword.as_str(), d.scope))
            .collect();
        fn refs(v: &[String]) -> Vec<&str> {
            v.iter().map(String::as_str).collect()
        }
        let modifiers = refs(&g.modifiers);
        let receivers = refs(&g.receivers);
        let block_open = refs(&g.block_open);
        let block_end = refs(&g.block_end);
        let exclude = refs(&self.calls.exclude);
        crate::generic::parse(
            src,
            &LangSpec {
                line_comment: &line,
                doc_comment: &docs,
                block_comment: block,
                ident_suffix_marks: self.lexical.identifier_suffix_marks,
                ident_dashes: self.lexical.identifier_dashes,
                quote_identifiers: self.lexical.quote_identifiers,
                raw_escapes: self.lexical.raw_string_escapes,
                definitions: &definitions,
                modifiers: &modifiers,
                not_a_call: &exclude,
                receivers: &receivers,
                block_end: &block_end,
                block_open: &block_open,
                braces: g.braces,
                bare_receiver_calls: g.bare_receiver_calls,
                constant_by_case: g.constant_by_case,
            },
        )
    }
}

/// An immutable, validated set. Overrides replace a complete language file.
#[derive(Debug)]
pub struct RuleSet {
    files: BTreeMap<String, RuleFile>,
    identity: String,
}

impl RuleSet {
    pub fn load(directory: Option<&Path>) -> io::Result<Self> {
        let mut sources: BTreeMap<String, String> = BUILTINS
            .iter()
            .map(|b| (b.name.to_string(), b.text.to_string()))
            .collect();
        if let Some(directory) = directory {
            let mut paths = std::fs::read_dir(directory)?
                .map(|e| e.map(|e| e.path()))
                .collect::<io::Result<Vec<_>>>()?;
            paths.sort();
            for path in paths {
                if path.extension().is_none_or(|e| e != "toml") {
                    continue;
                }
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !sources.contains_key(name) {
                    return Err(invalid(format!(
                        "{}: unsupported language rule file",
                        path.display()
                    )));
                }
                if !std::fs::metadata(&path)?.is_file() {
                    return Err(invalid(format!(
                        "{}: expected a regular file",
                        path.display()
                    )));
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&path)?
                    .take(MAX_RULE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > MAX_RULE_BYTES {
                    return Err(invalid(format!(
                        "{}: rule file exceeds 64 KiB",
                        path.display()
                    )));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
                sources.insert(name.to_owned(), text);
            }
        }
        Self::from_sources(sources)
    }

    fn from_sources(sources: BTreeMap<String, String>) -> io::Result<Self> {
        let mut files = BTreeMap::new();
        let mut identity = IMPLEMENTATION_VERSION.to_owned();
        for (name, text) in sources {
            files.insert(name.clone(), RuleFile::read(&name, &text)?);
            // Length framing makes the exact identity unambiguous; no hash collision
            // can cause a different rule set to reuse a stored graph.
            identity.push_str(&format!("\n{}:{name}{}:{text}", name.len(), text.len()));
        }
        Ok(Self { files, identity })
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The file's tokens without comments and line breaks, lexed with the
    /// language's own comment and string syntax. Empty for a language with no
    /// rule file.
    pub fn tokens<'a>(&self, language: Language, src: &'a str) -> Vec<crate::lexer::Token<'a>> {
        match BUILTINS.iter().find(|b| b.language == language) {
            Some(b) => self.files[b.name].tokens(src),
            None => Vec::new(),
        }
    }

    /// See `RuleFile::identifiers`. Empty for a language with no rule file,
    /// whose comment syntax is known only to its hand-written scanner.
    pub fn identifiers<'a>(
        &self,
        language: Language,
        src: &'a str,
    ) -> Vec<(u32, &'a str, Option<&'a str>)> {
        match BUILTINS.iter().find(|b| b.language == language) {
            Some(b) => self.files[b.name].identifiers(src),
            None => Vec::new(),
        }
    }

    /// The words a language's rule file gives a role — definition keywords,
    /// modifiers, block words and what is never a call — which no definition
    /// of that language is named. Empty for a language with no rule file.
    pub fn keywords(&self, language: Language) -> Vec<&str> {
        let Some(b) = BUILTINS.iter().find(|b| b.language == language) else {
            return Vec::new();
        };
        let file = &self.files[b.name];
        let mut out: Vec<&str> = file.calls.exclude.iter().map(String::as_str).collect();
        if let Some(g) = &file.generic {
            out.extend(g.definitions.iter().map(|d| d.keyword.as_str()));
            out.extend(g.modifiers.iter().map(String::as_str));
            out.extend(g.block_open.iter().map(String::as_str));
            out.extend(g.block_end.iter().map(String::as_str));
        }
        out
    }

    pub fn parse(&self, language: Language, src: &str) -> FileFacts {
        // A language with no rule file keeps its hand-written scanner. The
        // migration is per language and measured per language, so both routes
        // have to work at once.
        match BUILTINS.iter().find(|b| b.language == language) {
            Some(b) => self.files[b.name].parse(src),
            None => crate::parse(src, language),
        }
    }
}

static ACTIVE: OnceLock<RuleSet> = OnceLock::new();

pub fn active() -> &'static RuleSet {
    ACTIVE.get_or_init(|| RuleSet::load(None).expect("bundled language rules must validate"))
}

/// Called before indexing begins; no implicit discovery and no live mutation.
pub fn configure(directory: Option<&Path>) -> io::Result<()> {
    let rules = RuleSet::load(directory)?;
    ACTIVE
        .set(rules)
        .map_err(|_| invalid("language rules have already been initialized"))
}

fn significant(kind: &TokenKind<'_>) -> bool {
    !matches!(
        kind,
        TokenKind::Newline
            | TokenKind::LineComment(_)
            | TokenKind::DocComment(_)
            | TokenKind::BlockComment(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(name: &str, transform: impl FnOnce(&str) -> String) -> RuleSet {
        let mut sources: BTreeMap<_, _> = BUILTINS
            .iter()
            .map(|b| (b.name.to_string(), b.text.to_string()))
            .collect();
        sources.insert(name.to_owned(), transform(&sources[name]));
        RuleSet::from_sources(sources).unwrap()
    }

    #[test]
    fn a_member_assigned_function_defines_and_a_relative_require_does_not() {
        let rules = RuleSet::load(None).unwrap();
        let src = "var u = require('./u');\n\
                   var p = require('p');\n\
                   exports.a = function (x) { b(); };\n\
                   Foo.prototype.c = async (y) => { d(); };\n\
                   res.e = function named() { f(); };\n\
                   module.exports = function () {};\n\
                   const last = 1;\n\
                   kick();\n";
        let facts = rules.parse(Language::JavaScript, src);
        let defined = |n: &str| facts.defines.iter().any(|d| d == n);
        for name in ["p", "a", "c", "e"] {
            assert!(defined(name), "{name}: {:?}", facts.defines);
        }
        for name in ["u", "named", "exports"] {
            assert!(!defined(name), "{name}: {:?}", facts.defines);
        }
        let caller = |callee: &str| {
            facts
                .calls
                .iter()
                .find(|c| c.1 == callee)
                .map(|c| c.0.as_str())
        };
        assert_eq!(caller("b"), Some("a"));
        assert_eq!(caller("d"), Some("c"));
        assert_eq!(caller("f"), Some("e"));
        // A semicolon ends a file-level statement, so `kick` is nobody's call.
        assert_eq!(caller("kick"), Some("<module>"));
    }

    fn end_of_in_head_before_h(facts: &FileFacts, src: &str) -> bool {
        let start = src.find("def h(x)").unwrap() as u32;
        facts
            .ranges
            .iter()
            .any(|r| r.0 == "head" && r.1 .1 <= start)
    }

    #[test]
    fn elixir_blocks_end_where_their_keywords_say() {
        let rules = RuleSet::load(None).unwrap();
        let src = "defmodule M do\n\
                   @tag Enum.map(all, fn x ->\n  shape(x)\nend)\n\
                   @spec a?(t) :: t\n\
                   def a?(x) do\n  each(x, fn y -> y end)\n  after_fn()\n  b!(x)\nend\n\
                   def b!(x), do: one(x)\n\
                   def c(x), do: x\n\
                   defguardp is_d(x) when x > 0\n\
                   defdelegate e(x), to: M.Other\n\
                   defmacrop f(x) do\n  unless x in @tag do\n    in_unless()\n  end\n  after_unless()\nend\n\
                   def g(x) do\n  %{start: 1, end: 2}\n  [do: 1]\n  x.end\n  at(:end)\n  after_keys()\nend\n\
                   def head(o \\\\ [do: 1]) do\n  x()\n  in_head()\nend\n\
                   past_head()\n\
                   def h(x) do\n  ~S|a do b|\n  ~r/don't/i\n  ~s\"\"\"\n  say \"x do\n  \"\"\"\n  [?\", ?(]\n  after_sigils()\nend\n\
                   defmodule N do\n  def z(x), do: x\nend\n\
                   after_n()\n\
                   end\n";
        let facts = rules.parse(Language::Elixir, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        let caller = |callee: &str| {
            facts
                .calls
                .iter()
                .find(|c| c.1 == callee)
                .map(|c| c.0.as_str())
        };
        for name in ["a?", "b!", "c", "is_d", "e", "f", "g", "h"] {
            assert!(facts.defines.iter().any(|d| d == name), "{name}");
        }
        assert_eq!(caller("shape"), Some("M"));
        assert_eq!(caller("after_fn"), Some("a?"));
        assert_eq!(caller("b!"), Some("a?"));
        assert_eq!(caller("one"), Some("b!"));
        assert_eq!(caller("in_unless"), Some("f"));
        assert_eq!(caller("after_unless"), Some("f"));
        assert_eq!(caller("after_keys"), Some("g"));
        assert_eq!(caller("after_sigils"), Some("h"));
        assert_eq!(caller("after_n"), Some("M"));
        assert_eq!(caller("in_head"), Some("head"));
        assert_eq!(caller("past_head"), Some("M"));
        assert!(end_of_in_head_before_h(&facts, src));
        // A definition without a block ends where the next one begins.
        let start_of = |name: &str| src.find(name).unwrap() as u32;
        let end_of = |name: &str| {
            facts
                .ranges
                .iter()
                .find(|r| r.0 == name)
                .map(|r| r.1 .1)
                .unwrap()
        };
        assert!(end_of("b!") <= start_of("def c"));
        assert!(end_of("e") <= start_of("defmacrop f"));
    }

    #[test]
    fn dart_definitions_keep_their_bodies() {
        let rules = RuleSet::load(None).unwrap();
        let src = "T first<T extends Object>(List<Set<T>> a) {\n  one();\n}\n\
                   class C extends D {\n\
                   \x20 void named({int a = 1}) {\n    two();\n  }\n\
                   \x20 String get label {\n    three();\n  }\n\
                   \x20 String get short => four();\n\
                   \x20 int small() => 1;\n\
                   \x20 final made = seen();\n\
                   \x20 int size() => () { five(); six(); }();\n\
                   \x20 Function make() {\n    return (int x) {\n      seven();\n    };\n  }\n\
                   \x20 void use() {\n    first<Set<int>>(x);\n    if (a < b && c > d) {}\n  }\n\
                   }\n";
        let facts = rules.parse(Language::Dart, src);
        assert!(!facts.had_errors);
        let caller = |callee: &str| {
            facts
                .calls
                .iter()
                .find(|c| c.1 == callee)
                .map(|c| c.0.as_str())
        };
        assert_eq!(caller("one"), Some("first"));
        assert_eq!(caller("two"), Some("named"));
        assert_eq!(caller("three"), Some("label"));
        assert_eq!(caller("four"), Some("short"));
        assert_eq!(caller("seen"), Some("C"));
        assert_eq!(caller("five"), Some("size"));
        assert_eq!(caller("six"), Some("size"));
        assert_eq!(caller("seven"), Some("make"));
        assert_eq!(caller("first"), Some("use"));
        assert!(!facts.defines.iter().any(|d| d == "return"));
        assert!(caller("a").is_none());
    }

    #[test]
    fn ruby_members_and_literals_are_no_keywords() {
        let rules = RuleSet::load(None).unwrap();
        let src = "class A\n\
                   \x20 class << self\n    def made\n      made_body()\n    end\n  end\n\
                   \x20 def one\n    self.class.go\n    x = { class: 1, if: 2 }\n    run(:end, :class)\n    y.end\n    first()\n  end\n\
                   \x20 def two\n    text = <<~SQL\n      class Hidden\n      do end\n    SQL\n    assert_match(/class Foo/, text)\n    check /def bar/, text\n    z = %r{def baz}\n    w = %w[end class]\n    second()\n  end\n\
                   \x20 def three(a, b)\n    a / b\n    c = a / 2\n    third()\n  end\n\
                   end\n";
        let facts = rules.parse(Language::Ruby, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        for name in ["Hidden", "Foo", "bar", "baz", "SQL"] {
            assert!(!facts.defines.iter().any(|d| d == name), "{name}");
        }
        let caller = |callee: &str| {
            facts
                .calls
                .iter()
                .find(|c| c.1 == callee)
                .map(|c| c.0.as_str())
        };
        assert_eq!(caller("made_body"), Some("made"));
        assert_eq!(caller("first"), Some("one"));
        assert_eq!(caller("second"), Some("two"));
        assert_eq!(caller("third"), Some("three"));
    }

    #[test]
    fn script_regexes_and_decorators_keep_blocks_balanced() {
        let rules = RuleSet::load(None).unwrap();
        let src = "function a(s) {\n  var r = s.replace(/\\{([^}]+)\\}/g, function (m) { return m; });\n  inner();\n}\n\
                   function b(x) {\n  return x / 2 + later(x) / 3;\n}\n";
        let facts = rules.parse(Language::JavaScript, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        assert!(facts.calls.iter().any(|c| c.0 == "a" && c.1 == "inner"));
        assert!(facts.calls.iter().any(|c| c.0 == "b" && c.1 == "later"));
        let src = "@Component({\n  a: 'b',\n})\nexport class C {}\n\
                   @Injectable() export class D {\n  @Input() run() {\n    go();\n  }\n}\n";
        let facts = rules.parse(Language::TypeScript, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        for name in ["C", "D", "run"] {
            assert!(facts.defines.iter().any(|d| d == name), "{name}");
        }
        assert!(facts.calls.iter().any(|c| c.0 == "run" && c.1 == "go"));
    }

    #[test]
    fn long_strings_end_where_their_language_says() {
        let rules = RuleSet::load(None).unwrap();
        for (language, src) in [
            (
                Language::Php,
                "<?php\nfunction m() {\n  /* any application/* type */\n  run();\n}\n",
            ),
            (
                Language::Java,
                "class A {\n  void m() {\n    /* a /* b */\n    run();\n  }\n}\n",
            ),
            (
                Language::Rust,
                "fn m() {\n  /* a /* b */ { */\n  run();\n}\n",
            ),
            (
                Language::Cpp,
                "int m() {\n  auto t = R\"({\"a\":[}\")\";\n  run();\n}\n",
            ),
            (
                Language::Cpp,
                "int m() {\n  auto t = R\"x({)\"ü)x\";\n  run();\n}\n",
            ),
            (
                Language::Rust,
                "fn m() {\n  let x = b\"\\\nfoo{\n\";\n  run();\n}\n",
            ),
            (
                Language::Kotlin,
                "fun m() {\n  x(\"\"\"file{=\"t\"\"\"\")\n  run()\n}\n",
            ),
        ] {
            let facts = rules.parse(language, src);
            assert!(!facts.had_errors, "{language:?} {src}");
            assert!(
                facts.calls.iter().any(|c| c.0 == "m" && c.1 == "run"),
                "{language:?} {src}"
            );
        }
    }

    #[test]
    fn blocks_survive_what_only_looks_like_a_delimiter() {
        let rules = RuleSet::load(None).unwrap();
        for (language, src) in [
            (
                Language::Dart,
                "class A {\n  bool m(x) {\n    if (x.library.uri == u) {\n      a();\n    }\n    for (final part in parts) {\n      a();\n    }\n    run();\n  }\n}\n",
            ),
            (
                Language::Dart,
                "String m() {\n  x('${a.replaceAll('},', '},\\n')}');\n  run();\n}\n",
            ),
            (
                Language::Dart,
                "String m(p) {\n  x('${switch (p) {\n    1 => 'one',\n  }} b');\n  run();\n}\n",
            ),
            (
                Language::Kotlin,
                "fun m() {\n  x(\"(a ${n.map { it ?: \"\\\"$D\\\"\" }})\")\n  run()\n}\n",
            ),
            (
                Language::Kotlin,
                "fun m() {\n  x($$\"\"\"${a\"\"\")\n  run()\n}\n",
            ),
            (
                Language::Kotlin,
                "private fun List<Set<Int>>.m(): Int {\n  run()\n  return 1\n}\n",
            ),
            (
                Language::C,
                "#define P(x) { #x, x }\nint m() {\n  run();\n}\n",
            ),
            (
                Language::Cpp,
                "#define J(a) int a##b() { \\\n  return 1; }\nint m() {\n  run();\n}\n",
            ),
            (
                Language::C,
                "int m() {\n#if A\n  if (a) {\n#else\n  if (b) {\n#endif\n    c();\n  }\n  run();\n}\n",
            ),
            (
                Language::C,
                "int m() {\n  if (a &&\n#if A\n  b)\n#elif B\n  c)\n#else\n  d)\n#endif\n  {\n    run();\n  }\n}\n",
            ),
            (
                Language::C,
                "int m() {\n#if A\n#if B\n  if (a) {\n#else\n  if (b) {\n#endif\n#else\n  if (c) {\n#endif\n    d();\n  }\n  run();\n}\n",
            ),
            (
                Language::Dart,
                "Uri library(x) {\n  a();\n}\nvoid m() {\n  run();\n}\n",
            ),
            (
                Language::C,
                "int m() {\n#if A\n  if (a) {\n#else\n#if B\n  if (b) {\n#endif\n#endif\n    d();\n  }\n  run();\n}\n",
            ),
            (
                Language::Ruby,
                "def m\n  x('a\n  end\n  b')\n  run()\nend\n",
            ),
            (
                Language::Php,
                "<?php\n#[Bind(static function () {\n  return false;\n})]\nfunction m() {\n  run();\n}\n",
            ),
            (Language::Php, "<?php\n# {\nfunction m() {\n  run();\n}\n"),
            (
                Language::Php,
                "<?php\nfunction m() {\n  x('{{\n  }}}');\n  run();\n}\n",
            ),
            (
                Language::Ruby,
                "def m\n  @case = 1\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x = -$`\n  y = 'a'\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x = /\n    (['\"]) # quote\n  /x\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x || if a\n    b()\n  end\n  run()\nend\n",
            ),
            (
                Language::TypeScript,
                "const o = {\n  m() {\n    f(c)\n    const { a } = b\n    run();\n  },\n}\n",
            ),
            (
                Language::TypeScript,
                "function m() {\n  x(`a ${b ? `{` : `(`}`);\n  run();\n}\n",
            ),
            (
                Language::Go,
                "func m() {\n  x(`{\\`, `}`)\n  run()\n}\n",
            ),
            (
                Language::Ruby,
                "def m\n  x = +%(<i a=\"b\">)\n  y = +%{c's}\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x(\"#{a ? \"{ #{b} }\" : c}\")\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x = { a: /#{b.sub(/c/, \"\")}/ }\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  def a.b = 1\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  x = '#{'\n  run()\nend\n",
            ),
            (
                Language::TypeScript,
                "function m() {\n  a = i++ / 2; { b / 3 }\n  run();\n}\n",
            ),
            (
                Language::Ruby,
                "def m\n  a :[]= unless b\n  c(:d=) if e\n  run()\nend\n",
            ),
            (
                Language::Ruby,
                "def m\n  t << if a\n    b()\n  end\n  t << %(it's)\n  run()\nend\n",
            ),
        ] {
            let facts = rules.parse(language, src);
            assert!(!facts.had_errors, "{language:?} {src}");
            assert!(
                facts.calls.iter().any(|c| c.0 == "m" && c.1 == "run"),
                "{language:?} {src} {:?}",
                facts.calls
            );
        }
    }

    #[test]
    fn base_and_parent_are_keywords_only_where_the_language_says() {
        let rules = RuleSet::load(None).unwrap();
        let receiver = |language, src: &str| {
            let facts = rules.parse(language, src);
            facts.calls.iter().find(|c| c.1 == "run").map(|c| c.2)
        };
        let js = "function m(parent) {\n  parent.run();\n}\n";
        assert_eq!(receiver(Language::JavaScript, js), Some(true));
        let php = "<?php\nclass A {\n  function m() {\n    parent::run();\n  }\n}\n";
        assert_eq!(receiver(Language::Php, php), Some(false));
        let cs = "class A {\n  void M() {\n    base.run();\n  }\n}\n";
        assert_eq!(receiver(Language::CSharp, cs), Some(false));
    }

    #[test]
    fn a_shell_hash_inside_a_word_opens_no_comment() {
        let rules = RuleSet::load(None).unwrap();
        for src in [
            "m() {\n  if [[ ${#a[@]} -gt 0 ]]; then\n    run\n  fi\n}\n",
            "m() {\n  echo $# # }\n  run\n}\n",
        ] {
            let facts = rules.parse(Language::Bash, src);
            assert!(!facts.had_errors, "{src}");
        }
    }

    #[test]
    fn plain_names_declare_nothing() {
        let rules = RuleSet::load(None).unwrap();
        let facts = rules.parse(
            Language::Dart,
            "library shop.core;\nclass A {\n  bool m(x) => x.library.uri;\n}\n",
        );
        assert!(facts.defines.iter().any(|d| d == "shop.core"));
        assert!(
            !facts.defines.iter().any(|d| d == "uri"),
            "{:?}",
            facts.defines
        );
        let src = "const o = {\n  m() {\n    f(c)\n    const { a } = b\n    run();\n  },\n}\n";
        let facts = rules.parse(Language::TypeScript, src);
        assert!(
            !facts.defines.iter().any(|d| d == "f"),
            "{:?}",
            facts.defines
        );
        let facts = rules.parse(Language::Kotlin, "val String.size2: Int\n  get() = 1\n");
        assert!(
            facts.defines.iter().any(|d| d == "size2"),
            "{:?}",
            facts.defines
        );
        assert!(!facts.defines.iter().any(|d| d == "String"));
    }

    #[test]
    fn a_php_page_and_its_text_are_not_code() {
        let rules = RuleSet::load(None).unwrap();
        for src in [
            "<p>don't {</p>\n<?php\nfunction m() {\n  run();\n}\n?>\n<p>it's }</p>\n",
            "<?php\nfunction m() {\n  x(<<<EOT\n  it's {\n  EOT);\n  run();\n}\n",
            "<p>'</p>\n<? function m() {\n  run();\n}\n",
            "<?php\nfunction m() {\n  x(<<<'A_B'\nA_BC {\nA_B\n);\n  run();\n}\n",
            "<?php if ($a): ?><b>'</b><?= f() ?><i>'{</i><?php endif;\nfunction m() {\n  run();\n}\n",
        ] {
            let facts = rules.parse(Language::Php, src);
            assert!(!facts.had_errors, "{src}");
            assert!(
                facts.calls.iter().any(|c| c.0 == "m" && c.1 == "run"),
                "{src} {:?}",
                facts.calls
            );
        }
        let facts = rules.parse(Language::Php, "function m() {\n  run();\n}\n");
        assert!(facts.calls.iter().any(|c| c.0 == "m"));
        let page = rules.parse(Language::Php, "<?xml version=\"1\"?>\nfunction no() {}\n");
        assert!(page.defines.is_empty(), "{:?}", page.defines);
    }

    #[test]
    fn a_kotlin_raw_string_ends_after_a_backslash() {
        let rules = RuleSet::load(None).unwrap();
        let src = "fun m() {\n  x(\"\"\"C:\\\"\"\")\n  run()\n}\n";
        let facts = rules.parse(Language::Kotlin, src);
        assert!(facts.calls.iter().any(|c| c.0 == "m" && c.1 == "run"));
        let python = rules.parse(
            Language::Python,
            "x = \"\"\"a\\\"\"\"\ndef f(): pass\n\"\"\"\n",
        );
        assert!(python.defines.is_empty(), "{:?}", python.defines);
    }

    #[test]
    fn scans_stop_at_their_bounds() {
        use crate::lexer::{after_angles, keep_first_branches, LineStart, Token, TokenKind};
        let rules = RuleSet::load(None).unwrap();
        // Past the deepest nesting the file ends for every scanner.
        let src = format!(
            "void a() {{\n  run();\n}}\n{}\nvoid b() {{\n}}\n",
            "x(".repeat(1100)
        );
        let facts = rules.parse(Language::C, &src);
        assert!(facts.defines.iter().any(|d| d == "a"));
        assert!(!facts.defines.iter().any(|d| d == "b"));
        let fine = format!(
            "{}1{}\nvoid b() {{\n}}\n",
            "(".repeat(1000),
            ")".repeat(1000)
        );
        assert!(rules
            .parse(Language::C, &fine)
            .defines
            .iter()
            .any(|d| d == "b"));

        let angle = |i| Token {
            kind: TokenKind::Symbol('<'),
            start: i,
            end: i + 1,
        };
        let many: Vec<Token<'_>> = (0..5000).map(angle).collect();
        assert_eq!(after_angles(&many, 10), 10 + crate::lexer::LOOKAHEAD);

        let mut lines = LineStart::default();
        let text = "ab\ncd\n\nef";
        for at in 0..=text.len() {
            let want = text[..at].rfind('\n').map_or(0, |n| n + 1);
            assert_eq!(lines.of(text, at), want, "{at}");
        }

        // Nesting costs each mark one move, not one per level.
        let src = format!(
            "{}{}",
            "#if\n{}\n".repeat(300_000),
            "#endif\n".repeat(300_000)
        );
        let mut tokens = rules.tokens(Language::C, &src);
        let started = std::time::Instant::now();
        keep_first_branches(&mut tokens);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn a_jsx_tag_end_is_no_regexp() {
        let rules = RuleSet::load(None).unwrap();
        let language = Language::from_extension("tsx").unwrap();
        let src = "const m = () => {\n  x(<p>{a ? <B c={d} /> : e}</p>);\n  run();\n};\n";
        let facts = rules.parse(language, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
    }

    #[test]
    fn floods_cost_their_length() {
        let rules = RuleSet::load(None).unwrap();
        for (extensions, pattern) in [
            (&["c", "cpp", "ts", "java", "dart", "cs"][..], "a.b("),
            (&["kt", "ts", "dart", "cs"], "a<"),
            (&["ts"], "a."),
            (&["hs"], "get a "),
            (&["cs", "cpp"], "const a "),
            (&["py", "gd", "nim"], "a "),
            (&["ml"], "let "),
            (&["css"], "alias a.{"),
            (&["css"], "."),
            (&["html", "xml"], "<a"),
            (&["html", "xml"], "<!--"),
        ] {
            for extension in extensions {
                let language = Language::from_extension(extension).unwrap();
                // The largest file the indexer reads.
                let src = pattern.repeat((1 << 20) / pattern.len());
                let started = std::time::Instant::now();
                rules.parse(language, &src);
                let took = started.elapsed();
                assert!(took.as_secs() < 15, "{extension} {pattern:?} {took:?}");
            }
        }
    }

    #[test]
    fn a_css_rule_spans_its_block() {
        let rules = RuleSet::load(None).unwrap();
        let src = ".a { .b { c: d } }\n.e { }\n.f { ";
        let facts = rules.parse(Language::Css, src);
        let end = |name: &str| facts.ranges.iter().find(|r| r.0 == name).map(|r| r.1 .1);
        assert_eq!(end(".a"), Some(18));
        assert_eq!(end(".b"), Some(16));
        assert_eq!(end(".f"), Some(src.len() as u32 - 3));
    }

    #[test]
    fn hostile_strings_end_the_scan_cleanly() {
        let rules = RuleSet::load(None).unwrap();
        rules.parse(Language::Dart, &format!("x('{}", "${'".repeat(200_000)));
        rules.parse(Language::Kotlin, &format!("x(\"{}", "${\"".repeat(200_000)));
        for src in ["x = f\"\"\"é", "x = f\"\"\"aé", "x = f\"\"\"aaé"] {
            rules.parse(Language::Python, src);
        }
        rules.parse(Language::C, "#else\n#endif\n#if A\n#else\n");
        rules.parse(Language::Cpp, "#");
    }

    #[test]
    fn a_const_parameter_defines_no_constant() {
        let rules = RuleSet::load(None).unwrap();
        let src = "static int f(const char* a,\n    int b) {\n  int c = 1;\n  run();\n}\nconst int LIMIT = 3;\n";
        let facts = rules.parse(Language::Cpp, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        assert!(
            !facts.defines.iter().any(|d| d == "c"),
            "{:?}",
            facts.defines
        );
        assert!(facts.defines.iter().any(|d| d == "LIMIT"));
        assert!(facts.calls.iter().any(|c| c.0 == "f" && c.1 == "run"));
    }

    #[test]
    fn ruby_endless_methods_open_no_block() {
        let rules = RuleSet::load(None).unwrap();
        let src = "class Outer\n\
                   \x20 class Verb\n\
                   \x20   def self.name = \"GET\"\n\
                   \x20   def self.call(req) = go(req)\n\
                   \x20 end\n\
                   \x20 def value=(v)\n    @v = v\n    set()\n  end\n\
                   \x20 def after\n    last()\n  end\n\
                   end\n";
        let facts = rules.parse(Language::Ruby, src);
        assert!(!facts.had_errors, "{:?}", facts.ranges);
        let range = |name: &str| facts.ranges.iter().find(|r| r.0 == name).unwrap().1;
        assert!(range("Verb").1 <= range("value").0, "{:?}", facts.ranges);
        assert!(facts.calls.iter().any(|c| c.0 == "call" && c.1 == "go"));
        assert!(facts.calls.iter().any(|c| c.0 == "value" && c.1 == "set"));
        assert!(facts.calls.iter().any(|c| c.0 == "after" && c.1 == "last"));
    }

    #[test]
    fn swift_computed_properties_define_and_stored_ones_do_not() {
        let rules = RuleSet::load(None).unwrap();
        let src = "struct S {\n\
                   \x20 var body: [Int] {\n    work()\n  }\n\
                   \x20 var stored: Int = 1\n\
                   \x20 var plain: Int\n\
                   }\n";
        let facts = rules.parse(Language::Swift, src);
        assert!(facts.defines.iter().any(|d| d == "body"));
        assert!(!facts.defines.iter().any(|d| d == "stored" || d == "plain"));
        assert!(facts.calls.iter().any(|c| c.0 == "body" && c.1 == "work"));
    }

    #[test]
    fn an_empty_function_like_macro_is_no_definition() {
        let rules = RuleSet::load(None).unwrap();
        let src = "#define stub(a, b)\n\
                   #define flag\n\
                   #define spaced (1)\n\
                   #define real(a) go(a)\n\
                   #define long_one(a) \\\n  go(a)\n\
                   int after(void) { return 0; }\n";
        for language in [Language::C, Language::Cpp] {
            let facts = rules.parse(language, src);
            assert!(!facts.defines.iter().any(|d| d == "stub"), "{language:?}");
            for name in ["flag", "spaced", "real", "long_one", "after"] {
                assert!(
                    facts.defines.iter().any(|d| d == name),
                    "{language:?} {name}: {:?}",
                    facts.defines
                );
            }
        }
    }

    #[test]
    fn a_backslash_keeps_a_python_raw_string_open_and_a_rust_one_does_not() {
        let rules = RuleSet::load(None).unwrap();
        // Read the Rust way, the string closed at
        // `\"`, reopened at the next quote and swallowed the rest of the file.
        let py = "RE = re.compile(r\"(['\\\"]?)\\1\")\n\ndef after():\n    pass\n";
        for facts in [
            rules.parse(Language::Python, py),
            Language::Python.parse(py),
        ] {
            assert!(
                facts.defines.iter().any(|d| d == "after"),
                "{:?}",
                facts.defines
            );
        }
        // Rust's raw strings have no escapes: `r"C:\"` ends at its second quote.
        let rs = "const P: &str = r\"C:\\\";\nfn after() {}\n";
        assert!(
            rules
                .parse(Language::Rust, rs)
                .defines
                .iter()
                .any(|d| d == "after"),
            "the Rust reading must not change"
        );
    }

    #[test]
    fn a_python_constant_ends_with_its_statement() {
        let rules = RuleSet::load(None).unwrap();
        // Found through detect_changes: an edit at the end of a file was
        // attributed to every constant in it, because each one's range ran to
        // the end of the file — and get_code_snippet returned all of that.
        let py = "LIMIT = 3\nSTOP = frozenset({\n    \"a\",\n})\n\ndef after():\n    pass\n";
        let def_at = py.find("def after").unwrap() as u32;
        for facts in [
            rules.parse(Language::Python, py),
            Language::Python.parse(py),
        ] {
            for name in ["LIMIT", "STOP"] {
                let (_, (_, end)) = facts.ranges.iter().find(|(n, _)| n == name).unwrap();
                assert!(
                    *end < def_at,
                    "{name} runs to {end}, past `def after` at {def_at}"
                );
            }
            let (_, (_, stop_end)) = facts.ranges.iter().find(|(n, _)| n == "STOP").unwrap();
            assert!(
                *stop_end as usize >= py.find("})").unwrap(),
                "a bracketed constant spans its lines"
            );
        }
    }

    #[test]
    fn bundled_rules_are_deterministic_and_match_dispatch() {
        let first = RuleSet::load(None).unwrap();
        let second = RuleSet::load(None).unwrap();
        assert_eq!(first.identity(), second.identity());
        for (lang, source) in [
            (Language::Go, "package p\nfunc Run() { notify() }"),
            (Language::Rust, "fn run() { notify(); }"),
            (Language::Ruby, "def run\n notify()\nend"),
        ] {
            assert_eq!(first.parse(lang, source), lang.parse(source));
        }
    }

    #[test]
    fn a_file_changes_extraction_without_rust_changes() {
        let base = RuleSet::load(None).unwrap();
        let custom = changed("go", |s| {
            s.replace("keyword = \"func\"", "keyword = \"proc\"")
        });
        let source = "package p\n// Run handles work.\nproc Run() { notify() }";
        assert!(!base
            .parse(Language::Go, source)
            .defines
            .contains(&"Run".into()));
        let facts = custom.parse(Language::Go, source);
        assert!(facts.defines.contains(&"Run".into()));
        assert!(facts
            .calls
            .contains(&("Run".into(), "notify".into(), false)));
        assert!(facts
            .docs
            .iter()
            .any(|(n, d)| n == "Run" && d == "Run handles work."));
        let (_, (start, end)) = facts.ranges.iter().find(|(n, _)| n == "Run").unwrap();
        assert_eq!(
            &source[*start as usize..*end as usize],
            "proc Run() { notify() }"
        );
        assert_ne!(base.identity(), custom.identity());
        // Even without a revision bump, a changed file cannot reuse an old graph.
        assert_eq!(base.files["go"].revision, custom.files["go"].revision);
    }

    #[test]
    fn native_modules_obey_call_rules_and_keep_special_syntax() {
        for (name, language, src, definition) in [
            ("rust", Language::Rust, "extern \"C\" { fn foreign(); }\nmacro_rules! marker { () => {} }\nfn run() { notify(); }", "run"),
            ("ruby", Language::Ruby, "class C\n class << self\n def run!\n notify()\n end\n end\nend", "run!"),
        ] {
            let normal = RuleSet::load(None).unwrap().parse(language, src);
            let custom = changed(name, |s| s.replace("exclude = [", "exclude = [\"notify\", "));
            let filtered = custom.parse(language, src);
            assert_eq!(normal.defines, filtered.defines);
            assert!(filtered.defines.contains(&definition.to_string()));
            assert!(normal.calls.iter().any(|(_, n, _)| n == "notify"));
            assert!(!filtered.calls.iter().any(|(_, n, _)| n == "notify"));
            assert!(!filtered.defines.iter().any(|n| n == "foreign" || n == "self"));
        }
    }

    #[test]
    fn native_modules_obey_lexical_rules() {
        for (name, language, source) in [
            ("rust", Language::Rust, "# Hidden\nfn run() {}"),
            ("ruby", Language::Ruby, "// Hidden\ndef run\nend"),
        ] {
            let rules = changed(name, |s| {
                if name == "rust" {
                    s.replace(
                        "line_comments = [\"///\", \"//!\", \"//\"]",
                        "line_comments = [\"#\"]",
                    )
                    .replace(
                        "doc_comments = [\"///\", \"//!\"]",
                        "doc_comments = [\"#\"]",
                    )
                } else {
                    s.replace("[\"#\"]", "[\"//\"]")
                }
            });
            assert!(rules
                .parse(language, source)
                .docs
                .iter()
                .any(|(n, d)| n == "run" && d == "Hidden"));
        }
    }

    /// Every rule-driven language must produce exactly what the hand-written
    /// scanner it replaces produces.
    ///
    /// This is the migration's whole contract, and it is checked per language
    /// rather than in aggregate: a rule file that finds different symbols is a
    /// silent regression, since both routes return facts and neither errors.
    /// Measured across 8.2M lines before a language was added
    /// here — twenty-four others differed and kept their scanners rather than
    /// shipping a near-miss.
    #[test]
    fn every_rule_matches_the_scanner_it_replaces() {
        let set = RuleSet::load(None).unwrap();
        for b in &BUILTINS {
            let src = SAMPLES
                .iter()
                .find(|(n, _)| *n == b.name)
                .unwrap_or_else(|| panic!("{}: no sample; add one beside its rules", b.name))
                .1;
            // Go, Rust and Ruby migrated first and their `parse_*` now call
            // straight back here, so comparing the two routes for them
            // compares the rules with themselves. What is asserted instead is
            // that the sample carries a definition at all — a rule file that
            // finds nothing is the failure this catches for them.
            let ruled = set.parse(b.language, src);
            assert!(
                !ruled.defines.is_empty(),
                "{}: the rule file extracts nothing from its own sample",
                b.name
            );
            // Typst joins them for a different reason: it is a document format
            // *and* a scripting language, and `parse` merges both readings —
            // headings from the line scanner, `#let` bindings and their calls
            // from here. Comparing the two therefore compares halves of one
            // answer. What is asserted for it is the same as for the three
            // above: the sample must yield something.
            if !matches!(b.name, "go" | "rust" | "ruby" | "typst") {
                assert_eq!(
                    ruled,
                    crate::parse(src, b.language),
                    "{}: the rule file and the scanner it replaces disagree",
                    b.name
                );
            }
        }
    }

    /// One definition, one call and one comment per language — the shape the
    /// earlier sweeps found thirteen defects with, small enough to live here.
    /// One definition, one call and — where the language has one — a **control
    /// keyword from its own `exclude` list**.
    ///
    /// The last part is what makes this a check rather than a smoke test.
    /// Without it the assertion passes with `calls.allows` removed entirely,
    /// which is precisely what a rule file controls: measured, sabotaging C#
    /// is invisible on `class C { void Run() { Notify(); } }` and fails
    /// immediately once the sample says `if (x) { … } return;`.
    ///
    /// **Seven languages are not covered this way and cannot be.** Haskell,
    /// OCaml and Lean carry an empty `exclude` — their scanners exclude
    /// nothing, and the admission test is identity, so filling the list is a
    /// separate change made against the scanner first (Lean measured one edge
    /// better and was reverted). Rust and Ruby route their `parse_*` back into
    /// the rules, so the two sides of the comparison are the same code. And C,
    /// C++, Ada, Fortran and VB list only words `is_control_keyword` already
    /// filters upstream, so their list is redundant rather than unguarded —
    /// `sizeof`, `alignof` and `_Generic` never reach `calls.allows` at all.
    const SAMPLES: &[(&str, &str)] = &[
        ("rust", "/// Doc.\nfn run() { if x { return notify(); } }"),
        ("cmake", "# Doc.\nfunction(run x)\n  notify(${x})\nendfunction()"),
        ("objc", "// Doc.\n@implementation C\n- (int)run:(int)x {\n  return [self notify:x];\n}\n@end"),
        ("tcl", "# Doc.\nproc run {x} {\n  return [notify $x]\n}"),
        ("janet", "# Doc.\n(defn run [x]\n  (notify x))"),
        ("plsql", "-- Doc.\nFUNCTION run(x IN NUMBER) RETURN NUMBER IS\nBEGIN\n  RETURN notify(x);\nEND;"),
        ("apex", "// Doc.\nclass C { Integer run(Integer x) { return notify(x); } }"),
        ("bicep", "// Doc.\nresource run 'M/a@2023-01-01' = {\n  name: notify\n}"),
        ("puppet", "# Doc.\ndefine run($x) {\n  notify($x)\n}"),
        ("cuda", "// Doc.\n__device__ int run(int x) { return notify(x); }"),
        ("makefile", "# Doc.\nrun:\n\tnotify\n"),
        ("meson", "# Doc.\nrun = library('run', notify())"),
        ("jsonnet", "# Doc.\n{\n  run:: function(x) notify(x),\n}"),
        ("cfscript", "// Doc.\ncomponent { function run(x) { return notify(x); } }"),
        ("smali", "# Doc.\n.method public run(I)I\n    invoke-static {p1}, LC;->notify(I)V\n.end method"),
        ("prisma", "/// Doc.\nmodel Run {\n  id Int @id\n  notify Notify @relation(fields: [id], references: [id])\n}"),
        ("soql", "-- Doc.\nFUNCTION run(x IN NUMBER) RETURN NUMBER IS\nBEGIN\n  RETURN notify(x);\nEND;"),
        ("gotemplate", "{{define \"run\"}}\n  {{template \"notify\" .}}\n{{end}}"),
        ("liquid", "{% capture run %}\n  {% include 'notify' %}\n{% endcapture %}"),
        ("nasm", "; Doc.\nrun:\n    call notify\n    ret"),
        ("assembly", "; Doc.\nrun:\n            BSR     notify\n            RTS"),
        ("just", "# Doc.\nrun:\n\tnotify\n"),
        ("hare", "// Doc.\nfn run(x: int) int = {\n\treturn notify(x);\n};"),
        ("move", "/// Doc.\nmodule m::c {\n    fun run(x: u64): u64 { notify(x) }\n}"),
        ("squirrel", "// Doc.\nfunction run(x) { return notify(x); }"),
        ("luau", "-- Doc.\nlocal function run(x: number): number\n\treturn notify(x)\nend"),
        ("teal", "-- Doc.\nlocal function run(x: number): number\n   return notify(x)\nend"),
        ("fennel", ";; Doc.\n(fn run [x]\n  (notify x))"),
        ("jinja", "{% block run %}\n  {% include 'notify' %}\n{% endblock %}"),
        ("blade", "@section('run')\n    @include('notify')\n@endsection"),
        ("rescript", "// Doc.\nlet run = x => {\n  notify(x)\n}"),
        ("typst", "// Doc.\n#let run(x) = {\n  notify(x)\n}"),
        ("objectscript", "/// Doc.\nClass C Extends %Persistent\n{\nClassMethod Run(x As %Integer) As %Integer\n{\n    Return ..Notify(x)\n}\n}"),
        ("qml", "// Doc.\nItem {\n    function run(x) { return notify(x); }\n}"),
        ("cairo", "/// Doc.\nfn run(x: u64) -> u64 { notify(x) }"),
        ("llvm", "; Doc.\ndefine i64 @run(i64 %x) {\n  %1 = call i64 @notify(i64 %x)\n  ret i64 %1\n}"),
        ("wolfram", "(* Doc. *)\nrun[x_] := Module[{}, notify[x]]"),
        ("cfml", "<cffunction name=\"run\">\n    <cfset notify()>\n</cffunction>"),
        ("tlaplus", "\\* Doc.\nRun(x) == Notify(x)"),
        ("arkts", "// Doc.\nfunction run(x: number): number { return notify(x); }"),
        ("templ", "// Doc.\ntempl run(x string) {\n\t@notify(x)\n}"),
        ("ispc", "// Doc.\nexport uniform int run(uniform int x) { return notify(x); }"),
        ("chialisp", "; Doc.\n(mod (x)\n  (defun run (x) (notify x))\n)"),
        ("sosl", "-- Doc.\nFUNCTION run(x IN NUMBER) RETURN NUMBER IS\nBEGIN\n  RETURN notify(x);\nEND;"),
        ("agda", "-- Doc.\nrun : Nat -> Nat\nrun x = notify x"),
        ("astro", "---\n// Doc.\nfunction run(x) { return notify(x); }\n---\n<div/>"),
        ("slang", "// Doc.\nint run(int x) { return notify(x); }"),
        ("bitbake", "# Doc.\npython do_run() {\n    notify()\n}"),
        ("magma", "// Doc.\nrun := function(x)\n    return notify(x);\nend function;"),
        ("pine", "// Doc.\nrun(x) =>\n    notify(x)"),
        ("sway", "/// Doc.\nfn run(x: u64) -> u64 { notify(x) }"),
        ("smithy", "/// Doc.\noperation Run {\n    input: Notify\n}"),
        ("wit", "/// Doc.\ninterface run {\n    notify: func(x: u64);\n}"),
        ("mermaid", "%% Doc.\ngraph TD\n    run --> notify"),
        ("devicetree", "// Doc.\nrun: run@1000 {\n    target = <&notify>;\n};"),
        ("linkerscript", "/* Doc. */\nSECTIONS {\n    .run : { *(.notify) }\n}"),
        ("gomod", "// Doc.\nmodule run\n\nrequire notify v1.0.0"),
        ("nickel", "# Doc.\nlet run = fun x => notify x in run"),
        ("pkl", "// Doc.\nfunction run(x) = notify(x)"),
        ("tablegen", "// Doc.\nclass Base<string n> { string N = n; }\ndef Run : Base<\"run\">;"),
        ("ron", "// Doc.\nRun(\n    next: Notify,\n)"),
        ("beancount", "; Doc.\n2024-01-01 open Assets:Run\n2024-01-02 * \"run\"\n  Assets:Run  1 EUR"),
        ("rst", ".. _run:\n\nRun\n---\n\n.. include:: notify"),
        ("bibtex", "% Doc.\n@article{run2024,\n  crossref = {notify},\n}"),
        ("requirements", "# Doc.\nrun==1.0\n-r notify.txt"),
        ("gn", "# Doc.\nsource_set(\"run\") {\n  deps = [ \":notify\" ]\n}"),
        ("kconfig", "# Doc.\nconfig RUN\n\tbool \"Run\"\n\tselect NOTIFY"),
        ("properties", "# Doc.\nrun.limit = 1\nrun.notify = 2"),
        ("ini", "; Doc.\n[run]\nnotify = 1"),
        ("dotenv", "# Doc.\nRUN_LIMIT=1\nRUN_NOTIFY=2"),
        ("oracleforms", "-- Doc.\nFUNCTION run(x IN NUMBER) RETURN NUMBER IS\nBEGIN\n  RETURN notify(x);\nEND;"),
        ("func", ";; Doc.\nint run(int x) impure {\n    return notify(x);\n}"),
        ("sshconfig", "# Doc.\nHost run\n    ProxyJump notify"),
        ("hyprlang", "# Doc.\nsource = notify.hypr\n$run = 1"),
        ("crystal", "# Doc.\ndef run\n  notify()\nend"),
        ("awk", "# Doc.\nfunction run(x) {\n  return notify(x)\n}"),
        ("starlark", "# Doc.\ndef run(x):\n    return notify(x)"),
        ("pascal", "// Doc.\nfunction Run(X: Integer): Integer;\nbegin\n  Run := Notify(X);\nend;"),
        ("pony", "// Doc.\nclass C\n  fun run(x: I64): I64 =>\n    notify(x)"),
        ("vimscript", "\" Doc.\nfunction! s:Run(x)\n  return Notify(a:x)\nendfunction"),
        ("ruby", "# Doc.\ndef run\n return notify() if x\nend"),
        ("python", "def run():\n    \"\"\"Doc.\"\"\"\n    if (x):\n        return notify()"),
        ("java", "/** Doc. */\nclass C { void run() { if (x) { return; } notify(); } }"),
        ("haskell", "-- | Doc.\nrun :: Int\nrun = if x then notify 1 else 0"),
        ("ocaml", "(* Doc. *)\nlet run x = if x then notify x else 0"),
        ("scala", "/** Doc. */\nclass C { def run(): Unit = { if (x) return; notify() } }"),
        ("kotlin", "/** Doc. */\nfun run() { if (x) { return }\n notify() }"),
        ("r", "# Doc.\nrun <- function(x) { if (x) return(0)\n notify(x) }"),
        ("julia", "\"Doc.\"\nfunction run()\n if (x)\n  return notify()\n end\nend"),
        ("erlang", "%% Doc.
-module(ledger).
charge() -> case(x) of true -> notify(1) end."),
        ("nim", "## Doc.
proc run() =
  if (x):
    return
  notify()"),
        ("elixir", "# Doc.
defmodule Ledger do
 def run do
  if(x) do notify() end
 end
end"),
        ("gdscript", "## Doc.
class_name Ledger

func run():
 if (x):
  return
 notify()"),
        ("typescript", "// Doc.\nfunction run() { if (x) { return; } notify(); }"),
        ("javascript", "// Doc.\nfunction run() { if (x) { return; } notify(); }"),
        ("csharp", "/// Doc.\nclass C { void Run() { if (x) { Notify(); } return; } }"),
        ("php", "/// Doc.\nfunction run() { if ($x) { return; } notify(); }"),
        ("vb", "''' Doc.
Sub Run()
 If (x) Then Return
 Notify()
End Sub"),
        ("go", "// Doc.\nfunc Run() { if x { return }\n notify() }"),
        ("zig", "// Doc.\npub fn run() void { if (x) { return; }\n notify(); }"),
        ("c", "// Doc.
int run(void) { if (x) { return 0; }
 return notify(1); }"),
        ("cpp", "/// Doc.
int run() { if (x) { return 0; }
 return notify(); }"),
        ("swift", "/// Doc.
func run() { if (x) { return }
 notify() }"),
        ("d", "/// Doc.\nvoid run() { if (x) { return; } notify(); }"),
        ("ada", "-- Doc.
procedure Run is begin if (X) then Notify; end if; end Run;"),
        ("dart", "/// Doc.\nclass C { void run() { if (x) { return; } notify(); } }"),
        ("elm", "-- Doc.\nrun = if x then notify 1 else 0"),
        ("clojure", "; Doc.
(defn run [x] (if (x) (notify x) 0))"),
        ("lean", "-- Doc.\ndef run := Foo.bar (Baz.match 1)"),
        ("gleam", "// Doc.\npub fn run() { case x { True -> notify() } }"),
        ("purescript", "-- | Doc.\nrun = if x then notify 1 else 0"),
        ("lua", "-- Doc.
function run() if (x) then return notify() end end"),
        ("perl", "# Doc.
sub run { if ($x) { return notify() } }"),
        ("mojo", "# Doc.
fn run():
    if (x):
        return
    notify()"),
        ("fortran", "! Doc.
subroutine run
 if (x) return
 call notify
end subroutine"),
        ("graphql", "# Doc.\ntype Query { run: Int }"),
        ("protobuf", "// Doc.\nmessage M { int32 run = 1; }"),
        ("solidity", "// Doc.\nabstract contract C { function run() public { if (x) { return; } notify(); } }"),
        ("verilog", "// Doc.\nmodule run; task go(); if (x) notify(); endtask endmodule"),
        ("thrift", "// Doc.\nstruct S { 1: i32 run }"),
        ("flatbuffers", "// Doc.\ntable T { run: int; }"),
        ("capnproto", "# Doc.\nstruct S { run @0 :Int32; }"),
    ];

    #[test]
    fn invalid_rules_are_refused_with_context() {
        let go = builtin("go").unwrap().text;
        for (text, expected) in [
            (
                go.replace("schema_version = 1", "schema_version = 2"),
                "schema_version",
            ),
            (go.replace("revision = 1", "revision = 0"), "revision"),
            (
                go.replace("language = \"go\"", "language = \"ruby\""),
                "language",
            ),
            (go.replace("braces = true", "brcaes = true"), "brcaes"),
            (go.replace("body_after_receiver", "magic"), "magic"),
            (
                go.replace("line_comments = [\"//\"]", "line_comments = [\"\"]"),
                "nonempty",
            ),
            (
                go.replace("receivers = [\".\"]", "receivers = [\".\", \".\"]"),
                "duplicate",
            ),
            (
                go.replace("receivers = [\".\"]", "receivers = [\"???\"]"),
                "cannot emit",
            ),
            (
                go.replace("braces = true", "braces = true\nmodifiers = [\"func\"]"),
                "conflicting",
            ),
            ("x".repeat(MAX_RULE_BYTES + 1).to_string(), "64 KiB"),
        ] {
            let error = RuleFile::read("go", &text).unwrap_err().to_string();
            assert!(error.contains("go.toml"), "{error}");
            assert!(error.contains(expected), "{expected}: {error}");
        }
        let ruby = format!(
            "{}\n[generic]\nbraces = true\nreceivers = []\ndefinitions = []",
            builtin("ruby").unwrap().text
        );
        assert!(RuleFile::read("ruby", &ruby)
            .unwrap_err()
            .to_string()
            .contains("native syntax"));
    }

    #[test]
    fn go_handles_receivers_anonymous_bodies_and_unicode() {
        let src = "package p\n// Runner — executes work.\nfunc (r *Runner) Run() {\n work := func() { notify() }\n work()\n}\nfunc After() { new(T); function(); delete(m, key) }";
        let facts = RuleSet::load(None).unwrap().parse(Language::Go, src);
        assert_eq!(facts.defines, ["p", "Run", "After"]);
        assert!(!facts.had_errors, "{facts:?}");
        for (owner, callee) in [
            ("Run", "notify"),
            ("Run", "work"),
            ("After", "new"),
            ("After", "function"),
            ("After", "delete"),
        ] {
            assert!(
                facts
                    .calls
                    .iter()
                    .any(|(a, b, _)| a == owner && b == callee),
                "{facts:?}"
            );
        }
        for (_, (start, end)) in &facts.ranges {
            assert!(src.get(*start as usize..*end as usize).is_some());
        }
        assert!(
            RuleSet::load(None)
                .unwrap()
                .parse(Language::Go, "func Broken() {")
                .had_errors
        );
    }

    #[test]
    fn ruby_constant_paths_are_not_bare_method_calls() {
        let facts = RuleSet::load(None).unwrap().parse(
            Language::Ruby,
            "def run\n Registry::Thing\n client.fetch\nend",
        );
        assert!(facts
            .calls
            .iter()
            .any(|(a, b, _)| a == "run" && b == "fetch"));
        assert!(!facts.calls.iter().any(|(_, b, _)| b == "Thing"));
    }

    #[test]
    fn ruby_blocks_do_not_consume_the_following_method() {
        let source = "class C\n def first\n  return if done?\n  while ready? do\n   entries.each { |e| send_entry(e) }\n  end\n end\n def second\n  notify()\n end\nend";
        let facts = RuleSet::load(None).unwrap().parse(Language::Ruby, source);
        assert_eq!(facts.defines, ["C", "first", "second"]);
        assert!(!facts.had_errors, "{facts:?}");
        assert!(facts
            .calls
            .iter()
            .any(|(a, b, _)| a == "first" && b == "send_entry"));
        assert!(facts
            .calls
            .iter()
            .any(|(a, b, _)| a == "second" && b == "notify"));
        let (_, (start, end)) = facts.ranges.iter().find(|(n, _)| n == "second").unwrap();
        assert_eq!(
            &source[*start as usize..*end as usize],
            "def second\n  notify()\n end"
        );
    }
}
