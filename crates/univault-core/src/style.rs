//! Item rarity/style semantics: which of the game's display styles an
//! item renders in, and the exact palette colors the game gives them.
//! Ported from `TQVaultAE`'s `Item.ItemStyle`, `ItemStyle`, `TQColor`,
//! and `RecordId.ForItems` (MIT).

use crate::arz::{DbRecord, normalize};
use crate::cache::GameCache;
use crate::chr::{Item, RecordId};

/// The game's display style for an item — drives the name color and
/// the style caption exactly as Titan Quest renders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStyle {
    Broken,
    Mundane,
    Common,
    Rare,
    Epic,
    Legendary,
    Quest,
    Relic,
    Potion,
    Scroll,
    Parchment,
    Formulae,
    Artifact,
}

impl ItemStyle {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Broken => "Broken",
            Self::Mundane => "Mundane",
            Self::Common => "Common",
            Self::Rare => "Rare",
            Self::Epic => "Epic",
            Self::Legendary => "Legendary",
            Self::Quest => "Quest Item",
            Self::Relic => "Relic / Charm",
            Self::Potion => "Potion",
            Self::Scroll => "Scroll",
            Self::Parchment => "Parchment",
            Self::Formulae => "Arcane Formula",
            Self::Artifact => "Artifact",
        }
    }
}

/// An sRGB color from the game's fixed text palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The palette color of one of the game's `{^X}` color-tag letters —
/// `TQVaultAE`'s `TQColorHelper.ColorMap`.
#[must_use]
pub fn palette_color(tag: char) -> Rgb {
    let rgb = |r, g, b| Rgb { r, g, b };
    match tag.to_ascii_uppercase() {
        'A' => rgb(0, 255, 255),
        'B' => rgb(0, 163, 255),
        'C' => rgb(224, 255, 255),
        'D' => rgb(153, 153, 153),
        'F' => rgb(255, 0, 255),
        'G' => rgb(64, 255, 64),
        'I' => rgb(75, 0, 130),
        'K' => rgb(195, 176, 145),
        'L' => rgb(145, 203, 0),
        'M' => rgb(128, 0, 0),
        'O' => rgb(255, 173, 0),
        'P' => rgb(217, 5, 255),
        'R' => rgb(255, 0, 0),
        'S' => rgb(224, 224, 224),
        'T' => rgb(0, 255, 209),
        'Y' => rgb(255, 245, 43),
        _ => rgb(255, 255, 255),
    }
}

/// The palette color a style renders in — `TQVaultAE`'s
/// `ItemStyleExtension.TQColor` composed with its `ColorMap` values.
#[must_use]
pub fn style_color(style: ItemStyle) -> Rgb {
    let rgb = |r, g, b| Rgb { r, g, b };
    match style {
        ItemStyle::Broken => rgb(153, 153, 153),
        ItemStyle::Mundane => rgb(255, 255, 255),
        ItemStyle::Common => rgb(255, 245, 43),
        ItemStyle::Rare => rgb(64, 255, 64),
        ItemStyle::Epic | ItemStyle::Parchment => rgb(0, 163, 255),
        ItemStyle::Legendary | ItemStyle::Quest => rgb(217, 5, 255),
        ItemStyle::Relic => rgb(255, 173, 0),
        ItemStyle::Potion => rgb(255, 0, 0),
        ItemStyle::Scroll => rgb(145, 203, 0),
        ItemStyle::Formulae | ItemStyle::Artifact => rgb(0, 255, 209),
    }
}

/// `itemClassification` distilled to the values style derivation
/// matches; every other value behaves identically (`Other`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Classification {
    Broken,
    Rare,
    Epic,
    Legendary,
    Other,
}

impl Classification {
    pub(crate) fn of(record: &DbRecord) -> Self {
        match record.string("itemClassification") {
            Some(raw) if raw.eq_ignore_ascii_case("Broken") => Self::Broken,
            Some(raw) if raw.eq_ignore_ascii_case("Rare") => Self::Rare,
            Some(raw) if raw.eq_ignore_ascii_case("Epic") => Self::Epic,
            Some(raw) if raw.eq_ignore_ascii_case("Legendary") => Self::Legendary,
            Some(_) | None => Self::Other,
        }
    }
}

