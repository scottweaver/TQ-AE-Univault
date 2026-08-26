//! Stash files — the shared transfer stash (`SaveData/Sys/winsys.dxb`)
//! and per-character stashes (`SaveData/Main/_Name/winsys.dxb`), with
//! their `.dxg` backup twins. Read-only here; writing is the
//! targeted-splice milestone. Ported from `TQVaultAE`'s
//! `StashProvider.cs` (MIT).
//!
//! Layout: a leading `i32` CRC32 (of the file with this field zeroed —
//! skipped on read, like `TQVaultAE`), then `begin_block`,
//! `stashVersion`, `fName` (the file's own name), `sackWidth`,
//! `sackHeight`, and one stash-type sack: `numItems`, then items each
//! carrying an explicit `stackCount` (stored as stack size − 1) and
//! float `xOffset`/`yOffset` grid positions, closed by `end_block`.

use crate::chr::{
    END_BLOCK_VALUE, Item, ItemContext, ParseError, encode_item_body, parse_raw_item,
};
use crate::reader::{ByteReader, Offset};
use crate::writer::{write_cstring, write_f32, write_keyed_i32};

/// A parsed stash: its grid dimensions and items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    pub version: i32,
    pub width: i32,
    pub height: i32,
    pub items: Vec<Item>,
}

/// Errors from parsing a stash file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StashError {
    #[error("item at {at} has an empty baseName")]
    EmptyBaseName { at: Offset },
    #[error("item at {at} has negative stackCount {count}")]
    NegativeStackCount { at: Offset, count: i32 },
    #[error("invalid item count {count}")]
    InvalidItemCount { count: i32 },
    #[error(transparent)]
    Parse(#[from] ParseError),
}

/// Parses a stash file byte image (`.dxb`, or its identical-format
/// `.dxg` backup twin).
///
/// # Errors
/// Shape errors from the underlying stream, or the item-level
/// validation errors above.
pub fn parse_stash(data: &[u8]) -> Result<Stash, StashError> {
    let mut reader = ByteReader::new(data);
    reader.read_i32().map_err(ParseError::from)?;

    let step = |reader: &mut ByteReader<'_>, key: &'static str| -> Result<i32, ParseError> {
        reader.expect_key(key)?;
        Ok(reader.read_i32()?)
    };
    step(&mut reader, "begin_block")?;
    let version = step(&mut reader, "stashVersion")?;
    reader.expect_key("fName").map_err(ParseError::from)?;
    reader.read_cstring().map_err(ParseError::from)?;
    let width = step(&mut reader, "sackWidth")?;
    let height = step(&mut reader, "sackHeight")?;

    let count = step(&mut reader, "numItems")?;
    let count = usize::try_from(count).map_err(|_| StashError::InvalidItemCount { count })?;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let at = Offset(reader.pos());
        let stack_count = step(&mut reader, "stackCount")?;
        let stack_size = u32::try_from(stack_count)
            .map_err(|_| StashError::NegativeStackCount {
                at,
                count: stack_count,
            })?
            .saturating_add(1);
        let raw = parse_raw_item(&mut reader, ItemContext::Stash)?;
        let mut item = raw.into_item().ok_or(StashError::EmptyBaseName { at })?;
        item.stack_size = stack_size;
        items.push(item);
    }
    step(&mut reader, "end_block")?;

    Ok(Stash {
        version,
        width,
        height,
        items,
    })
}

/// Rebuilds the stash's item region from `items`, copying the header
/// bytes of `original` through untouched and recomputing the leading
/// CRC — the targeted-splice rule in ARCHITECTURE.md. Never writes to
/// disk; the shell owns file IO, the backup-first step, and the
/// `.dxg` twin.
///
/// # Errors
/// The parse errors of locating the original item region, or
/// [`StashError::InvalidItemCount`] on absurd item counts.
pub fn replace_items(original: &[u8], items: &[Item]) -> Result<Vec<u8>, StashError> {
    let mut reader = ByteReader::new(original);
    reader.read_i32().map_err(ParseError::from)?;
    for key in ["begin_block", "stashVersion"] {
        reader.expect_key(key).map_err(ParseError::from)?;
        reader.read_i32().map_err(ParseError::from)?;
    }
    reader.expect_key("fName").map_err(ParseError::from)?;
    reader.read_cstring().map_err(ParseError::from)?;
    for key in ["sackWidth", "sackHeight"] {
        reader.expect_key(key).map_err(ParseError::from)?;
        reader.read_i32().map_err(ParseError::from)?;
    }
    let items_start = reader.pos();

    let count =
        i32::try_from(items.len()).map_err(|_| StashError::InvalidItemCount { count: i32::MAX })?;
    let mut out = Vec::with_capacity(original.len());
    out.extend_from_slice(&original[..items_start]);
    write_keyed_i32(&mut out, "numItems", count);
    for item in items {
        encode_stash_item(&mut out, item);
    }
    write_keyed_i32(&mut out, "end_block", END_BLOCK_VALUE);

    out[..4].fill(0);
    let crc = stash_crc(&out);
    out[..4].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}

