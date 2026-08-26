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

/// The display lines of one record's own attribute block (its first
/// shard slot) — lets shells preview a relic completion bonus.
#[must_use]
pub fn record_lines(db: &GameCache, id: &RecordId) -> Vec<StatLine> {
    db.entry(id)
        .and_then(|entry| entry.stats.as_ref())
        .and_then(|stats| stats.attr.first())
        .cloned()
        .unwrap_or_default()
}

/// Assembles the tooltip body from cached stat blocks.
#[must_use]
pub fn item_details(db: &GameCache, item: &Item) -> ItemDetails {
    let assembler = Assembler { db, item };
    assembler.build()
}

/// The item's effective requirements, typed: explicit values
/// max-merged across base, affixes, and socketed relics, with the
/// item-cost equations filling the gaps (`GetRequirementVariables` +
/// `GetRequirements`). Sorted by requirement tag; zero-valued
/// entries dropped.
#[must_use]
pub fn item_requirements(db: &GameCache, item: &Item) -> Vec<(Requirement, i32)> {
    fn merge_max(merged: &mut Vec<(Requirement, i32)>, requirements: &[(Requirement, i32)]) {
        for (key, value) in requirements {
            match merged.iter_mut().find(|(existing, _)| existing == key) {
                Some((_, existing_value)) => *existing_value = (*existing_value).max(*value),
                None => merged.push((*key, *value)),
            }
        }
    }
    let stats_of = |id: &RecordId| db.entry(id).and_then(|entry| entry.stats.as_ref());
    let base = stats_of(&item.base);
    let mut merged: Vec<(Requirement, i32)> = Vec::new();
    if let Some(stats) = base {
        merge_max(&mut merged, &stats.requirements);
        let total_att_count: i32 = [Some(&item.base), item.prefix.as_ref(), item.suffix.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(stats_of)
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
        item.prefix.as_ref(),
        item.suffix.as_ref(),
        item.relic.as_ref(),
        item.atlantis
            .as_ref()
            .and_then(|extra| extra.relic.as_ref()),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(stats) = stats_of(source) {
            merge_max(&mut merged, &stats.requirements);
        }
    }
    merged.retain(|(_, value)| *value > 0);
    merged.sort_by_key(|(key, _)| key.text_tag());
    merged
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
        crate::query::expansion_origin(self.item).map(crate::query::Expansion::label)
    }

    /// `GetRequirementVariables` + `GetRequirements`, formatted.
    fn requirements(&self) -> Vec<StatLine> {
        let spec = self
            .label("MeetsRequirement")
            .map_or_else(|| parse_format("?Required? {%s0}: {%.0f1}"), parse_format);
        item_requirements(self.db, self.item)
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

pub(crate) fn attr_slot(stats: &StatBlock, slot: usize) -> &[StatLine] {
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
            "records\\item\\stormblade.dbr",
            "WeaponMelee_Sword",
            &[
                ("itemNameTag", Values::Strings(&["tagStorm"])),
                ("offensivePhysicalMin", Values::Floats(&[10.0])),
                ("offensivePhysicalMax", Values::Floats(&[10.0])),
                (
                    "itemSkillName",
                    Values::Strings(&["records\\skills\\stormnova.dbr"]),
                ),
                ("itemSkillLevel", Values::Ints(&[2])),
                (
                    "itemSkillAutoController",
                    Values::Strings(&["records\\controllers\\onhit.dbr"]),
                ),
            ],
        );
        builder.record(
            "records\\skills\\stormnova.dbr",
            "Skill_AttackRadius",
            &[
                ("skillDisplayName", Values::Strings(&["tagNova"])),
                ("skillBaseDescription", Values::Strings(&["tagNovaDesc"])),
                ("offensiveLightningMin", Values::Floats(&[20.0, 40.0])),
                ("offensiveLightningMax", Values::Floats(&[20.0, 40.0])),
            ],
        );
        builder.record(
            "records\\controllers\\onhit.dbr",
            "SkillControl",
            &[("triggerType", Values::Strings(&["HitByEnemy"]))],
        );
        builder.record(
            "records\\item\\augmentring.dbr",
            "ArmorJewelry_Ring",
            &[
                (
                    "augmentSkillName1",
                    Values::Strings(&["records\\skills\\warwind.dbr"]),
                ),
                ("augmentSkillLevel1", Values::Ints(&[2])),
                (
                    "augmentMasteryName1",
                    Values::Strings(&["records\\skills\\warfare.dbr"]),
                ),
                ("augmentMasteryLevel1", Values::Ints(&[3])),
                ("augmentAllLevel", Values::Ints(&[1])),
                ("racialBonusPercentDamage", Values::Floats(&[25.0])),
                ("racialBonusRace", Values::Strings(&["Beastman"])),
            ],
        );
        builder.record(
            "records\\skills\\warwind.dbr",
            "Skill_Attack",
            &[("skillDisplayName", Values::Strings(&["tagWarWind"]))],
        );
        builder.record(
            "records\\skills\\warfare.dbr",
            "Skill_Mastery",
            &[("skillDisplayName", Values::Strings(&["tagWarfare"]))],
        );
        builder.record(
            "records\\item\\chaosaxe.dbr",
            "WeaponMelee_Axe",
            &[
                ("offensiveGlobalChance", Values::Floats(&[10.0])),
                ("offensiveFireMin", Values::Floats(&[5.0])),
                ("offensiveFireMax", Values::Floats(&[5.0])),
                ("offensiveFireGlobal", Values::Bools(&[true])),
                ("offensiveFireXOR", Values::Bools(&[true])),
                ("offensiveColdMin", Values::Floats(&[7.0])),
                ("offensiveColdMax", Values::Floats(&[7.0])),
                ("offensiveColdGlobal", Values::Bools(&[true])),
                ("offensiveColdXOR", Values::Bools(&[true])),
            ],
        );
        builder.record(
            "records\\item\\wardhelm.dbr",
            "ArmorProtective_Head",
            &[
                ("damageAbsorption", Values::Floats(&[50.0])),
                ("physicalDamageQualifier", Values::Bools(&[true])),
                ("fireDamageQualifier", Values::Bools(&[true])),
            ],
        );
        builder.record(
            "records\\item\\pawbonus.dbr",
            "LootRandomizer",
            &[("offensiveColdModifier", Values::Floats(&[10.0]))],
        );
        builder.record(
            "records\\item\\venomblade.dbr",
            "WeaponMelee_Sword",
            &[
                ("offensiveSlowPoisonMin", Values::Floats(&[30.0])),
                ("offensiveSlowPoisonMax", Values::Floats(&[30.0])),
                ("offensiveSlowPoisonDurationMin", Values::Floats(&[3.0])),
                ("offensiveSlowPoisonDurationMax", Values::Floats(&[6.0])),
                ("offensiveSlowPoisonChance", Values::Floats(&[15.0])),
                ("offensiveSlowPoisonModifier", Values::Floats(&[20.0])),
                (
                    "offensiveSlowPoisonDurationModifier",
                    Values::Floats(&[30.0]),
                ),
                ("offensiveSlowPoisonModifierChance", Values::Floats(&[25.0])),
            ],
        );
        builder.record(
            "records\\item\\summonstaff.dbr",
            "WeaponMagical_Staff",
            &[
                (
                    "itemSkillName",
                    Values::Strings(&["records\\skills\\summonwolf.dbr"]),
                ),
                (
                    "itemSkillAutoController",
                    Values::Strings(&["records\\controllers\\onhit.dbr"]),
                ),
            ],
        );
        builder.record(
            "records\\skills\\summonwolf.dbr",
            "Skill_SpawnPet",
            &[
                ("skillDisplayName", Values::Strings(&["tagWolf"])),
                ("petLimit", Values::Ints(&[2])),
                (
                    "spawnObjects",
                    Values::Strings(&["records\\pets\\wolf.dbr"]),
                ),
                ("spawnObjectsTimeToLive", Values::Floats(&[30.0])),
            ],
        );
        builder.record(
            "records\\pets\\wolf.dbr",
            "Pet",
            &[
                ("description", Values::Strings(&["tagWolfName"])),
                ("characterLife", Values::Floats(&[500.0])),
                ("handHitDamageMin", Values::Floats(&[10.0])),
                ("handHitDamageMax", Values::Floats(&[20.0])),
                (
                    "skillName0",
                    Values::Strings(&["records\\skills\\bite.dbr"]),
                ),
                ("skillLevel0", Values::Ints(&[1])),
            ],
        );
        builder.record(
            "records\\skills\\bite.dbr",
            "Skill_Attack",
            &[
                ("skillDisplayName", Values::Strings(&["tagBite"])),
                ("offensivePhysicalMin", Values::Floats(&[5.0])),
                ("offensivePhysicalMax", Values::Floats(&[5.0])),
            ],
        );
        builder.record(
            "records\\item\\artifacts\\stormeye.dbr",
            "ItemArtifact",
            &[
                ("description", Values::Strings(&["tagStormEye"])),
                ("artifactClassification", Values::Strings(&["Greater"])),
                ("characterDexterity", Values::Floats(&[15.0])),
            ],
        );
        builder.record(
            "records\\item\\equipmentshield\\default\\pinebuckler.dbr",
            "WeaponArmor_Shield",
            &[
                ("description", Values::Strings(&["Buckler Ornate"])),
                ("itemQualityTag", Values::Strings(&["Pine"])),
                ("defensiveBlock", Values::Floats(&[36.0])),
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
             tagSpeedFast=Fast\ntagStorm=Stormblade\n\
             DamageBasePhysical=Damage\n\
             DamageBasePierceRatio={%.0f0}% Pierce Ratio\n\
             DamageFire=Fire Damage\nDamageCold=Cold Damage\n\
             DamageLightning=Lightning Damage\n\
             DamageModifierCold={%+.0f0}% Cold Damage\n\
             DamageSingleFormat={%.0f0}\n\
             DamageRangeFormat={%.0f0} ~ {%.0f1}\n\
             characterDexterity={%+.0f0} Dexterity\n\
             MeetsRequirement=Required {%s0}: {%.0f1}\n\
             Strength=Strength\nDexterity=Dexterity\nLevelRequirement=Player Level\n\
             tagAnimalPart=Charm\ntagAnimalPartRatio={%s0 - %d1 / %d2}\n\
             tagAnimalPartComplete=Completed\n\
             tagAnimalPartcompleteBonus=Charm Bonus:\n\
             xtagArtifactBonus=Completion Bonus :\n\
             xtagArtifactClass02=Greater Artifact\n\
             tagStormEye=Eye of the Storm\n\
             tagNova=Storm Nova\ntagNovaDesc=Unleashes a nova of storm.\n\
             xtagAutoSkillCondition03=Activated upon taking damage\n\
             MenuLevel=Level:   {%d0}\n\
             tagItemGrantSkill=Grants Skill :\n\
             DamageDurationPoison={%.0f0} Poison Damage\n\
             DamageDurationModifierPoison={%+.0f0}% Poison Damage\n\
             ChanceOfTag={%.1f0}% Chance of\n\
             DamageRangeFormatTime=over {%.1f0} - {%.1f1} Seconds\n\
             ImprovedTimeFormat=with {%+.0f0}% Improved Duration\n\
             tagWolf=Summon Wolf\ntagWolfName=Wolf\ntagBite=Bite\n\
             SkillPetLimit={%d0} Summon Limit\n\
             SkillPetDescriptionHeading={%s0} Attributes:\n\
             tagSkillPetTimeToLive=Life Time {%.1f0} Seconds\n\
             SkillPetDescriptionHealth={%.0f0} Health\n\
             tagSkillPetAbilities={%s0} Abilities:\n\
             SkillPetDescriptionDamageMinMax={%.0f0} - {%.0f1} Damage\n\
             tagWarWind=War Wind\ntagWarfare=Warfare\n\
             ItemSkillIncrement={+%d0} to {%s1}\n\
             ItemMasteryIncrement={+%d0} to all skills in {%s1}\n\
             ItemAllSkillIncrement={+%d0} to all Skills\n\
             racialBonusRaceBeastman=Beastmen\n\
             RacialBonusPercentDamage={%.0f0}% Damage to {%s1}\n\
             GlobalPercentChanceOfOneTag={%.0f0}% Chance for one of the following:\n\
             SkillDamageAbsorption={%.0f0} Damage Absorption\n\
             tagDamageAbsorptionTitle=Protects Against :\n\
             tagQualifyingDamagePhysical=Physical\ntagQualifyingDamageFire=Fire\n\
             formatQualifyingDamage={     %s0}\n",
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

    fn attributes_of(details: &ItemDetails, needle: &str) -> Vec<String> {
        details
            .blocks
            .iter()
            .find(|block| block.iter().any(|line| line.text.contains(needle)))
            .unwrap_or_else(|| panic!("no block containing {needle:?}: {:?}", details.blocks))
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    #[test]
    fn granted_skills_render_name_description_and_leveled_effects() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\stormblade.dbr"));
        assert_eq!(
            attributes_of(&details, "Grants Skill"),
            vec![
                "10 Damage",
                "",
                "Grants Skill :",
                "Storm Nova Activated upon taking damage",
                "    Unleashes a nova of storm.",
                "Level:   2",
                "",
                // itemSkillLevel 2 selects the second value slot.
                "40 Lightning Damage",
            ]
        );
    }

    #[test]
    fn slow_damage_renders_chance_duration_and_modifier_lines() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\venomblade.dbr"));
        assert_eq!(
            attributes_of(&details, "Poison"),
            vec![
                // Amount scales by the duration (30 × 3 seconds).
                "15.0% Chance of 90 Poison Damage over 3.0 - 6.0 Seconds",
                "25.0% Chance of +20% Poison Damage with +30% Improved Duration",
            ]
        );
    }

    #[test]
    fn summon_skills_render_pet_stats_and_abilities() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\summonstaff.dbr"));
        let lines = attributes_of(&details, "Summon Limit");
        for expected in [
            "2 Summon Limit",
            "Wolf Attributes:",
            "Life Time 30.0 Seconds",
            "500 Health",
            "Wolf Abilities:",
            "10 - 20 Damage",
            "Bite",
            "5 Damage",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "missing {expected:?} in {lines:?}"
            );
        }
    }

    #[test]
    fn augments_and_racial_bonuses_render() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\augmentring.dbr"));
        let lines = attributes_of(&details, "Skills");
        for expected in [
            "+2 to War Wind",
            "+3 to all skills in Warfare",
            "+1 to all Skills",
            "25% Damage to Beastmen",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "missing {expected:?} in {lines:?}"
            );
        }
    }

    #[test]
    fn global_xor_chance_groups_render_indented_under_one_of() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\chaosaxe.dbr"));
        assert_eq!(
            attributes_of(&details, "Chance for one"),
            vec![
                "10% Chance for one of the following:",
                // Cold sorts before fire by dictionary suborder.
                "    7 Cold Damage",
                "    5 Fire Damage",
            ]
        );
    }

    #[test]
    fn damage_qualifiers_render_under_a_single_title() {
        let db = sample_cache();
        let details = item_details(&db, &item("records\\item\\wardhelm.dbr"));
        assert_eq!(
            attributes_of(&details, "Protects Against"),
            vec![
                "50 Damage Absorption",
                "Protects Against :",
                "     Physical",
                "     Fire",
            ]
        );
    }

    #[test]
    fn socketed_relics_render_their_section_and_bonus() {
        let db = sample_cache();
        let mut sword = item("records\\item\\broadsword.dbr");
        sword.relic = Some(record_id("records\\item\\animalrelics\\monkeypaw.dbr"));
        sword.relic_bonus = Some(record_id("records\\item\\pawbonus.dbr"));
        sword.var1 = 3;
        sword.atlantis = Some(crate::chr::AtlantisRelic {
            relic: Some(record_id("records\\item\\animalrelics\\monkeypaw.dbr")),
            bonus: None,
            var2: 1,
        });
        let details = item_details(&db, &sword);
        assert_eq!(
            attributes_of(&details, "Charm Bonus:"),
            vec![
                "Monkey Paw",
                // var1 = 3 selects the third shard slot.
                "12 Cold Damage",
                "Charm Bonus:",
                "+10% Cold Damage",
            ]
        );
        let second: Vec<_> = details
            .blocks
            .iter()
            .filter(|block| block.first().is_some_and(|line| line.text == "Monkey Paw"))
            .collect();
        assert_eq!(second.len(), 2, "expected two relic sections");
        assert_eq!(second[1][1].text, "4 Cold Damage");
    }

    #[test]
    fn artifacts_render_class_and_completion_bonus() {
        let db = sample_cache();
        let mut artifact = item("records\\item\\artifacts\\stormeye.dbr");
        artifact.relic_bonus = Some(record_id("records\\item\\pawbonus.dbr"));
        let details = item_details(&db, &artifact);
        assert!(
            details
                .blocks
                .iter()
                .flatten()
                .any(|line| line.text == "Greater Artifact")
        );
        assert_eq!(
            attributes_of(&details, "Completion Bonus :"),
            vec!["Completion Bonus :", "+10% Cold Damage"]
        );
    }

    #[test]
    fn expansion_origin_comes_from_the_record_path() {
        let db = sample_cache();
        for (path, origin) in [
            ("records\\xpack\\item\\thing.dbr", "Immortal Throne"),
            ("records\\xpack2\\item\\thing.dbr", "Ragnarök"),
            ("records\\xpack3\\item\\thing.dbr", "Atlantis"),
            ("records\\xpack4\\item\\thing.dbr", "Eternal Embers"),
        ] {
            let details = item_details(&db, &item(path));
            assert!(
                details
                    .blocks
                    .iter()
                    .flatten()
                    .any(|line| line.text == origin),
                "expected {origin:?} for {path}"
            );
        }
        let base_game = item_details(&db, &item("records\\item\\plainsword.dbr"));
        for line in base_game.blocks.iter().flatten() {
            assert!(!line.text.contains("Throne"));
        }
    }

    #[test]
    fn default_records_use_literal_name_and_quality_text() {
        let db = sample_cache();
        let buckler = item("records\\item\\equipmentshield\\default\\pinebuckler.dbr");
        assert_eq!(
            db.record_name(&buckler.base),
            Some("Buckler Ornate".to_string())
        );
        let details = item_details(&db, &buckler);
        assert_eq!(details.quality.as_deref(), Some("Pine"));
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
