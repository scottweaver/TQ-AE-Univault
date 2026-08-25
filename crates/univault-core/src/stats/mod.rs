//! Item statistics for tooltips. The heavy lifting — turning record
//! variables into display lines — runs once per record at import
//! (`render`), lands in the cache, and [`item_details`] assembles the
//! per-item tooltip from cached blocks at display time, mirroring
//! `TQVaultAE`'s `ItemTooltip.FillToolTip` order (MIT).

// The ported engine mixes the game's small numeric domains (levels,
// counts, chances) across int/float exactly as the reference does,
// compares floats it read from identical file bytes, and keeps the
// reference's long decision functions whole for auditability.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines
)]

mod dictionary;
mod format;
mod render;

use std::collections::HashMap;

pub(crate) use render::{Renderer, StatBlock};
pub use render::{Requirement, StatLine};

use crate::cache::GameCache;
use crate::chr::{Item, RecordId};
use crate::gamedata::GameData;
use crate::stats::format::{Arg, parse_format};
use crate::stats::render::{COLOR_BROKEN, COLOR_COMMON, COLOR_MUNDANE, COLOR_RARE, COLOR_RELIC};
use crate::style::{self, ItemStyle};

/// The localization tags whose translated text the tooltip assembler
/// needs at display time; captured into the cache at import.
pub(crate) const RUNTIME_LABEL_TAGS: &[&str] = &[
    "MeetsRequirement",
    "LevelRequirement",
    "Strength",
    "Dexterity",
    "Intelligence",
    "tagRelicComplete",
    "tagAnimalPartComplete",
    "tagRelicBonus",
    "tagAnimalPartcompleteBonus",
    "tagRelicShard",
    "tagAnimalPart",
    "tagRelicRatio",
    "tagAnimalPartRatio",
    "xtagArtifactBonus",
    "xtagArtifactRecipe",
    "xtagArtifactReagents",
];

pub(crate) fn capture_runtime_labels(data: &GameData) -> HashMap<String, String> {
    RUNTIME_LABEL_TAGS
        .iter()
        .filter_map(|tag| {
            data.tag_text(tag)
                .map(|text| ((*tag).to_string(), format::strip_color_tags(text)))
        })
        .collect()
}

/// The fully assembled tooltip body for one item: name particles the
/// title should include, and the display blocks below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDetails {
    /// Translated `itemQualityTag` ("Ancient", …) — precedes the base
    /// name in the game's full item name.
    pub quality: Option<String>,
    /// Translated `itemStyleTag` for gear — follows the base name.
    pub style_word: Option<String>,
    /// Line blocks, in display order; render a separator between.
    pub blocks: Vec<Vec<StatLine>>,
}

/// The game's palette color for one of its `{^X}` tag letters.
#[must_use]
pub fn palette_color(tag: char) -> style::Rgb {
    style::palette_color(tag)
}

/// Assembles the tooltip body from cached stat blocks.
#[must_use]
pub fn item_details(db: &GameCache, item: &Item) -> ItemDetails {
    let assembler = Assembler { db, item };
    assembler.build()
}

struct Assembler<'a> {
    db: &'a GameCache,
    item: &'a Item,
}

