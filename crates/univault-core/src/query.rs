//! Cross-vault item questions: full display names, expansion
//! origin, coarse categories, and filter matching — the pure engine
//! behind the search view and the MCP server's item naming.

use crate::cache::GameCache;
use crate::chr::{Item, RecordId};
use crate::stats::{self, Requirement, StatLine};
use crate::style::{self, GearSlot, ItemKind, ItemStyle};

/// A record's display name, falling back to the record file stem
/// when the cache is absent or silent.
#[must_use]
pub fn record_name(db: Option<&GameCache>, id: &RecordId) -> String {
    db.and_then(|db| db.record_name(id))
        .unwrap_or_else(|| id.file_stem().to_string())
}

/// The game's full item name: prefix, quality, base, style word,
/// suffix, and the stack count — the same assembly the GUI titles
/// tooltips with.
#[must_use]
pub fn item_name(db: Option<&GameCache>, item: &Item) -> String {
    let details = db.map(|db| stats::item_details(db, item));
    let mut parts: Vec<String> = Vec::new();
    if let Some(prefix) = &item.prefix {
        parts.push(record_name(db, prefix));
    }
    if let Some(quality) = details.as_ref().and_then(|d| d.quality.clone()) {
        parts.push(quality);
    }
    parts.push(record_name(db, &item.base));
    if let Some(style_word) = details.as_ref().and_then(|d| d.style_word.clone()) {
        parts.push(style_word);
    }
    if let Some(suffix) = &item.suffix {
        parts.push(record_name(db, suffix));
    }
    if item.stack_size > 1 {
        parts.push(format!("×{}", item.stack_size));
    }
    parts.join(" ")
}

/// The expansion an item hails from, ranked highest across base and
/// affixes; `None` is the base game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Expansion {
    ImmortalThrone,
    Ragnarok,
    Atlantis,
    EternalEmbers,
}

impl Expansion {
    pub const ALL: [Self; 4] = [
        Self::ImmortalThrone,
        Self::Ragnarok,
        Self::Atlantis,
        Self::EternalEmbers,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ImmortalThrone => "Immortal Throne",
            Self::Ragnarok => "Ragnarök",
            Self::Atlantis => "Atlantis",
            Self::EternalEmbers => "Eternal Embers",
        }
    }
}

/// The highest expansion among the item's base and affix record
/// paths (`XPACK`..`XPACK4` prefixes under `RECORDS`).
#[must_use]
pub fn expansion_origin(item: &Item) -> Option<Expansion> {
    let rank = |id: Option<&RecordId>| -> Option<Expansion> {
        let id = id?;
        let upper = id.as_str().to_uppercase();
        let head = upper.trim_start_matches(['\\', '/']);
        let head = head
            .strip_prefix("RECORDS")
            .map_or(head, |rest| rest.trim_start_matches(['\\', '/']));
        if head.starts_with("XPACK4") {
            Some(Expansion::EternalEmbers)
        } else if head.starts_with("XPACK3") {
            Some(Expansion::Atlantis)
        } else if head.starts_with("XPACK2") {
            Some(Expansion::Ragnarok)
        } else if head.starts_with("XPACK") {
            Some(Expansion::ImmortalThrone)
        } else {
            None
        }
    };
    [Some(&item.base), item.prefix.as_ref(), item.suffix.as_ref()]
        .into_iter()
        .filter_map(rank)
        .max()
}

/// Coarse item category for filtering: an equipment family for gear,
/// the item kind for everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCategory {
    Gear(GearSlot),
    Relic,
    Charm,
    Artifact,
    Formula,
    Scroll,
    Potion,
    Quest,
}

