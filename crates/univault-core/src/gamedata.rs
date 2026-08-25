//! The combined game database: ARZ records plus localized text, and
//! the name-resolution rules that turn a record path into a display
//! name. The per-type variable dispatch is ported from `TQVaultAE`'s
//! `Info.AssignVariableNames` (MIT).

use crate::arc::{ArcError, ArcFile};
use crate::arz::{ArzError, ArzFile, DbRecord};
use crate::chr::{Item, RecordId};
use crate::text::TextDb;

/// Parsed game reference data: `database.arz` plus the localization
/// table from a `Text_XX.arc`.
pub struct GameData {
    arz: ArzFile,
    text: TextDb,
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
        Self { arz, text }
    }

    /// See [`ArzFile::record`].
    #[must_use]
    pub fn record(&self, id: &RecordId) -> Option<Result<DbRecord, ArzError>> {
        self.arz.record(id)
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
        let tag = name_tag(&record)?.to_string();
        self.text.get(&tag).map(str::to_string)
    }

    /// Conservative grid footprint (width, height) for an item, by its
    /// base record's class. Real footprints come from bitmap textures
    /// (not read yet) and `TQVaultAE` recomputes them on load, so these
    /// upper bounds only have to guarantee that placements made with
    /// them never overlap at true sizes.
    #[must_use]
    pub fn item_footprint(&self, item: &Item) -> (i32, i32) {
        let record_type = self
            .arz
            .record(&item.base)
            .and_then(Result::ok)
            .map(|record| record.record_type);
        match record_type.as_deref() {
            Some(class) if class.starts_with("Weapon") => (2, 5),
            Some(class) if class.starts_with("ArmorProtective") => (2, 3),
            Some(_) => (2, 2),
            None => FALLBACK_FOOTPRINT,
        }
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
