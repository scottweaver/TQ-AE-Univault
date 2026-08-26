//! Pure item-movement operations between containers — the model side
//! of vault ↔ character/stash transfers. Nothing here touches disk;
//! the shell reads files, calls these, splices, and writes with
//! backup-first.
//!
//! Every placement uses conservative footprints (see
//! [`GameCache::item_footprint`]) through the grid search ported from
//! `TQVaultAE`, so items never overlap at their true sizes. A failed
//! placement returns the item to the caller instead of dropping it.

use crate::cache::GameCache;
use crate::chr::{GridPos, Item, PlayerCharacter, sack_dimensions};
use crate::gamedata::FALLBACK_FOOTPRINT;
use crate::grid::{CellRect, find_open_cells, fits as grid_fits};
use crate::stash::Stash;
use crate::vault::{TAB_HEIGHT, TAB_WIDTH, Vault, VaultItem};

/// Why a placement failed; the item itself travels back separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("no free space in the target container")]
    NoRoom,
    #[error("no such container")]
    BadIndex,
    #[error("that spot is occupied or out of bounds")]
    Occupied,
}

/// A failed placement, handing the item back to the caller (boxed to
/// keep the `Err` variant small).
#[derive(Debug)]
pub struct Rejected {
    pub item: Box<Item>,
    pub reason: TransferError,
}

impl Rejected {
    fn no_room(item: Item) -> Self {
        Self {
            item: Box::new(item),
            reason: TransferError::NoRoom,
        }
    }

    fn because(item: Item, reason: TransferError) -> Self {
        Self {
            item: Box::new(item),
            reason,
        }
    }
}

fn footprint(db: Option<&GameCache>, item: &Item) -> (i32, i32) {
    db.map_or(FALLBACK_FOOTPRINT, |db| db.item_footprint(item))
}

/// Removes an item from a character sack. `None` on a stale index.
pub fn take_from_character(
    character: &mut PlayerCharacter,
    sack: usize,
    index: usize,
) -> Option<Item> {
    let sack = character.sacks.get_mut(sack)?;
    (index < sack.items.len()).then(|| sack.items.remove(index))
}

/// Removes an item from a stash. `None` on a stale index.
pub fn take_from_stash(stash: &mut Stash, index: usize) -> Option<Item> {
    (index < stash.items.len()).then(|| stash.items.remove(index))
}

/// Removes an item from a vault tab. `None` on a stale index.
pub fn take_from_vault(vault: &mut Vault, tab: usize, index: usize) -> Option<VaultItem> {
    let sack = vault.sacks.get_mut(tab)?;
    (index < sack.items.len()).then(|| sack.items.remove(index))
}

/// Places an item into the vault, trying `preferred_tab` first and
/// then every other tab. Returns the tab it landed in, or hands the
/// item back on failure.
///
/// # Errors
/// [`TransferError::NoRoom`] when no tab can fit the footprint.
pub fn place_in_vault(
    vault: &mut Vault,
    item: Item,
    preferred_tab: usize,
    db: Option<&GameCache>,
) -> Result<usize, Rejected> {
    let (width, height) = footprint(db, &item);
    let tab_count = vault.sacks.len();
    let order = (0..tab_count).cycle().skip(preferred_tab).take(tab_count);
    for tab in order {
        let taken = vault_occupancy(&vault.sacks[tab].items, None, db);
        if let Some(position) = find_open_cells(&taken, width, height, TAB_WIDTH, TAB_HEIGHT) {
            let mut item = item;
            item.position = position;
            // Store 0×0 ("unknown"): TQVaultAE recomputes real
            // footprints from game data on load.
            vault.sacks[tab].items.push(VaultItem::new(item, 0, 0));
            return Ok(tab);
        }
    }
    Err(Rejected::no_room(item))
}

/// Places an item into a character's inventory, trying
/// `preferred_sack` first and then every sack. Returns the sack it
/// landed in, or hands the item back on failure.
///
/// # Errors
/// [`TransferError::NoRoom`] when no sack can fit the footprint.
pub fn place_in_character(
    character: &mut PlayerCharacter,
    item: Item,
    preferred_sack: usize,
    db: Option<&GameCache>,
) -> Result<usize, Rejected> {
    let (width, height) = footprint(db, &item);
    let sack_count = character.sacks.len();
    let order = (0..sack_count)
        .cycle()
        .skip(preferred_sack)
        .take(sack_count);
    for index in order {
        let (sack_width, sack_height) = sack_dimensions(index);
        let taken = occupancy(&character.sacks[index].items, None, db);
        if let Some(position) = find_open_cells(&taken, width, height, sack_width, sack_height) {
            let mut item = item;
            item.position = position;
            character.sacks[index].items.push(item);
            return Ok(index);
        }
    }
    Err(Rejected::no_room(item))
}

