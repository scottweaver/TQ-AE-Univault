//! The unified vault store — this app's authoritative container for
//! vaulted items (ARCHITECTURE.md "Source of truth"): a flat set of
//! identified items in one versioned, self-describing JSON file.
//! Type buckets are computed views ([`bucket_of`]), never stored, so
//! an item cannot be misfiled — there is no stored membership to get
//! wrong. `TQVaultAE` vaults are an interchange format:
//! [`VaultStore::import_vault`] pulls one in (provenance recorded via
//! [`VaultStore::record_import`]), [`export_to_vault`] packs items
//! back out into a `TQVaultAE`-readable vault.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::cache::GameCache;
use crate::chr::{AtlantisRelic, GridPos, Item, ItemSeed, RecordId};
use crate::query::{self, ItemCategory};
use crate::style::GearSlot;
use crate::transfer;
use crate::vault::{Vault, VaultItem, VaultSack};

const FORMAT: &str = "univault-store";
const VERSION: u32 = 1;

/// Identity of one stored item, unique for the store's lifetime —
/// ids are never reused, so a stale id can only miss, never alias a
/// different item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoredItemId(u64);

/// One stored item and its identity.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEntry {
    id: StoredItemId,
    pub item: Item,
    extra: Map<String, Value>,
}

impl StoredEntry {
    #[must_use]
    pub fn id(&self) -> StoredItemId {
        self.id
    }
}

/// Provenance of one vault-file import. Matching is by `source` file
/// name — once a vault file has been migrated it stays migrated even
/// if the file changes later; re-importing is an explicit user act
/// that simply adds again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub source: String,
    pub size: u64,
    pub mtime: i64,
    pub count: usize,
}

/// The unified store: every vaulted item, flat and identified.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VaultStore {
    next_id: u64,
    entries: Vec<StoredEntry>,
    imports: Vec<ImportRecord>,
    extra: Map<String, Value>,
}

/// Errors from reading or writing a store file.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid store JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a univault store (format {found:?})")]
    WrongFormat { found: String },
    #[error("store version {found} is newer than this build understands (max {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("item {index} has an empty baseName")]
    EmptyBaseName { index: usize },
    #[error("duplicate stored-item id {id}")]
    DuplicateId { id: u64 },
}

