//! The combined game database: ARZ records plus localized text, and
//! the name-resolution rules that turn a record path into a display
//! name. The per-type variable dispatch is ported from `TQVaultAE`'s
//! `Info.AssignVariableNames` (MIT).

use crate::arc::{ArcError, ArcFile};
use crate::arz::{ArzError, ArzFile, DbRecord, normalize};
use crate::chr::{Item, RecordId};
use crate::tex;
use crate::text::TextDb;

/// Parsed game reference data: `database.arz`, the localization table
/// from a `Text_XX.arc`, and any number of `Items.arc` bitmap
/// archives (base game + expansions) for real item footprints.
pub struct GameData {
    arz: ArzFile,
    text: TextDb,
    /// `(expansion label, archive)` — label `""` for the base game,
    /// `"XPACK"`…`"XPACK4"` for expansions, matching resource-id
    /// prefixes.
    item_archives: Vec<(String, ArcFile)>,
}

/// Errors from assembling the game database.
#[derive(Debug, thiserror::Error)]
pub enum GameDataError {
    #[error("database.arz: {0}")]
    Arz(#[from] ArzError),
    #[error("text archive: {0}")]
    Arc(#[from] ArcError),
}

impl GameData {
    /// Assembles the database from the two files' raw bytes (reading
    /// them from disk is the shell's job — core stays IO-free).
    ///
    /// # Errors
    /// Structural errors from either file.
    pub fn from_bytes(database: Vec<u8>, text_archive: Vec<u8>) -> Result<Self, GameDataError> {
        let arz = ArzFile::parse(database)?;
        let arc = ArcFile::parse(text_archive)?;
        let text = load_text(&arc)?;
        Ok(Self::from_parts(arz, text))
    }

    #[must_use]
    pub fn from_parts(arz: ArzFile, text: TextDb) -> Self {
        Self {
            arz,
            text,
            item_archives: Vec::new(),
        }
    }

    /// Registers an `Items.arc` bitmap archive. `expansion` is the
    /// resource-id prefix it serves: `""` for the base game,
    /// `"XPACK"`…`"XPACK4"` for expansions.
    pub fn add_items_archive(&mut self, expansion: &str, archive: ArcFile) {
        self.item_archives.push((expansion.to_uppercase(), archive));
    }

    /// Extracts a resource by its record path (e.g.
    /// `XPack2\Items\...\foo.tex`) from the registered item archives.
    /// The archive named by the id is tried first, then all others —
    /// `TQVaultAE` does the same because records sometimes name the
    /// wrong home.
    #[must_use]
    pub fn resource(&self, id: &str) -> Option<Vec<u8>> {
        let normalized = normalize(id);
        let mut segments = normalized.splitn(2, '\\');
        let first = segments.next()?;
        let (label, rest) = if first.starts_with("XPACK") {
            (first, segments.next()?)
        } else {
            ("", normalized.as_str())
        };
        let (archive_name, entry) = rest.split_once('\\')?;
        if archive_name != "ITEMS" {
            return None;
        }
        let preferred = self
            .item_archives
            .iter()
            .filter(|(archive_label, _)| archive_label == label)
            .chain(
                self.item_archives
                    .iter()
                    .filter(|(archive_label, _)| archive_label != label),
            );
        for (_, archive) in preferred {
            if let Some(Ok(bytes)) = archive.file(entry) {
                return Some(bytes);
            }
        }
        None
    }

    /// See [`ArzFile::record`].
    #[must_use]
    pub fn record(&self, id: &RecordId) -> Option<Result<DbRecord, ArzError>> {
        self.arz.record(id)
    }

    /// Raw localization lookup for the display engine.
    pub(crate) fn tag_text(&self, tag: &str) -> Option<&str> {
        self.text.get(tag)
    }

    pub fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        self.arz.record_ids()
    }

    /// Localized name of a record (item base, affix, relic, …) via the
    /// per-type name variable. `None` when the record is unknown or
    /// corrupt, or its tag has no localization.
    #[must_use]
    pub fn record_name(&self, id: &RecordId) -> Option<String> {
        let record = self.arz.record(id)?.ok()?;
        self.display_name(&record)
    }

    /// The game's `default\` template records store literal display
    /// text where regular records store a localization tag
    /// (`description = "Buckler Ornate"`); a space marks it as text,
    /// since real tags never contain one.
    fn display_name(&self, record: &DbRecord) -> Option<String> {
        let tag = name_tag(record)?;
        match self.text.get(tag) {
            Some(text) => Some(text.to_string()),
            None if tag.contains(' ') => Some(tag.to_string()),
            None => None,
        }
    }

    /// Grid footprint (width, height) for an item: the real size from
    /// its bitmap texture when the item archives are loaded (pixels ÷
    /// 32-pixel cells, `TQVaultAE`'s definition), otherwise a
    /// conservative per-class upper bound that guarantees placements
    /// never overlap at true sizes.
    #[must_use]
    pub fn item_footprint(&self, item: &Item) -> (i32, i32) {
        let record = self.arz.record(&item.base).and_then(Result::ok);
        record
            .as_ref()
            .and_then(|record| self.texture_footprint(record))
            .unwrap_or_else(|| class_upper_bound(record.as_ref()))
    }

    /// Decoded icon bitmap for an item, when its texture exists and
    /// uses one of the game's uncompressed formats (see
    /// [`tex::decode`]).
    #[must_use]
    pub fn item_icon(&self, item: &Item) -> Option<tex::RgbaImage> {
        let record = self.arz.record(&item.base)?.ok()?;
        let bitmap = record
            .string(bitmap_variable(&record.record_type))
            .filter(|path| !path.is_empty())?;
        tex::decode(&self.resource(bitmap)?).ok()
    }

    fn texture_footprint(&self, record: &DbRecord) -> Option<(i32, i32)> {
        let bitmap = record
            .string(bitmap_variable(&record.record_type))
            .filter(|path| !path.is_empty())?;
        let bytes = self.resource(bitmap)?;
        let (width_px, height_px) = tex::dimensions(&bytes).ok()?;
        Some(tex::cells(width_px, height_px))
    }

    /// Distills every item-classed record into a [`GameCache`]: name,
    /// footprint, and icon resolved once, so launches can skip the
    /// game archives entirely. `stamps` identify the source files for
    /// freshness checks.
    #[must_use]
    pub fn build_cache(&self, stamps: Vec<crate::cache::SourceStamp>) -> crate::cache::GameCache {
        self.build_cache_with_progress(stamps, |_, _| {})
    }

    /// [`Self::build_cache`], reporting `(records scanned, total)` at
    /// coarse intervals so a shell can drive a progress display.
    pub fn build_cache_with_progress(
        &self,
        stamps: Vec<crate::cache::SourceStamp>,
        mut progress: impl FnMut(usize, usize),
    ) -> crate::cache::GameCache {
        let renderer = crate::stats::Renderer { data: self };
        let total = self.arz.record_ids().count();
        let mut entries = std::collections::HashMap::new();
        for (scanned, id) in self.arz.record_ids().enumerate() {
            if scanned % 512 == 0 {
                progress(scanned, total);
            }
            let Some(Ok(record)) = self.arz.record(id) else {
                continue;
            };
            if !is_item_class(&record.record_type) {
                continue;
            }
            let name = self.display_name(&record);
            let footprint = self
                .texture_footprint(&record)
                .unwrap_or_else(|| class_upper_bound(Some(&record)));
            let icon = record
                .string(bitmap_variable(&record.record_type))
                .filter(|path| !path.is_empty())
                .and_then(|bitmap| self.resource(bitmap))
                .and_then(|bytes| tex::decode(&bytes).ok())
                .as_ref()
                .and_then(crate::cache::compress_icon);
            entries.insert(
                normalize(id.as_str()),
                crate::cache::CacheEntry {
                    name,
                    footprint,
                    classification: crate::style::Classification::of(&record),
                    kind: crate::style::ItemKind::of(&record),
                    stats: Some(renderer.stat_block(&record)),
                    icon,
                },
            );
        }
        progress(total, total);
        crate::cache::GameCache::from_entries(
            stamps,
            crate::stats::capture_runtime_labels(self),
            entries,
        )
    }

    /// Display name for an item: localized prefix + base + suffix.
    /// Every part that fails to resolve falls back to its record file
    /// stem, so an item never renders empty.
    #[must_use]
    pub fn item_name(&self, item: &Item) -> String {
        let part = |id: &RecordId| {
            self.record_name(id)
                .unwrap_or_else(|| id.file_stem().to_string())
        };
        let parts = [
            item.prefix.as_ref().map(&part),
            Some(part(&item.base)),
            item.suffix.as_ref().map(&part),
        ];
        parts.into_iter().flatten().collect::<Vec<_>>().join(" ")
    }
}

/// Footprint used when the base record is unknown (or no game data is
/// loaded): the largest footprint any TQ item has.
pub const FALLBACK_FOOTPRINT: (i32, i32) = (2, 5);

/// The conservative per-class upper bounds used when no texture is
/// available.
fn class_upper_bound(record: Option<&DbRecord>) -> (i32, i32) {
    match record.map(|record| record.record_type.as_str()) {
        Some(class) if class.starts_with("Weapon") => (2, 5),
        Some(class) if class.starts_with("ArmorProtective") => (2, 3),
        Some(_) => (2, 2),
        None => FALLBACK_FOOTPRINT,
    }
}

/// Whether a record class belongs to the item domain the app cares
/// about (equipment, consumables, relics, artifacts, quest items,
/// affixes) — the cache only carries these.
fn is_item_class(record_type: &str) -> bool {
    [
        "Weapon",
        "Armor",
        "Item",
        "OneShot",
        "QuestItem",
        "LootRandomizer",
    ]
    .iter()
    .any(|prefix| {
        record_type
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })
}

/// Which variable holds a record's bitmap path — the bitmap column of
/// `TQVaultAE`'s `Info.AssignVariableNames` dispatch.
fn bitmap_variable(record_type: &str) -> &'static str {
    let starts = |prefix: &str| {
        record_type
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    if starts("ItemRelic") || starts("ItemCharm") {
        "relicBitmap"
    } else if starts("ItemArtifactFormula") {
        "artifactFormulaBitmapName"
    } else if starts("ItemArtifact") {
        "artifactBitmap"
    } else {
        "bitmap"
    }
}

/// Which variable holds a record's name tag — `TQVaultAE`'s
/// `Info.AssignVariableNames` dispatch, collapsed to the naming
/// column. When the dispatched variable is absent the other one is
/// tried (`TQVaultAE` shows nothing there; the leniency helps mods).
fn name_tag(record: &DbRecord) -> Option<&str> {
    let starts = |prefix: &str| {
        record
            .record_type
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    let uses_description = starts("LOOTRANDOMIZER")
        || starts("ItemRelic")
        || starts("ItemCharm")
        || starts("OneShot")
        || starts("QuestItem")
        || starts("ItemArtifact")
        || starts("ItemEquipment");
    let (primary, fallback) = if starts("LOOTRANDOMIZER") {
        ("lootRandomizerName", "description")
    } else if uses_description {
        ("description", "itemNameTag")
    } else {
        ("itemNameTag", "description")
    };
    let non_empty = |name: &'static str| record.string(name).filter(|tag| !tag.is_empty());
    non_empty(primary).or_else(|| non_empty(fallback))
}

/// Loads every `.txt` in the text archive in expansion order (base
/// game, then IT `x`, then `x2`–`x4`), so later expansions override
/// earlier tags — the order `TQVaultAE` loads them in.
fn load_text(arc: &ArcFile) -> Result<TextDb, GameDataError> {
    let mut names: Vec<&str> = arc
        .file_names()
        .filter(|name| name.to_lowercase().ends_with(".txt"))
        .collect();
    names.sort_by_key(|name| (expansion_rank(name), name.to_lowercase()));
    let mut text = TextDb::new();
    for name in names {
        if let Some(bytes) = arc.file(name) {
            text.add_file(&bytes?);
        }
    }
    Ok(text)
}

fn expansion_rank(name: &str) -> u8 {
    let stem = name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    match stem.as_bytes() {
        [b'x', b'4', ..] => 4,
        [b'x', b'3', ..] => 3,
        [b'x', b'2', ..] => 2,
        [b'x', ..] => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::chr::{GridPos, ItemSeed};
    use crate::style::{self, ItemStyle};

    fn record_id(raw: &str) -> RecordId {
        RecordId::parse(raw.to_string()).unwrap()
    }

    fn text_file(content: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    fn sample_db() -> GameData {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\sword.dbr",
            "WeaponMelee_Sword",
            &[
                ("itemNameTag", Values::Strings(&["tagSword"])),
                ("description", Values::Strings(&["tagWrong"])),
            ],
        );
        builder.record(
            "records\\item\\sharp.dbr",
            "LootRandomizer",
            &[("lootRandomizerName", Values::Strings(&["tagSharp"]))],
        );
        builder.record(
            "records\\item\\ofbears.dbr",
            "LootRandomizer",
            &[("lootRandomizerName", Values::Strings(&["tagOfBears"]))],
        );
        builder.record(
            "records\\item\\relic.dbr",
            "ItemRelic",
            &[("description", Values::Strings(&["tagRelic"]))],
        );
        builder.record(
            "records\\item\\oldbow.dbr",
            "WeaponHunting_Bow",
            &[("description", Values::Strings(&["tagBow"]))],
        );
        let arz = ArzFile::parse(builder.build()).unwrap();

        let mut text = TextDb::new();
        text.add_file(&text_file(
            "tagSword=Bronze Sword\ntagWrong=WRONG\ntagSharp=Sharp\n\
             tagOfBears=of Bears\ntagRelic=Ankh of Isis\ntagBow=Short Bow\n",
        ));
        GameData::from_parts(arz, text)
    }

    #[test]
    fn gear_prefers_item_name_tag_over_description() {
        let db = sample_db();
        assert_eq!(
            db.record_name(&record_id("records\\item\\sword.dbr")),
            Some("Bronze Sword".to_string())
        );
    }

    #[test]
    fn affixes_resolve_via_loot_randomizer_name() {
        let db = sample_db();
        assert_eq!(
            db.record_name(&record_id("records\\item\\sharp.dbr")),
            Some("Sharp".to_string())
        );
    }

    #[test]
    fn relics_resolve_via_description() {
        let db = sample_db();
        assert_eq!(
            db.record_name(&record_id("records\\item\\relic.dbr")),
            Some("Ankh of Isis".to_string())
        );
    }

    #[test]
    fn missing_primary_variable_falls_back_to_the_other() {
        let db = sample_db();
        assert_eq!(
            db.record_name(&record_id("records\\item\\oldbow.dbr")),
            Some("Short Bow".to_string())
        );
    }

    #[test]
    fn unknown_record_has_no_name() {
        let db = sample_db();
        assert_eq!(db.record_name(&record_id("records\\nothing.dbr")), None);
    }

    #[test]
    fn item_name_joins_affixes_and_falls_back_to_stems() {
        let db = sample_db();
        let item = Item {
            base: record_id("records\\item\\sword.dbr"),
            prefix: Some(record_id("records\\item\\sharp.dbr")),
            suffix: Some(record_id("records\\item\\unknownaffix.dbr")),
            relic: None,
            relic_bonus: None,
            seed: ItemSeed::new(1),
            var1: 0,
            atlantis: None,
            position: GridPos { x: 0, y: 0 },
            stack_size: 1,
            folded_members: Vec::new(),
        };
        assert_eq!(db.item_name(&item), "Sharp Bronze Sword unknownaffix");
    }

    fn bare_item(base: &str) -> Item {
        Item {
            base: record_id(base),
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
    fn footprint_comes_from_the_bitmap_texture() {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\axe.dbr",
            "WeaponMelee_Axe",
            &[("bitmap", Values::Strings(&["Items\\gear\\axe.tex"]))],
        );
        let mut db = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), TextDb::new());
        let archive = crate::arc::fixture::build_arc(&[(
            "gear\\axe.tex",
            crate::tex::fixture::tex(64, 128).as_slice(),
        )]);
        db.add_items_archive("", crate::arc::ArcFile::parse(archive).unwrap());
        assert_eq!(
            db.item_footprint(&bare_item("records\\item\\axe.dbr")),
            (2, 4)
        );
    }

    #[test]
    fn footprint_falls_back_to_class_bounds_without_texture() {
        let db = sample_db();
        assert_eq!(
            db.item_footprint(&bare_item("records\\item\\sword.dbr")),
            (2, 5)
        );
        assert_eq!(
            db.item_footprint(&bare_item("records\\item\\unknown.dbr")),
            FALLBACK_FOOTPRINT
        );
    }

    #[test]
    fn xpack_resources_fall_back_across_archives() {
        let mut db = GameData::from_parts(
            ArzFile::parse(ArzBuilder::default().build()).unwrap(),
            TextDb::new(),
        );
        let archive = crate::arc::fixture::build_arc(&[(
            "gear\\spear.tex",
            crate::tex::fixture::tex(32, 160).as_slice(),
        )]);
        db.add_items_archive("", crate::arc::ArcFile::parse(archive).unwrap());
        // The id claims XPack2, but only the base archive has it.
        let bytes = db.resource("XPack2\\Items\\gear\\spear.tex").unwrap();
        assert_eq!(crate::tex::dimensions(&bytes), Ok((32, 160)));
        assert!(db.resource("Items\\gear\\missing.tex").is_none());
        assert!(db.resource("Creatures\\gear\\spear.tex").is_none());
    }

    #[test]
    fn cache_round_trips_names_footprints_and_icons() {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\axe.dbr",
            "WeaponMelee_Axe",
            &[
                ("itemNameTag", Values::Strings(&["tagAxe"])),
                ("bitmap", Values::Strings(&["Items\\gear\\axe.tex"])),
            ],
        );
        builder.record(
            "records\\item\\sharp.dbr",
            "LootRandomizer",
            &[
                ("lootRandomizerName", Values::Strings(&["tagSharp"])),
                ("itemClassification", Values::Strings(&["Rare"])),
            ],
        );
        builder.record(
            "records\\item\\boarhide.dbr",
            "ItemCharm",
            &[("completedRelicLevel", Values::Ints(&[10]))],
        );
        builder.record(
            "records\\creature\\monster.dbr",
            "Monster",
            &[("description", Values::Strings(&["tagAxe"]))],
        );
        let mut text = TextDb::new();
        text.add_file(&text_file("tagAxe=War Axe\ntagSharp=Sharp\n"));
        let mut db = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), text);
        let payload = [0xFF, 0x00, 0x00, 0xFF];
        let archive = crate::arc::fixture::build_arc(&[(
            "gear\\axe.tex",
            crate::tex::fixture::tex_with_pixels(32, 32, 32, &payload.repeat(32 * 32)).as_slice(),
        )]);
        db.add_items_archive("", crate::arc::ArcFile::parse(archive).unwrap());