/// The fifteen equipment families a relic/charm record carries
/// allow-flags for, mapped from equipment record classes. Socketing
/// rules are per family (a ring relic fits any ring); rarity never
/// enters into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GearSlot {
    Head,
    UpperBody,
    Forearm,
    LowerBody,
    Shield,
    Amulet,
    Ring,
    Bracelet,
    Sword,
    Axe,
    Mace,
    Spear,
    Bow,
    Thrown,
    Staff,
}

impl GearSlot {
    /// Serialization / bitmask order — append-only.
    pub(crate) const ALL: [Self; 15] = [
        Self::Head,
        Self::UpperBody,
        Self::Forearm,
        Self::LowerBody,
        Self::Shield,
        Self::Amulet,
        Self::Ring,
        Self::Bracelet,
        Self::Sword,
        Self::Axe,
        Self::Mace,
        Self::Spear,
        Self::Bow,
        Self::Thrown,
        Self::Staff,
    ];

    pub(crate) fn of_class(class: &str) -> Option<Self> {
        let matches = |wanted: &str| class.eq_ignore_ascii_case(wanted);
        if matches("ArmorProtective_Head") {
            Some(Self::Head)
        } else if matches("ArmorProtective_UpperBody") {
            Some(Self::UpperBody)
        } else if matches("ArmorProtective_Forearm") {
            Some(Self::Forearm)
        } else if matches("ArmorProtective_LowerBody") {
            Some(Self::LowerBody)
        } else if matches("WeaponArmor_Shield") {
            Some(Self::Shield)
        } else if matches("ArmorJewelry_Amulet") {
            Some(Self::Amulet)
        } else if matches("ArmorJewelry_Ring") {
            Some(Self::Ring)
        } else if matches("ArmorJewelry_Bracelet") {
            Some(Self::Bracelet)
        } else if matches("WeaponMelee_Sword") {
            Some(Self::Sword)
        } else if matches("WeaponMelee_Axe") {
            Some(Self::Axe)
        } else if matches("WeaponMelee_Mace") {
            Some(Self::Mace)
        } else if matches("WeaponHunting_Spear") {
            Some(Self::Spear)
        } else if matches("WeaponHunting_Bow") {
            Some(Self::Bow)
        } else if matches("WeaponHunting_RangedOneHand") {
            Some(Self::Thrown)
        } else if matches("WeaponMagical_Staff") {
            Some(Self::Staff)
        } else {
            None
        }
    }

    /// The allow-flag variable on relic/charm records for this
    /// family.
    pub(crate) fn flag_variable(self) -> &'static str {
        match self {
            Self::Head => "helmet",
            Self::UpperBody => "bodyArmor",
            Self::Forearm => "armband",
            Self::LowerBody => "greaves",
            Self::Shield => "shield",
            Self::Amulet => "amulet",
            Self::Ring => "ring",
            Self::Bracelet => "bracelet",
            Self::Sword => "sword",
            Self::Axe => "axe",
            Self::Mace => "mace",
            Self::Spear => "spear",
            Self::Bow => "bow",
            Self::Thrown => "rangedOneHand",
            Self::Staff => "staff",
        }
    }
}

/// Coarse kind of a base record, derived from its class at import —
/// the class-based half of `TQVaultAE`'s `Is*` predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Gear,
    Artifact,
    Formula,
    Scroll,
    Potion,
    RelicOrCharm {
        completed_level: Option<i32>,
        is_charm: bool,
    },
    Quest,
}

impl ItemKind {
    pub(crate) fn of(record: &DbRecord) -> Self {
        let class = |candidate: &str| record.record_type.eq_ignore_ascii_case(candidate);
        if class("ItemArtifact") {
            Self::Artifact
        } else if class("ItemArtifactFormula") {
            Self::Formula
        } else if class("OneShot_Scroll") {
            Self::Scroll
        } else if class("OneShot_PotionHealth")
            || class("OneShot_PotionMana")
            || class("OneShot_Scroll_Eternal")
        {
            Self::Potion
        } else if class("ItemRelic") || class("ItemCharm") {
            Self::RelicOrCharm {
                completed_level: record.integer("completedRelicLevel"),
                is_charm: class("ItemCharm"),
            }
        } else if class("QuestItem")
            || record
                .string("itemClassification")
                .is_some_and(|raw| raw.eq_ignore_ascii_case("Quest"))
        {
            Self::Quest
        } else {
            Self::Gear
        }
    }
}

