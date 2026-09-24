//! Native symbol and documentation scanners for ten language categories.
//! TOML rules configure extraction; language-specific syntax stays in Rust.

pub mod doc;
pub mod facts;
pub mod generic;
mod languages;
pub mod lexer;
pub mod rules;
pub mod scope;

pub mod category_01_backend;
pub mod category_02_modern_systems;
pub mod category_03_functional;
pub mod category_04_devops_config;
pub mod category_05_web_ui;
pub mod category_06_database;
pub mod category_07_scripting;
pub mod category_08_data_science;
pub mod category_09_contracts_hardware;
pub mod category_10_documents;

pub use doc::clean_doc;
pub use facts::FileFacts;
pub use scope::ScopeStack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageCategory {
    MainstreamBackend,
    ModernSystems,
    Functional,
    DevopsConfig,
    WebUI,
    Database,
    Scripting,
    DataScience,
    ContractsHardware,
    Documents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    // 1. Mainstream & Backend
    Rust,
    Go,
    Python,
    TypeScript,
    JavaScript,
    Java,
    CSharp,
    Ruby,
    Crystal,
    Cmake,
    ObjC,
    Tcl,
    Janet,
    Plsql,
    Apex,
    Bicep,
    Puppet,
    Cuda,
    Makefile,
    Meson,
    Jsonnet,
    Cfscript,
    Smali,
    Prisma,
    Soql,
    GoTemplate,
    Liquid,
    Nasm,
    Just,
    Hare,
    Move,
    Squirrel,
    Luau,
    Teal,
    Fennel,
    Jinja,
    Blade,
    ReScript,
    ObjectScript,
    Qml,
    Cairo,
    LlvmIr,
    Wolfram,
    Cfml,
    TlaPlus,
    ArkTs,
    Templ,
    Ispc,
    ChiaLisp,
    Sosl,
    Agda,
    Astro,
    Slang,
    BitBake,
    Magma,
    Pine,
    Sway,
    Smithy,
    Wit,
    Mermaid,
    DeviceTree,
    LinkerScript,
    GoMod,
    Nickel,
    Pkl,
    TableGen,
    Ron,
    Beancount,
    Rst,
    BibTeX,
    Requirements,
    Gn,
    Kconfig,
    Properties,
    Ini,
    DotEnv,
    OracleForms,
    FunC,
    SshConfig,
    Hyprlang,
    Awk,
    Starlark,
    Pascal,
    Pony,
    Vimscript,
    Php,
    Groovy,
    Vb,
    Cobol,
    // 2. Modern Systems
    Zig,
    Nim,
    Odin,
    C,
    Cpp,
    Swift,
    Ada,
    D,
    Wat,
    // 3. Functional
    Haskell,
    Elixir,
    OCaml,
    Scala,
    Kotlin,
    Erlang,
    FSharp,
    Clojure,
    Elm,
    Gleam,
    PureScript,
    Lisp,
    Lean,
    // 4. DevOps & Config
    HclTerraform,
    Yaml,
    Toml,
    Dockerfile,
    Nix,
    Json,
    // 5. Web & UI
    Html,
    Css,
    Dart,
    Vue,
    Svelte,
    Xml,
    // 6. Databases & Serialization
    Sql,
    Protobuf,
    GraphQL,
    Thrift,
    FlatBuffers,
    CapnProto,
    Cypher,
    // 7. Scripting
    Bash,
    Perl,
    Lua,
    PowerShell,
    GdScript,
    Batch,
    Fish,
    // 8. Data Science
    R,
    Julia,
    Matlab,
    Mojo,
    Fortran,
    // 9. Contracts & Hardware
    Solidity,
    Verilog,
    Vhdl,
    Shader,
    // 10. Documents
    Markdown,
    Typst,
    Latex,
}