impl VaultStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores an item, assigning the next id. Grid position and
    /// chr-encoding bookkeeping are container facts, not item facts —
    /// both are normalized away so the in-memory entry equals its
    /// serialized form.
    pub fn add(&mut self, item: Item) -> StoredItemId {
        let id = StoredItemId(self.next_id);
        self.next_id += 1;
        self.entries.push(StoredEntry {
            id,
            item: Item {
                position: GridPos { x: 0, y: 0 },
                folded_members: Vec::new(),
                ..item
            },
            extra: Map::new(),
        });
        id
    }

    /// Stores every item, returning how many were added.
    pub fn add_all(&mut self, items: impl IntoIterator<Item = Item>) -> usize {
        items.into_iter().map(|item| self.add(item)).count()
    }

    /// Removes and returns the item. `None` on an unknown or already
    /// removed id.
    pub fn remove(&mut self, id: StoredItemId) -> Option<Item> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        Some(self.entries.remove(index).item)
    }

    #[must_use]
    pub fn get(&self, id: StoredItemId) -> Option<&Item> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.item)
    }

    pub fn get_mut(&mut self, id: StoredItemId) -> Option<&mut Item> {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .map(|entry| &mut entry.item)
    }

    pub fn entries(&self) -> impl Iterator<Item = &StoredEntry> {
        self.entries.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds every item of a `TQVaultAE` vault (all sacks, flattened),
    /// returning how many came in. Provenance is the caller's to
    /// record via [`Self::record_import`] — this function never
    /// touches the import ledger.
    pub fn import_vault(&mut self, vault: &Vault) -> usize {
        let items: Vec<Item> = vault
            .sacks
            .iter()
            .flat_map(|sack| &sack.items)
            .map(|entry| entry.item.clone())
            .collect();
        self.add_all(items)
    }

    pub fn record_import(&mut self, record: ImportRecord) {
        self.imports.push(record);
    }

    #[must_use]
    pub fn is_imported(&self, source: &str) -> bool {
        self.imports.iter().any(|record| record.source == source)
    }

    #[must_use]
    pub fn imports(&self) -> &[ImportRecord] {
        &self.imports
    }

    /// Parses a store file.
    ///
    /// # Errors
    /// [`StoreError::Json`] on malformed JSON;
    /// [`StoreError::WrongFormat`] / [`StoreError::UnsupportedVersion`]
    /// when the file is not a store this build may edit; the
    /// item-level validation errors otherwise.
    pub fn from_json(json: &str) -> Result<Self, StoreError> {
        let dto: StoreDto = serde_json::from_str(json)?;
        if dto.format != FORMAT {
            return Err(StoreError::WrongFormat { found: dto.format });
        }
        if dto.version > VERSION {
            return Err(StoreError::UnsupportedVersion {
                found: dto.version,
                supported: VERSION,
            });
        }
        let entries = dto
            .items
            .into_iter()
            .enumerate()
            .map(|(index, dto)| entry_from_dto(dto, index))
            .collect::<Result<Vec<_>, _>>()?;
        let mut seen = HashSet::new();
        for entry in &entries {
            if !seen.insert(entry.id.0) {
                return Err(StoreError::DuplicateId { id: entry.id.0 });
            }
        }
        let highest = entries.iter().map(|entry| entry.id.0).max();
        Ok(Self {
            next_id: highest.map_or(dto.next_id, |highest| dto.next_id.max(highest + 1)),
            entries,
            imports: dto.imports,
            extra: dto.extra,
        })
    }

    /// Serializes to the indented store JSON form.
    ///
    /// # Errors
    /// [`StoreError::Json`] only on serializer failure, which these
    /// types do not normally produce.
    pub fn to_json(&self) -> Result<String, StoreError> {
        let dto = StoreDto {
            format: FORMAT.to_string(),
            version: VERSION,
            next_id: self.next_id,
            items: self.entries.iter().map(entry_to_dto).collect(),
            imports: self.imports.clone(),
            extra: self.extra.clone(),
        };
        Ok(serde_json::to_string_pretty(&dto)?)
    }
}

/// Packs items into a fresh `TQVaultAE`-readable vault: first-fit at
/// conservative footprints, growing a new sack whenever the existing
/// ones are full. An item too big for even an empty sack (degenerate
/// data) is pinned at that sack's origin rather than lost.
#[must_use]
pub fn export_to_vault(items: impl IntoIterator<Item = Item>, db: Option<&GameCache>) -> Vault {
    let mut vault = Vault::new(1);
    for item in items {
        let mut pending = item;
        loop {
            let fresh = vault.sacks.last().is_some_and(|sack| sack.items.is_empty());
            match transfer::place_in_vault(&mut vault, pending, 0, db) {
                Ok(_) => break,
                Err(rejected) => {
                    let mut next = VaultSack::new();
                    if fresh {
                        let mut oversized = *rejected.item;
                        oversized.position = GridPos { x: 0, y: 0 };
                        next.items.push(VaultItem::new(oversized, 0, 0));
                        vault.sacks.push(next);
                        break;
                    }
                    vault.sacks.push(next);
                    pending = *rejected.item;
                }
            }
        }
    }
    vault
}

/// The computed bucket an item files into: its [`ItemCategory`], or
/// [`Bucket::Unknown`] when no cache is loaded or the record isn't in
/// it (a mod item after the mod is gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Bucket {
    Category(ItemCategory),
    Unknown,
}

#[must_use]
pub fn bucket_of(db: Option<&GameCache>, item: &Item) -> Bucket {
    db.and_then(|db| query::item_category(db, item))
        .map_or(Bucket::Unknown, Bucket::Category)
}