impl ItemCategory {
    /// Every category, in dropdown order: armor, jewelry, weapons,
    /// then the non-gear kinds.
    pub const ALL: [Self; 22] = [
        Self::Gear(GearSlot::Head),
        Self::Gear(GearSlot::UpperBody),
        Self::Gear(GearSlot::Forearm),
        Self::Gear(GearSlot::LowerBody),
        Self::Gear(GearSlot::Shield),
        Self::Gear(GearSlot::Amulet),
        Self::Gear(GearSlot::Ring),
        Self::Gear(GearSlot::Bracelet),
        Self::Gear(GearSlot::Sword),
        Self::Gear(GearSlot::Axe),
        Self::Gear(GearSlot::Mace),
        Self::Gear(GearSlot::Spear),
        Self::Gear(GearSlot::Bow),
        Self::Gear(GearSlot::Thrown),
        Self::Gear(GearSlot::Staff),
        Self::Relic,
        Self::Charm,
        Self::Artifact,
        Self::Formula,
        Self::Scroll,
        Self::Potion,
        Self::Quest,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Gear(GearSlot::Head) => "Helmet",
            Self::Gear(GearSlot::UpperBody) => "Torso armor",
            Self::Gear(GearSlot::Forearm) => "Armband",
            Self::Gear(GearSlot::LowerBody) => "Greaves",
            Self::Gear(GearSlot::Shield) => "Shield",
            Self::Gear(GearSlot::Amulet) => "Amulet",
            Self::Gear(GearSlot::Ring) => "Ring",
            Self::Gear(GearSlot::Bracelet) => "Bracelet",
            Self::Gear(GearSlot::Sword) => "Sword",
            Self::Gear(GearSlot::Axe) => "Axe",
            Self::Gear(GearSlot::Mace) => "Mace",
            Self::Gear(GearSlot::Spear) => "Spear",
            Self::Gear(GearSlot::Bow) => "Bow",
            Self::Gear(GearSlot::Thrown) => "Thrown",
            Self::Gear(GearSlot::Staff) => "Staff",
            Self::Relic => "Relic",
            Self::Charm => "Charm",
            Self::Artifact => "Artifact",
            Self::Formula => "Arcane formula",
            Self::Scroll => "Scroll",
            Self::Potion => "Potion",
            Self::Quest => "Quest item",
        }
    }
}

/// The item's category; `None` when the record is unknown to the
/// cache or gear of no recognized family.
#[must_use]
pub fn item_category(db: &GameCache, item: &Item) -> Option<ItemCategory> {
    let entry = db.entry(&item.base)?;
    match entry.kind {
        ItemKind::Gear => entry.gear_slot.map(ItemCategory::Gear),
        ItemKind::RelicOrCharm { is_charm: true, .. } => Some(ItemCategory::Charm),
        ItemKind::RelicOrCharm {
            is_charm: false, ..
        } => Some(ItemCategory::Relic),
        ItemKind::Artifact => Some(ItemCategory::Artifact),
        ItemKind::Formula => Some(ItemCategory::Formula),
        ItemKind::Scroll => Some(ItemCategory::Scroll),
        ItemKind::Potion => Some(ItemCategory::Potion),
        ItemKind::Quest => Some(ItemCategory::Quest),
    }
}

/// The name of the set the item's base belongs to, from the cached
/// set block's title line.
#[must_use]
pub fn set_name(db: &GameCache, item: &Item) -> Option<String> {
    db.entry(&item.base)?
        .stats
        .as_ref()?
        .set_lines
        .first()
        .map(|line| line.text.clone())
}

/// Every stat-bearing display line on the item: base attributes
/// (shard-slot aware), affix attributes, socketed relics, and
/// rolled completion bonuses. Excludes meta lines (requirements,
/// seed, set membership, flavor text).
#[must_use]
pub fn stat_lines(db: &GameCache, item: &Item) -> Vec<StatLine> {
    let mut lines: Vec<StatLine> = Vec::new();
    let mut extend = |id: Option<&RecordId>, slot_count: i32| {
        let Some(stats) = id
            .and_then(|id| db.entry(id))
            .and_then(|e| e.stats.as_ref())
        else {
            return;
        };
        let slot = usize::try_from(slot_count.max(1) - 1).unwrap_or(0);
        lines.extend(stats::attr_slot(stats, slot).iter().cloned());
    };
    let base_slot = db.entry(&item.base).map_or(1, |entry| match entry.kind {
        ItemKind::RelicOrCharm { .. } => item.var1,
        ItemKind::Gear
        | ItemKind::Artifact
        | ItemKind::Formula
        | ItemKind::Scroll
        | ItemKind::Potion
        | ItemKind::Quest => 1,
    });
    extend(Some(&item.base), base_slot);
    extend(item.prefix.as_ref(), 1);
    extend(item.suffix.as_ref(), 1);
    extend(item.relic.as_ref(), item.var1);
    extend(item.relic_bonus.as_ref(), 1);
    if let Some(second) = &item.atlantis {
        extend(second.relic.as_ref(), second.var2);
        extend(second.bonus.as_ref(), 1);
    }
    lines
}