/// Every extension a scanner answers to. Kept beside `from_extension` so a
/// caller can enumerate what is covered without restating the table.
pub const EXTENSIONS: &[&str] = &[
    "ada",
    "adb",
    "ads",
    "apex",
    "asm",
    "awk",
    "bash",
    "bat",
    "bicep",
    "blade",
    "build",
    "bzl",
    "c",
    "c++",
    "cairo",
    "capnp",
    "cbl",
    "cc",
    "cfc",
    "cfm",
    "cjs",
    "cl",
    "clj",
    "cljc",
    "cljs",
    "cls",
    "clsp",
    "cmake",
    "cmd",
    "cob",
    "comp",
    "cpp",
    "cpy",
    "cql",
    "cr",
    "cs",
    "css",
    "cts",
    "cu",
    "cxx",
    "cypher",
    "d",
    "dart",
    "ddl",
    "dockerfile",
    "dtx",
    "edn",
    "el",
    "elm",
    "erl",
    "ets",
    "ex",
    "exs",
    "f",
    "f03",
    "f08",
    "f90",
    "f95",
    "fbs",
    "fish",
    "fnl",
    "for",
    "frag",
    "fs",
    "fsi",
    "fsx",
    "gd",
    "gemspec",
    "geom",
    "gleam",
    "glsl",
    "go",
    "gql",
    "gradle",
    "graphql",
    "groovy",
    "gsh",
    "gvy",
    "gy",
    "h",
    "h++",
    "ha",
    "hcl",
    "hh",
    "hlsl",
    "hpp",
    "hrl",
    "hs",
    "htm",
    "html",
    "hxx",
    "ins",
    "ispc",
    "j2",
    "janet",
    "java",
    "jl",
    "js",
    "json",
    "json5",
    "jsonc",
    "jsonnet",
    "jsx",
    "just",
    "kt",
    "kts",
    "latex",
    "lean",
    "less",
    "lhs",
    "liquid",
    "lisp",
    "ll",
    "lsp",
    "ltx",
    "lua",
    "luau",
    "m",
    "mac",
    "markdown",
    "matlab",
    "md",
    "mjs",
    "mk",
    "ml",
    "mli",
    "mm",
    "mojo",
    "move",
    "mts",
    "nim",
    "nix",
    "nut",
    "oct",
    "odin",
    "pas",
    "php",
    "php3",
    "php4",
    "php5",
    "php7",
    "php8",
    "phps",
    "phtml",
    "pl",
    "plist",
    "pls",
    "pm",
    "pony",
    "pp",
    "prisma",
    "proto",
    "ps1",
    "psd1",
    "psm1",
    "purs",
    "py",
    "pyi",
    "qml",
    "r",
    "rahit",
    "rake",
    "rb",
    "rcall",
    "rchit",
    "res",
    "rgen",
    "rint",
    "rkt",
    "rmiss",
    "rs",
    "sc",
    "scala",
    "scheme",
    "scm",
    "scss",
    "sh",
    "smali",
    "sol",
    "soql",
    "sosl",
    "sql",
    "sty",
    "sv",
    "svelte",
    "svg",
    "svh",
    "swift",
    "tcl",
    "templ",
    "tesc",
    "tese",
    "tex",
    "tf",
    "tfvars",
    "thrift",
    "tl",
    "tla",
    "tmpl",
    "toml",
    "trigger",
    "ts",
    "tsx",
    "typ",
    "v",
    "vb",
    "vbs",
    "vert",
    "vhd",
    "vhdl",
    "vim",
    "vue",
    "wast",
    "wat",
    "wgsl",
    "wl",
    "wsdl",
    "xaml",
    "xml",
    "xsd",
    "yaml",
    "yml",
    "zig",
    "zsh",
];

