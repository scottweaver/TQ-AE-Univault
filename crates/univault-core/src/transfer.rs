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
use crate::chr::{EquipSlot, GridPos, Item, PlayerCharacter, sack_dimensions};
use crate::gamedata::FALLBACK_FOOTPRINT;
use crate::grid::{CellRect, find_open_cells, fits as grid_fits};
use crate::stash::Stash;
use crate::style::GearSlot;
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

/// Removes the item worn in `slot`. `None` when the slot is empty.
pub fn take_equipped(character: &mut PlayerCharacter, slot: EquipSlot) -> Option<Item> {
    character.equipment.slot_mut(slot).take()
}

/// Whether `item` may be worn in `slot` — the game's own type rules:
/// armor and jewelry to their matching slots, any weapon or shield in
/// any hand slot (the reference implementation's rule; the game
/// resolves wielding on load), artifacts only in the artifact slot.
/// Needs game data; stacks are never wearable.
#[must_use]
pub fn can_equip(db: Option<&GameCache>, item: &Item, slot: EquipSlot) -> bool {
    let Some(db) = db else {
        return false;
    };
    if item.stack_size > 1 {
        return false;
    }
    if db.is_artifact(&item.base) {
        return slot == EquipSlot::Artifact;
    }
    db.gear_slot(&item.base)
        .is_some_and(|family| family_allows(family, slot))
}

fn family_allows(family: GearSlot, slot: EquipSlot) -> bool {
    match family {
        GearSlot::Head => slot == EquipSlot::Head,
        GearSlot::Amulet => slot == EquipSlot::Neck,
        GearSlot::UpperBody => slot == EquipSlot::Torso,
        GearSlot::LowerBody => slot == EquipSlot::Legs,
        GearSlot::Forearm => slot == EquipSlot::Arms,
        GearSlot::Ring => matches!(slot, EquipSlot::Ring1 | EquipSlot::Ring2),
        GearSlot::Shield
        | GearSlot::Sword
        | GearSlot::Axe
        | GearSlot::Mace
        | GearSlot::Spear
        | GearSlot::Bow
        | GearSlot::Thrown
        | GearSlot::Staff => slot.is_hand(),
        // A relic allow-flag family with no equipment slot behind it.
        GearSlot::Bracelet => false,
    }
}

/// Wears `item` in `slot`, normalizing its grid position away. The
/// slot must be empty — swapping is the caller's affair — and the
/// caller gates type rules with [`can_equip`].
///
/// # Errors
/// [`TransferError::Occupied`] when something is already worn there.
pub fn equip(character: &mut PlayerCharacter, item: Item, slot: EquipSlot) -> Result<(), Rejected> {
    let target = character.equipment.slot_mut(slot);
    if target.is_some() {
        return Err(Rejected::because(item, TransferError::Occupied));
    }
    *target = Some(Item {
        position: GridPos { x: 0, y: 0 },
        ..item
    });
    Ok(())
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

/// A standalone relic/charm's shard count. The game stores a single
/// freshly dropped shard as `var1 = 0` — one shard and "unset" share
/// an encoding — so the effective count is never below one (the same
/// rule the stats renderer ports from `TQVaultAE`).
#[must_use]
pub fn shard_count(item: &Item) -> i32 {
    item.var1.max(1)
}

/// Whether `source` can pour shards into `target`: the same
/// relic/charm record, both still short of completion (a completed
/// piece is a finished item carrying its bonus — never a pour
/// source), and the completion level known from game data.
#[must_use]
pub fn can_combine(db: Option<&GameCache>, source: &Item, target: &Item) -> bool {
    let Some(needed) = db.and_then(|db| db.completed_relic_level(&target.base)) else {
        return false;
    };
    source.base == target.base && shard_count(source) < needed && shard_count(target) < needed
}

/// Pours shards from `source` into `target` up to `needed` — the
/// game's merge rule: the remainder stays in the source, so shards
/// are never destroyed. Counts are the effective [`shard_count`]s,
/// so zero-encoded single shards pour their one shard.
pub fn combine_shards(target: &mut Item, source: &mut Item, needed: i32) -> Combined {
    let source_count = shard_count(source);
    let target_count = shard_count(target);
    let transferred = source_count.min(needed - target_count).max(0);
    target.var1 = target_count + transferred;
    source.var1 = source_count - transferred;
    Combined {
        transferred,
        source_emptied: source.var1 <= 0,
        target_completed: target.var1 >= needed,
    }
}

/// Which socket of a gear item holds the piece to extract: the
/// classic relic/charm socket, or the Atlantis-era second socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelicSlot {
    First,
    Second,
}