/// One search criterion; a query is a conjunction of these. All text
/// matching is case-insensitive substring containment, and an empty
/// needle matches everything.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// The full display name contains the text.
    NameContains(String),
    /// A prefix or suffix whose name contains the text is present.
    HasAffix(String),
    /// A prefix or suffix grants a stat line containing the text —
    /// with `min`, the line's largest number must reach it.
    AffixStat { text: String, min: Option<f32> },
    /// Any stat line on the item (base, affix, relic, bonus)
    /// matches — with `min`, the line's largest number must reach it.
    StatContains { text: String, min: Option<f32> },
    /// The item's requirement for the key is at most the cap (items
    /// without that requirement pass).
    RequirementAtMost(Requirement, i32),
    /// The item's base belongs to a set whose name contains the text.
    InSet(String),
    /// The item renders in exactly this style (rarity).
    Style(ItemStyle),
    /// The item is of exactly this category.
    Category(ItemCategory),
    /// A relic/charm is (or is not) socketed into the item.
    Socketed(bool),
    /// The item hails from this expansion (`None`: base game only).
    Origin(Option<Expansion>),
}

/// Whether the item passes every filter.
#[must_use]
pub fn matches(db: &GameCache, item: &Item, filters: &[Filter]) -> bool {
    filters.iter().all(|filter| passes(db, item, filter))
}

fn passes(db: &GameCache, item: &Item, filter: &Filter) -> bool {
    match filter {
        Filter::NameContains(text) => contains_ci(&item_name(Some(db), item), text),
        Filter::HasAffix(name) => {
            affix_records(item).any(|affix| contains_ci(&record_name(Some(db), affix), name))
        }
        Filter::AffixStat { text, min } => affix_records(item)
            .filter_map(|affix| db.entry(affix))
            .filter_map(|entry| entry.stats.as_ref())
            .flat_map(|stats| stats::attr_slot(stats, 0))
            .any(|line| line_passes(line, text, *min)),
        Filter::StatContains { text, min } => stat_lines(db, item)
            .iter()
            .any(|line| line_passes(line, text, *min)),
        Filter::RequirementAtMost(key, cap) => stats::item_requirements(db, item)
            .into_iter()
            .find(|(existing, _)| existing == key)
            .is_none_or(|(_, value)| value <= *cap),
        Filter::InSet(text) => set_name(db, item).is_some_and(|name| contains_ci(&name, text)),
        Filter::Style(style) => style::item_style(Some(db), item) == *style,
        Filter::Category(category) => item_category(db, item) == Some(*category),
        Filter::Socketed(wanted) => {
            let socketed = item.relic.is_some()
                || item
                    .atlantis
                    .as_ref()
                    .is_some_and(|second| second.relic.is_some());
            socketed == *wanted
        }
        Filter::Origin(origin) => expansion_origin(item) == *origin,
    }
}

fn affix_records(item: &Item) -> impl Iterator<Item = &RecordId> {
    [item.prefix.as_ref(), item.suffix.as_ref()]
        .into_iter()
        .flatten()
}

