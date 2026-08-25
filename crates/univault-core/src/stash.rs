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

use crate::chr::{Item, ItemContext, ParseError, parse_raw_item};
use crate::reader::{ByteReader, Offset};

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