/// Places an item into a stash grid, or hands it back on failure.
///
/// # Errors
/// [`TransferError::NoRoom`] when the stash cannot fit the footprint.
pub fn place_in_stash(
    stash: &mut Stash,
    item: Item,
    db: Option<&GameCache>,
) -> Result<(), Rejected> {
    let (width, height) = footprint(db, &item);
    let taken = occupancy(&stash.items, None, db);
    match find_open_cells(&taken, width, height, stash.width, stash.height) {
        Some(position) => {
            let mut item = item;
            item.position = position;
            stash.items.push(item);
            Ok(())
        }
        None => Err(Rejected::no_room(item)),
    }
}

/// The [`GridPos`] every take leaves behind conceptually; exposed for
/// tests and shells that show "removed from (x, y)".
#[must_use]
pub fn position_of(item: &Item) -> GridPos {
    item.position
}

/// Whether `item` could sit with its top-left at `position` in the
/// given container without overlap — the drop-preview query. The
/// occupancy list is the container's current items; the shell
/// excludes the dragged item itself before asking.
#[must_use]
pub fn fits_at(
    occupied: &[CellRect],
    footprint: (i32, i32),
    position: GridPos,
    container: (i32, i32),
) -> bool {
    grid_fits(
        occupied,
        CellRect {
            x: position.x,
            y: position.y,
            width: footprint.0,
            height: footprint.1,
        },
        container.0,
        container.1,
    )
}

/// Occupancy rectangles of plain item lists (character sacks, the
/// stash), optionally skipping one index (the item being dragged).
#[must_use]
pub fn occupancy(items: &[Item], skip: Option<usize>, db: Option<&GameCache>) -> Vec<CellRect> {
    items
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != skip)
        .map(|(_, item)| {
            let (width, height) = footprint(db, item);
            CellRect {
                x: item.position.x,
                y: item.position.y,
                width,
                height,
            }
        })
        .collect()
}

/// Occupancy rectangles of a vault tab, optionally skipping one index.
#[must_use]
pub fn vault_occupancy(
    items: &[VaultItem],
    skip: Option<usize>,
    db: Option<&GameCache>,
) -> Vec<CellRect> {
    items
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != skip)
        .map(|(_, entry)| {
            let (width, height) = if entry.width > 0 && entry.height > 0 {
                (entry.width, entry.height)
            } else {
                footprint(db, &entry.item)
            };
            CellRect {
                x: entry.item.position.x,
                y: entry.item.position.y,
                width,
                height,
            }
        })
        .collect()
}

/// Places an item at an exact cell in a character sack, or hands it
/// back when the spot is occupied or out of bounds.
///
/// # Errors
/// [`TransferError::BadIndex`] for an unknown sack;
/// [`TransferError::Occupied`] when the footprint does not fit there.
pub fn place_in_character_at(
    character: &mut PlayerCharacter,
    item: Item,
    sack: usize,
    position: GridPos,
    db: Option<&GameCache>,
) -> Result<(), Rejected> {
    let Some(target) = character.sacks.get_mut(sack) else {
        return Err(Rejected::because(item, TransferError::BadIndex));
    };
    let taken = occupancy(&target.items, None, db);
    if !fits_at(
        &taken,
        footprint(db, &item),
        position,
        sack_dimensions(sack),
    ) {
        return Err(Rejected::because(item, TransferError::Occupied));
    }
    let mut item = item;
    item.position = position;
    target.items.push(item);
    Ok(())
}

/// Places an item at an exact cell in the stash, or hands it back.
///
/// # Errors
/// [`TransferError::Occupied`] when the footprint does not fit there.
pub fn place_in_stash_at(
    stash: &mut Stash,
    item: Item,
    position: GridPos,
    db: Option<&GameCache>,
) -> Result<(), Rejected> {
    let taken = occupancy(&stash.items, None, db);
    if !fits_at(
        &taken,
        footprint(db, &item),
        position,
        (stash.width, stash.height),
    ) {
        return Err(Rejected::because(item, TransferError::Occupied));
    }
    let mut item = item;
    item.position = position;
    stash.items.push(item);
    Ok(())
}

