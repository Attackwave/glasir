//! Every scanner must survive malformed input: no panic, no endless loop.
//!
//! Eleven crashes in this crate's history were one class — a slice at a byte
//! offset inside a multi-byte character — and each was found on real code
//! rather than by a test. This feeds every language fixture through the real
//! parse path cut short at random character boundaries and with the sequences
//! scanners trip on inserted: unclosed strings and comments, escapes, and
//! characters of two, three and four bytes. Deterministic, so a failure names
//! a seed that reproduces it.

use native_parsers::{Language, EXTENSIONS};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const MUTATIONS: u64 = 200;
const RISKY: &[&str] = &[
    "\"", "'", "`", "\\", "/*", "*/", "//", "#", "(", ")", "{", "}", "[", "]", "<", ">", "\"\"\"",
    "r\"", "r#\"", "--[[", "{-", "(*", "<!--", "$", "@", ":", "—", "é", "€", "😀", "\n", "\t",
];

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A random character boundary of `s`, so the result stays a `&str`.
fn boundary(s: &str, state: &mut u64) -> usize {
    let bounds: Vec<usize> = s.char_indices().map(|(i, _)| i).chain([s.len()]).collect();
    bounds[(splitmix(state) % bounds.len() as u64) as usize]
}

fn mutate(src: &str, seed: u64) -> String {
    let mut state = seed;
    let cut = boundary(src, &mut state).max(1.min(src.len()));
    let mut out = src[..cut].to_string();
    for _ in 0..(splitmix(&mut state) % 4) {
        let at = boundary(&out, &mut state);
        let piece = RISKY[(splitmix(&mut state) % RISKY.len() as u64) as usize];
        out.insert_str(at, piece);
    }
    out
}

fn fixtures() -> Vec<(PathBuf, Language)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/langs");
    let mut out: Vec<(PathBuf, Language)> = std::fs::read_dir(&dir)
        .expect("bench/langs holds one fixture per language")
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            let ext = p.extension()?.to_str()?.to_string();
            EXTENSIONS.contains(&ext.as_str()).then_some(())?;
            Some((p, Language::from_extension(&ext)?))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn mutated_fixtures_neither_panic_nor_hang() {
    let fixtures = fixtures();
    assert!(fixtures.len() > 100, "found {} fixtures", fixtures.len());
    let mut failures = Vec::new();
    for (path, lang) in &fixtures {
        let src = std::fs::read_to_string(path).unwrap();
        for m in 0..MUTATIONS {
            let seed = m ^ (path.to_string_lossy().len() as u64) << 32;
            let mutated = mutate(&src, seed);
            let (tx, rx) = mpsc::channel();
            let lang = *lang;
            let input = mutated.clone();
            std::thread::spawn(move || {
                let ok = std::panic::catch_unwind(|| {
                    native_parsers::rules::active().parse(lang, &input);
                    native_parsers::rules::active().identifiers(lang, &input);
                })
                .is_ok();
                let _ = tx.send(ok);
            });
            let name = path.file_name().unwrap().to_string_lossy();
            match rx.recv_timeout(Duration::from_secs(10)) {
                Ok(true) => {}
                Ok(false) => failures.push(format!("{name} seed {seed}: panicked on {mutated:?}")),
                Err(_) => failures.push(format!(
                    "{name} seed {seed}: no answer in 10 s on {mutated:?}"
                )),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Inputs that crashed a scanner once, kept so the fix stays checked without
/// depending on a seed finding them again.
#[test]
fn inputs_that_crashed_once() {
    for (ext, src) in [
        // A declaration closed before any rule opened: the brace depth
        // underflowed.
        ("css", ".a {\n  b@c: d;\n}\n\n.e {\n  border): 1px;"),
        // An unterminated triple-quoted string ending inside a character.
        ("py", "\"\"\"Ke(e\u{e9}p"),
        // A prefixed string ending inside a replacement character.
        ("gd", "B\'\'\'\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{fffd}\'"),
    ] {
        let lang = Language::from_extension(ext).unwrap();
        native_parsers::rules::active().parse(lang, src);
        native_parsers::rules::active().identifiers(lang, src);
    }
}
