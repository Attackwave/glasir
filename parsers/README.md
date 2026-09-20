# Maintaining language extraction rules

Glasir extracts definitions, documentation, calls and byte ranges. It does not
build a complete compiler AST. TOML rules configure the shared extraction
mechanisms; syntax that requires language-specific state belongs in the Rust
module for that language.

## Current coverage

| Language | Extraction implementation | What TOML controls |
|---|---|---|
| Some languages | Shared generic scanner | Bundled definition keywords, scopes, comments, call exclusions and lexical options |
| Some languages | Native parser module plus bundled rules | Syntax that requires state remains Rust; bundled rules supply lexical options |
| Remaining languages | Native category modules | Not configurable through the public override interface |

A language moves to a shared rule-driven path only when its fixtures preserve
the expected definitions, calls, ownership, documentation, and source ranges.
Languages whose syntax needs additional state keep native scanners.
What separates them is not language family but how a definition is written —
C's `int charge(...)`, Bash's `name() {` and R's `name <- function(` define by
shape, and no list of keywords expresses that. Lua came closest and still
failed: it writes a constant as `local MAX = 3` at file scope only, and a depth
condition is not a word list.

Requiring the two to agree found three defects in the shared scanner that
neither the fixtures nor the retrieval floors could see, each of them affecting
every `[generic]` language rather than the one being migrated:

- A statement boundary did not clear the receiver flag, so `a.b();` left it set
  and the next statement's first call was recorded as receiver-qualified.
- A modifier was skipped without its byte offset, so `abstract contract C` had
  a range beginning at `contract` — and that range is what `get_code_snippet`
  returns, so the snippet was source that does not compile on its own.
- A rule file narrowing `doc_comments` to a language's real marker is *better*
  than the scanner it replaces rather than merely equal: Solidity's `//` licence
  header was being attributed to the first symbol as documentation. Identity is
  therefore the admission test on definitions, calls and ranges, with a
  narrowing of documentation accepted.

The modules share one crate and one `FileFacts` contract. Adding another crate
is not required to isolate syntax. `Language` in `src/lib.rs` remains the
extension registry; TOML does not install new language implementations or
change extension dispatch in this version.

The scanners' output remains `Inferred` in Glasir. Compiler-backed SCIP/LSP
information remains the higher-confidence tier.

## Use and override rules

Bundled files under `languages/` are compiled into the binary with `include_str!`
and validated once per process. They need no files alongside an installation.
Editing a bundled file requires rebuilding the binary.

For a project-specific rule set, copy the files to change into a directory and
pass that directory explicitly:

```sh
mkdir -p /tmp/my-glasir-rules
cp parsers/languages/go.toml /tmp/my-glasir-rules/go.toml
cargo run --release -- analyse /path/to/project --language-rules /tmp/my-glasir-rules
cargo run --release -- serve /path/to/project --watch --language-rules /tmp/my-glasir-rules
```

An override replaces the **whole file**, not selected keys. Missing language
files use the bundled version. The public override interface currently accepts
`go.toml`, `rust.toml`, and `ruby.toml`; an unknown TOML file is an error.
Non-TOML files are ignored. No directory is discovered implicitly and no
executable extension is loaded.
Use the flag on every analysis or serving invocation that should use overrides;
`install` does not persist it in editor registrations or generated git hooks.

Rules are immutable after startup, including under `--watch`. Restart after a
rule edit. A snapshot stores the exact selected rule contents with a parser
implementation version. Both must match before either a warm or incremental
load can reuse it. Stored map layouts also reject a different rule identity.
Changing rules without incrementing their revision still
invalidates the old snapshot. Concurrent processes using different rules may
replace the same cache, but neither accepts the other's cache as its own.

## Schema version 1

Every table rejects unknown fields. Required top-level fields:

```toml
schema_version = 1
language = "go"
revision = 1
```

`language` must match the filename. `revision` is a positive integer for human
release tracking; increment it when distributing changed rules. Change
`schema_version` only when introducing an incompatible schema, with reader
support in the same change.

All files require a `[lexical]` table:

```toml
[lexical]
line_comments = ["//"]
doc_comments = ["//"]
block_comment = ["/*", "*/"]
identifier_suffix_marks = false
identifier_dashes = false
```

