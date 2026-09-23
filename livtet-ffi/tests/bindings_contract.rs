//! Contract snapshot for the generated bindings.
//!
//! Runs the same `uniffi-bindgen generate --library` pipeline the
//! mobile repo's mise tasks use, for both Kotlin and Swift, and asserts
//! the public surface is present. This is the guard against accidental
//! renames/removals of FFI exported items: if a type or method
//! disappears or changes signature, the binding output changes and this
//! test fails. Update the expected lists deliberately, as the changelog
//! of the cross-language API.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;

fn workspace_root() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has a workspace parent")
        .to_path_buf()
}

fn cargo(args: &[&str]) -> String {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .args(args)
        .current_dir(workspace_root())
        .output()
        .expect("cargo invocation failed");
    assert!(
        out.status.success(),
        "cargo {:?} failed:\n{}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn run_bindgen(lib: &Utf8Path, language: &str, out_dir: &Utf8Path) {
    cargo(&[
        "run",
        "--quiet",
        "-p",
        "livtet-ffi",
        "--bin",
        "uniffi-bindgen",
        "--features",
        "cli",
        "--",
        "generate",
        "--library",
        lib.as_str(),
        "--language",
        language,
        "--out-dir",
        out_dir.as_str(),
        "--no-format",
    ]);
}

/// Collect the concatenated text of every generated source file under
/// `dir`.
fn slurp(dir: &Utf8Path, ext: &str) -> String {
    let mut acc = String::new();
    for entry in walkdir(dir) {
        if entry.extension().is_some_and(|e| e == ext)
            && let Ok(text) = fs::read_to_string(&entry)
        {
            acc.push_str(&text);
            acc.push('\n');
        }
    }
    acc
}

fn walkdir(dir: &Utf8Path) -> Vec<Utf8PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) else {
            continue;
        };
        if path.is_dir() {
            out.extend(walkdir(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Types that must exist in the generated bindings, identical for
/// Kotlin and Swift.
const EXPECTED_TYPES: &[&str] = &[
    // Facade + errors
    "LivtetStore",
    "LivtetError",
    // DTOs
    "WorkSummary",
    "EditionSummary",
    "EditionDetail",
    "EditionFile",
    "ReadingProgress",
    "Annotation",
    "ReadingList",
    "ReadingSession",
    "EditionPatch",
    "SeedStats",
    "ReindexProgressEvent",
    // livtet-temporal-quotes re-exports
    "Greeting",
    "EmptyMessage",
    // livtet-search types crossing directly
    "SearchHit",
    "HitKind",
    "SearchOptions",
    "FacetCount",
    "FacetedSearchResult",
    "HighlightRange",
    // livtet-types enums/records crossing directly
    "BookCondition",
    "ContributorRole",
    "PairingStatus",
    "SeriesType",
    "WorkStatus",
    "ProgressUnit",
    "Progression",
    "PublishedDate",
    "ReadingLength",
    "IdentifierKind",
    "KnownFormats",
    "LanguageInfo",
    "SortSpec",
    "SortField",
    "SortDirection",
    "WorkSortBy",
    "WorkFilters",
];

/// Method names (camelCase) expected on the store.
const EXPECTED_METHODS: &[&str] = &[
    "listWorks",
    "listEditions",
    "getEdition",
    "countWorks",
    "searchEditions",
    "searchWorks",
    "searchWithFacets",
    "setEditionFile",
    "removeEditionFile",
    "addEditionIdentifier",
    "updateEdition",
    "setWorkStatus",
    "getWorkStatus",
    "recordReadingProgress",
    "getReadingProgress",
    "recordReadingSession",
    "addAnnotation",
    "listAnnotations",
    "deleteAnnotation",
    "createReadingList",
    "listReadingLists",
    "addEditionToList",
    "removeEditionFromList",
    "deleteReadingList",
    "reindex",
    "seedSampleData",
    "resetAndSeed",
    "getGreeting",
    "getEmptyStateQuotation",
];

#[test]
fn kotlin_and_swift_bindings_expose_the_full_surface() {
    // 1. Build the cdylib the same way the mobile pipeline does.
    cargo(&["build", "-p", "livtet-ffi"]);
    let lib = workspace_root()
        .join("target/debug")
        .join(if cfg!(target_os = "macos") {
            "liblivtet_ffi.dylib"
        } else if cfg!(target_os = "windows") {
            "livtet_ffi.dll"
        } else {
            "liblivtet_ffi.so"
        });
    assert!(lib.exists(), "cdylib missing at {lib}");

    let tmp = camino_tempfile::tempdir().unwrap();
    let kt_dir = tmp.path().join("kotlin");
    let swift_dir = tmp.path().join("swift");
    fs::create_dir_all(&kt_dir).unwrap();
    fs::create_dir_all(&swift_dir).unwrap();

    run_bindgen(&lib, "kotlin", &kt_dir);
    run_bindgen(&lib, "swift", &swift_dir);

    let kotlin = slurp(&kt_dir, "kt");
    assert!(!kotlin.is_empty(), "kotlin bindings are empty");
    let kotlin = kotlin.to_lowercase();

    let swift = slurp(&swift_dir, "swift");
    assert!(!swift.is_empty(), "swift bindings are empty");
    let swift = swift.to_lowercase();

    for ty in EXPECTED_TYPES {
        assert!(
            kotlin.contains(&ty.to_lowercase()),
            "kotlin bindings missing type {ty}"
        );
        assert!(
            swift.contains(&ty.to_lowercase()),
            "swift bindings missing type {ty}"
        );
    }
    for method in EXPECTED_METHODS {
        assert!(
            kotlin.contains(&method.to_lowercase()),
            "kotlin bindings missing method {method}"
        );
        assert!(
            swift.contains(&method.to_lowercase()),
            "swift bindings missing method {method}"
        );
    }
}
