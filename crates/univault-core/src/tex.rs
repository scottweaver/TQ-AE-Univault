//! Header-only reader for the game's `.tex` textures — enough to know
//! a bitmap's pixel dimensions, which is how item grid footprints are
//! defined (pixels ÷ 32-pixel cells). Ported from `TQVaultAE`'s
//! `BitmapService.LoadFromTexMemory` (MIT).
//!
//! A `.tex` is a 12-byte wrapper (magic `TEX\x01`, or `TEX\x02` with
//! one extra pad byte — the Atlantis-era variant) around an embedded
//! DDS image (`"DDS "` or `"DDSR"` magic, then the standard
//! `DDS_HEADER` with height at +12 and width at +16).

use crate::reader::ByteReader;

/// Pixels per inventory grid cell (`TQVaultAE`'s `ITEMUNITSIZE`).
pub const CELL_PIXELS: i32 = 32;

/// Errors from reading a texture header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TexError {
    #[error("not a TEX file (bad magic or too short)")]
    NotTex,
    #[error("embedded image is not DDS")]
    NotDds,
    #[error("implausible dimensions {width}x{height}")]
    BadDimensions { width: i32, height: i32 },
    #[error("unsupported pixel format ({bit_count}-bit or compressed)")]
    UnsupportedFormat { bit_count: i32 },
}

/// Pixel dimensions (width, height) of a `.tex` image.
///
/// # Errors
/// [`TexError`] when the TEX or embedded DDS structure is invalid.
pub fn dimensions(data: &[u8]) -> Result<(i32, i32), TexError> {
    locate_dds(data).map(|dds| (dds.width, dds.height))
}

struct DdsView {
    start: usize,
    width: i32,
    height: i32,
}

fn locate_dds(data: &[u8]) -> Result<DdsView, TexError> {
    let mut reader = ByteReader::new(data);
    let magic = reader.read_i32().map_err(|_| TexError::NotTex)?;
    let pad = match magic {
        0x0158_4554 => 0,
        0x0258_4554 => 1,
        _ => return Err(TexError::NotTex),
    };
    let texture_offset = reader.read_i32().map_err(|_| TexError::NotTex)?;
    let texture_offset = usize::try_from(texture_offset).map_err(|_| TexError::NotTex)?;

    let start = texture_offset + 12 + pad;
    let mut dds = ByteReader::at(data, start);
    let dds_magic = dds.read_i32().map_err(|_| TexError::NotDds)?;
    // "DDS " or "DDSR".
    if dds_magic != 0x2053_4444 && dds_magic != 0x5253_4444 {
        return Err(TexError::NotDds);
    }
    let mut header = ByteReader::at(data, start + 12);
    let height = header.read_i32().map_err(|_| TexError::NotDds)?;
    let width = header.read_i32().map_err(|_| TexError::NotDds)?;
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(TexError::BadDimensions { width, height });
    }
    Ok(DdsView {
        start,
        width,
        height,
    })
}