impl Language {
    /// How many languages have a scanner. Counted from the extension table
    /// rather than stated, so the number cannot drift from what `from_extension`
    /// actually answers to.
    pub fn count() -> usize {
        let mut seen: Vec<Self> = Vec::new();
        for e in EXTENSIONS {
            if let Some(l) = Self::from_extension(e) {
                if !seen.contains(&l) {
                    seen.push(l);
                }
            }
        }
        seen.len()
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "py" | "pyi" => Some(Self::Python),
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "java" => Some(Self::Java),
            "cs" => Some(Self::CSharp),
            "rb" | "rake" | "gemspec" => Some(Self::Ruby),
            "cr" => Some(Self::Crystal),
            "cmake" => Some(Self::Cmake),
            "mm" => Some(Self::ObjC),
            "tcl" => Some(Self::Tcl),
            "janet" => Some(Self::Janet),
            "pls" => Some(Self::Plsql),
            //  is LaTeX's class file and was here first; Salesforce
            // also writes  and , which do not collide.
            "apex" | "trigger" => Some(Self::Apex),
            "bicep" => Some(Self::Bicep),
            "pp" => Some(Self::Puppet),
            "cu" => Some(Self::Cuda),
            "mk" => Some(Self::Makefile),
            "build" => Some(Self::Meson),
            "jsonnet" => Some(Self::Jsonnet),
            "cfc" => Some(Self::Cfscript),
            "smali" => Some(Self::Smali),
            "prisma" => Some(Self::Prisma),
            "soql" => Some(Self::Soql),
            "tmpl" => Some(Self::GoTemplate),
            "liquid" => Some(Self::Liquid),
            "asm" => Some(Self::Nasm),
            "just" => Some(Self::Just),
            "ha" => Some(Self::Hare),
            "move" => Some(Self::Move),
            "nut" => Some(Self::Squirrel),
            "luau" => Some(Self::Luau),
            "tl" => Some(Self::Teal),
            "fnl" => Some(Self::Fennel),
            "j2" => Some(Self::Jinja),
            "blade" => Some(Self::Blade),
            "res" => Some(Self::ReScript),
            "mac" => Some(Self::ObjectScript),
            "qml" => Some(Self::Qml),
            "cairo" => Some(Self::Cairo),
            "ll" => Some(Self::LlvmIr),
            "wl" => Some(Self::Wolfram),
            "cfm" => Some(Self::Cfml),
            "tla" => Some(Self::TlaPlus),
            "ets" => Some(Self::ArkTs),
            "templ" => Some(Self::Templ),
            "ispc" => Some(Self::Ispc),
            "clsp" => Some(Self::ChiaLisp),
            "sosl" => Some(Self::Sosl),
            "agda" => Some(Self::Agda),
            "astro" => Some(Self::Astro),
            "slang" => Some(Self::Slang),
            "bb" => Some(Self::BitBake),
            "mag" => Some(Self::Magma),
            "pine" => Some(Self::Pine),
            "sw" => Some(Self::Sway),
            "smithy" => Some(Self::Smithy),
            "wit" => Some(Self::Wit),
            "mmd" => Some(Self::Mermaid),
            "dts" => Some(Self::DeviceTree),
            "ld" => Some(Self::LinkerScript),
            "mod" => Some(Self::GoMod),
            "ncl" => Some(Self::Nickel),
            "pkl" => Some(Self::Pkl),
            "td" => Some(Self::TableGen),
            "ron" => Some(Self::Ron),
            "beancount" => Some(Self::Beancount),
            "rst" => Some(Self::Rst),
            "bib" => Some(Self::BibTeX),
            "gn" => Some(Self::Gn),
            "requirements" => Some(Self::Requirements),
            "kconfig" => Some(Self::Kconfig),
            "properties" => Some(Self::Properties),
            "ini" => Some(Self::Ini),
            "env" => Some(Self::DotEnv),
            "frm" => Some(Self::OracleForms),
            "fc" => Some(Self::FunC),
            "sshconfig" => Some(Self::SshConfig),
            "hypr" => Some(Self::Hyprlang),
            "prc" => Some(Self::OracleForms),
            "awk" => Some(Self::Awk),
            "bzl" => Some(Self::Starlark),
            "pas" => Some(Self::Pascal),
            "pony" => Some(Self::Pony),
            "vim" => Some(Self::Vimscript),
            "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "php8" | "phps" => {
                Some(Self::Php)
            }
            "groovy" | "gvy" | "gy" | "gsh" | "gradle" => Some(Self::Groovy),
            "vb" | "vbs" => Some(Self::Vb),
            "cob" | "cbl" | "cpy" => Some(Self::Cobol),

