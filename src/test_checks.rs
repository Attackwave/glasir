//! Every assert-based check, individually runnable.
//!
//! The checks themselves stay in `main.rs`: they are documentation of the
//! pitfalls each phase was built around, they are deliberately broken and
//! re-run when written, and `glasir selfcheck` still runs them all in order in
//! one process. What was missing is the other half — running *one* of them,
//! which is what you want when a change breaks exactly one thing.
//!
//! Thin wrappers rather than moved bodies: a body moved here would drift from
//! the one `selfcheck` runs, and then two things claim to test the same
//! property while only one of them does.
//!
//! **In its own file, and the name is load-bearing.** As a `mod tests` inside
//! `main.rs` these wrappers became graph symbols named `community`, `search`,
//! `docs` — competing with the modules they test for exactly those words.
//! Measured: 4 points off both code question sets. `resolve::is_test_path`
//! already refuses definitions from a file whose name says "test"; putting them
//! here is what lets that existing rule do its job.

#[test]
fn fixture_id_is_safe_for_windows_paths() {
    let id = crate::fixture_id();
    assert!(
        !id.chars()
            .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')),
        "fixture identifier contains a Windows-reserved path character: {id}"
    );
}

#[test]
fn audit() {
    crate::demo_audit();
}

#[test]
fn auth() {
    crate::demo_auth();
}

#[test]
fn edges() {
    crate::demo_edges();
}

#[test]
fn baseline() {
    crate::demo_baseline();
}

#[test]
fn extension_collision() {
    crate::demo_extension_collision();
}

#[test]
fn langcheck() {
    crate::demo_langcheck();
}

#[test]
fn batch_build() {
    crate::demo_batch_build();
}

#[test]
fn bench() {
    crate::demo_bench();
}

#[test]
fn subsystem_scale() {
    crate::demo_subsystem_scale();
}

#[test]
fn cohesion_scale() {
    crate::demo_cohesion_scale();
}

#[test]
fn community() {
    crate::demo_community();
}

#[test]
fn concurrent_compaction() {
    crate::demo_concurrent_compaction();
}

#[test]
fn cycles() {
    crate::demo_cycles();
}

#[test]
fn overview() {
    crate::demo_overview();
}

#[test]
fn impact_of() {
    crate::demo_impact_of();
}

#[test]
fn guard() {
    crate::demo_guard();
}

#[test]
fn doc_coverage() {
    crate::demo_doc_coverage();
}

#[test]
fn doc_ranking() {
    crate::demo_doc_ranking();
}

#[test]
fn docs() {
    crate::demo_docs();
}

#[test]
fn embed() {
    crate::demo_embed();
}

#[test]
fn hooks() {
    crate::demo_hooks();
}

#[test]
fn http() {
    crate::demo_http();
}

#[test]
fn http_concurrent() {
    crate::demo_http_concurrent();
}

#[test]
fn http_events() {
    crate::demo_http_events();
}

#[test]
fn snippet() {
    crate::demo_snippet();
}

#[test]
fn deterministic() {
    crate::demo_deterministic();
}

#[test]
fn tls() {
    crate::demo_tls();
}

#[test]
fn ignored_paths() {
    crate::demo_ignored_paths();
}

#[test]
fn import() {
    crate::demo_import();
}

#[test]
fn incremental() {
    crate::demo_incremental();
}

#[test]
fn index_scale() {
    crate::demo_index_scale();
}

#[test]
fn ingest() {
    crate::demo_ingest();
}

#[test]
fn install() {
    crate::demo_install();
}

#[test]
fn markdown() {
    crate::demo_markdown();
}

#[test]
fn walk_order() {
    crate::demo_walk_order();
}

#[test]
fn mcp() {
    crate::demo_mcp();
}

#[test]
fn parallel_ingest() {
    crate::demo_parallel_ingest();
}

#[test]
fn native_rust() {
    crate::demo_native_rust();
}

#[test]
fn parse() {
    crate::demo_parse();
}

#[test]
fn physics() {
    crate::demo_physics();
}

#[test]
fn resolve() {
    crate::demo_resolve();
}

#[test]
fn scip() {
    crate::demo_scip();
}

#[test]
fn search() {
    crate::demo_search();
}

#[test]
fn snapshot() {
    crate::demo_snapshot();
}

#[test]
fn view() {
    crate::demo_view();
}

/// `demo_delta` takes its fixture from the caller, so it is exercised
/// through the full sweep rather than on its own.
#[test]
fn full_sweep() {
    crate::demo().unwrap();
}

/// Every parameterless check has a wrapper above.
///
/// The likely mistake here is not a broken test but a missing one: someone
/// adds `demo_something`, wires it into `demo()`, and it never becomes
/// individually runnable. Reading this file for `fn demo_*()` and comparing
/// against the wrappers catches that without anyone having to remember.
#[test]
fn every_check_is_wrapped() {
    let src = include_str!("main.rs");
    let wrappers = include_str!("test_checks.rs");
    let demos: Vec<&str> = src
        .lines()
        .filter_map(|l| l.strip_prefix("fn demo_"))
        .filter_map(|l| l.split_once("() {"))
        .map(|(name, _)| name)
        .collect();
    assert!(
        demos.len() > 25,
        "found {} checks, expected the full set",
        demos.len()
    );
    let missing: Vec<&&str> = demos
        .iter()
        .filter(|name| !wrappers.contains(&format!("crate::demo_{name}();")))
        .collect();
    assert!(
        missing.is_empty(),
        "checks with no #[test] wrapper, so they cannot be run on their own: {missing:?}"
    );
}
