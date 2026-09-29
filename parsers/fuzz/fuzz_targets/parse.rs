//! Any bytes, as any language: the first byte picks the language, the rest is
//! the source. What `tests/robustness_test.rs` does with a few thousand seeded
//! mutations, for as long as it is left running:
//! `cargo +nightly fuzz run parse -- -max_total_time=600`.
#![no_main]
use libfuzzer_sys::fuzz_target;
use native_parsers::{Language, EXTENSIONS};

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else {
        return;
    };
    let ext = EXTENSIONS[pick as usize % EXTENSIONS.len()];
    let Some(lang) = Language::from_extension(ext) else {
        return;
    };
    let src = String::from_utf8_lossy(rest);
    native_parsers::rules::active().parse(lang, &src);
    native_parsers::rules::active().identifiers(lang, &src);
});