            "zig" => Some(Self::Zig),
            "nim" => Some(Self::Nim),
            "odin" => Some(Self::Odin),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => Some(Self::Cpp),
            "swift" => Some(Self::Swift),
            "adb" | "ads" | "ada" => Some(Self::Ada),
            "d" => Some(Self::D),
            "wat" | "wast" => Some(Self::Wat),

            "hs" | "lhs" => Some(Self::Haskell),
            "ex" | "exs" => Some(Self::Elixir),
            "ml" | "mli" => Some(Self::OCaml),
            "scala" | "sc" => Some(Self::Scala),
            "kt" | "kts" => Some(Self::Kotlin),
            "erl" | "hrl" => Some(Self::Erlang),
            "fs" | "fsi" | "fsx" => Some(Self::FSharp),
            "clj" | "cljs" | "cljc" | "edn" => Some(Self::Clojure),
            "elm" => Some(Self::Elm),
            "gleam" => Some(Self::Gleam),
            "purs" => Some(Self::PureScript),
            "lisp" | "lsp" | "cl" | "el" | "rkt" | "scm" | "scheme" => Some(Self::Lisp),
            "lean" => Some(Self::Lean),

            "tf" | "tfvars" | "hcl" => Some(Self::HclTerraform),
            "yml" | "yaml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "dockerfile" => Some(Self::Dockerfile),
            "nix" => Some(Self::Nix),
            "json" | "jsonc" | "json5" => Some(Self::Json),

            "html" | "htm" => Some(Self::Html),
            "css" | "scss" | "less" => Some(Self::Css),
            "dart" => Some(Self::Dart),
            "vue" => Some(Self::Vue),
            "svelte" => Some(Self::Svelte),
            "xml" | "svg" | "xaml" | "plist" | "xsd" | "wsdl" => Some(Self::Xml),

            "sql" | "ddl" => Some(Self::Sql),
            "proto" => Some(Self::Protobuf),
            "graphql" | "gql" => Some(Self::GraphQL),
            "thrift" => Some(Self::Thrift),
            "fbs" => Some(Self::FlatBuffers),
            "capnp" => Some(Self::CapnProto),
            "cql" | "cypher" => Some(Self::Cypher),

            "sh" | "bash" | "zsh" => Some(Self::Bash),
            "pl" | "pm" => Some(Self::Perl),
            "lua" => Some(Self::Lua),
            "ps1" | "psm1" | "psd1" => Some(Self::PowerShell),
            "gd" => Some(Self::GdScript),
            "bat" | "cmd" => Some(Self::Batch),
            "fish" => Some(Self::Fish),

            "r" => Some(Self::R),
            "jl" => Some(Self::Julia),
            "m" | "matlab" | "oct" => Some(Self::Matlab),
            "mojo" => Some(Self::Mojo),
            "f" | "f90" | "f95" | "f03" | "f08" | "for" => Some(Self::Fortran),

            "sol" => Some(Self::Solidity),
            "v" | "sv" | "svh" => Some(Self::Verilog),
            "vhd" | "vhdl" => Some(Self::Vhdl),
            "wgsl" | "glsl" | "hlsl" | "vert" | "frag" | "geom" | "comp" | "tesc" | "tese"
            | "rgen" | "rint" | "rahit" | "rchit" | "rmiss" | "rcall" => Some(Self::Shader),

