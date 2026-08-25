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
}

/// Pixel dimensions (width, height) of a `.tex` image.
///
/// # Errors
/// [`TexError`] when the TEX or embedded DDS structure is invalid.
pub fn dimensions(data: &[u8]) -> Result<(i32, i32), TexError> {
    let mut reader = ByteReader::new(data);
    let magic = reader.read_i32().map_err(|_| TexError::NotTex)?;
    let pad = match magic {
        0x0158_4554 => 0,
        0x0258_4554 => 1,
        _ => return Err(TexError::NotTex),
    };
    let texture_offset = reader.read_i32().map_err(|_| TexError::NotTex)?;
    let texture_offset = usize::try_from(texture_offset).map_err(|_| TexError::NotTex)?;

    let dds_start = texture_offset + 12 + pad;
    let mut dds = ByteReader::at(data, dds_start);
    let dds_magic = dds.read_i32().map_err(|_| TexError::NotDds)?;
    // "DDS " or "DDSR".
    if dds_magic != 0x2053_4444 && dds_magic != 0x5253_4444 {
        return Err(TexError::NotDds);
    }
    let mut header = ByteReader::at(data, dds_start + 12);
    let height = header.read_i32().map_err(|_| TexError::NotDds)?;
    let width = header.read_i32().map_err(|_| TexError::NotDds)?;
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return Err(TexError::BadDimensions { width, height });
    }
    Ok((width, height))
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
    /// Minimal valid TEX bytes for a `width`×`height` image.
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
    fn cells_round_and_clamp() {
        assert_eq!(cells(64, 160), (2, 5));
        assert_eq!(cells(32, 32), (1, 1));
        assert_eq!(cells(16, 8), (1, 1));
        assert_eq!(cells(48, 96), (2, 3));
    }
}