/// Why an extraction failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    #[error("nothing is socketed in that slot")]
    NoRelic,
}

/// The socket an extraction would take from, first socket first;
/// `None` when nothing is socketed.
#[must_use]
pub fn socketed_slot(item: &Item) -> Option<RelicSlot> {
    if item.relic.is_some() {
        return Some(RelicSlot::First);
    }
    item.atlantis
        .as_ref()
        .and_then(|extra| extra.relic.as_ref())
        .map(|_| RelicSlot::Second)
}

/// Splits the socketed relic/charm out of `gear` without destroying
/// either side — the app-side alternative to the Enchanter's
/// destroy-one-half recovery. The returned standalone piece carries
/// the socket's shard count (clamped to the record's completion
/// level — the second socket stores a completed-marker sentinel, not
/// a count) and the socket's completion bonus; the gear keeps
/// everything else. On error, `gear` is unchanged.
///
/// # Errors
/// [`ExtractError::NoRelic`] when the requested socket is empty.
pub fn extract_relic(
    db: Option<&GameCache>,
    gear: &mut Item,
    slot: RelicSlot,
) -> Result<Item, ExtractError> {
    let (record, bonus, count) = match slot {
        RelicSlot::First => {
            let record = gear.relic.take().ok_or(ExtractError::NoRelic)?;
            let bonus = gear.relic_bonus.take();
            let count = gear.var1;
            gear.var1 = 0;
            (record, bonus, count)
        }
        RelicSlot::Second => {
            let extra = gear.atlantis.take().ok_or(ExtractError::NoRelic)?;
            let Some(record) = extra.relic else {
                gear.atlantis = Some(extra);
                return Err(ExtractError::NoRelic);
            };
            (record, extra.bonus, extra.var2)
        }
    };
    let mut piece = Item::bare(record, gear.seed);
    let needed = db.and_then(|db| db.completed_relic_level(&piece.base));
    piece.var1 = needed.map_or_else(|| count.max(1), |needed| count.max(1).min(needed));
    piece.relic_bonus = bonus;
    Ok(piece)
}

/// Whether the standalone relic/charm `piece` may be socketed into
/// `target`: the target's socket is empty, the target is equipment
/// of a family the piece's record allows (the game's own type
/// rules), and game data is loaded. Rarity is deliberately not
/// checked — that gate lives in the game's socketing UI, not its
/// item model, so epics, legendaries, and set pieces are all fair
/// targets here.
#[must_use]
pub fn can_socket(db: Option<&GameCache>, piece: &Item, target: &Item) -> bool {
    let Some(db) = db else {
        return false;
    };
    if db.completed_relic_level(&piece.base).is_none() || target.relic.is_some() {
        return false;
    }
    db.gear_slot(&target.base)
        .is_some_and(|slot| db.relic_allows(&piece.base, slot))
}