            "md" | "markdown" => Some(Self::Markdown),
            "typ" => Some(Self::Typst),
            "tex" | "latex" | "sty" | "cls" | "dtx" | "ins" | "ltx" => Some(Self::Latex),

            _ => None,
        }
    }

    pub fn category(&self) -> LanguageCategory {
        match self {
            Self::Rust
            | Self::Go
            | Self::Python
            | Self::TypeScript
            | Self::JavaScript
            | Self::Java
            | Self::CSharp
            | Self::Ruby
            | Self::Php
            | Self::Groovy
            | Self::Vb
            | Self::Crystal
            | Self::Cmake
            | Self::ObjC
            | Self::Tcl
            | Self::Janet
            | Self::Plsql
            | Self::Apex
            | Self::Bicep
            | Self::Puppet
            | Self::Cuda
            | Self::Makefile
            | Self::Meson
            | Self::Jsonnet
            | Self::Cfscript
            | Self::Smali
            | Self::Prisma
            | Self::Soql
            | Self::GoTemplate
            | Self::Liquid
            | Self::Nasm
            | Self::Just
            | Self::Hare
            | Self::Move
            | Self::Squirrel
            | Self::Luau
            | Self::Teal
            | Self::Fennel
            | Self::Jinja
            | Self::Blade
            | Self::ReScript
            | Self::ObjectScript
            | Self::Qml
            | Self::Cairo
            | Self::LlvmIr
            | Self::Wolfram
            | Self::Cfml
            | Self::TlaPlus
            | Self::ArkTs
            | Self::Templ
            | Self::Ispc
            | Self::ChiaLisp
            | Self::Sosl
            | Self::Agda
            | Self::Astro
            | Self::Slang
            | Self::BitBake
            | Self::Magma
            | Self::Pine
            | Self::Sway
            | Self::Smithy
            | Self::Wit
            | Self::Mermaid
            | Self::DeviceTree
            | Self::LinkerScript
            | Self::GoMod
            | Self::Nickel
            | Self::Pkl
            | Self::TableGen
            | Self::Ron
            | Self::Beancount
            | Self::Rst
            | Self::BibTeX
            | Self::Requirements
            | Self::Gn
            | Self::Kconfig
            | Self::Properties
            | Self::Ini
            | Self::DotEnv
            | Self::OracleForms
            | Self::FunC
            | Self::SshConfig
            | Self::Hyprlang
            | Self::Awk
            | Self::Starlark
            | Self::Pascal
            | Self::Pony
            | Self::Vimscript
            | Self::Cobol => LanguageCategory::MainstreamBackend,

            Self::Zig
            | Self::Nim
            | Self::Odin
            | Self::C
            | Self::Cpp
            | Self::Swift
            | Self::Ada
            | Self::D
            | Self::Wat => LanguageCategory::ModernSystems,

            Self::Haskell
            | Self::Elixir
            | Self::OCaml
            | Self::Scala
            | Self::Kotlin
            | Self::Erlang
            | Self::FSharp
            | Self::Clojure
            | Self::Elm
            | Self::Gleam
            | Self::PureScript
            | Self::Lisp
            | Self::Lean => LanguageCategory::Functional,

            Self::HclTerraform
            | Self::Yaml
            | Self::Toml
            | Self::Dockerfile
            | Self::Nix
            | Self::Json => LanguageCategory::DevopsConfig,

            Self::Html | Self::Css | Self::Dart | Self::Vue | Self::Svelte | Self::Xml => {
                LanguageCategory::WebUI
            }

            Self::Sql
            | Self::Protobuf
            | Self::GraphQL
            | Self::Thrift
            | Self::FlatBuffers
            | Self::CapnProto
            | Self::Cypher => LanguageCategory::Database,

            Self::Bash
            | Self::Perl
            | Self::Lua
            | Self::PowerShell
            | Self::GdScript
            | Self::Batch
            | Self::Fish => LanguageCategory::Scripting,

            Self::R | Self::Julia | Self::Matlab | Self::Mojo | Self::Fortran => {
                LanguageCategory::DataScience
            }

            Self::Solidity | Self::Verilog | Self::Vhdl | Self::Shader => {
                LanguageCategory::ContractsHardware
            }

            Self::Markdown | Self::Typst | Self::Latex => LanguageCategory::Documents,
        }
    }

    pub fn parse(&self, src: &str) -> FileFacts {
        parse(src, *self)
    }
}