/// A decoded image: straight (unmultiplied) RGBA8, row-major,
/// top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Decodes a `.tex` into RGBA pixels. Covers the game's uncompressed
/// 32-bit (BGRA — `TQVaultAE` force-treats these masks as A8R8G8B8)
/// and 24-bit (BGR) formats, which a census of a real install put at
/// 99.9% of item bitmaps; the few DXT-compressed ones report
/// [`TexError::UnsupportedFormat`] and callers fall back to a plain
/// rectangle.
///
/// # Errors
/// [`TexError`] on structural problems or a compressed pixel format.
pub fn decode(data: &[u8]) -> Result<RgbaImage, TexError> {
    let dds = locate_dds(data)?;
    let mut format = ByteReader::at(data, dds.start + 80);
    let pf_flags = format.read_i32().map_err(|_| TexError::NotDds)?;
    let mut bits = ByteReader::at(data, dds.start + 88);
    let bit_count = bits.read_i32().map_err(|_| TexError::NotDds)?;
    // 0x4 = DDPF_FOURCC: a compressed format.
    if pf_flags & 0x4 != 0 {
        return Err(TexError::UnsupportedFormat { bit_count });
    }
    let bytes_per_pixel = match bit_count {
        32 => 4,
        24 => 3,
        _ => return Err(TexError::UnsupportedFormat { bit_count }),
    };

    let width = usize::try_from(dds.width).map_err(|_| TexError::NotDds)?;
    let height = usize::try_from(dds.height).map_err(|_| TexError::NotDds)?;
    let payload_start = dds.start + 128;
    let payload_length = width * height * bytes_per_pixel;
    let payload = data
        .get(payload_start..payload_start + payload_length)
        .ok_or(TexError::NotDds)?;

    let mut pixels = Vec::with_capacity(width * height * 4);
    for source in payload.chunks_exact(bytes_per_pixel) {
        pixels.push(source[2]);
        pixels.push(source[1]);
        pixels.push(source[0]);
        pixels.push(if bytes_per_pixel == 4 {
            source[3]
        } else {
            0xFF
        });
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Converts pixel dimensions to grid cells, rounding like
/// `TQVaultAE` (`Convert.ToInt32(px / 32.0)`) and never below 1×1.
#[must_use]
pub fn cells(width_px: i32, height_px: i32) -> (i32, i32) {
    let cell = |pixels: i32| ((pixels + CELL_PIXELS / 2) / CELL_PIXELS).max(1);
    (cell(width_px), cell(height_px))
}

#[cfg(test)]
pub(crate) mod fixture {
    /// Minimal valid TEX bytes for a `width`×`height` image (header
    /// only — enough for [`super::dimensions`]).
    pub(crate) fn tex(width: i32, height: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0158_4554_i32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&128_i32.to_le_bytes());
        bytes.extend_from_slice(b"DDS ");
        bytes.extend_from_slice(&124_i32.to_le_bytes());
        bytes.extend_from_slice(&0_i32.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes
    }

    /// A complete TEX with a full 128-byte DDS header and pixel
    /// payload, decodable by [`super::decode`].
    pub(crate) fn tex_with_pixels(
        width: i32,
        height: i32,
        bit_count: i32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut bytes = tex(width, height);
        // Pad the DDS header out to its full 128 bytes (wrapper is 12).
        bytes.resize(12 + 128, 0);
        // DDPF_RGB at pixel-format flags; bit count at +88.
        bytes[12 + 80] = 0x40;
        bytes[12 + 88..12 + 92].copy_from_slice(&bit_count.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dimensions_from_the_dds_header() {
        assert_eq!(dimensions(&fixture::tex(64, 160)), Ok((64, 160)));
    }

    #[test]
    fn atlantis_variant_has_one_pad_byte() {
        let mut bytes = fixture::tex(32, 32);
        bytes[..4].copy_from_slice(&0x0258_4554_i32.to_le_bytes());
        bytes.insert(8, 0);
        assert_eq!(dimensions(&bytes), Ok((32, 32)));
    }

    #[test]
    fn rejects_non_tex_data() {
        assert_eq!(dimensions(b"not a texture"), Err(TexError::NotTex));
    }

    #[test]
    fn rejects_missing_dds() {
        let mut bytes = fixture::tex(32, 32);
        bytes[12..16].copy_from_slice(b"NOPE");
        assert_eq!(dimensions(&bytes), Err(TexError::NotDds));
    }

    #[test]
    fn decodes_32_bit_bgra_pixels() {
        // One blue pixel, one semi-transparent red pixel (stored BGRA).
        let payload = [0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x80];
        let image = decode(&fixture::tex_with_pixels(2, 1, 32, &payload)).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(
            image.pixels,
            vec![0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x80]
        );
    }

    #[test]
    fn decodes_24_bit_bgr_as_opaque() {
        let payload = [0x10, 0x20, 0x30];
        let image = decode(&fixture::tex_with_pixels(1, 1, 24, &payload)).unwrap();
        assert_eq!(image.pixels, vec![0x30, 0x20, 0x10, 0xFF]);
    }

    #[test]
    fn compressed_formats_are_reported_unsupported() {
        let mut bytes = fixture::tex_with_pixels(1, 1, 32, &[0; 4]);
        bytes[12 + 80] = 0x04;
        assert!(matches!(
            decode(&bytes),
            Err(TexError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn truncated_payload_is_rejected() {
        let bytes = fixture::tex_with_pixels(4, 4, 32, &[0; 8]);
        assert_eq!(decode(&bytes), Err(TexError::NotDds));
    }

    #[test]
    fn cells_round_and_clamp() {
        assert_eq!(cells(64, 160), (2, 5));
        assert_eq!(cells(32, 32), (1, 1));
        assert_eq!(cells(16, 8), (1, 1));
        assert_eq!(cells(48, 96), (2, 3));
    }
}