        let stamps = vec![crate::cache::SourceStamp {
            path: "database.arz".to_string(),
            size: 42,
            mtime_seconds: 1_756_000_000,
        }];
        let cache = db.build_cache(stamps.clone());
        let reloaded = crate::cache::GameCache::from_bytes(&cache.to_bytes()).unwrap();

        assert_eq!(reloaded.stamps(), stamps.as_slice());
        // Item-classed records survive; the monster does not.
        assert_eq!(reloaded.len(), 3);
        assert_eq!(
            reloaded.record_name(&record_id("records\\item\\axe.dbr")),
            Some("War Axe".to_string())
        );
        assert_eq!(
            reloaded.record_name(&record_id("records\\item\\sharp.dbr")),
            Some("Sharp".to_string())
        );
        assert_eq!(
            reloaded.record_name(&record_id("records\\creature\\monster.dbr")),
            None
        );

        let axe = bare_item("records\\item\\axe.dbr");
        assert_eq!(reloaded.item_footprint(&axe), db.item_footprint(&axe));
        assert_eq!(reloaded.item_icon(&axe), db.item_icon(&axe));
        let unknown = bare_item("records\\item\\unknown.dbr");
        assert_eq!(reloaded.item_footprint(&unknown), FALLBACK_FOOTPRINT);