/// Main entry point to parse source code for any supported language.
pub fn parse(src: &str, lang: Language) -> FileFacts {
    match lang {
        Language::Rust => category_01_backend::parse_rust(src),
        Language::Go => category_01_backend::parse_go(src),
        Language::Python => category_01_backend::parse_python(src),
        Language::TypeScript | Language::JavaScript => {
            category_01_backend::parse_typescript_javascript(src)
        }
        Language::Java => category_01_backend::parse_java(src),
        Language::CSharp => category_01_backend::parse_csharp(src),
        Language::Ruby => category_01_backend::parse_ruby(src),
        Language::Php => category_01_backend::parse_php(src),
        Language::Groovy => category_01_backend::parse_groovy(src),
        Language::Vb => category_01_backend::parse_vb(src),
        Language::Cobol => category_01_backend::parse_cobol(src),
        // Driven entirely by `crystal.toml`. `parse` here is the fallback
        // chain; the production path is `rules::active().parse`, which reaches
        // the `[generic]` table without a Rust module. Routing it back through
        // the rules keeps one implementation rather than two.
        Language::Crystal => rules::active().parse(Language::Crystal, src),
        Language::Cmake => rules::active().parse(Language::Cmake, src),
        Language::ObjC => rules::active().parse(Language::ObjC, src),
        Language::Tcl => rules::active().parse(Language::Tcl, src),
        Language::Janet => rules::active().parse(Language::Janet, src),
        Language::Plsql => rules::active().parse(Language::Plsql, src),
        Language::Apex => rules::active().parse(Language::Apex, src),
        Language::Bicep => rules::active().parse(Language::Bicep, src),
        Language::Puppet => rules::active().parse(Language::Puppet, src),
        Language::Cuda => rules::active().parse(Language::Cuda, src),
        Language::Makefile => rules::active().parse(Language::Makefile, src),
        Language::Meson => rules::active().parse(Language::Meson, src),
        Language::Jsonnet => rules::active().parse(Language::Jsonnet, src),
        Language::Cfscript => rules::active().parse(Language::Cfscript, src),
        Language::Smali => rules::active().parse(Language::Smali, src),
        Language::Prisma => rules::active().parse(Language::Prisma, src),
        Language::Soql => rules::active().parse(Language::Soql, src),
        Language::GoTemplate => rules::active().parse(Language::GoTemplate, src),
        Language::Liquid => rules::active().parse(Language::Liquid, src),
        Language::Nasm => rules::active().parse(Language::Nasm, src),
        Language::Just => rules::active().parse(Language::Just, src),
        Language::Hare => rules::active().parse(Language::Hare, src),
        Language::Move => rules::active().parse(Language::Move, src),
        Language::Squirrel => rules::active().parse(Language::Squirrel, src),
        Language::Luau => rules::active().parse(Language::Luau, src),
        Language::Teal => rules::active().parse(Language::Teal, src),
        Language::Fennel => rules::active().parse(Language::Fennel, src),
        Language::Jinja => rules::active().parse(Language::Jinja, src),
        Language::Blade => rules::active().parse(Language::Blade, src),
        Language::ReScript => rules::active().parse(Language::ReScript, src),
        Language::ObjectScript => rules::active().parse(Language::ObjectScript, src),
        Language::Qml => rules::active().parse(Language::Qml, src),
        Language::Cairo => rules::active().parse(Language::Cairo, src),
        Language::LlvmIr => rules::active().parse(Language::LlvmIr, src),
        Language::Wolfram => rules::active().parse(Language::Wolfram, src),
        Language::Cfml => rules::active().parse(Language::Cfml, src),
        Language::TlaPlus => rules::active().parse(Language::TlaPlus, src),
        Language::ArkTs => rules::active().parse(Language::ArkTs, src),
        Language::Templ => rules::active().parse(Language::Templ, src),
        Language::Ispc => rules::active().parse(Language::Ispc, src),
        Language::ChiaLisp => rules::active().parse(Language::ChiaLisp, src),
        Language::Sosl => rules::active().parse(Language::Sosl, src),
        Language::Agda => rules::active().parse(Language::Agda, src),
        Language::Astro => rules::active().parse(Language::Astro, src),
        Language::Slang => rules::active().parse(Language::Slang, src),
        Language::BitBake => rules::active().parse(Language::BitBake, src),
        Language::Magma => rules::active().parse(Language::Magma, src),
        Language::Pine => rules::active().parse(Language::Pine, src),
        Language::Sway => rules::active().parse(Language::Sway, src),
        Language::Smithy => rules::active().parse(Language::Smithy, src),
        Language::Wit => rules::active().parse(Language::Wit, src),
        Language::Mermaid => rules::active().parse(Language::Mermaid, src),
        Language::DeviceTree => rules::active().parse(Language::DeviceTree, src),
        Language::LinkerScript => rules::active().parse(Language::LinkerScript, src),
        Language::GoMod => rules::active().parse(Language::GoMod, src),
        Language::Nickel => rules::active().parse(Language::Nickel, src),
        Language::Pkl => rules::active().parse(Language::Pkl, src),
        Language::TableGen => rules::active().parse(Language::TableGen, src),
        Language::Ron => rules::active().parse(Language::Ron, src),
        Language::Beancount => rules::active().parse(Language::Beancount, src),
        Language::Rst => rules::active().parse(Language::Rst, src),
        Language::BibTeX => rules::active().parse(Language::BibTeX, src),
        Language::Requirements => rules::active().parse(Language::Requirements, src),
        Language::Gn => rules::active().parse(Language::Gn, src),
        Language::Kconfig => rules::active().parse(Language::Kconfig, src),
        Language::Properties => rules::active().parse(Language::Properties, src),
        Language::Ini => rules::active().parse(Language::Ini, src),
        Language::DotEnv => rules::active().parse(Language::DotEnv, src),
        Language::OracleForms => rules::active().parse(Language::OracleForms, src),
        Language::FunC => rules::active().parse(Language::FunC, src),
        Language::SshConfig => rules::active().parse(Language::SshConfig, src),
        Language::Hyprlang => rules::active().parse(Language::Hyprlang, src),
        Language::Awk => rules::active().parse(Language::Awk, src),
        Language::Starlark => rules::active().parse(Language::Starlark, src),
        Language::Pascal => rules::active().parse(Language::Pascal, src),
        Language::Pony => rules::active().parse(Language::Pony, src),
        Language::Vimscript => rules::active().parse(Language::Vimscript, src),

        Language::Zig => category_02_modern_systems::parse_zig(src),
        Language::Nim => category_02_modern_systems::parse_nim(src),
        Language::Odin => category_02_modern_systems::parse_odin(src),
        Language::C => category_02_modern_systems::parse_c(src),
        Language::Cpp => category_02_modern_systems::parse_cpp(src),
        Language::Swift => category_02_modern_systems::parse_swift(src),
        Language::Ada => category_02_modern_systems::parse_ada(src),
        Language::D => category_02_modern_systems::parse_d(src),
        Language::Wat => category_02_modern_systems::parse_wat(src),

        Language::Haskell => category_03_functional::parse_haskell(src),
        Language::Elixir => category_03_functional::parse_elixir(src),
        Language::OCaml => category_03_functional::parse_ocaml(src),
        Language::Scala => category_03_functional::parse_scala(src),
        Language::Kotlin => category_03_functional::parse_kotlin(src),
        Language::Erlang => category_03_functional::parse_erlang(src),
        Language::FSharp => category_03_functional::parse_fsharp(src),
        Language::Clojure => category_03_functional::parse_clojure(src),
        Language::Elm => category_03_functional::parse_elm(src),
        Language::Gleam => category_03_functional::parse_gleam(src),
        Language::PureScript => category_03_functional::parse_purescript(src),
        Language::Lisp => category_03_functional::parse_lisp(src),
        Language::Lean => category_03_functional::parse_lean(src),

        Language::HclTerraform => category_04_devops_config::parse_hcl_terraform(src),
        Language::Yaml => category_04_devops_config::parse_yaml(src),
        Language::Toml => category_04_devops_config::parse_toml(src),
        Language::Dockerfile => category_04_devops_config::parse_dockerfile(src),
        Language::Nix => category_04_devops_config::parse_nix(src),
        Language::Json => category_04_devops_config::parse_json(src),

        Language::Html => category_05_web_ui::parse_html(src),
        Language::Css => category_05_web_ui::parse_css(src),
        Language::Dart => category_05_web_ui::parse_dart(src),
        Language::Vue => category_05_web_ui::parse_vue(src),
        Language::Svelte => category_05_web_ui::parse_svelte(src),
        Language::Xml => category_05_web_ui::parse_xml(src),

        Language::Sql => category_06_database::parse_sql(src),
        Language::Protobuf => category_06_database::parse_protobuf(src),
        Language::GraphQL => category_06_database::parse_graphql(src),
        Language::Thrift => category_06_database::parse_thrift(src),
        Language::FlatBuffers => category_06_database::parse_flatbuffers(src),
        Language::CapnProto => category_06_database::parse_capnp(src),
        Language::Cypher => category_06_database::parse_cypher(src),

        Language::Bash => category_07_scripting::parse_bash(src),
        Language::Perl => category_07_scripting::parse_perl(src),
        Language::Lua => category_07_scripting::parse_lua(src),
        Language::PowerShell => category_07_scripting::parse_powershell(src),
        Language::GdScript => category_07_scripting::parse_gdscript(src),
        Language::Batch => category_07_scripting::parse_batch(src),
        Language::Fish => category_07_scripting::parse_fish(src),

        Language::R => category_08_data_science::parse_r(src),
        Language::Julia => category_08_data_science::parse_julia(src),
        Language::Matlab => category_08_data_science::parse_matlab(src),
        Language::Mojo => category_08_data_science::parse_mojo(src),
        Language::Fortran => category_08_data_science::parse_fortran(src),

        Language::Solidity => category_09_contracts_hardware::parse_solidity(src),
        Language::Verilog => category_09_contracts_hardware::parse_verilog(src),
        Language::Vhdl => category_09_contracts_hardware::parse_vhdl(src),
        Language::Shader => category_09_contracts_hardware::parse_shader(src),

        Language::Markdown => category_10_documents::parse_markdown(src),
        // Both, because each has half the answer. The line-based scanner
        // finds headings (`= Introduction`), which a code scanner cannot see;
        // the rule file reads `#let charge(…) = …` as the Meson shape, which
        // the line scanner took as the name `charge(owner, amount)` with no
        // call recorded at all — 750 definitions per 1,000 lines and zero
        // edges. Merging keeps the document structure and gains the calls.
        Language::Typst => {
            let mut facts = rules::active().parse(Language::Typst, src);
            facts.merge_definitions_from(category_10_documents::parse_typst(src));
            facts
        }
        Language::Latex => category_10_documents::parse_latex(src),
    }
}
