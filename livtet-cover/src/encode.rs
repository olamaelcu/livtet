use camino::{Utf8Path, Utf8PathBuf};
use image::{GenericImageView, ImageReader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverMetadata {
    pub blurhash: String,
    pub dominant_color: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("cover file not found: {0}")]
    NotFound(Utf8PathBuf),
    #[error("image decode failed: {0}")]
    Decode(String),
    #[error("encoder failed: {0}")]
    Encode(String),
}

pub fn encode_cover(path: &Utf8Path) -> Result<CoverMetadata, EncodeError> {
    if !path.exists() {
        return Err(EncodeError::NotFound(path.to_path_buf()));
    }

    let img = ImageReader::open(path)
        .map_err(|e| EncodeError::Decode(e.to_string()))?
        .with_guessed_format()
        .map_err(|e| EncodeError::Decode(e.to_string()))?
        .decode()
        .map_err(|e| EncodeError::Decode(e.to_string()))?;

    let rgba = img.to_rgba8();
    let (w, h) = img.dimensions();
    let pixels: &[u8] = rgba.as_raw();

    let hash =
        blurhash::encode(4, 3, w, h, pixels).map_err(|e| EncodeError::Encode(e.to_string()))?;

    let dominant = dominant_color(&rgba);

    Ok(CoverMetadata {
        blurhash: hash,
        dominant_color: dominant,
    })
}

/// Average sRGB of the full-resolution image, rounded to the nearest
/// integer per channel. This is the color the blurhash DC component
/// approximates for a near-uniform image and is what the frontend uses
/// as the placeholder background.
fn dominant_color(rgba: &image::RgbaImage) -> String {
    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    let mut n: u64 = 0;
    for px in rgba.pixels() {
        r_sum += px[0] as u64;
        g_sum += px[1] as u64;
        b_sum += px[2] as u64;
        n += 1;
    }
    if n == 0 {
        return "#00000000".to_string();
    }
    let r = (r_sum / n) as u8;
    let g = (g_sum / n) as u8;
    let b = (b_sum / n) as u8;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}
