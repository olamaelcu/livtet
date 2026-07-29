use camino::{Utf8Path as Path, Utf8PathBuf as PathBuf};
use livtet_covers::{EncodeError, encode_cover};

fn fixture() -> PathBuf {
    PathBuf::new()
        .join(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cover_32.png")
}

#[test]
fn encodes_golden_fixture() {
    let m = encode_cover(&fixture()).expect("encode fixture");
    assert_eq!(m.dominant_color.len(), 7);
    assert!(m.dominant_color.starts_with('#'));

    let r = u8::from_str_radix(&m.dominant_color[1..3], 16).unwrap();
    let g = u8::from_str_radix(&m.dominant_color[3..5], 16).unwrap();
    let b = u8::from_str_radix(&m.dominant_color[5..7], 16).unwrap();

    // Average sRGB of the gradient #3366cc -> #f2c14e lives near
    // (~147, ~148, ~141). Allow a 50-unit tolerance per channel for
    // the gradient and the RGB->sRGB encoding step.
    assert!((100..=200).contains(&r), "r = {r}");
    assert!((100..=200).contains(&g), "g = {g}");
    assert!((100..=200).contains(&b), "b = {b}");

    assert!(!m.blurhash.is_empty());
    assert!(m.blurhash.len() < 64);
}

#[test]
fn errors_on_missing_file() {
    let err = encode_cover(&Path::new("/nonexistent/cover.png")).expect_err("missing file errors");
    assert!(matches!(err, EncodeError::NotFound(_)));
}