/// Kind heuristics from the record path alone, for records the
/// database doesn't know — `TQVaultAE`'s `RecordId.ForItems` checks.
fn kind_from_path(normalized: &str) -> ItemKind {
    let contains = |needle: &str| normalized.contains(needle);
    let charm = contains("ANIMALRELICS") || contains(r"\CHARMS\");
    if contains(r"\ARCANEFORMULAE\") {
        ItemKind::Formula
    } else if contains(r"\ARTIFACTS\") {
        ItemKind::Artifact
    } else if contains(r"\SCROLLS\") {
        ItemKind::Scroll
    } else if contains(r"ONESHOT\POTION") {
        ItemKind::Potion
    } else if charm || contains("RELICS") {
        ItemKind::RelicOrCharm {
            completed_level: None,
            is_charm: charm,
        }
    } else if contains("QUEST") {
        ItemKind::Quest
    } else {
        ItemKind::Gear
    }
}

/// The style an item renders in — `TQVaultAE`'s `Item.ItemStyle`
/// decision order. Records the database doesn't know (or a missing
/// database) fall back to path heuristics, so items still color
/// sensibly without imported game data.
#[must_use]
pub fn item_style(db: Option<&GameCache>, item: &Item) -> ItemStyle {
    if classification(db, item.prefix.as_ref()) == Classification::Broken {
        return ItemStyle::Broken;
    }
    let base = db.and_then(|db| db.entry(&item.base));
    let path = normalize(item.base.as_str());
    let kind = base.map_or_else(|| kind_from_path(&path), |entry| entry.kind);
    match kind {
        ItemKind::Artifact => return ItemStyle::Artifact,
        ItemKind::Formula => return ItemStyle::Formulae,
        ItemKind::Scroll => return ItemStyle::Scroll,
        ItemKind::Gear | ItemKind::Potion | ItemKind::RelicOrCharm { .. } | ItemKind::Quest => {}
    }
    // Parchments have no class of their own; the game knows them only
    // by path, which is why this check outranks the kinds below.
    if path.contains("PARCHMENT") {
        return ItemStyle::Parchment;
    }
    match kind {
        ItemKind::RelicOrCharm { .. } => return ItemStyle::Relic,
        ItemKind::Potion => return ItemStyle::Potion,
        ItemKind::Quest => return ItemStyle::Quest,
        ItemKind::Gear | ItemKind::Artifact | ItemKind::Formula | ItemKind::Scroll => {}
    }
    match base.map_or(Classification::Other, |entry| entry.classification) {
        Classification::Epic => return ItemStyle::Epic,
        Classification::Legendary => return ItemStyle::Legendary,
        Classification::Rare => return ItemStyle::Rare,
        Classification::Broken | Classification::Other => {}
    }
    if classification(db, item.prefix.as_ref()) == Classification::Rare
        || classification(db, item.suffix.as_ref()) == Classification::Rare
    {
        return ItemStyle::Rare;
    }
    if item.prefix.is_some() || item.suffix.is_some() {
        ItemStyle::Common
    } else {
        ItemStyle::Mundane
    }
}

fn classification(db: Option<&GameCache>, id: Option<&RecordId>) -> Classification {
    id.and_then(|id| db.and_then(|db| db.entry(id)))
        .map_or(Classification::Other, |entry| entry.classification)
}

/// Shard progress of a standalone relic or charm (`None` for any
/// other item): `have` is the item's effective collected count (the
/// game encodes a single shard as `var1 = 0`, so this is never below
/// one), `needed` the record's completion level when the database
/// knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelicShards {
    pub have: i32,
    pub needed: Option<i32>,
}