/// Stash items carry an explicit `stackCount` (stack size − 1) and
/// float grid offsets instead of the player-sack repetition.
fn encode_stash_item(buf: &mut Vec<u8>, item: &Item) {
    let stack_count = i32::try_from(item.stack_size.max(1) - 1).unwrap_or(i32::MAX);
    write_keyed_i32(buf, "stackCount", stack_count);
    encode_item_body(buf, item, item.seed.value());
    write_cstring(buf, "xOffset");
    write_f32(buf, cell_to_offset(item.position.x));
    write_cstring(buf, "yOffset");
    write_f32(buf, cell_to_offset(item.position.y));
}

// Grid cells are small integers; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn cell_to_offset(cell: i32) -> f32 {
    cell as f32
}

/// Builds the `.dxg` backup-twin bytes for a `.dxb` image: identical
/// content with the stored `fName`'s trailing `b` patched to `g` and
/// the CRC recomputed — `TQVaultAE`'s `EncodeBackupFile`. The game
/// falls back to the twin when the `.dxb` is corrupt.
///
/// # Errors
/// The parse errors of locating `fName`.
pub fn backup_twin(dxb: &[u8]) -> Result<Vec<u8>, StashError> {
    patched_name_copy(dxb, *b"bB", b'g')
}

/// Rebuilds `.dxb` bytes from a `.dxg` backup twin — the inverse of
/// [`backup_twin`], for recovering a corrupt or truncated `.dxb` the
/// way the game does (the twin is a complete copy of the last good
/// write).
///
/// # Errors
/// The parse errors of locating `fName`.
pub fn restore_from_twin(dxg: &[u8]) -> Result<Vec<u8>, StashError> {
    patched_name_copy(dxg, *b"gG", b'b')
}

/// A copy of `data` with the stored `fName`'s trailing byte patched
/// (when it is one of `from`) and the leading CRC recomputed.
fn patched_name_copy(data: &[u8], from: [u8; 2], to: u8) -> Result<Vec<u8>, StashError> {
    let mut reader = ByteReader::new(data);
    reader.read_i32().map_err(ParseError::from)?;
    for key in ["begin_block", "stashVersion"] {
        reader.expect_key(key).map_err(ParseError::from)?;
        reader.read_i32().map_err(ParseError::from)?;
    }
    reader.expect_key("fName").map_err(ParseError::from)?;
    let name_length = reader.read_i32().map_err(ParseError::from)?;
    let name_length = usize::try_from(name_length)
        .map_err(|_| StashError::InvalidItemCount { count: name_length })?;
    let last_name_byte = reader.pos() + name_length.saturating_sub(1);

    let mut copy = data.to_vec();
    if let Some(byte) = copy.get_mut(last_name_byte)
        && from.contains(byte)
    {
        *byte = to;
    }
    copy[..4].fill(0);
    let crc = stash_crc(&copy);
    copy[..4].copy_from_slice(&crc.to_le_bytes());
    Ok(copy)
}

/// `TQVaultAE`'s `CalculateCRC`: the standard reflected CRC-32 table
/// (polynomial 0xEDB88320) but with a zero initial value and no final
/// complement. Computed over the whole file with the checksum field
/// zeroed.
pub(crate) fn stash_crc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        let low = u8::try_from(crc & 0xFF).expect("masked to one byte");
        crc = (crc >> 8) ^ CRC_TABLE[usize::from(low ^ byte)];
    }
    crc
}

const CRC_TABLE: [u32; 256] = build_crc_table();