/// Places an item at an exact cell in a vault tab, or hands it back.
///
/// # Errors
/// [`TransferError::BadIndex`] for an unknown tab;
/// [`TransferError::Occupied`] when the footprint does not fit there.
pub fn place_in_vault_at(
    vault: &mut Vault,
    item: Item,
    tab: usize,
    position: GridPos,
    db: Option<&GameCache>,
) -> Result<(), Rejected> {
    let Some(target) = vault.sacks.get_mut(tab) else {
        return Err(Rejected::because(item, TransferError::BadIndex));
    };
    let taken = vault_occupancy(&target.items, None, db);
    if !fits_at(
        &taken,
        footprint(db, &item),
        position,
        (TAB_WIDTH, TAB_HEIGHT),
    ) {
        return Err(Rejected::because(item, TransferError::Occupied));
    }
    let mut item = item;
    item.position = position;
    target.items.push(VaultItem::new(item, 0, 0));
    Ok(())
}

/// The footprint used for placement decisions — exposed so the shell
/// paints previews with the same numbers placements use.
#[must_use]
pub fn placement_footprint(db: Option<&GameCache>, item: &Item) -> (i32, i32) {
    footprint(db, item)
}

/// Result of pouring shards from one partial relic/charm into
/// another of the same record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combined {
    pub transferred: i32,
    pub source_emptied: bool,
    pub target_completed: bool,
}

/// Whether `source` can pour shards into `target`: the same
/// relic/charm record, the target still short of completion, and
/// the completion level known from game data.
#[must_use]
pub fn can_combine(db: Option<&GameCache>, source: &Item, target: &Item) -> bool {
    let Some(needed) = db.and_then(|db| db.completed_relic_level(&target.base)) else {
        return false;
    };
    source.base == target.base && source.var1 > 0 && target.var1 < needed
}