#[must_use]
pub fn relic_shards(db: Option<&GameCache>, item: &Item) -> Option<RelicShards> {
    (item_style(db, item) == ItemStyle::Relic).then(|| RelicShards {
        have: crate::transfer::shard_count(item),
        needed: db
            .and_then(|db| db.entry(&item.base))
            .and_then(|entry| match entry.kind {
                ItemKind::RelicOrCharm {
                    completed_level, ..
                } => completed_level,
                ItemKind::Gear
                | ItemKind::Artifact
                | ItemKind::Formula
                | ItemKind::Scroll
                | ItemKind::Potion
                | ItemKind::Quest => None,
            }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arz::ArzFile;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::chr::{GridPos, ItemSeed};
    use crate::gamedata::GameData;
    use crate::text::TextDb;

    fn record_id(raw: &str) -> RecordId {
        RecordId::parse(raw.to_string()).unwrap()
    }

    fn item(base: &str) -> Item {
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

    fn sample_cache() -> GameCache {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\plainsword.dbr",
            "WeaponMelee_Sword",
            &[("itemClassification", Values::Strings(&["Common"]))],
        );
        builder.record(
            "records\\item\\epicsword.dbr",
            "WeaponMelee_Sword",
            &[("itemClassification", Values::Strings(&["Epic"]))],
        );
        builder.record(
            "records\\item\\legendarysword.dbr",
            "WeaponMelee_Sword",
            &[("itemClassification", Values::Strings(&["Legendary"]))],
        );
        builder.record(
            "records\\item\\raresword.dbr",
            "WeaponMelee_Sword",
            &[("itemClassification", Values::Strings(&["Rare"]))],
        );
        builder.record(
            "records\\item\\brokenprefix.dbr",
            "LootRandomizer",
            &[("itemClassification", Values::Strings(&["Broken"]))],
        );
        builder.record(
            "records\\item\\rareprefix.dbr",
            "LootRandomizer",
            &[("itemClassification", Values::Strings(&["Rare"]))],
        );
        builder.record(
            "records\\item\\commonprefix.dbr",
            "LootRandomizer",
            &[("itemClassification", Values::Strings(&["Common"]))],
        );
        builder.record(
            "records\\item\\monkeycharm.dbr",
            "ItemCharm",
            &[("completedRelicLevel", Values::Ints(&[5]))],
        );
        builder.record(
            "records\\item\\eternalscroll.dbr",
            "OneShot_Scroll_Eternal",
            &[],
        );
        builder.record("records\\item\\battlescroll.dbr", "OneShot_Scroll", &[]);
        builder.record("records\\item\\theeye.dbr", "ItemArtifact", &[]);
        builder.record("records\\item\\eyeformula.dbr", "ItemArtifactFormula", &[]);
        builder.record(
            "records\\item\\herakleskey.dbr",
            "QuestItem",
            &[("itemClassification", Values::Strings(&["Quest"]))],
        );
        let data = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), TextDb::new());
        data.build_cache(Vec::new())
    }

    #[test]
    fn base_classification_sets_gear_styles() {
        let db = Some(sample_cache());
        let db = db.as_ref();
        assert_eq!(
            item_style(db, &item("records\\item\\plainsword.dbr")),
            ItemStyle::Mundane
        );
        assert_eq!(
            item_style(db, &item("records\\item\\raresword.dbr")),
            ItemStyle::Rare
        );
        assert_eq!(
            item_style(db, &item("records\\item\\epicsword.dbr")),
            ItemStyle::Epic
        );
        assert_eq!(
            item_style(db, &item("records\\item\\legendarysword.dbr")),
            ItemStyle::Legendary
        );
    }

    #[test]
    fn broken_prefix_outranks_everything() {
        let db = Some(sample_cache());
        let mut broken = item("records\\item\\epicsword.dbr");
        broken.prefix = Some(record_id("records\\item\\brokenprefix.dbr"));
        assert_eq!(item_style(db.as_ref(), &broken), ItemStyle::Broken);
    }

    #[test]
    fn affixes_lift_a_plain_base_to_common_or_rare() {
        let db = Some(sample_cache());
        let db = db.as_ref();
        let mut affixed = item("records\\item\\plainsword.dbr");
        affixed.prefix = Some(record_id("records\\item\\commonprefix.dbr"));
        assert_eq!(item_style(db, &affixed), ItemStyle::Common);
        affixed.suffix = Some(record_id("records\\item\\rareprefix.dbr"));
        assert_eq!(item_style(db, &affixed), ItemStyle::Rare);
    }

    #[test]
    fn classed_records_map_to_their_styles() {
        let db = Some(sample_cache());
        let db = db.as_ref();
        assert_eq!(
            item_style(db, &item("records\\item\\monkeycharm.dbr")),
            ItemStyle::Relic
        );
        assert_eq!(
            item_style(db, &item("records\\item\\battlescroll.dbr")),
            ItemStyle::Scroll
        );
        assert_eq!(
            item_style(db, &item("records\\item\\eternalscroll.dbr")),
            ItemStyle::Potion
        );
        assert_eq!(
            item_style(db, &item("records\\item\\theeye.dbr")),
            ItemStyle::Artifact
        );
        assert_eq!(
            item_style(db, &item("records\\item\\eyeformula.dbr")),
            ItemStyle::Formulae
        );
        assert_eq!(
            item_style(db, &item("records\\item\\herakleskey.dbr")),
            ItemStyle::Quest
        );
    }

    #[test]
    fn unknown_records_fall_back_to_path_heuristics() {
        for (path, expected) in [
            ("records\\xpack\\item\\scrolls\\zap.dbr", ItemStyle::Scroll),
            (
                "records\\item\\animalrelics\\boarhide.dbr",
                ItemStyle::Relic,
            ),
            (
                "records\\xpack\\item\\relics\\peleusshield.dbr",
                ItemStyle::Relic,
            ),
            ("records\\xpack\\item\\charms\\wing.dbr", ItemStyle::Relic),
            (
                "records\\item\\oneshot\\potionhealth.dbr",
                ItemStyle::Potion,
            ),
            (
                "records\\xpack\\item\\artifacts\\gaze.dbr",
                ItemStyle::Artifact,
            ),
            (
                "records\\xpack\\item\\artifacts\\arcaneformulae\\gaze.dbr",
                ItemStyle::Formulae,
            ),
            (
                "records\\xpack3\\items\\parchments\\note.dbr",
                ItemStyle::Parchment,
            ),
            ("records\\item\\quests\\goldenfleece.dbr", ItemStyle::Quest),
            ("records\\item\\equipment\\sword.dbr", ItemStyle::Mundane),
        ] {
            assert_eq!(item_style(None, &item(path)), expected, "{path}");
        }
    }

    #[test]
    fn relic_shards_report_progress_only_for_relics() {
        let db = Some(sample_cache());
        let db = db.as_ref();
        let mut charm = item("records\\item\\monkeycharm.dbr");
        charm.var1 = 3;
        assert_eq!(
            relic_shards(db, &charm),
            Some(RelicShards {
                have: 3,
                needed: Some(5)
            })
        );
        assert_eq!(
            relic_shards(db, &item("records\\item\\plainsword.dbr")),
            None
        );
        // A zero-encoded `var1` is the game's freshly dropped single
        // shard — one shard, not zero.
        let unknown = item("records\\item\\relics\\mystery.dbr");
        assert_eq!(
            relic_shards(None, &unknown),
            Some(RelicShards {
                have: 1,
                needed: None
            })
        );
    }

    #[test]
    fn every_style_has_a_label() {
        for style in [
            ItemStyle::Broken,
            ItemStyle::Mundane,
            ItemStyle::Common,
            ItemStyle::Rare,
            ItemStyle::Epic,
            ItemStyle::Legendary,
            ItemStyle::Quest,
            ItemStyle::Relic,
            ItemStyle::Potion,
            ItemStyle::Scroll,
            ItemStyle::Parchment,
            ItemStyle::Formulae,
            ItemStyle::Artifact,
        ] {
            assert!(!style.label().is_empty());
        }
    }

    #[test]
    fn palette_letters_match_the_game_color_map() {
        assert_eq!(
            palette_color('B'),
            Rgb {
                r: 0,
                g: 163,
                b: 255
            }
        );
        assert_eq!(
            palette_color('o'),
            Rgb {
                r: 255,
                g: 173,
                b: 0
            }
        );
        assert_eq!(
            palette_color('S'),
            Rgb {
                r: 224,
                g: 224,
                b: 224
            }
        );
        assert_eq!(
            palette_color('L'),
            Rgb {
                r: 145,
                g: 203,
                b: 0
            }
        );
        // Unknown letters render white, like the game's fallback.
        assert_eq!(
            palette_color('?'),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(
            palette_color('W'),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
    }

    #[test]
    fn style_colors_match_the_game_palette() {
        assert_eq!(
            style_color(ItemStyle::Legendary),
            Rgb {
                r: 217,
                g: 5,
                b: 255
            }
        );
        assert_eq!(
            style_color(ItemStyle::Epic),
            Rgb {
                r: 0,
                g: 163,
                b: 255
            }
        );
        assert_eq!(
            style_color(ItemStyle::Rare),
            Rgb {
                r: 64,
                g: 255,
                b: 64
            }
        );
        assert_eq!(
            style_color(ItemStyle::Relic),
            Rgb {
                r: 255,
                g: 173,
                b: 0
            }
        );
        assert_eq!(
            style_color(ItemStyle::Common),
            Rgb {
                r: 255,
                g: 245,
                b: 43
            }
        );
    }
}
