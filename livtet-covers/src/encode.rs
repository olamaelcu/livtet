use camino::Utf8Path;
use fs_err as fs;
use image::GenericImageView;
use thiserror::Error;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverMetadata {
    pub blurhash: String,
    pub dominant_color: String,
}

#[derive(Error, Debug)]
pub enum EncodeError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Decode error: {0}")]
    Decode(String),
    #[error("Encode error: {0}")]
    Encode(String),
}

pub type EncodeResult<T> = Result<T, EncodeError>;

pub fn encode_cover(path: &Utf8Path) -> EncodeResult<CoverMetadata> {
    let bytes = fs::read(path).map_err(|e| EncodeError::NotFound(e.to_string()))?;

    let img = image::load_from_memory(&bytes).map_err(|e| EncodeError::Decode(e.to_string()))?;

    let (width, height) = img.dimensions();

    let rgba = img.to_rgba8();
    let blurhash = blurhash::encode(width, height, 3, 2, rgba.as_raw())
        .map_err(|e| EncodeError::Encode(e.to_string()))?;

    let dominant_color = average_rgb(&rgba);

    Ok(CoverMetadata {
        blurhash,
        dominant_color,
    })
}

fn average_rgb(rgba: &[u8]) -> String {
    let mut r: u64 = 0;
    let mut g: u64 = 0;
    let mut b: u64 = 0;
    let mut n: u64 = 0;

    for px in rgba.chunks_exact(4) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        n += 1;
    }

    if n == 0 {
        return "#000000".to_string();
    }

    format!(
        "#{:02x}{:02x}{:02x}",
        (r / n) as u8,
        (g / n) as u8,
        (b / n) as u8
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_generated_png() {
        let dir = camino_tempfile::Utf8TempDir::new().unwrap();
        let path = dir.path().join("cover.png");
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30]));
        image::DynamicImage::ImageRgb8(img).save(&path).unwrap();

        let meta = encode_cover(&path).unwrap();
        assert!(!meta.blurhash.is_empty());
        assert!(meta.dominant_color.starts_with('#'));
        assert_eq!(meta.dominant_color.len(), 7);
    }

    #[test]
    fn encode_missing_path_errors() {
        let err = encode_cover(camino::Utf8Path::new("/nonexistent/cover.png")).unwrap_err();
        assert!(matches!(err, EncodeError::NotFound(_)));
    }
}