/// What a bulk send's duplicate filter matches on: an item's roll
/// seed within its own type bucket. The seed is what survives a copy
/// — the app's own duplicate, a sack sent twice — while the bucket
/// keeps two unrelated types that happened to roll the same number
/// from being read as the same item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemIdentity {
    bucket: Bucket,
    seed: ItemSeed,
}

impl ItemIdentity {
    #[must_use]
    pub fn of(db: Option<&GameCache>, item: &Item) -> Self {
        Self {
            bucket: bucket_of(db, item),
            seed: item.seed,
        }
    }
}

/// A bulk send's running duplicate filter: an item is admitted only
/// if its [`ItemIdentity`] is neither already in the store nor
/// already admitted earlier in the same batch — so one send can
/// neither re-add what is stored nor duplicate within itself.
///
/// Absent this guard a bulk send admits everything; the filter is a
/// property of the operation, never of the store, which is free to
/// hold duplicates that arrived by other routes.
#[derive(Debug, Clone)]
pub struct DuplicateGuard {
    seen: HashSet<ItemIdentity>,
    skipped: usize,
}

impl DuplicateGuard {
    #[must_use]
    pub fn over(store: &VaultStore, db: Option<&GameCache>) -> Self {
        Self {
            seen: store
                .entries()
                .map(|entry| ItemIdentity::of(db, &entry.item))
                .collect(),
            skipped: 0,
        }
    }

    /// Whether this item should be sent, recording it as seen when it
    /// is and counting it as skipped when it is not.
    pub fn admit(&mut self, db: Option<&GameCache>, item: &Item) -> bool {
        let admitted = self.seen.insert(ItemIdentity::of(db, item));
        if !admitted {
            self.skipped += 1;
        }
        admitted
    }

    #[must_use]
    pub fn skipped(&self) -> usize {
        self.skipped
    }
}

impl Bucket {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Category(category) => category.label(),
            Self::Unknown => "Unknown",
        }
    }

    #[must_use]
    pub fn family(self) -> Family {
        match self {
            Self::Unknown => Family::Misc,
            Self::Category(category) => match category {
                ItemCategory::Gear(slot) => match slot {
                    GearSlot::Head
                    | GearSlot::UpperBody
                    | GearSlot::Forearm
                    | GearSlot::LowerBody => Family::Armor,
                    GearSlot::Amulet | GearSlot::Ring | GearSlot::Bracelet => Family::Jewelry,
                    GearSlot::Shield
                    | GearSlot::Sword
                    | GearSlot::Axe
                    | GearSlot::Mace
                    | GearSlot::Spear
                    | GearSlot::Bow
                    | GearSlot::Thrown
                    | GearSlot::Staff => Family::Weapons,
                },
                ItemCategory::Relic | ItemCategory::Charm => Family::RelicsCharms,
                ItemCategory::Artifact | ItemCategory::Formula | ItemCategory::Scroll => {
                    Family::Artifacts
                }
                ItemCategory::Potion | ItemCategory::Quest => Family::Misc,
            },
        }
    }
}

/// Top-level grouping of buckets — the vault pane's family tabs.
/// Every bucket belongs to exactly one family (guarded by test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Armor,
    Jewelry,
    Weapons,
    RelicsCharms,
    Artifacts,
    Misc,
}