impl Assembler<'_> {
    fn build(&self) -> ItemDetails {
        let item_style = style::item_style(Some(self.db), self.item);
        let base = self.stats(&self.item.base);
        let is_flavor_kind = matches!(
            item_style,
            ItemStyle::Potion
                | ItemStyle::Relic
                | ItemStyle::Scroll
                | ItemStyle::Parchment
                | ItemStyle::Quest
        );
        let mut blocks: Vec<Vec<StatLine>> = Vec::new();

        // Header extras: artifact class, relic completion, recipe tag.
        let mut header = Vec::new();
        if let Some(class) = base.and_then(|stats| stats.artifact_class.clone()) {
            header.push(StatLine {
                color: COLOR_BROKEN,
                text: class,
            });
        }
        if item_style == ItemStyle::Relic {
            header.push(self.relic_completion_line());
        }
        if item_style == ItemStyle::Formulae {
            let recipe = self
                .label("xtagArtifactRecipe")
                .unwrap_or("Recipe")
                .to_string();
            header.push(StatLine {
                color: COLOR_BROKEN,
                text: recipe,
            });
        }
        if !header.is_empty() {
            blocks.push(header);
        }

        // Flavor text.
        if is_flavor_kind && let Some(flavor) = base.and_then(|stats| stats.style_text.as_deref()) {
            blocks.push(
                format::wrap_words(flavor, 40)
                    .into_iter()
                    .map(|line| StatLine {
                        color: COLOR_COMMON,
                        text: line,
                    })
                    .collect(),
            );
        }

        // Formula reagents header.
        if item_style == ItemStyle::Formulae {
            let spec = self.label("xtagArtifactReagents").map_or_else(
                || parse_format("Required Reagents  ({%d0}/{%d1})"),
                parse_format,
            );
            blocks.push(vec![StatLine {
                color: COLOR_RELIC,
                text: spec.format(&[Arg::Number(0.0), Arg::Number(3.0)]),
            }]);
        }

        // Attributes: base, prefix, suffix — the classic display.
        let mut attributes = Vec::new();
        if let Some(stats) = base {
            let slot = if item_style == ItemStyle::Relic {
                (self.item.var1.max(1) - 1) as usize
            } else {
                0
            };
            attributes.extend(attr_slot(stats, slot).iter().cloned());
        }
        for affix in [self.item.prefix.as_ref(), self.item.suffix.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Some(stats) = self.stats(affix) {
                attributes.extend(attr_slot(stats, 0).iter().cloned());
            }
        }
        if !attributes.is_empty() {
            blocks.push(attributes);
        }

        // Formula: the artifact it builds.
        if let Some(formula) = base.and_then(|stats| stats.formula.as_ref()) {
            let mut section = vec![StatLine {
                color: 'T',
                text: formula.artifact_name.clone(),
            }];
            if let Some(class) = &formula.artifact_class {
                section.push(StatLine {
                    color: COLOR_BROKEN,
                    text: class.clone(),
                });
            }
            section.extend(formula.attr.iter().cloned());
            blocks.push(section);
        }

        // Socketed relics.
        for (relic, bonus, count) in [
            (
                self.item.relic.as_ref(),
                self.item.relic_bonus.as_ref(),
                self.item.var1,
            ),
            (
                self.item
                    .atlantis
                    .as_ref()
                    .and_then(|extra| extra.relic.as_ref()),
                self.item
                    .atlantis
                    .as_ref()
                    .and_then(|extra| extra.bonus.as_ref()),
                self.item.atlantis.as_ref().map_or(0, |extra| extra.var2),
            ),
        ] {
            if let Some(section) = self.socketed_relic_section(relic, bonus, count) {
                blocks.push(section);
            }
        }

        // Completion bonuses of artifacts and standalone relics.
        if matches!(item_style, ItemStyle::Artifact | ItemStyle::Relic)
            && let Some(bonus) = self.item.relic_bonus.as_ref()
            && let Some(stats) = self.stats(bonus)
        {
            let title = if item_style == ItemStyle::Artifact {
                self.label("xtagArtifactBonus")
                    .unwrap_or("Completion Bonus :")
            } else if self.is_charm(&self.item.base) {
                self.label("tagAnimalPartcompleteBonus")
                    .unwrap_or("Completion Bonus: ")
            } else {
                self.label("tagRelicBonus").unwrap_or("Completion Bonus: ")
            };
            let mut section = vec![StatLine {
                color: COLOR_RELIC,
                text: title.to_string(),
            }];
            section.extend(attr_slot(stats, 0).iter().cloned());
            blocks.push(section);
        }

        // Seed and expansion origin.
        let mut footer = vec![StatLine {
            color: COLOR_BROKEN,
            text: format!("Seed: {}", self.item.seed.value()),
        }];
        if let Some(origin) = self.dlc_origin() {
            footer.push(StatLine {
                color: COLOR_RARE,
                text: origin.to_string(),
            });
        }
        blocks.push(footer);

        // Set membership.
        if let Some(stats) = base
            && !stats.set_lines.is_empty()
        {
            blocks.push(stats.set_lines.clone());
        }

        // Requirements.
        let requirements = self.requirements();
        if !requirements.is_empty() {
            blocks.push(requirements);
        }

        ItemDetails {
            quality: base.and_then(|stats| stats.quality_text.clone()),
            style_word: (!is_flavor_kind)
                .then(|| base.and_then(|stats| stats.style_text.clone()))
                .flatten(),
            blocks,
        }
    }

    fn stats(&self, id: &RecordId) -> Option<&StatBlock> {
        self.db.entry(id)?.stats.as_ref()
    }

    fn is_charm(&self, id: &RecordId) -> bool {
        self.db.entry(id).is_some_and(|entry| match entry.kind {
            crate::style::ItemKind::RelicOrCharm { is_charm, .. } => is_charm,
            crate::style::ItemKind::Gear
            | crate::style::ItemKind::Artifact
            | crate::style::ItemKind::Formula
            | crate::style::ItemKind::Scroll
            | crate::style::ItemKind::Potion
            | crate::style::ItemKind::Quest => false,
        })
    }

    fn label(&self, tag: &str) -> Option<&str> {
        self.db.runtime_label(tag)
    }

    /// "Relic Shard - 3 / 5", or the completed label.
    fn relic_completion_line(&self) -> StatLine {
        let charm = self.is_charm(&self.item.base);
        let shards = style::relic_shards(Some(self.db), self.item);
        let (complete_tag, class_tag, ratio_tag) = if charm {
            (
                "tagAnimalPartComplete",
                "tagAnimalPart",
                "tagAnimalPartRatio",
            )
        } else {
            ("tagRelicComplete", "tagRelicShard", "tagRelicRatio")
        };
        let text = match shards {
            Some(shards) => match shards.needed {
                Some(needed) if shards.have >= needed => {
                    self.label(complete_tag).unwrap_or("Completed").to_string()
                }
                needed => {
                    let class = self.label(class_tag).unwrap_or("Relic").to_string();
                    let spec = self
                        .label(ratio_tag)
                        .map_or_else(|| parse_format("{%s0} - {%d1} / {%d2}"), parse_format);
                    spec.format(&[
                        Arg::Text(class),
                        Arg::Number(shards.have.max(1) as f32),
                        Arg::Number(needed.unwrap_or(0) as f32),
                    ])
                }
            },
            None => String::new(),
        };
        StatLine {
            color: COLOR_MUNDANE,
            text,
        }
    }

    fn socketed_relic_section(
        &self,
        relic: Option<&RecordId>,
        bonus: Option<&RecordId>,
        count: i32,
    ) -> Option<Vec<StatLine>> {
        let relic = relic?;
        let stats = self.stats(relic)?;
        let name = self
            .db
            .record_name(relic)
            .unwrap_or_else(|| relic.file_stem().to_string());
        let mut section = vec![StatLine {
            color: COLOR_RELIC,
            text: name,
        }];
        let slot = (count.max(1) - 1) as usize;
        section.extend(attr_slot(stats, slot).iter().cloned());
        if let Some(bonus_stats) = bonus.and_then(|id| self.stats(id)) {
            let title = if self.is_charm(relic) {
                self.label("tagAnimalPartcompleteBonus")
            } else {
                self.label("tagRelicBonus")
            };
            section.push(StatLine {
                color: COLOR_RELIC,
                text: title.unwrap_or("Completion Bonus: ").to_string(),
            });
            section.extend(attr_slot(bonus_stats, 0).iter().cloned());
        }
        Some(section)
    }

    fn dlc_origin(&self) -> Option<&'static str> {
        let rank = |id: Option<&RecordId>| -> u8 {
            let Some(id) = id else { return 0 };
            let upper = id.as_str().to_uppercase();
            let head = upper.trim_start_matches(['\\', '/']);
            if head.starts_with("XPACK4") {
                4
            } else if head.starts_with("XPACK3") {
                3
            } else if head.starts_with("XPACK2") {
                2
            } else {
                u8::from(head.starts_with("XPACK"))
            }
        };
        let highest = [
            Some(&self.item.base),
            self.item.prefix.as_ref(),
            self.item.suffix.as_ref(),
        ]
        .into_iter()
        .map(rank)
        .max()
        .unwrap_or(0);
        match highest {
            1 => Some("Immortal Throne"),
            2 => Some("Ragnarök"),
            3 => Some("Atlantis"),
            4 => Some("Eternal Embers"),
            _ => None,
        }
    }

    /// `GetRequirementVariables` + `GetRequirements`: explicit values
    /// max-merged across records, equations filling the gaps.
    fn requirements(&self) -> Vec<StatLine> {
        fn merge_max(merged: &mut Vec<(Requirement, i32)>, requirements: &[(Requirement, i32)]) {
            for (key, value) in requirements {
                match merged.iter_mut().find(|(existing, _)| existing == key) {
                    Some((_, existing_value)) => *existing_value = (*existing_value).max(*value),
                    None => merged.push((*key, *value)),
                }
            }
        }
        let base = self.stats(&self.item.base);
        let mut merged: Vec<(Requirement, i32)> = Vec::new();
        if let Some(stats) = base {
            merge_max(&mut merged, &stats.requirements);
        }
        if let Some(stats) = base {
            let total_att_count: i32 = [
                Some(&self.item.base),
                self.item.prefix.as_ref(),
                self.item.suffix.as_ref(),
            ]
            .into_iter()
            .flatten()
            .filter_map(|id| self.stats(id))
            .map(|block| block.attribute_count)
            .sum();
            for (key, expression) in &stats.equations {
                if merged.iter().any(|(existing, _)| existing == key) {
                    continue;
                }
                let item_level = f64::from(stats.item_level);
                let lookup = move |name: &str| match name {
                    "itemLevel" => Some(item_level),
                    "totalAttCount" => Some(f64::from(total_att_count)),
                    _ => None,
                };
                if let Some(value) = format::eval_equation(expression, &lookup) {
                    let value = value.ceil();
                    if value.is_finite() {
                        merged.push((*key, value as i32));
                    }
                }
            }
        }
        for source in [
            self.item.prefix.as_ref(),
            self.item.suffix.as_ref(),
            self.item.relic.as_ref(),
            self.item
                .atlantis
                .as_ref()
                .and_then(|extra| extra.relic.as_ref()),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(stats) = self.stats(source) {
                merge_max(&mut merged, &stats.requirements);
            }
        }
        merged.retain(|(_, value)| *value > 0);
        merged.sort_by_key(|(key, _)| key.text_tag());
        let spec = self
            .label("MeetsRequirement")
            .map_or_else(|| parse_format("?Required? {%s0}: {%.0f1}"), parse_format);
        merged
            .into_iter()
            .map(|(key, value)| {
                let name = self
                    .label(key.text_tag())
                    .map_or_else(|| format!("?{}?", key.text_tag()), str::to_string);
                StatLine {
                    color: COLOR_BROKEN,
                    text: spec.format(&[Arg::Text(name), Arg::Number(value as f32)]),
                }
            })
            .collect()
    }
}