// Index fits u32 by construction (i < 256).
#[allow(clippy::cast_possible_truncation)]
const fn build_crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut index = 0_usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chr::fixture::Fixture;

    fn stash_file() -> Vec<u8> {
        Fixture::default()
            .int(0) // checksum, unchecked on read
            .begin_block()
            .keyed_int("stashVersion", 2)
            .cstr("fName", "winsys.dxb")
            .keyed_int("sackWidth", 15)
            .keyed_int("sackHeight", 10)
            .keyed_int("numItems", 2)
            .stash_item(
                "records\\item\\equipmentweapon\\club_01.dbr",
                7,
                0,
                3.0,
                4.0,
            )
            .stash_item("records\\item\\potion\\healthpotion.dbr", 8, 9, 0.0, 0.0)
            .end_block()
            .bytes
    }

    #[test]
    fn parses_header_and_items() {
        let stash = parse_stash(&stash_file()).unwrap();
        assert_eq!(stash.version, 2);
        assert_eq!((stash.width, stash.height), (15, 10));
        assert_eq!(stash.items.len(), 2);
    }

    #[test]
    fn stack_count_is_stored_as_size_minus_one() {
        let stash = parse_stash(&stash_file()).unwrap();
        assert_eq!(stash.items[0].stack_size, 1);
        assert_eq!(stash.items[1].stack_size, 10);
    }

    #[test]
    fn float_offsets_become_grid_cells() {
        let stash = parse_stash(&stash_file()).unwrap();
        assert_eq!(
            (stash.items[0].position.x, stash.items[0].position.y),
            (3, 4)
        );
    }

    #[test]
    fn unchanged_stash_resplices_byte_identically_including_crc() {
        let built = stash_file();
        let mut original = built.clone();
        let crc = stash_crc(&built);
        original[..4].copy_from_slice(&crc.to_le_bytes());

        let stash = parse_stash(&original).unwrap();
        let respliced = replace_items(&original, &stash.items).unwrap();
        assert!(
            respliced == original,
            "splice of unchanged stash altered bytes"
        );
    }

    #[test]
    fn adding_an_item_keeps_the_header_and_recomputes_the_crc() {
        let original = stash_file();
        let mut stash = parse_stash(&original).unwrap();
        stash.items.push(stash.items[0].clone());

        let modified = replace_items(&original, &stash.items).unwrap();
        let reparsed = parse_stash(&modified).unwrap();
        assert_eq!(reparsed.items.len(), 3);
        assert_eq!((reparsed.width, reparsed.height), (15, 10));

        let mut zeroed = modified.clone();
        zeroed[..4].fill(0);
        assert_eq!(
            modified[..4],
            stash_crc(&zeroed).to_le_bytes(),
            "stored CRC must match the recomputed one"
        );
    }

    #[test]
    fn backup_twin_patches_the_stored_name_and_crc() {
        let original = stash_file();
        let twin = backup_twin(&original).unwrap();

        let name_at = crate::reader::find_key(&twin, "fName", 0).unwrap();
        let mut reader = crate::reader::ByteReader::at(&twin, name_at);
        assert_eq!(reader.read_cstring().unwrap(), "winsys.dxg");

        let mut zeroed = twin.clone();
        zeroed[..4].fill(0);
        assert_eq!(twin[..4], stash_crc(&zeroed).to_le_bytes());
        assert_eq!(parse_stash(&twin).unwrap(), parse_stash(&original).unwrap());
    }

    #[test]
    fn restore_from_twin_round_trips_the_dxb_image() {
        let mut original = stash_file();
        let crc = {
            let mut zeroed = original.clone();
            zeroed[..4].fill(0);
            stash_crc(&zeroed)
        };
        original[..4].copy_from_slice(&crc.to_le_bytes());

        let twin = backup_twin(&original).unwrap();
        let restored = restore_from_twin(&twin).unwrap();
        assert_eq!(restored, original, "twin restore must invert backup_twin");

        let name_at = crate::reader::find_key(&restored, "fName", 0).unwrap();
        let mut reader = crate::reader::ByteReader::at(&restored, name_at);
        assert_eq!(reader.read_cstring().unwrap(), "winsys.dxb");
    }

    #[test]
    fn negative_stack_count_is_rejected() {
        let bytes = Fixture::default()
            .int(0)
            .begin_block()
            .keyed_int("stashVersion", 2)
            .cstr("fName", "winsys.dxb")
            .keyed_int("sackWidth", 15)
            .keyed_int("sackHeight", 10)
            .keyed_int("numItems", 1)
            .stash_item("records\\item\\a.dbr", 1, -2, 0.0, 0.0)
            .end_block()
            .bytes;
        assert!(matches!(
            parse_stash(&bytes),
            Err(StashError::NegativeStackCount { count: -2, .. })
        ));
    }

    #[test]
    fn truncated_stash_reports_shape_error() {
        let full = stash_file();
        let truncated = &full[..full.len() - 10];
        assert!(matches!(parse_stash(truncated), Err(StashError::Parse(_))));
    }

    #[test]
    fn non_stash_data_is_a_key_mismatch() {
        let bytes = Fixture::default()
            .int(0)
            .keyed_int("somethingElse", 1)
            .bytes;
        assert!(matches!(parse_stash(&bytes), Err(StashError::Parse(_))));
    }
}
