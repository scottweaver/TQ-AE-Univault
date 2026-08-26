//! Pure shaping of core types into the JSON the tools return. No IO
//! here — everything takes the already-loaded cache and items.

use serde::Serialize;
use univault_core::cache::GameCache;
use univault_core::chr::{Item, RecordId};
use univault_core::stats;
use univault_core::style;

/// The game's fixed equipment order, labeled from core's
/// [`univault_core::chr::EquipSlot`] — the wielded weapon lives in
/// the *right* hand (two-handers included); 7/9 are the shield-side
/// left hands.
#[must_use]
pub fn equipment_slot_names() -> [&'static str; univault_core::chr::EQUIPMENT_SLOTS] {
    univault_core::chr::EquipSlot::ALL.map(univault_core::chr::EquipSlot::label)
}

#[derive(Serialize)]
pub struct ShardsView {
    pub have: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needed: Option<i32>,
}

#[derive(Serialize)]
pub struct ItemView {
    pub name: String,
    pub style: &'static str,
    pub base_record: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relic_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relic_bonus_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relic2_record: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relic2_bonus_record: Option<String>,
    pub seed: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_size: Option<u32>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shards: Option<ShardsView>,
}

pub use univault_core::query::item_name;

pub fn item_view(db: Option<&GameCache>, item: &Item) -> ItemView {
    let (width, height) = db.map_or(univault_core::gamedata::FALLBACK_FOOTPRINT, |db| {
        db.item_footprint(item)
    });
    let record_of = |id: &Option<RecordId>| id.as_ref().map(|id| id.as_str().to_string());
    ItemView {
        name: item_name(db, item),
        style: style::item_style(db, item).label(),
        base_record: item.base.as_str().to_string(),
        prefix_record: record_of(&item.prefix),
        suffix_record: record_of(&item.suffix),
        relic_record: record_of(&item.relic),
        relic_bonus_record: record_of(&item.relic_bonus),
        relic2_record: item
            .atlantis
            .as_ref()
            .and_then(|extra| record_of(&extra.relic)),
        relic2_bonus_record: item
            .atlantis
            .as_ref()
            .and_then(|extra| record_of(&extra.bonus)),
        seed: item.seed.value(),
        stack_size: (item.stack_size > 1).then_some(item.stack_size),
        x: item.position.x,
        y: item.position.y,
        width,
        height,
        shards: style::relic_shards(db, item).map(|shards| ShardsView {
            have: shards.have,
            needed: shards.needed,
        }),
    }
}

/// Tooltip-grade stat lines for one item, as blocks of plain text —
/// the same blocks the GUI separates with rules.
pub fn item_stat_blocks(db: &GameCache, item: &Item) -> Vec<Vec<String>> {
    stats::item_details(db, item)
        .blocks
        .iter()
        .map(|block| block.iter().map(|line| line.text.clone()).collect())
        .collect()
}

/// Derives the mastery directory names from the learned-skill record
/// paths (`RECORDS\SKILLS\<mastery>\...` and expansion variants).
pub fn masteries_of(skills: &[univault_core::respec::LearnedSkill]) -> Vec<String> {
    let mut masteries: Vec<String> = Vec::new();
    for skill in skills.iter().filter(|skill| skill.mastery) {
        let Some(at) = skill.record.find(r"\SKILLS\") else {
            continue;
        };
        let rest = &skill.record[at + r"\SKILLS\".len()..];
        let Some((mastery, _)) = rest.split_once('\\') else {
            continue;
        };
        let pretty = titlecase(mastery);
        if !masteries.contains(&pretty) {
            masteries.push(pretty);
        }
    }
    masteries
}

fn titlecase(word: &str) -> String {
    let lower = word.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use univault_core::chr::{Item, ItemSeed, RecordId};
    use univault_core::respec::LearnedSkill;

    fn record(path: &str) -> RecordId {
        RecordId::parse(path.to_string()).unwrap()
    }

    #[test]
    fn item_name_falls_back_to_file_stems_without_a_cache() {
        let mut item = Item::bare(
            record(r"records\item\equipmentweapon\sword01.dbr"),
            ItemSeed::new(7),
        );
        item.prefix = Some(record(r"records\item\prefix\sharp.dbr"));
        assert_eq!(item_name(None, &item), "sharp sword01");
    }

    #[test]
    fn masteries_derive_from_skill_record_paths() {
        let skills = vec![
            LearnedSkill {
                record: r"RECORDS\SKILLS\EARTH\EARTHMASTERY.DBR".to_string(),
                level: 11,
                mastery: true,
            },
            LearnedSkill {
                record: r"RECORDS\XPACK3\SKILLS\RUNE\RUNEMASTERY.DBR".to_string(),
                level: 4,
                mastery: true,
            },
            LearnedSkill {
                record: r"RECORDS\SKILLS\DEFAULT\ATTACK.DBR".to_string(),
                level: 1,
                mastery: false,
            },
        ];
        assert_eq!(masteries_of(&skills), ["Earth", "Rune"]);
    }
}