        // Classification and kind survive the byte round trip.
        let mut rare_axe = axe.clone();
        rare_axe.suffix = Some(record_id("records\\item\\sharp.dbr"));
        assert_eq!(
            style::item_style(Some(&reloaded), &rare_axe),
            ItemStyle::Rare
        );
        let mut charm = bare_item("records\\item\\boarhide.dbr");
        charm.var1 = 4;
        assert_eq!(
            style::relic_shards(Some(&reloaded), &charm),
            Some(style::RelicShards {
                have: 4,
                needed: Some(10)
            })
        );
    }

    #[test]
    fn previous_cache_versions_are_rejected() {
        let cache = GameData::from_parts(
            ArzFile::parse(ArzBuilder::default().build()).unwrap(),
            TextDb::new(),
        )
        .build_cache(Vec::new());
        let mut bytes = cache.to_bytes();
        bytes[..4].copy_from_slice(&0x3143_5655_i32.to_le_bytes()); // "UVC1"
        assert!(matches!(
            crate::cache::GameCache::from_bytes(&bytes),
            Err(crate::cache::CacheError::BadMagic)
        ));
    }

    #[test]
    fn expansion_rank_orders_base_then_x_then_numbered() {
        let mut names = vec![
            "x2ui.txt",
            "xui.txt",
            "ui.txt",
            "x4items_nonvoiced.txt",
            "x3items_nonvoiced.txt",
        ];
        names.sort_by_key(|name| (expansion_rank(name), name.to_lowercase()));
        assert_eq!(
            names,
            vec![
                "ui.txt",
                "xui.txt",
                "x2ui.txt",
                "x3items_nonvoiced.txt",
                "x4items_nonvoiced.txt"
            ]
        );
    }
}