fn attr_slot(stats: &StatBlock, slot: usize) -> &[StatLine] {
    stats
        .attr
        .get(slot.min(stats.attr.len().saturating_sub(1)))
        .map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arz::ArzFile;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::chr::{GridPos, Item, ItemSeed, RecordId};
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
                ("swordCostEquation", Values::Strings(&["broken ^ junk ("])),
            ],
        );
        builder.record(
            "records\\item\\broadsword.dbr",
            "WeaponMelee_Sword",
            &[
                ("itemNameTag", Values::Strings(&["tagSword"])),
                ("offensivePhysicalMin", Values::Floats(&[12.0])),
                ("offensivePhysicalMax", Values::Floats(&[31.0])),
                ("offensivePierceRatioMin", Values::Floats(&[15.0])),
                (
                    "characterBaseAttackSpeedTag",
                    Values::Strings(&["tagSpeedFast"]),
                ),
                ("characterDexterity", Values::Floats(&[24.0])),
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
            "records\\item\\animalrelics\\monkeypaw.dbr",
            "ItemCharm",
            &[
                ("description", Values::Strings(&["tagPaw"])),
                ("completedRelicLevel", Values::Ints(&[3])),
                ("offensiveColdMin", Values::Floats(&[4.0, 8.0, 12.0])),
                ("offensiveColdMax", Values::Floats(&[4.0, 8.0, 12.0])),
            ],
        );
        let mut text = TextDb::new();
        text.add_file(&text_file(
            "tagSword=Broadsword\ntagSharp=Sharp\ntagPaw=Monkey Paw\n\
             tagSpeedFast=Fast\n\
             DamageBasePhysical=Damage\n\
             DamageBasePierceRatio={%.0f0}% Pierce Ratio\n\
             DamageFire=Fire Damage\nDamageCold=Cold Damage\n\
             DamageSingleFormat={%.0f0}\n\
             DamageRangeFormat={%.0f0} ~ {%.0f1}\n\
             characterDexterity={%+.0f0} Dexterity\n\
             MeetsRequirement=Required {%s0}: {%.0f1}\n\
             Strength=Strength\nDexterity=Dexterity\nLevelRequirement=Player Level\n\
             tagAnimalPart=Charm\ntagAnimalPartRatio={%s0 - %d1 / %d2}\n\
             tagAnimalPartComplete=Completed\n",
        ));
        let data = GameData::from_parts(ArzFile::parse(builder.build()).unwrap(), text);
        let cache = data.build_cache(Vec::new());
        // Round-trip through the file format so the test also proves
        // stat blocks and labels survive serialization.
        GameCache::from_bytes(&cache.to_bytes()).unwrap()
    }

    fn texts(block: &[StatLine]) -> Vec<&str> {
        block.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn weapon_tooltip_renders_stats_and_requirements() {
        let db = sample_cache();
        let mut sword = item("records\\item\\broadsword.dbr");
        sword.prefix = Some(record_id("records\\item\\sharp.dbr"));
        let details = item_details(&db, &sword);

        let attributes = details
            .blocks
            .iter()
            .find(|block| block.iter().any(|line| line.text.contains("Damage")))
            .expect("attribute block");
        assert_eq!(
            texts(attributes),
            vec![
                "12 ~ 31 Damage",
                "15% Pierce Ratio",
                "Fast",
                "+24 Dexterity",
                "10 Fire Damage",
            ]
        );
        // Base weapon damage renders white; magical bonuses blue.
        assert_eq!(attributes[0].color, 'W');
        assert_eq!(attributes[3].color, 'B');
        assert_eq!(attributes[4].color, 'B');

        let requirements = details.blocks.last().expect("requirements block");
        assert_eq!(
            texts(requirements),
            vec![
                // Alphabetical by tag, dynamic values from the cost
                // equations at itemLevel 20 (ceil), explicit level 24.
                "Required Dexterity: 122",
                "Required Player Level: 24",
                "Required Strength: 141",
            ]
        );
    }

    #[test]
    fn relic_slots_track_shard_count() {
        let db = sample_cache();
        let mut charm = item("records\\item\\animalrelics\\monkeypaw.dbr");
        charm.var1 = 2;
        let details = item_details(&db, &charm);
        assert!(
            details
                .blocks
                .iter()
                .flatten()
                .any(|line| line.text == "8 Cold Damage"),
            "expected the two-shard value, got {:?}",
            details.blocks
        );
        assert!(
            details
                .blocks
                .iter()
                .flatten()
                .any(|line| line.text == "Charm - 2 / 3")
        );

        charm.var1 = 3;
        let details = item_details(&db, &charm);
        assert!(
            details
                .blocks
                .iter()
                .flatten()
                .any(|line| line.text == "12 Cold Damage")
        );
        assert!(
            details
                .blocks
                .iter()
                .flatten()
                .any(|line| line.text == "Completed")
        );
    }
}