impl Family {
    pub const ALL: [Self; 6] = [
        Self::Armor,
        Self::Jewelry,
        Self::Weapons,
        Self::RelicsCharms,
        Self::Artifacts,
        Self::Misc,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Armor => "Armor",
            Self::Jewelry => "Jewelry",
            Self::Weapons => "Weapons",
            Self::RelicsCharms => "Relics & Charms",
            Self::Artifacts => "Artifacts",
            Self::Misc => "Misc",
        }
    }

    /// The family's buckets, in display order.
    #[must_use]
    pub fn buckets(self) -> &'static [Bucket] {
        const fn gear(slot: GearSlot) -> Bucket {
            Bucket::Category(ItemCategory::Gear(slot))
        }
        const ARMOR: [Bucket; 4] = [
            gear(GearSlot::Head),
            gear(GearSlot::UpperBody),
            gear(GearSlot::Forearm),
            gear(GearSlot::LowerBody),
        ];
        const JEWELRY: [Bucket; 3] = [
            gear(GearSlot::Amulet),
            gear(GearSlot::Ring),
            gear(GearSlot::Bracelet),
        ];
        const WEAPONS: [Bucket; 8] = [
            gear(GearSlot::Sword),
            gear(GearSlot::Axe),
            gear(GearSlot::Mace),
            gear(GearSlot::Spear),
            gear(GearSlot::Bow),
            gear(GearSlot::Thrown),
            gear(GearSlot::Staff),
            gear(GearSlot::Shield),
        ];
        const RELICS_CHARMS: [Bucket; 2] = [
            Bucket::Category(ItemCategory::Relic),
            Bucket::Category(ItemCategory::Charm),
        ];
        const ARTIFACTS: [Bucket; 3] = [
            Bucket::Category(ItemCategory::Artifact),
            Bucket::Category(ItemCategory::Formula),
            Bucket::Category(ItemCategory::Scroll),
        ];
        const MISC: [Bucket; 3] = [
            Bucket::Category(ItemCategory::Potion),
            Bucket::Category(ItemCategory::Quest),
            Bucket::Unknown,
        ];
        match self {
            Self::Armor => &ARMOR,
            Self::Jewelry => &JEWELRY,
            Self::Weapons => &WEAPONS,
            Self::RelicsCharms => &RELICS_CHARMS,
            Self::Artifacts => &ARTIFACTS,
            Self::Misc => &MISC,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoreDto {
    format: String,
    version: u32,
    #[serde(rename = "nextId")]
    next_id: u64,
    #[serde(default)]
    items: Vec<EntryDto>,
    #[serde(default)]
    imports: Vec<ImportRecord>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct EntryDto {
    id: u64,
    #[serde(rename = "baseName")]
    base_name: String,
    #[serde(
        rename = "prefixName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    prefix_name: Option<String>,
    #[serde(
        rename = "suffixName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    suffix_name: Option<String>,
    #[serde(rename = "relicName", default, skip_serializing_if = "Option::is_none")]
    relic_name: Option<String>,
    #[serde(
        rename = "relicBonus",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    relic_bonus: Option<String>,
    seed: i32,
    var1: i32,
    #[serde(rename = "stackSize")]
    stack_size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    atlantis: Option<AtlantisDto>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct AtlantisDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bonus: Option<String>,
    var2: i32,
}

fn entry_to_dto(entry: &StoredEntry) -> EntryDto {
    let item = &entry.item;
    let name = |id: Option<&RecordId>| id.map(|id| id.as_str().to_string());
    EntryDto {
        id: entry.id.0,
        base_name: item.base.as_str().to_string(),
        prefix_name: name(item.prefix.as_ref()),
        suffix_name: name(item.suffix.as_ref()),
        relic_name: name(item.relic.as_ref()),
        relic_bonus: name(item.relic_bonus.as_ref()),
        seed: item.seed.value(),
        var1: item.var1,
        stack_size: item.stack_size,
        atlantis: item.atlantis.as_ref().map(|second| AtlantisDto {
            relic: name(second.relic.as_ref()),
            bonus: name(second.bonus.as_ref()),
            var2: second.var2,
        }),
        extra: entry.extra.clone(),
    }
}

fn entry_from_dto(dto: EntryDto, index: usize) -> Result<StoredEntry, StoreError> {
    let base = RecordId::parse(dto.base_name).ok_or(StoreError::EmptyBaseName { index })?;
    let record = |raw: Option<String>| raw.and_then(RecordId::parse);
    Ok(StoredEntry {
        id: StoredItemId(dto.id),
        item: Item {
            base,
            prefix: record(dto.prefix_name),
            suffix: record(dto.suffix_name),
            relic: record(dto.relic_name),
            relic_bonus: record(dto.relic_bonus),
            seed: ItemSeed::new(dto.seed),
            var1: dto.var1,
            atlantis: dto.atlantis.map(|second| AtlantisRelic {
                relic: record(second.relic),
                bonus: record(second.bonus),
                var2: second.var2,
            }),
            position: GridPos { x: 0, y: 0 },
            stack_size: dto.stack_size.max(1),
            folded_members: Vec::new(),
        },
        extra: dto.extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arz::ArzFile;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::gamedata::GameData;
    use crate::text::TextDb;

    fn item(base: &str) -> Item {
        Item::bare(RecordId::parse(base.to_string()).unwrap(), ItemSeed::new(7))
    }

    fn db() -> GameCache {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\equipmenthelm\\bronzehelm.dbr",
            "ArmorProtective_Head",
            &[],
        );
        builder.record(
            "records\\item\\equipmentweapon\\gladius.dbr",
            "WeaponMelee_Sword",
            &[],
        );
        builder.record(
            "records\\item\\animalrelics\\boarhide.dbr",
            "ItemCharm",
            &[("completedRelicLevel", Values::Ints(&[3]))],
        );
        let data = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), TextDb::new());
        data.build_cache(Vec::new())
    }

    #[test]
    fn add_assigns_fresh_ids_and_normalizes_container_facts() {
        let mut store = VaultStore::new();
        let mut placed = item("records\\a.dbr");
        placed.position = GridPos { x: 7, y: 3 };
        let first = store.add(placed);
        let second = store.add(item("records\\b.dbr"));
        assert_ne!(first, second);
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(first).unwrap().position, GridPos { x: 0, y: 0 });
    }

    #[test]
    fn the_guard_skips_seeds_already_stored_and_repeats_within_one_batch() {
        let db = db();
        let db = Some(&db);
        let seeded = |base: &str, seed: i32| {
            Item::bare(
                RecordId::parse(base.to_string()).unwrap(),
                ItemSeed::new(seed),
            )
        };
        let helm = "records\\item\\equipmenthelm\\bronzehelm.dbr";
        let sword = "records\\item\\equipmentweapon\\gladius.dbr";
        let mut store = VaultStore::new();
        store.add(seeded(helm, 41823));

        let mut guard = DuplicateGuard::over(&store, db);
        assert!(!guard.admit(db, &seeded(helm, 41823)));
        assert!(guard.admit(db, &seeded(helm, 90210)));
        // Admitting once claims the identity for the rest of the batch.
        assert!(!guard.admit(db, &seeded(helm, 90210)));
        // Same seed, different type bucket — a coincidence, not a copy.
        assert!(guard.admit(db, &seeded(sword, 41823)));
        assert_eq!(guard.skipped(), 2);
    }

    #[test]
    fn a_guard_over_an_empty_store_admits_everything_once() {
        let db = db();
        let db = Some(&db);
        let mut guard = DuplicateGuard::over(&VaultStore::new(), db);
        assert!(guard.admit(db, &item("records\\item\\equipmenthelm\\bronzehelm.dbr")));
        assert!(!guard.admit(db, &item("records\\item\\equipmenthelm\\bronzehelm.dbr")));
        assert_eq!(guard.skipped(), 1);
    }

    #[test]
    fn remove_returns_the_item_and_never_reuses_its_id() {
        let mut store = VaultStore::new();
        let id = store.add(item("records\\a.dbr"));
        let removed = store.remove(id).unwrap();
        assert_eq!(removed.base.file_stem(), "a");
        assert!(store.get(id).is_none());
        assert!(store.remove(id).is_none());
        let next = store.add(item("records\\b.dbr"));
        assert_ne!(next, id);
    }

    #[test]
    fn json_round_trip_preserves_everything() {
        let mut store = VaultStore::new();
        let mut fancy = item("records\\item\\equipmentweapon\\gladius.dbr");
        fancy.prefix = RecordId::parse("records\\prefix.dbr".to_string());
        fancy.relic = RecordId::parse("records\\relic.dbr".to_string());
        fancy.var1 = 3;
        fancy.atlantis = Some(AtlantisRelic {
            relic: RecordId::parse("records\\second.dbr".to_string()),
            bonus: None,
            var2: 9,
        });
        fancy.stack_size = 4;
        store.add(fancy);
        store.add(item("records\\plain.dbr"));
        store.record_import(ImportRecord {
            source: "Old.vault.json".to_string(),
            size: 123,
            mtime: 456,
            count: 2,
        });

        let reread = VaultStore::from_json(&store.to_json().unwrap()).unwrap();
        assert_eq!(reread, store);
        assert!(reread.is_imported("Old.vault.json"));
        assert!(!reread.is_imported("Other.vault.json"));
    }

    #[test]
    fn reread_store_keeps_assigning_fresh_ids() {
        let mut store = VaultStore::new();
        let first = store.add(item("records\\a.dbr"));
        let mut reread = VaultStore::from_json(&store.to_json().unwrap()).unwrap();
        let second = reread.add(item("records\\b.dbr"));
        assert_ne!(first, second);
    }

    #[test]
    fn foreign_json_is_refused_by_format_and_version() {
        assert!(matches!(
            VaultStore::from_json(r#"{"format":"tqvault","version":1,"nextId":0}"#),
            Err(StoreError::WrongFormat { .. })
        ));
        assert!(matches!(
            VaultStore::from_json(r#"{"format":"univault-store","version":99,"nextId":0}"#),
            Err(StoreError::UnsupportedVersion {
                found: 99,
                supported: VERSION,
            })
        ));
        // A TQVaultAE vault has neither field: a plain Json error.
        assert!(matches!(
            VaultStore::from_json(r#"{"sacks":[]}"#),
            Err(StoreError::Json(_))
        ));
    }

    #[test]
    fn corrupt_ids_are_detected_and_a_stale_next_id_heals() {
        let duplicated = r#"{"format":"univault-store","version":1,"nextId":9,
            "items":[{"id":1,"baseName":"records\\a.dbr","seed":1,"var1":0,"stackSize":1},
                     {"id":1,"baseName":"records\\b.dbr","seed":1,"var1":0,"stackSize":1}]}"#;
        assert!(matches!(
            VaultStore::from_json(duplicated),
            Err(StoreError::DuplicateId { id: 1 })
        ));

        let stale = r#"{"format":"univault-store","version":1,"nextId":0,
            "items":[{"id":5,"baseName":"records\\a.dbr","seed":1,"var1":0,"stackSize":1}]}"#;
        let mut store = VaultStore::from_json(stale).unwrap();
        let fresh = store.add(item("records\\b.dbr"));
        assert_eq!(store.get(fresh).unwrap().base.file_stem(), "b");
        assert_eq!(store.len(), 2);
        let ids: Vec<StoredItemId> = store.entries().map(StoredEntry::id).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn empty_base_name_is_rejected_with_location() {
        let json = r#"{"format":"univault-store","version":1,"nextId":2,
            "items":[{"id":0,"baseName":"records\\a.dbr","seed":1,"var1":0,"stackSize":1},
                     {"id":1,"baseName":" ","seed":1,"var1":0,"stackSize":1}]}"#;
        assert!(matches!(
            VaultStore::from_json(json),
            Err(StoreError::EmptyBaseName { index: 1 })
        ));
    }

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let json = r#"{"format":"univault-store","version":1,"nextId":1,"futureTop":"kept",
            "items":[{"id":0,"baseName":"records\\a.dbr","seed":1,"var1":0,"stackSize":1,"futureItem":7}]}"#;
        let store = VaultStore::from_json(json).unwrap();
        let value: Value = serde_json::from_str(&store.to_json().unwrap()).unwrap();
        assert_eq!(value["futureTop"], "kept");
        assert_eq!(value["items"][0]["futureItem"], 7);
    }

    #[test]
    fn import_vault_flattens_every_sack() {
        let vault = Vault::from_json(
            r#"{"sacks":[
                {"items":[{"baseName":"records\\a.dbr","seed":1,"stackSize":1,"pointX":3,"pointY":4}]},
                {"items":[{"baseName":"records\\b.dbr","seed":2,"stackSize":1},
                          {"baseName":"records\\c.dbr","seed":3,"stackSize":1}]}
            ]}"#,
        )
        .unwrap();
        let mut store = VaultStore::new();
        assert_eq!(store.import_vault(&vault), 3);
        assert_eq!(store.len(), 3);
        let entry = store.entries().next().unwrap();
        assert_eq!(entry.item.position, GridPos { x: 0, y: 0 });
    }

    #[test]
    fn export_packs_first_fit_and_grows_sacks() {
        // Fallback footprint is 2×5 in an 18×20 sack: 36 per sack.
        let items = (0..40).map(|index| item(&format!("records\\i{index}.dbr")));
        let vault = export_to_vault(items, None);
        assert_eq!(vault.sacks.len(), 2);
        assert_eq!(vault.sacks[0].items.len(), 36);
        assert_eq!(vault.sacks[1].items.len(), 4);
        let reread = Vault::from_json(&vault.to_json().unwrap()).unwrap();
        let positions: HashSet<(i32, i32)> = reread.sacks[0]
            .items
            .iter()
            .map(|entry| (entry.item.position.x, entry.item.position.y))
            .collect();
        assert_eq!(positions.len(), 36);
    }

    #[test]
    fn export_of_nothing_is_an_empty_vault() {
        let vault = export_to_vault(std::iter::empty(), None);
        assert_eq!(vault.sacks.len(), 1);
        assert!(vault.sacks[0].items.is_empty());
    }

    #[test]
    fn buckets_classify_through_the_cache_and_fall_back_to_unknown() {
        let db = db();
        let helm = item("records\\item\\equipmenthelm\\bronzehelm.dbr");
        let sword = item("records\\item\\equipmentweapon\\gladius.dbr");
        let charm = item("records\\item\\animalrelics\\boarhide.dbr");
        let alien = item("records\\mod\\gone.dbr");
        assert_eq!(
            bucket_of(Some(&db), &helm),
            Bucket::Category(ItemCategory::Gear(GearSlot::Head))
        );
        assert_eq!(
            bucket_of(Some(&db), &sword),
            Bucket::Category(ItemCategory::Gear(GearSlot::Sword))
        );
        assert_eq!(
            bucket_of(Some(&db), &charm),
            Bucket::Category(ItemCategory::Charm)
        );
        assert_eq!(bucket_of(Some(&db), &alien), Bucket::Unknown);
        assert_eq!(bucket_of(None, &helm), Bucket::Unknown);
    }

    #[test]
    fn every_bucket_belongs_to_exactly_one_family() {
        let listed: Vec<Bucket> = Family::ALL
            .iter()
            .flat_map(|family| family.buckets().iter().copied())
            .collect();
        assert_eq!(listed.len(), ItemCategory::ALL.len() + 1);
        for category in ItemCategory::ALL {
            let bucket = Bucket::Category(category);
            assert_eq!(
                listed.iter().filter(|other| **other == bucket).count(),
                1,
                "{bucket:?} must appear exactly once"
            );
        }
        assert!(listed.contains(&Bucket::Unknown));
        for family in Family::ALL {
            for bucket in family.buckets() {
                assert_eq!(bucket.family(), family, "{bucket:?} lists under {family:?}");
            }
        }
    }
}
