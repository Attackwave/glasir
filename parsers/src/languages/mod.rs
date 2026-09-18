pub(crate) mod csharp;
pub(crate) mod elixir;
pub(crate) mod gdscript;
pub(crate) mod erlang;
pub(crate) mod haskell;
pub(crate) mod java;
pub(crate) mod julia;
pub(crate) mod kotlin;
pub(crate) mod nim;
pub(crate) mod ocaml;
pub(crate) mod php;
pub(crate) mod python;
pub(crate) mod r;
pub(crate) mod ruby;
pub(crate) mod rust;
pub(crate) mod scala;
pub(crate) mod typescript;
pub(crate) mod vb;
pub(crate) mod ada;
pub(crate) mod c;
pub(crate) mod cpp;
pub(crate) mod clojure;
pub(crate) mod d;
pub(crate) mod dart;
pub(crate) mod elm;
pub(crate) mod fortran;
pub(crate) mod lean;
pub(crate) mod mojo;
pub(crate) mod lua;
pub(crate) mod perl;
pub(crate) mod purescript;
pub(crate) mod swift;
pub(crate) mod verilog;
pub(crate) mod cmake;
pub(crate) mod objc;
pub(crate) mod tcl;
pub(crate) mod janet;
pub(crate) mod plsql;
pub(crate) mod makefile;
pub(crate) mod smali;
pub(crate) mod template;
pub(crate) mod nasm;
pub(crate) mod sigil;
pub(crate) mod tagged;
pub(crate) mod arrow;
pub(crate) mod schema;
pub(crate) mod reference;
pub(crate) mod keyvalue;
pub(crate) mod labelled;
pub(crate) mod blockconf;
pub(crate) mod meson;

