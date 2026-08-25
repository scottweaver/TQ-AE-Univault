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
use crate::grid::{CellRect, find_open_cells};
use crate::stash::Stash;
use crate::vault::{TAB_HEIGHT, TAB_WIDTH, Vault, VaultItem};

/// Why a placement failed; the item itself travels back separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("no free space in the target container")]
    NoRoom,
    #[error("no such container")]
    BadIndex,
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
}

fn footprint(db: Option<&GameCache>, item: &Item) -> (i32, i32) {
    db.map_or(FALLBACK_FOOTPRINT, |db| db.item_footprint(item))
}

fn occupied(items: &[Item], db: Option<&GameCache>) -> Vec<CellRect> {
    items
        .iter()
        .map(|item| {
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

fn vault_occupied(items: &[VaultItem], db: Option<&GameCache>) -> Vec<CellRect> {
    items
        .iter()
        .map(|entry| {
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
        let taken = vault_occupied(&vault.sacks[tab].items, db);
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
        let taken = occupied(&character.sacks[index].items, db);
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
    let taken = occupied(&stash.items, db);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chr::fixture::Fixture;
    use crate::chr::{ItemSeed, RecordId, parse_player};
    use crate::vault::TAB_HEIGHT;

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
    fn stale_indices_take_nothing() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        assert!(take_from_character(&mut character, 5, 0).is_none());
        assert!(take_from_character(&mut character, 0, 9).is_none());
    }
}