The comment lists are required; an empty list disables that form. Omit
`block_comment` to disable block comments. The two identifier flags default to
false. Suffix marks keep Ruby's `empty?` and `save!` intact. Dashes allow names
such as Lisp's `commit-entry`; enabling them for Go would reinterpret
subtraction, so this is a syntax choice, not cosmetic formatting. String forms
are handled by the lexer and native syntax modules, not arbitrary TOML regexes.

All files require explicit call exclusions:

```toml
[calls]
exclude = ["if", "for", "switch", "select", "return", "go", "defer", "range"]
```

The list is language-local. There is no implicit union of other languages'
keywords: Go's `new(T)` is a call even though Java uses `new` differently.
Removing or adding an exclusion changes which recognized call sites are
emitted, including in the Rust and Ruby modules.

Go additionally requires `[generic]`:

```toml
[generic]
braces = true
receivers = ["."]
definitions = [
    { keyword = "func", scope = "body_after_receiver" },
    { keyword = "type", scope = "body" },
    { keyword = "package", scope = "bare" },
    { keyword = "const", scope = "bare" },
    { keyword = "var", scope = "bare" },
]
```

| Scope | Behavior |
|---|---|
| `body` | Definition with a body; does not collect its inner comments as documentation |
| `body_with_docs` | Definition with a body; collects body comments |
| `body_after_receiver` | Function body with documentation; accepts a parenthesized receiver before the name |
| `statement` | Definition owns calls until a semicolon |
| `bare` | Definition without its own enclosing call scope |

Optional generic fields are `modifiers`, `block_open`, `block_end` (empty lists
by default), `bare_receiver_calls` and `constant_by_case` (false by default).
Accepted receiver tokens are `.`, `::`, `->`, `&.` and `?->`. Modifiers are
skipped; block lists describe word-delimited nesting. They are mechanisms for
simple syntax, not a substitute for a language module when context determines
whether a word opens a block. Rust and Ruby reject a `[generic]` table instead
of silently ignoring it.

For example, changing the `func` rule to `keyword = "proc"` makes the scanner
recognize `proc Run() { notify() }` without changing Rust. This is a useful
dialect test, not a recommendation to alter standard Go's rules.

Files are capped at 64 KiB before parsing. Lists allow at most 128 entries;
tokens must be nonempty, without whitespace or control characters, and at most
128 bytes. Duplicate
entries, conflicting generic roles, unknown scopes and unsupported versions
are errors. These are structural checks; valid rules still need extraction
tests to establish their meaning.

## Validation and known limits

```sh
cargo test --manifest-path parsers/Cargo.toml --target-dir target/parser-tests
cargo test --test language_rules_test
cargo run --release -- langcheck bench/langs --check
cargo run --release -- benchmark . --check
```

The parser suite includes all existing language fixtures and rule-specific
checks for changed behavior, invalid input, Unicode, receiver methods,
anonymous Go function bodies, Rust extern declarations and Ruby block scopes.
The CLI tests cover explicit selection, failure before indexing and cache
invalidation when only rule contents change. CI runs both suites.

Validate parser changes against checked-in fixtures and representative local
repositories. Compare extracted definitions, calls, ownership, documentation
and source ranges; investigate every unexpected difference before accepting a
migration. Parser counts alone are not a recall or accuracy guarantee.

Language coverage remains intentionally explicit. Go grouped declarations,
precise spans for bare declarations and implicit semicolon handling need
dedicated tests. Ruby scope handling also needs language-specific validation.

The Java baseline fix stops class docs accumulating field and method comments;
methods continue to collect their own body comments. Both contracts are tested.

For each further migration, write expected facts from the source, compare a
real corpus, inspect differences in ownership/docs/ranges as well as counts,
and run Glasir's retrieval and extraction floors. Keep special syntax in
`src/languages/<language>.rs`. Bump `IMPLEMENTATION_VERSION` in `src/rules.rs`
when scanner behavior changes, and `snapshot::FORMAT_VERSION` when Glasir's
graph interpretation or snapshot format changes. Update bundled revisions and
document the supported subset in the same change.

## Dependencies

The scanner crate uses `serde` and `toml` for strict rule deserialization.
Only TOML parsing, serde support and the standard library feature are enabled;
there is no grammar generator, JavaScript engine, C parser build or dynamic
plugin runtime. These dependencies replace writing and maintaining a partial
TOML implementation. Their lockfile entries are included in the repository's
normal dependency inventory.