fn line_passes(line: &StatLine, text: &str, min: Option<f32>) -> bool {
    contains_ci(&line.text, text)
        && min.is_none_or(|min| {
            numbers_in(&line.text)
                .into_iter()
                .fold(None::<f32>, |best, value| {
                    Some(best.map_or(value, |best| best.max(value)))
                })
                .is_some_and(|largest| largest >= min)
        })
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Every number in a display line, sign included only when directly
/// attached ("-15%" is negative; the dash in "3.0 - 6.0" is not).
fn numbers_in(text: &str) -> Vec<f32> {
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let signed =
                i > 0 && bytes[i - 1] == b'-' && (i == 1 || !bytes[i - 2].is_ascii_digit());
            let start = if signed { i - 1 } else { i };
            let mut end = i;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                end += 1;
            }
            if let Ok(value) = text[start..end].trim_end_matches('.').parse::<f32>() {
                values.push(value);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    values
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)] // comparisons are against parsed literals

    use super::*;
    use crate::arz::ArzFile;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::chr::{GridPos, ItemSeed};
    use crate::gamedata::GameData;
    use crate::text::TextDb;

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

    fn item(base: &str) -> Item {
        Item {
            base: record_id(base),
            prefix: None,
            suffix: None,
            relic: None,
            relic_bonus: None,
            seed: ItemSeed::new(7),
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
            "records\\game\\itemcost.dbr",
            "",
            &[
                (
                    "swordStrengthEquation",
                    Values::Strings(&["50+((itemLevel-1)*4.75)"]),
                ),
                (
                    "swordDexterityEquation",
                    Values::Strings(&["50+((itemLevel-1)*3.75)"]),
                ),
            ],
        );
        builder.record(
            "records\\item\\broadsword.dbr",
            "WeaponMelee_Sword",
            &[
                ("itemNameTag", Values::Strings(&["tagSword"])),
                ("itemClassification", Values::Strings(&["Epic"])),
                ("offensivePhysicalMin", Values::Floats(&[12.0])),
                ("offensivePhysicalMax", Values::Floats(&[31.0])),
                ("itemLevel", Values::Ints(&[20])),
                ("levelRequirement", Values::Ints(&[24])),
            ],
        );
        builder.record(
            "records\\item\\sharp.dbr",
            "LootRandomizer",
            &[
                ("lootRandomizerName", Values::Strings(&["tagSharp"])),
                ("offensiveFireMin", Values::Floats(&[10.0])),
                ("offensiveFireMax", Values::Floats(&[10.0])),
            ],
        );
        builder.record(
            "records\\xpack4\\item\\ofembers.dbr",
            "LootRandomizer",
            &[
                ("lootRandomizerName", Values::Strings(&["tagEmbers"])),
                ("offensiveColdMin", Values::Floats(&[7.0])),
                ("offensiveColdMax", Values::Floats(&[7.0])),
            ],
        );
        builder.record(
            "records\\item\\skyhelm.dbr",
            "ArmorProtective_Head",
            &[
                ("description", Values::Strings(&["tagHelm"])),
                (
                    "itemSetName",
                    Values::Strings(&["records\\item\\sets\\olympus.dbr"]),
                ),
            ],
        );
        builder.record(
            "records\\item\\sets\\olympus.dbr",
            "ItemSet",
            &[
                ("setName", Values::Strings(&["tagOlympus"])),
                (
                    "setMembers",
                    Values::Strings(&["records\\item\\skyhelm.dbr"]),
                ),
            ],
        );
        builder.record(
            "records\\item\\monkeypaw.dbr",
            "ItemCharm",
            &[
                ("description", Values::Strings(&["tagPaw"])),
                ("completedRelicLevel", Values::Ints(&[3])),
                ("offensiveColdMin", Values::Floats(&[4.0, 8.0, 12.0])),
                ("offensiveColdMax", Values::Floats(&[4.0, 8.0, 12.0])),
            ],
        );
        builder.record(
            "records\\item\\pawbonus.dbr",
            "LootRandomizer",
            &[("offensiveColdModifier", Values::Floats(&[10.0]))],
        );
        let mut text = TextDb::new();
        text.add_file(&text_file(
            "tagSword=Broadsword\ntagSharp=Sharp\ntagEmbers=of Embers\n\
             tagHelm=Skyguard Helm\ntagOlympus=Guardians of Olympus\n\
             tagPaw=Monkey Paw\n\
             DamageBasePhysical=Damage\nDamageFire=Fire Damage\n\
             DamageCold=Cold Damage\n\
             DamageModifierCold={%+.0f0}% Cold Damage\n\
             DamageSingleFormat={%.0f0}\n\
             DamageRangeFormat={%.0f0} ~ {%.0f1}\n\
             MeetsRequirement=Required {%s0}: {%.0f1}\n\
             Strength=Strength\nDexterity=Dexterity\n\
             LevelRequirement=Player Level\n",
        ));
        let data = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), text);
        GameCache::from_bytes(&data.build_cache(Vec::new()).to_bytes()).unwrap()
    }

    fn affixed_sword() -> Item {
        let mut sword = item("records\\item\\broadsword.dbr");
        sword.prefix = Some(record_id("records\\item\\sharp.dbr"));
        sword.suffix = Some(record_id("records\\xpack4\\item\\ofembers.dbr"));
        sword
    }

    fn one(db: &GameCache, item: &Item, filter: Filter) -> bool {
        matches(db, item, &[filter])
    }

    #[test]
    fn item_name_assembles_prefix_base_suffix() {
        let db = sample_cache();
        assert_eq!(
            item_name(Some(&db), &affixed_sword()),
            "Sharp Broadsword of Embers"
        );
    }

    #[test]
    fn name_filter_is_case_insensitive_substring() {
        let db = sample_cache();
        let sword = affixed_sword();
        assert!(one(&db, &sword, Filter::NameContains("sharp broad".into())));
        assert!(one(&db, &sword, Filter::NameContains(String::new())));
        assert!(!one(&db, &sword, Filter::NameContains("buckler".into())));
    }

    #[test]
    fn affix_presence_matches_either_affix_name() {
        let db = sample_cache();
        let sword = affixed_sword();
        assert!(one(&db, &sword, Filter::HasAffix("sharp".into())));
        assert!(one(&db, &sword, Filter::HasAffix("embers".into())));
        assert!(!one(&db, &sword, Filter::HasAffix("frost".into())));
        // Empty text means "any affix" — an affixless item fails it.
        let plain = item("records\\item\\broadsword.dbr");
        assert!(one(&db, &sword, Filter::HasAffix(String::new())));
        assert!(!one(&db, &plain, Filter::HasAffix(String::new())));
    }

    #[test]
    fn affix_value_compares_against_the_lines_largest_number() {
        let db = sample_cache();
        let sword = affixed_sword();
        let fire = |min| Filter::AffixStat {
            text: "fire".into(),
            min,
        };
        assert!(one(&db, &sword, fire(None)));
        assert!(one(&db, &sword, fire(Some(10.0))));
        assert!(!one(&db, &sword, fire(Some(10.5))));
        // Base lines don't count as affix stats: 12~31 Damage is on
        // the base, so no affix line reaches 30.
        assert!(!one(
            &db,
            &sword,
            Filter::AffixStat {
                text: String::new(),
                min: Some(30.0),
            }
        ));
    }

    #[test]
    fn stat_filter_spans_base_and_affix_lines() {
        let db = sample_cache();
        let sword = affixed_sword();
        let stat = |text: &str, min| Filter::StatContains {
            text: text.into(),
            min,
        };
        assert!(one(&db, &sword, stat("damage", Some(31.0))));
        assert!(one(&db, &sword, stat("cold damage", Some(7.0))));
        assert!(!one(&db, &sword, stat("cold damage", Some(8.0))));
        assert!(!one(&db, &sword, stat("poison", None)));
    }

    #[test]
    fn charm_stat_lines_track_the_shard_slot() {
        let db = sample_cache();
        let mut charm = item("records\\item\\monkeypaw.dbr");
        charm.var1 = 2;
        let cold = |min| Filter::StatContains {
            text: "cold".into(),
            min,
        };
        assert!(one(&db, &charm, cold(Some(8.0))));
        assert!(!one(&db, &charm, cold(Some(9.0))));
    }

    #[test]
    fn typed_requirements_merge_explicit_and_equations() {
        let db = sample_cache();
        assert_eq!(
            stats::item_requirements(&db, &affixed_sword()),
            vec![
                (Requirement::Dexterity, 122),
                (Requirement::Level, 24),
                (Requirement::Strength, 141),
            ]
        );
    }

    #[test]
    fn requirement_caps_pass_at_the_boundary_and_when_absent() {
        let db = sample_cache();
        let sword = affixed_sword();
        let cap = Filter::RequirementAtMost;
        assert!(one(&db, &sword, cap(Requirement::Strength, 141)));
        assert!(!one(&db, &sword, cap(Requirement::Strength, 140)));
        assert!(one(&db, &sword, cap(Requirement::Level, 24)));
        assert!(!one(&db, &sword, cap(Requirement::Level, 23)));
        assert!(one(&db, &sword, cap(Requirement::Intelligence, 1)));
    }

    #[test]
    fn set_filter_matches_the_set_title() {
        let db = sample_cache();
        let helm = item("records\\item\\skyhelm.dbr");
        assert!(one(&db, &helm, Filter::InSet("olympus".into())));
        assert!(one(&db, &helm, Filter::InSet(String::new())));
        assert!(!one(&db, &helm, Filter::InSet("titans".into())));
        let sword = affixed_sword();
        assert!(!one(&db, &sword, Filter::InSet(String::new())));
    }

    #[test]
    fn style_and_category_filters_match_exactly() {
        let db = sample_cache();
        let sword = affixed_sword();
        assert!(one(&db, &sword, Filter::Style(ItemStyle::Epic)));
        assert!(!one(&db, &sword, Filter::Style(ItemStyle::Rare)));
        assert!(one(
            &db,
            &sword,
            Filter::Category(ItemCategory::Gear(GearSlot::Sword))
        ));
        let charm = item("records\\item\\monkeypaw.dbr");
        assert!(one(&db, &charm, Filter::Category(ItemCategory::Charm)));
        assert!(!one(&db, &charm, Filter::Category(ItemCategory::Relic)));
        let helm = item("records\\item\\skyhelm.dbr");
        assert!(one(
            &db,
            &helm,
            Filter::Category(ItemCategory::Gear(GearSlot::Head))
        ));
    }

    #[test]
    fn socketed_filter_sees_first_and_second_sockets() {
        let db = sample_cache();
        let mut socketed = item("records\\item\\broadsword.dbr");
        socketed.relic = Some(record_id("records\\item\\monkeypaw.dbr"));
        socketed.var1 = 2;
        assert!(one(&db, &socketed, Filter::Socketed(true)));
        let plain = item("records\\item\\broadsword.dbr");
        assert!(one(&db, &plain, Filter::Socketed(false)));
        // A socketed relic's lines join the item's stat lines.
        assert!(one(
            &db,
            &socketed,
            Filter::StatContains {
                text: "cold".into(),
                min: Some(8.0),
            }
        ));
    }

    #[test]
    fn origin_ranks_the_highest_expansion_across_records() {
        let db = sample_cache();
        let sword = affixed_sword();
        assert_eq!(expansion_origin(&sword), Some(Expansion::EternalEmbers));
        assert!(one(
            &db,
            &sword,
            Filter::Origin(Some(Expansion::EternalEmbers))
        ));
        let helm = item("records\\item\\skyhelm.dbr");
        assert_eq!(expansion_origin(&helm), None);
        assert!(one(&db, &helm, Filter::Origin(None)));
        let raider = item("records\\xpack2\\item\\seaxe.dbr");
        assert_eq!(expansion_origin(&raider), Some(Expansion::Ragnarok));
    }

    #[test]
    fn filters_are_a_conjunction() {
        let db = sample_cache();
        let sword = affixed_sword();
        assert!(matches(
            &db,
            &sword,
            &[
                Filter::NameContains("broadsword".into()),
                Filter::Socketed(false),
            ]
        ));
        assert!(!matches(
            &db,
            &sword,
            &[
                Filter::NameContains("broadsword".into()),
                Filter::Socketed(true),
            ]
        ));
        assert!(matches(&db, &sword, &[]));
    }

    #[test]
    fn numbers_in_reads_ranges_decimals_and_attached_signs() {
        assert_eq!(numbers_in("12 ~ 31 Damage"), vec![12.0, 31.0]);
        assert_eq!(numbers_in("over 3.0 - 6.0 Seconds"), vec![3.0, 6.0]);
        assert_eq!(numbers_in("-15% Something"), vec![-15.0]);
        assert_eq!(numbers_in("+24 Dexterity"), vec![24.0]);
        assert_eq!(numbers_in("no digits here"), Vec::<f32>::new());
    }
}