/// Sockets `piece` into `target`'s first socket — record, shard
/// count, and completion bonus move onto the gear; the standalone
/// piece ceases to exist. Callers gate with [`can_socket`].
pub fn socket_relic(target: &mut Item, piece: Item) {
    target.var1 = shard_count(&piece);
    target.relic_bonus = piece.relic_bonus;
    target.relic = Some(piece.base);
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

    /// The game stores one freshly dropped shard as `var1 = 0`; two
    /// such singles must merge into a two-shard stack.
    #[test]
    fn combine_pours_zero_encoded_single_shards() {
        let mut target = shard(r"records\item\animalrelics\boarhide.dbr", 0);
        let mut source = shard(r"records\item\animalrelics\boarhide.dbr", 0);
        let outcome = combine_shards(&mut target, &mut source, 3);
        assert_eq!(
            outcome,
            Combined {
                transferred: 1,
                source_emptied: true,
                target_completed: false,
            }
        );
        assert_eq!(target.var1, 2);
        assert_eq!(source.var1, 0);
    }

    fn charm_db() -> GameCache {
        use crate::arz::ArzFile;
        use crate::arz::fixture::{ArzBuilder, Values};
        use crate::gamedata::GameData;
        use crate::text::TextDb;
        let mut builder = ArzBuilder::default();
        // A charm that only enchants head and torso armor.
        builder.record(
            "records\\item\\animalrelics\\boarhide.dbr",
            "ItemCharm",
            &[
                ("completedRelicLevel", Values::Ints(&[3])),
                ("helmet", Values::Bools(&[true])),
                ("bodyArmor", Values::Bools(&[true])),
                ("sword", Values::Bools(&[false])),
            ],
        );
        builder.record(
            "records\\item\\equipmenthelm\\bronzehelm.dbr",
            "ArmorProtective_Head",
            &[("itemClassification", Values::Strings(&["Legendary"]))],
        );
        builder.record(
            "records\\item\\equipmentweapon\\gladius.dbr",
            "WeaponMelee_Sword",
            &[("itemClassification", Values::Strings(&["Epic"]))],
        );
        builder.record(
            "records\\item\\equipmentring\\loop.dbr",
            "ArmorJewelry_Ring",
            &[],
        );
        builder.record(
            "records\\item\\equipmentshield\\tower.dbr",
            "WeaponArmor_Shield",
            &[],
        );
        builder.record(
            "records\\xpack\\item\\artifacts\\fool.dbr",
            "ItemArtifact",
            &[],
        );
        let data = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), TextDb::new());
        data.build_cache(Vec::new())
    }

    #[test]
    fn socketing_honors_type_rules_and_ignores_rarity() {
        let db = charm_db();
        let charm = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        let legendary_helm = shard(r"records\item\equipmenthelm\bronzehelm.dbr", 0);
        let epic_sword = shard(r"records\item\equipmentweapon\gladius.dbr", 0);
        // A head charm fits a legendary helm (rarity never gates)…
        assert!(can_socket(Some(&db), &charm, &legendary_helm));
        // …but never a sword (the game's type rules stand)…
        assert!(!can_socket(Some(&db), &charm, &epic_sword));
        // …and gear can't be socketed into gear, nor without data.
        assert!(!can_socket(Some(&db), &legendary_helm, &epic_sword));
        assert!(!can_socket(None, &charm, &legendary_helm));
        // An occupied socket refuses.
        let mut socketed = legendary_helm.clone();
        socketed.relic = Some(RecordId::parse("records\\x.dbr".to_string()).unwrap());
        assert!(!can_socket(Some(&db), &charm, &socketed));
    }

    #[test]
    fn socket_moves_record_count_and_bonus_onto_the_gear() {
        let bonus = RecordId::parse(r"records\item\bonus.dbr".to_string()).unwrap();
        let mut charm = shard(r"records\item\animalrelics\boarhide.dbr", 0);
        charm.relic_bonus = Some(bonus.clone());
        let mut helm = shard(r"records\item\equipmenthelm\bronzehelm.dbr", 0);
        socket_relic(&mut helm, charm);
        assert_eq!(
            helm.relic.as_ref().map(crate::chr::RecordId::as_str),
            Some(r"records\item\animalrelics\boarhide.dbr")
        );
        // The zero-encoded single shard sockets as one shard.
        assert_eq!(helm.var1, 1);
        assert_eq!(helm.relic_bonus, Some(bonus));
    }

    #[test]
    fn extract_keeps_gear_and_returns_the_piece_with_its_count_and_bonus() {
        let db = charm_db();
        let bonus = RecordId::parse(r"records\item\bonus.dbr".to_string()).unwrap();
        let mut gear = shard(r"records\item\equipmentweapon\sword.dbr", 0);
        gear.relic =
            Some(RecordId::parse(r"records\item\animalrelics\boarhide.dbr".to_string()).unwrap());
        gear.relic_bonus = Some(bonus.clone());
        gear.var1 = 2;
        assert_eq!(socketed_slot(&gear), Some(RelicSlot::First));

        let piece = extract_relic(Some(&db), &mut gear, RelicSlot::First).unwrap();
        assert_eq!(
            piece.base.as_str(),
            r"records\item\animalrelics\boarhide.dbr"
        );
        assert_eq!(piece.var1, 2);
        assert_eq!(piece.relic_bonus, Some(bonus));
        assert_eq!(gear.relic, None);
        assert_eq!(gear.relic_bonus, None);
        assert_eq!(gear.var1, 0);
        assert_eq!(socketed_slot(&gear), None);
    }

    #[test]
    fn extract_second_socket_clamps_the_completed_sentinel() {
        let db = charm_db();
        let mut gear = shard(r"records\item\equipmentweapon\sword.dbr", 0);
        gear.atlantis = Some(crate::chr::AtlantisRelic {
            relic: RecordId::parse(r"records\item\animalrelics\boarhide.dbr".to_string()),
            bonus: None,
            var2: crate::vault::VAR2_DEFAULT,
        });
        assert_eq!(socketed_slot(&gear), Some(RelicSlot::Second));

        let piece = extract_relic(Some(&db), &mut gear, RelicSlot::Second).unwrap();
        // The sentinel means "completed", not a count of two million.
        assert_eq!(piece.var1, 3);
        assert!(gear.atlantis.is_none());
    }

    #[test]
    fn extract_from_an_empty_socket_changes_nothing() {
        let mut gear = shard(r"records\item\equipmentweapon\sword.dbr", 0);
        let before = gear.clone();
        assert_eq!(
            extract_relic(None, &mut gear, RelicSlot::First),
            Err(ExtractError::NoRelic)
        );
        assert_eq!(
            extract_relic(None, &mut gear, RelicSlot::Second),
            Err(ExtractError::NoRelic)
        );
        assert_eq!(gear, before);
    }

    #[test]
    fn can_combine_accepts_single_shards_and_rejects_completed_pieces() {
        let db = charm_db();
        let single = shard(r"records\item\animalrelics\boarhide.dbr", 0);
        let partial = shard(r"records\item\animalrelics\boarhide.dbr", 2);
        let completed = shard(r"records\item\animalrelics\boarhide.dbr", 3);
        assert!(can_combine(Some(&db), &single, &partial));
        assert!(can_combine(Some(&db), &partial, &single));
        // A completed piece is a finished item: never a source and
        // never a target.
        assert!(!can_combine(Some(&db), &completed, &partial));
        assert!(!can_combine(Some(&db), &partial, &completed));
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
    fn can_equip_maps_families_to_slots() {
        let db = charm_db();
        let helm = item(r"records\item\equipmenthelm\bronzehelm.dbr");
        let sword = item(r"records\item\equipmentweapon\gladius.dbr");
        let ring = item(r"records\item\equipmentring\loop.dbr");
        let shield = item(r"records\item\equipmentshield\tower.dbr");
        let artifact = item(r"records\xpack\item\artifacts\fool.dbr");
        let charm = shard(r"records\item\animalrelics\boarhide.dbr", 2);

        assert!(can_equip(Some(&db), &helm, EquipSlot::Head));
        assert!(!can_equip(Some(&db), &helm, EquipSlot::Torso));
        assert!(!can_equip(Some(&db), &helm, EquipSlot::Artifact));
        for hand in [
            EquipSlot::LeftHand,
            EquipSlot::RightHand,
            EquipSlot::LeftHandAlternate,
            EquipSlot::RightHandAlternate,
        ] {
            assert!(can_equip(Some(&db), &sword, hand));
            assert!(can_equip(Some(&db), &shield, hand));
        }
        assert!(!can_equip(Some(&db), &sword, EquipSlot::Head));
        assert!(can_equip(Some(&db), &ring, EquipSlot::Ring1));
        assert!(can_equip(Some(&db), &ring, EquipSlot::Ring2));
        assert!(!can_equip(Some(&db), &ring, EquipSlot::Neck));
        assert!(can_equip(Some(&db), &artifact, EquipSlot::Artifact));
        assert!(!can_equip(Some(&db), &artifact, EquipSlot::RightHand));
        assert!(!can_equip(Some(&db), &charm, EquipSlot::Head));
        assert!(!can_equip(None, &helm, EquipSlot::Head));

        let mut stack = sword;
        stack.stack_size = 2;
        assert!(!can_equip(Some(&db), &stack, EquipSlot::RightHand));
    }

    #[test]
    fn equip_takes_the_empty_slot_and_refuses_an_occupied_one() {
        let bytes = player_bytes();
        let mut character = parse_player(&bytes).unwrap();
        let mut sword = item(r"records\item\equipmentweapon\gladius.dbr");
        sword.position = GridPos { x: 4, y: 1 };

        equip(&mut character, sword.clone(), EquipSlot::RightHand).unwrap();
        let worn = character.equipment.get(EquipSlot::RightHand).unwrap();
        assert_eq!(worn.position, GridPos { x: 0, y: 0 });

        let rejected = equip(&mut character, sword, EquipSlot::RightHand).unwrap_err();
        assert_eq!(rejected.reason, TransferError::Occupied);

        let taken = take_equipped(&mut character, EquipSlot::RightHand).unwrap();
        assert_eq!(taken.base.file_stem(), "gladius");
        assert!(character.equipment.get(EquipSlot::RightHand).is_none());
        assert!(take_equipped(&mut character, EquipSlot::RightHand).is_none());
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
