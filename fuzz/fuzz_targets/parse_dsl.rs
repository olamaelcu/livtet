#![no_main]
//! Fuzz the saved-search DSL parser.
//!
//! This is the highest-risk parser: it feeds tantivy's recursive-descent
//! query grammar, which overflows the stack on deeply nested groups. The
//! `parse_user_dsl` length/depth guards exist to make this target safe —
//! it must never panic, abort, or hang on arbitrary input. The known
//! deep-nesting crasher lives as a regression test in
//! `livtet-types/src/search/saved_search.rs`; seed the corpus from it.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Only the return value is discarded; a panic/overflow is the bug.
        let _ = livtet_types::search::saved_search::parse_user_dsl(s);
    }
});
