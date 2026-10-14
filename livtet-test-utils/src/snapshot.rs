//! Deterministic snapshot settings.
//!
//! Rendered output carries three things that differ between runs, and a
//! snapshot that includes any of them is a flaky test rather than a check:
//!
//! * **Terminal escape sequences.** `livtet-cli-common` hyperlinks on-disk
//!   paths with OSC 8 only when stdout is a plain TTY, so the same input
//!   produces different bytes under `cargo test` than under a shell with
//!   output captured.
//! * **ULIDs.** Every `DbId` is minted at run time.
//! * **Temp paths.** Fixtures live under a fresh temp directory each run.
//!
//! [`settings`] filters all three out, so a snapshot can be taken of real
//! output -- including a whole captured stdout -- without hand-scrubbing it
//! first. Prefer passing the decision in (as `render_detail_with` does for
//! the TTY question) when a function can be given a seam; use these filters
//! for output you can only observe after the fact, such as a spawned
//! binary's stdout.

use std::sync::LazyLock;

use regex::Regex;

/// An OSC 8 hyperlink wrapper: `ESC ] 8 ; ; <uri> ESC \ <text> ESC ] 8 ; ; ESC \`.
/// Replaced by the text it wrapped, so the path still shows.
static OSC8: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\]8;;[^\x1b]*\x1b\\(?<text>[^\x1b]*)\x1b\]8;;\x1b\\").expect("OSC 8 pattern")
});

/// A CSI sequence: colour, cursor movement, erase.
static CSI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("CSI pattern"));

/// A Crockford base-32 ULID: 26 chars, no I, L, O or U.
static ULID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-7][0-9A-HJKMNP-TV-Z]{25}\b").expect("ULID pattern"));

/// Strip terminal escape sequences, keeping the text an OSC 8 link wrapped.
///
/// For assertions that are not snapshots — `assert_eq!` against an expected
/// string, say — where the output may or may not be decorated.
pub fn strip_ansi(text: &str) -> String {
    let unlinked = OSC8.replace_all(text, "$text");
    CSI.replace_all(&unlinked, "").into_owned()
}

/// Replace every ULID with `<ulid>`.
pub fn mask_ulids(text: &str) -> String {
    ULID.replace_all(text, "<ulid>").into_owned()
}

/// `insta` settings that filter out everything which differs run to run.
///
/// ```ignore
/// livtet_test_utils::snapshot::settings()
///     .bind(|| insta::assert_snapshot!(rendered));
/// ```
///
/// Filters are applied to the snapshot text before comparison, so an
/// existing snapshot keeps matching whether or not the run happened to be
/// attached to a terminal.
pub fn settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    // Order matters: unwrap OSC 8 links before stripping CSI, so the link
    // text survives.
    settings.add_filter(OSC8.as_str(), "$text");
    settings.add_filter(CSI.as_str(), "");
    settings.add_filter(ULID.as_str(), "<ulid>");
    // Temp directories: both the fixture root and anything beneath it.
    settings.add_filter(r"/tmp/[A-Za-z0-9._\-/]+", "<tmp>");
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINKED: &str =
        "  On-disk: * \x1b]8;;file:///books/a.epub\x1b\\/books/a.epub\x1b]8;;\x1b\\";

    #[test]
    fn stripping_a_hyperlink_keeps_the_path_it_wrapped() {
        assert_eq!(strip_ansi(LINKED), "  On-disk: * /books/a.epub");
    }

    #[test]
    fn stripping_removes_colour_without_touching_the_text() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m text"), "red text");
        assert_eq!(
            strip_ansi("plain"),
            "plain",
            "undecorated text is untouched"
        );
    }

    /// The whole point: decorated and bare renderings of the same content
    /// must normalise to the same string, because which one a run produces
    /// depends only on how its stdout is attached.
    #[test]
    fn decorated_and_bare_output_normalise_together() {
        assert_eq!(strip_ansi(LINKED), strip_ansi("  On-disk: * /books/a.epub"));
    }

    #[test]
    fn ulids_are_masked_and_other_identifiers_are_not() {
        assert_eq!(
            mask_ulids("id=01ARZ3NDEKTSV4RRFFQ69G5FAV done"),
            "id=<ulid> done"
        );
        // Too short, and hex of the same length, must survive.
        assert_eq!(mask_ulids("01ARZ3NDEK"), "01ARZ3NDEK");
        assert_eq!(
            mask_ulids("deadbeefdeadbeefdeadbeefdeadbeef"),
            "deadbeefdeadbeefdeadbeefdeadbeef",
            "a 32-char hex id is not a ULID"
        );
    }

    #[test]
    fn settings_normalise_a_snapshot_regardless_of_decoration() {
        // Binding the settings twice with the two renderings must agree.
        settings().bind(|| {
            insta::assert_snapshot!("on_disk_line", LINKED);
        });
        settings().bind(|| {
            insta::assert_snapshot!("on_disk_line", "  On-disk: * /books/a.epub");
        });
    }
}