/// Dispatches a language whose syntax needs Rust. `None` means the language is
/// driven entirely by its `[generic]` table.
///
/// A match arm rather than a field on `Builtin`: a function pointer there would
/// let a language name a module that does not exist, where this fails to
/// compile.
pub(crate) fn parse(
    language: &str,
    src: &str,
    style: crate::lexer::CommentStyle<'_>,
    calls: &crate::rules::Calls,
) -> Option<crate::facts::FileFacts> {
    Some(match language {
        "csharp" => csharp::parse(src, style, calls),
        "elixir" => elixir::parse(src, style, calls),
        "gdscript" => gdscript::parse(src, style, calls),
        "erlang" => erlang::parse(src, style, calls),
        "rust" => rust::parse(src, style, calls),
        "r" => r::parse(src, style, calls),
        "cmake" => cmake::parse(src, style, calls),
        "objc" => objc::parse(src, style, calls),
        "tcl" => tcl::parse(src, style, calls),
        "janet" => janet::parse(src, style, calls),
        "plsql" => plsql::parse(src, style, calls),
        // Apex is Java's syntax with Salesforce's library on top: a method is
        // `Integer charge(...)`, type then name, which is the shape the Java
        // module already reads. Measured with a keyword table it found only
        // the class — 43 definitions per 1,000 lines against 174 here.
        "apex" => java::parse(src, style, calls),
        // CUDA and GLSL are C's declaration shape — `float charge(float a)`,
        // type then name — which is what the C++ module already reads. A
        // keyword table cannot express it, and a third copy of that loop would
        // drift from this one.
        "cuda" => cpp::parse(src, style, calls),
        "makefile" => makefile::parse(src, style, calls),
        "smali" => smali::parse(src, style, calls),
        // SOQL is Salesforce's SQL dialect: case-insensitive, and written
        // upper-case by convention, which is the property the PL/SQL module
        // exists for.
        "soql" => plsql::parse(src, style, calls),
        // Go templates and Liquid name a block inside a delimiter; the
        // openers differ, the shape does not.
        "gotemplate" | "liquid" => template::parse(src, style, calls),
        "nasm" => nasm::parse(src, style, calls),
        // Just is Make's shape — `target:` then an indented recipe.
        "just" => makefile::parse(src, style, calls),
        // Luau and Teal are Lua with type annotations; the annotation sits
        // where Lua allows any expression, so the scanner reads both already.
        "luau" | "teal" => lua::parse(src, style, calls),
        // Fennel is a Lisp on Lua: `(fn name [args] ...)`, the shape the Janet
        // module reads.
        "fennel" => janet::parse(src, style, calls),
        // Jinja names a block inside a delimiter exactly as Liquid does, and
        // Blade does the same with `@section('name')`.
        "jinja" | "blade" => template::parse(src, style, calls),
        // ReScript defines by assignment — `let name = (...) => ...` — which
        // is the shape the Meson/Jsonnet module reads.
        "rescript" => meson::parse(src, style, calls),
        // Typst writes `#let charge(owner, amount) = {…}` — the Meson shape,
        // name then assignment. Its line-based scanner recorded no calls at
        // all: 750 definitions per 1,000 lines and zero edges.
        "typst" => meson::parse(src, style, calls),
        // LLVM IR puts a sigil between keyword and name (`define i32 @run`),
        // Wolfram puts a bracket between name and `:=`. Both separate the two
        // halves a keyword table needs adjacent.
        "llvm" | "wolfram" => sigil::parse(src, style, calls),
        // CFML carries the name in a tag attribute, TLA+ marks a definition
        // with `==` after the parameter list; both put it past a delimiter.
        "cfml" | "tlaplus" => tagged::parse(src, style, calls),
        // ArkTS is TypeScript with a UI dialect on top.
        "arkts" => typescript::parse(src, style, calls),
        // ISPC is C's declaration shape with `uniform`/`varying` modifiers.
        "ispc" => cpp::parse(src, style, calls),
        // ChiaLisp is a Lisp: `(defun name (args) …)`.
        "chialisp" => janet::parse(src, style, calls),
        // SOSL is Salesforce's search dialect beside SOQL, same PL/SQL shape.
        "sosl" => plsql::parse(src, style, calls),
        // Agda declares like Haskell: `charge : A -> B` then `charge x = …`.
        "agda" => haskell::parse(src, style, calls),
        // Astro's frontmatter is TypeScript between `---` fences.
        "astro" => typescript::parse(src, style, calls),
        // Slang is HLSL's declaration shape, which C++ already reads.
        "slang" => cpp::parse(src, style, calls),
        // Magma and Pine define by assignment: `f := function(x) … end` and
        // `f(x) => body`.
        "magma" | "pine" => meson::parse(src, style, calls),
        // A diagram states its edges directly — `charge --> refuse` — and has
        // no call syntax at all.
        "mermaid" => arrow::parse(src, style, calls),
        // A BitBake task is a shell function — `do_compile() { … }` — which
        // defines by shape, so a keyword table found nothing: 0 definitions
        // with every reference on `<module>`.
        "bitbake" => crate::category_07_scripting::parse_bash(src),
        // An interface definition language states its dependencies as
        // references in a field rather than as calls.
        "smithy" | "wit" => schema::parse(src, style, calls),
        // Description formats: a device tree's `<&gic>`, a linker script's
        // section references, TableGen's `def X : Base<…>`, RON's nested
        // structs, a Go module's `require`, a ledger's accounts. Each states
        // dependencies as references rather than calls.
        "devicetree" | "linkerscript" | "gomod" | "tablegen" | "ron" | "beancount" => {
            reference::parse(src, style, calls)
        }
        // Nickel and Pkl are configuration *languages* with real functions,
        // defining by assignment like Meson.
        "nickel" | "pkl" => meson::parse(src, style, calls),
        // A settings file declares by `key = value`; its structure is the
        // dotted path and the `[section]` header, which is the only thing in
        // such a file worth asking about.
        "properties" | "ini" | "dotenv" => keyvalue::parse(src, style, calls),
        // Oracle Forms is PL/SQL with a form around it: same case-insensitive
        // package body, same IS/BEGIN/END nesting.
        "oracleforms" => plsql::parse(src, style, calls),
        // FunC is C's declaration shape with an `impure` modifier after the
        // parameter list.
        "func" => cpp::parse(src, style, calls),
        // A named block plus references between blocks: `Host charge` with
        // `ProxyJump refuse`, `source = other.conf`.
        "sshconfig" | "hyprlang" => blockconf::parse(src, style, calls),
        // Four formats that declare a label and refer to it by name: RST's
        // `.. _charge:`, BibTeX's `@article{charge}`, GN's
        // `source_set("charge")`, Kconfig's `config CHARGE`.
        "rst" | "bibtex" | "requirements" | "gn" | "kconfig" => {
            labelled::parse(src, style, calls)
        }
        // Meson and Jsonnet share a module: both define by assignment, and the
        // only difference is the token.
        "meson" | "jsonnet" => meson::parse(src, style, calls),
        "ruby" => ruby::parse(src, style, calls),
        "python" => python::parse(src, style, calls),
        "java" => java::parse(src, style, calls),
        "haskell" => haskell::parse(src, style, calls),
        "ocaml" => ocaml::parse(src, style, calls),
        "scala" => scala::parse(src, style, calls),
        "kotlin" => kotlin::parse(src, style, calls),
        "julia" => julia::parse(src, style, calls),
        "nim" => nim::parse(src, style, calls),
        "php" => php::parse(src, style, calls),
        "typescript" | "javascript" => typescript::parse(src, style, calls),
        "vb" => vb::parse(src, style, calls),
        "ada" => ada::parse(src, style, calls),
        "c" => c::parse(src, style, calls),
        "cpp" => cpp::parse(src, style, calls),
        "d" => d::parse(src, style, calls),
        "dart" => dart::parse(src, style, calls),
        "swift" => swift::parse(src, style, calls),
        "clojure" => clojure::parse(src, style, calls),
        "elm" => elm::parse(src, style, calls),
        "purescript" => purescript::parse(src, style, calls),
        "lean" => lean::parse(src, style, calls),
        "mojo" => mojo::parse(src, style, calls),
        "fortran" => fortran::parse(src, style, calls),
        "verilog" => verilog::parse(src, style, calls),
        "perl" => perl::parse(src, style, calls),
        "lua" => lua::parse(src, style, calls),
        _ => return None,
    })
}