/// Pours shards from `source` into `target` up to `needed` — the
/// game's merge rule: the remainder stays in the source, so shards
/// are never destroyed.
pub fn combine_shards(target: &mut Item, source: &mut Item, needed: i32) -> Combined {
    let transferred = source.var1.min(needed - target.var1).max(0);
    target.var1 += transferred;
    source.var1 -= transferred;
    Combined {
        transferred,
        source_emptied: source.var1 <= 0,
        target_completed: target.var1 >= needed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chr::fixture::Fixture;
    use crate::chr::{ItemSeed, RecordId, parse_player};
    use crate::vault::TAB_HEIGHT;

    fn shard(record: &str, count: i32) -> Item {
        let mut item = Item::bare(
            RecordId::parse(record.to_string()).unwrap(),
            ItemSeed::new(7),
        );
        item.var1 = count;
        item
    }

    #[test]
    fn combine_pours_shards_and_keeps_the_remainder() {
        let mut target = shard(r"records\item\animalrelics\boarhide.dbr", 4);
        let mut source = shard(r"records\item\animalrelics\boarhide.dbr", 3);
        let outcome = combine_shards(&mut target, &mut source, 5);
        assert_eq!(
            outcome,
            Combined {
                transferred: 1,
                source_emptied: false,
                target_completed: true,
            }
        );
        assert_eq!(target.var1, 5);
        assert_eq!(source.var1, 2);
    }

    #[test]
    fn combine_empties_the_source_when_it_all_fits() {
        let mut target = shard(r"records\item\animalrelics\boarhide.dbr", 1);
        let mut source = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        let outcome = combine_shards(&mut target, &mut source, 5);
        assert_eq!(
            outcome,
            Combined {
                transferred: 2,
                source_emptied: true,
                target_completed: false,
            }
        );
        assert_eq!(target.var1, 3);
        assert_eq!(source.var1, 0);
    }

    #[test]
    fn combine_moves_nothing_into_a_complete_target() {
        let mut target = shard(r"records\item\animalrelics\boarhide.dbr", 5);
        let mut source = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        let outcome = combine_shards(&mut target, &mut source, 5);
        assert_eq!(outcome.transferred, 0);
        assert!(!outcome.source_emptied);
    }

    #[test]
    fn can_combine_needs_game_data_and_matching_partials() {
        let a = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        let b = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        assert!(!can_combine(None, &a, &b));
    }

    fn player_bytes() -> Vec<u8> {
        let mut fixture = Fixture::default()
            .utf16("myPlayerName", "Ajax")
            .cstr("playerClassTag", "tagC")
            .keyed_int("playerLevel", 5)
            .keyed_int("money", 100)
            .begin_block()
            .keyed_int("itemPositionsSavedAsGridCoords", 1)
            .keyed_int("numberOfSacks", 2)
            .keyed_int("currentlyFocusedSackNumber", 0)
            .keyed_int("currentlySelectedSackNumber", 0)
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 1)
            .sack_item("records\\item\\equipmentweapon\\sword_01.dbr", 11, 0, 0)
            .end_block()
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 0)
            .end_block()
            .begin_block()
            .keyed_int("useAlternate", 0)
            .keyed_int("equipmentCtrlIOStreamVersion", 0);
        for slot in 0..crate::chr::EQUIPMENT_SLOTS {
            if slot == 7 || slot == 9 {
                fixture = fixture
                    .begin_block()
                    .keyed_int("alternate", i32::from(slot == 9));
            }
            fixture = fixture.equipment_slot("");
            if slot == 8 || slot == 10 {
                fixture = fixture.end_block();
            }
        }
        fixture.end_block().end_block().bytes
    }

    fn item(base: &str) -> Item {
        Item {
            base: RecordId::parse(base.to_string()).unwrap(),
            prefix: None,
            suffix: None,
            relic: None,
            relic_bonus: None,
            seed: ItemSeed::new(1),
            var1: 0,
            atlantis: None,
            position: GridPos { x: 0, y: 0 },
            stack_size: 1,
            folded_members: Vec::new(),
        }
    }

    #[test]
    fn character_to_vault_round_trip_preserves_the_item() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        let mut vault = Vault::new(2);

        let taken = take_from_character(&mut character, 0, 0).unwrap();
        let original = taken.clone();
        let tab = place_in_vault(&mut vault, taken, 0, None).unwrap();
        assert_eq!(tab, 0);
        assert!(character.sacks[0].items.is_empty());

        let back = take_from_vault(&mut vault, 0, 0).unwrap();
        assert_eq!(back.item.base, original.base);
        let sack = place_in_character(&mut character, back.item, 0, None).unwrap();
        assert_eq!(sack, 0);
        assert_eq!(character.sacks[0].items[0].base, original.base);
    }

    #[test]
    fn placement_avoids_existing_vault_items() {
        let mut vault = Vault::new(1);
        place_in_vault(&mut vault, item("records\\a.dbr"), 0, None).unwrap();
        place_in_vault(&mut vault, item("records\\b.dbr"), 0, None).unwrap();
        let first = vault.sacks[0].items[0].item.position;
        let second = vault.sacks[0].items[1].item.position;
        assert_ne!(first, second);
        // Fallback footprint is 2x5: the second item lands below or
        // beside, never overlapping.
        assert!(second.y >= first.y + 5 || second.x >= first.x + 2);
    }

    #[test]
    fn full_vault_hands_the_item_back() {
        let mut vault = Vault::new(1);
        let per_tab = usize::try_from((TAB_WIDTH / 2) * (TAB_HEIGHT / 5)).unwrap();
        for index in 0..per_tab {
            let id = format!("records\\filler{index}.dbr");
            place_in_vault(&mut vault, item(&id), 0, None).unwrap();
        }
        let rejected =
            place_in_vault(&mut vault, item("records\\overflow.dbr"), 0, None).unwrap_err();
        assert_eq!(rejected.reason, TransferError::NoRoom);
        assert_eq!(rejected.item.base.file_stem(), "overflow");
        assert_eq!(vault.sacks[0].items.len(), per_tab);
    }

    #[test]
    fn overflow_spills_into_the_next_tab() {
        let mut vault = Vault::new(2);
        let per_tab = usize::try_from((TAB_WIDTH / 2) * (TAB_HEIGHT / 5)).unwrap();
        for index in 0..per_tab {
            let id = format!("records\\filler{index}.dbr");
            place_in_vault(&mut vault, item(&id), 0, None).unwrap();
        }
        let tab = place_in_vault(&mut vault, item("records\\next.dbr"), 0, None).unwrap();
        assert_eq!(tab, 1);
    }

    #[test]
    fn stash_placement_respects_its_grid() {
        let mut stash = Stash {
            version: 2,
            width: 2,
            height: 5,
            items: Vec::new(),
        };
        place_in_stash(&mut stash, item("records\\a.dbr"), None).unwrap();
        let rejected = place_in_stash(&mut stash, item("records\\b.dbr"), None).unwrap_err();
        assert_eq!(rejected.reason, TransferError::NoRoom);
        assert_eq!(rejected.item.base.file_stem(), "b");
    }

    #[test]
    fn exact_placement_honors_position_and_rejects_overlap() {
        let mut vault = Vault::new(1);
        place_in_vault_at(
            &mut vault,
            item("records\\a.dbr"),
            0,
            GridPos { x: 3, y: 4 },
            None,
        )
        .unwrap();
        assert_eq!(
            vault.sacks[0].items[0].item.position,
            GridPos { x: 3, y: 4 }
        );

        // Fallback footprint is 2x5: overlapping cells are refused…
        let rejected = place_in_vault_at(
            &mut vault,
            item("records\\b.dbr"),
            0,
            GridPos { x: 4, y: 8 },
            None,
        )
        .unwrap_err();
        assert_eq!(rejected.reason, TransferError::Occupied);
        // …and the item comes back intact.
        assert_eq!(rejected.item.base.file_stem(), "b");

        // Just clear of the first item is fine.
        place_in_vault_at(&mut vault, *rejected.item, 0, GridPos { x: 5, y: 0 }, None).unwrap();
        assert_eq!(vault.sacks[0].items.len(), 2);

        let bad_tab = place_in_vault_at(
            &mut vault,
            item("records\\c.dbr"),
            7,
            GridPos { x: 0, y: 0 },
            None,
        )
        .unwrap_err();
        assert_eq!(bad_tab.reason, TransferError::BadIndex);
    }

    #[test]
    fn exact_placement_respects_container_bounds() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        let (width, height) = sack_dimensions(1);
        let out_of_bounds = place_in_character_at(
            &mut character,
            item("records\\a.dbr"),
            1,
            GridPos {
                x: width - 1,
                y: height - 1,
            },
            None,
        )
        .unwrap_err();
        assert_eq!(out_of_bounds.reason, TransferError::Occupied);

        place_in_character_at(
            &mut character,
            item("records\\a.dbr"),
            1,
            GridPos { x: 0, y: 0 },
            None,
        )
        .unwrap();
        assert_eq!(character.sacks[1].items.len(), 1);

        let mut stash = Stash {
            version: 2,
            width: 4,
            height: 5,
            items: Vec::new(),
        };
        place_in_stash_at(
            &mut stash,
            item("records\\s.dbr"),
            GridPos { x: 2, y: 0 },
            None,
        )
        .unwrap();
        let clash = place_in_stash_at(
            &mut stash,
            item("records\\t.dbr"),
            GridPos { x: 1, y: 4 },
            None,
        )
        .unwrap_err();
        assert_eq!(clash.reason, TransferError::Occupied);
    }

    #[test]
    fn moving_within_a_container_via_take_and_place_at() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        let taken = take_from_character(&mut character, 0, 0).unwrap();
        let original = taken.position;
        assert_eq!(original, GridPos { x: 0, y: 0 });
        // Sacks are 5 rows tall and the fallback footprint 2×5, so
        // a move can only change the column.
        place_in_character_at(&mut character, taken, 0, GridPos { x: 5, y: 0 }, None).unwrap();
        let moved = &character.sacks[0].items[0];
        assert_eq!(moved.position, GridPos { x: 5, y: 0 });
    }

    #[test]
    fn occupancy_skip_frees_the_dragged_items_cells() {
        let mut vault = Vault::new(1);
        place_in_vault_at(
            &mut vault,
            item("records\\a.dbr"),
            0,
            GridPos { x: 0, y: 0 },
            None,
        )
        .unwrap();
        let items = &vault.sacks[0].items;
        // With the item excluded, its own cells count as free — the
        // preview a drag shows over the item's original spot.
        let without = vault_occupancy(items, Some(0), None);
        assert!(fits_at(&without, (2, 5), GridPos { x: 0, y: 0 }, (18, 20)));
        let with = vault_occupancy(items, None, None);
        assert!(!fits_at(&with, (2, 5), GridPos { x: 0, y: 0 }, (18, 20)));
        // Out of bounds is never a fit.
        assert!(!fits_at(
            &without,
            (2, 5),
            GridPos { x: 17, y: 0 },
            (18, 20)
        ));
        assert!(!fits_at(
            &without,
            (2, 5),
            GridPos { x: -1, y: 0 },
            (18, 20)
        ));
    }

    #[test]
    fn stale_indices_take_nothing() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        assert!(take_from_character(&mut character, 5, 0).is_none());
        assert!(take_from_character(&mut character, 0, 9).is_none());
    }
}
