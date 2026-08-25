//! The attribute dictionary: every DBR variable the display engine
//! understands, classified by effect type with its grouping key and
//! ordering. Ported verbatim from `TQVaultAE`'s
//! `ItemAttributeProvider` (MIT) — the arrays below are that file's
//! tables and must stay in its order (suborder is positional).

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EffectType {
    ShieldEffect,
    SkillEffect,
    Offense,
    OffenseModifier,
    OffenseSlow,
    OffenseSlowModifier,
    Retaliation,
    RetaliationModifier,
    RetaliationSlow,
    RetaliationSlowModifier,
    Defense,
    Character,
    DamageQualifier,
    Other,
    Reagent,
}

impl EffectType {
    /// The reference enum's integer value — the base of the display
    /// ordering.
    pub(crate) fn order(self) -> i32 {
        match self {
            Self::ShieldEffect => 0,
            Self::SkillEffect => 1,
            Self::Offense => 2,
            Self::OffenseModifier => 3,
            Self::OffenseSlow => 4,
            Self::OffenseSlowModifier => 5,
            Self::Retaliation => 6,
            Self::RetaliationModifier => 7,
            Self::RetaliationSlow => 8,
            Self::RetaliationSlowModifier => 9,
            Self::Defense => 10,
            Self::Character => 11,
            Self::DamageQualifier => 12,
            Self::Other => 13,
            Self::Reagent => 14,
        }
    }
}

/// One dictionary entry: how a variable groups (`effect`), which
/// sub-variable of the group it is (`variable`: Min/Max/Chance/…),
/// and where its group sorts (`suborder`).
#[derive(Debug, Clone)]
pub(crate) struct AttributeData {
    pub(crate) effect_type: EffectType,
    pub(crate) full_attribute: String,
    pub(crate) effect: String,
    pub(crate) variable: String,
    pub(crate) suborder: i32,
}

const OTHER_EFFECTS: &[&str] = &[
    "characterBaseAttackSpeedTag",
    "levelRequirement",
    "offensiveGlobalChance",
    "retaliationGlobalChance",
    "racialBonusPercentDamage",
    "racialBonusAbsoluteDefense",
    "itemSkillName",
    "skillName",
];

const CHARACTER_EFFECTS: &[&str] = &[
    "characterStrength",
    "characterDexterity",
    "characterIntelligence",
    "characterLife",
    "characterMana",
    "characterStrengthModifier",
    "characterDexterityModifier",
    "characterIntelligenceModifier",
    "characterLifeModifier",
    "characterManaModifier",
    "characterIncreasedExperience",
    "characterRunSpeed",
    "characterAttackSpeed",
    "characterSpellCastSpeed",
    "characterRunSpeedModifier",
    "characterAttackSpeedModifier",
    "characterSpellCastSpeedModifier",
    "characterTotalSpeedModifier",
    "characterLifeRegen",
    "characterManaRegen",
    "characterLifeRegenModifier",
    "characterManaRegenModifier",
    "characterOffensiveAbility",
    "characterDefensiveAbility",
    "characterOffensiveAbilityModifier",
    "characterDefensiveAbilityModifier",
    "characterDefensiveBlockRecoveryReduction",
    "characterEnergyAbsorptionPercent",
    "characterDodgePercent",
    "characterDeflectProjectile",
    "characterDeflectProjectiles",
    "characterManaLimitReserve",
    "characterManaLimitReserveReduction",
    "characterManaLimitReserveModifier",
    "characterManaLimitReserveReductionModifier",
    "characterGlobalReqReduction",
    "characterWeaponStrengthReqReduction",
    "characterWeaponDexterityReqReduction",
    "characterWeaponIntelligenceReqReduction",
    "characterMeleeStrengthReqReduction",
    "characterMeleeDexterityReqReduction",
    "characterMeleeIntelligenceReqReduction",
    "characterHuntingStrengthReqReduction",
    "characterHuntingDexterityReqReduction",
    "characterHuntingIntelligenceReqReduction",
    "characterStaffStrengthReqReduction",
    "characterStaffDexterityReqReduction",
    "characterStaffIntelligenceReqReduction",
    "characterShieldStrengthReqReduction",
    "characterShieldIntelligenceReqReduction",
    "characterShieldDexterityReqReduction",
    "characterArmorStrengthReqReduction",
    "characterArmorDexterityReqReduction",
    "characterArmorIntelligenceReqReduction",
    "characterJewelryStrengthReqReduction",
    "characterJewelryDexterityReqReduction",
    "characterJewelryIntelligenceReqReduction",
    "characterLevelReqReduction",
    "skillCooldownReduction",
    "skillCooldownReductionModifier",
    "skillManaCostReduction",
    "skillManaCostReductionModifier",
    "projectileLaunchNumber",
    "projectilePiercingChance",
    "projectileLaunchRotation",
    "skillLifeBonus",
];

const DEFENSE_EFFECTS: &[&str] = &[
    "defensiveProtection",
    "defensiveProtectionModifier",
    "defensiveAbsorption",
    "defensiveAbsorptionModifier",
    "defensivePhysical",
    "defensivePhysicalDuration",
    "defensivePhysicalDurationChanceModifier",
    "defensivePhysicalDurationModifier",
    "defensivePhysicalModifier",
    "defensivePierce",
    "defensivePierceDuration",
    "defensivePierceDurationModifier",
    "defensivePierceModifier",
    "defensiveFire",
    "defensiveFireDuration",
    "defensiveFireDurationModifier",
    "defensiveFireModifier",
    "defensiveCold",
    "defensiveColdDuration",
    "defensiveColdDurationModifier",
    "defensiveColdModifier",
    "defensiveLightning",
    "defensiveLightningDuration",
    "defensiveLightningDurationModifier",
    "defensiveLightningModifier",
    "defensivePoison",
    "defensivePoisonDuration",
    "defensivePoisonDurationModifier",
    "defensivePoisonModifier",
    "defensiveLife",
    "defensiveLifeDuration",
    "defensiveLifeDurationModifier",
    "defensiveLifeModifier",
    "defensiveDisruption",
    "defensiveElemental",
    "defensiveElementalModifier",
    "defensiveElementalResistance",
    "defensiveSlowLifeLeach",
    "defensiveSlowLifeLeachDuration",
    "defensiveSlowLifeLeachDurationModifier",
    "defensiveSlowLifeLeachModifier",
    "defensiveSlowManaLeach",
    "defensiveSlowManaLeachDuration",
    "defensiveSlowManaLeachDurationModifier",
    "defensiveSlowManaLeachModifier",
    "defensiveBleeding",
    "defensiveBleedingDuration",
    "defensiveBleedingDurationModifier",
    "defensiveBleedingModifier",
    "defensiveBlockModifier",
    "defensiveReflect",
    "defensiveConfusion",
    "defensiveTaunt",
    "defensiveFear",
    "defensiveConvert",
    "defensiveTrap",
    "defensivePetrify",
    "defensiveFreeze",
    "defensiveStun",
    "defensiveStunModifier",
    "defensiveSleep",
    "defensiveSleepModifier",
    "defensiveManaBurnRatio",
    "defensivePercentCurrentLife",
    "defensiveTotalSpeedResistance",
    "damageAbsorption",
    "damageAbsorptionPercent",
    "defensiveTotalSpeedChance",
];

const OFFENSIVE_EFFECTS: &[&str] = &[
    "offensiveBasePhysical",
    "offensiveBaseCold",
    "offensiveBaseFire",
    "offensiveBasePoison",
    "offensiveBaseLightning",
    "offensiveBaseLife",
    "offensivePhysical",
    "offensivePierceRatio",
    "offensivePierce",
    "offensiveCold",
    "offensiveFire",
    "offensivePoison",
    "offensiveLightning",
    "offensiveLife",
    "offensivePercentCurrentLife",
    "offensiveManaBurn",
    "offensiveDisruption",
    "offensiveLifeLeech",
    "offensiveElemental",
    "offensiveTotalDamageReductionPercent",
    "offensiveTotalDamageReductionAbsolute",
    "offensiveTotalResistanceReductionPercent",
    "offensiveTotalResistanceReductionAbsolute",
    "offensiveFumble",
    "offensiveProjectileFumble",
    "offensiveConvert",
    "offensiveTaunt",
    "offensiveFear",
    "offensiveConfusion",
    "offensiveTrap",
    "offensiveFreeze",
    "offensivePetrify",
    "offensiveStun",
    "offensiveSleep",
    "offensiveBonusPhysical",
];

const OFFENSIVE_EFFECT_VARIABLES: &[&str] = &[
    "Min",
    "Max",
    "Chance",
    "XOR",
    "Global",
    "DurationMin",
    "DurationMax",
    "DrainMin",
    "DrainMax",
    "DamageRatio",
];

const OFFENSIVE_MODIFIER_EFFECTS: &[&str] = &[
    "offensivePhysicalModifier",
    "offensivePierceRatioModifier",
    "offensivePierceModifier",
    "offensiveColdModifier",
    "offensiveFireModifier",
    "offensivePoisonModifier",
    "offensiveLightningModifier",
    "offensiveLifeModifier",
    "offensiveManaBurnRatioAdder",
    "offensiveElementalModifier",
    "offensiveTotalDamageModifier",
    "offensiveStunModifier",
    "offensiveSleepModifier",
    "skillProjectileSpeedModifier",
    "sparkMaxNumber",
];

const OFFENSIVE_SLOW_EFFECTS: &[&str] = &[
    "offensiveSlowPhysical",
    "offensiveSlowBleeding",
    "offensiveSlowCold",
    "offensiveSlowFire",
    "offensiveSlowPoison",
    "offensiveSlowLightning",
    "offensiveSlowLife",
    "offensiveSlowTotalSpeed",
    "offensiveSlowAttackSpeed",
    "offensiveSlowRunSpeed",
    "offensiveSlowLifeLeach",
    "offensiveSlowManaLeach",
    "offensiveSlowOffensiveAbility",
    "offensiveSlowDefensiveAbility",
    "offensiveSlowOffensiveReduction",
    "offensiveSlowDefensiveReduction",
];

const OFFENSIVE_SLOW_EFFECT_VARIABLES: &[&str] = &[
    "Min",
    "Max",
    "DurationMin",
    "DurationMax",
    "Chance",
    "XOR",
    "Global",
];

const OFFENSIVE_SLOW_MODIFIER_EFFECT_VARIABLES: &[&str] =
    &["DurationModifier", "Modifier", "ModifierChance"];

const RETALIATION_EFFECTS: &[&str] = &[
    "retaliationPhysical",
    "retaliationPierceRatio",
    "retaliationPierce",
    "retaliationCold",
    "retaliationFire",
    "retaliationPoison",
    "retaliationLightning",
    "retaliationLife",
    "retaliationStun",
    "retaliationPercentCurrentLife",
    "retaliationElemental",
];

const RETALIATION_EFFECT_VARIABLES: &[&str] = &["Chance", "Max", "Min", "Global", "XOR"];

const RETALIATION_MODIFIER_EFFECTS: &[&str] = &[
    "retaliationPhysicalModifier",
    "retaliationPierceRatioModifier",
    "retaliationPierceModifier",
    "retaliationColdModifier",
    "retaliationFireModifier",
    "retaliationPoisonModifier",
    "retaliationLightningModifier",
    "retaliationLifeModifier",
    "retaliationStunModifier",
    "retaliationElementalModifier",
];

const RETALIATION_SLOW_EFFECTS: &[&str] = &[
    "retaliationSlowPhysical",
    "retaliationSlowBleeding",
    "retaliationSlowCold",
    "retaliationSlowFire",
    "retaliationSlowPoison",
    "retaliationSlowLightning",
    "retaliationSlowLife",
    "retaliationSlowTotalSpeed",
    "retaliationSlowAttackSpeed",
    "retaliationSlowRunSpeed",
    "retaliationSlowLifeLeach",
    "retaliationSlowManaLeach",
    "retaliationSlowOffensiveAbility",
    "retaliationSlowDefensiveAbility",
    "retaliationSlowOffensiveReduction",
];

const RETALIATION_SLOW_EFFECT_VARIABLES: &[&str] = &[
    "Chance",
    "Max",
    "Min",
    "DurationMax",
    "DurationMin",
    "Global",
    "XOR",
];

const RETALIATION_SLOW_MODIFIER_EFFECT_VARIABLES: &[&str] =
    &["Modifier", "ModifierChance", "DurationModifier"];

const REAGENTS: &[&str] = &["reagent1BaseName", "reagent2BaseName", "reagent3BaseName"];

const SKILL_EFFECTS: &[&str] = &[
    "skillManaCost",
    "skillActiveManaCost",
    "skillChanceWeight",
    "skillActiveDuration",
    "skillTargetRadius",
    "projectileExplosionRadius",
    "skillTargetAngle",
    "skillTargetNumber",
    "skillChargeDuration",
    "skillChargeLevel",
    "piercingProjectile",
    "headVelocity",
    "maxDistance",
    "tailVelocity",
    "refreshTime",
    "skillCooldownTime",
];

const DAMAGE_QUALIFIER_EFFECTS: &[&str] = &[
    "physicalDamageQualifier",
    "pierceDamageQualifier",
    "lightningDamageQualifier",
    "fireDamageQualifier",
    "coldDamageQualifier",
    "poisonDamageQualifier",
    "lifeDamageQualifier",
    "bleedingDamageQualifier",
    "elementalDamageQualifier",
];

const SHIELD_EFFECTS: &[&str] = &["defensiveBlock", "blockRecoveryTime"];

pub(crate) fn is_reagent(name: &str) -> bool {
    REAGENTS
        .iter()
        .any(|reagent| reagent.eq_ignore_ascii_case(name))
}

fn dictionary() -> &'static HashMap<String, AttributeData> {
    static DICTIONARY: OnceLock<HashMap<String, AttributeData>> = OnceLock::new();
    DICTIONARY.get_or_init(build_dictionary)
}

/// Looks up a variable name (case-insensitive) in the dictionary.
pub(crate) fn attribute_data(name: &str) -> Option<&'static AttributeData> {
    dictionary().get(&name.to_uppercase())
}

/// The fallback the reference constructs for unknown variables.
pub(crate) fn unknown_attribute(name: &str) -> AttributeData {
    AttributeData {
        effect_type: EffectType::Other,
        full_attribute: name.to_string(),
        effect: name.to_string(),
        variable: String::new(),
        suborder: 0,
    }
}

fn build_dictionary() -> HashMap<String, AttributeData> {
    let mut dict = HashMap::new();
    let mut add =
        |name: String, effect_type, full: String, effect: &str, variable: &str, suborder| {
            dict.insert(
                name.to_uppercase(),
                AttributeData {
                    effect_type,
                    full_attribute: full,
                    effect: effect.to_string(),
                    variable: variable.to_string(),
                    suborder,
                },
            );
        };

    for (suborder, effect) in OTHER_EFFECTS.iter().enumerate() {
        let suborder = suborder as i32;
        add(
            (*effect).to_string(),
            EffectType::Other,
            (*effect).to_string(),
            effect,
            "",
            suborder,
        );
    }
    let min_chance_family = [
        (EffectType::ShieldEffect, SHIELD_EFFECTS),
        (EffectType::Character, CHARACTER_EFFECTS),
        (EffectType::Defense, DEFENSE_EFFECTS),
        (EffectType::OffenseModifier, OFFENSIVE_MODIFIER_EFFECTS),
        (
            EffectType::RetaliationModifier,
            RETALIATION_MODIFIER_EFFECTS,
        ),
    ];
    for (effect_type, effects) in min_chance_family {
        for (suborder, effect) in effects.iter().enumerate() {
            let suborder = suborder as i32;
            add(
                (*effect).to_string(),
                effect_type,
                (*effect).to_string(),
                effect,
                "Min",
                suborder,
            );
            add(
                format!("{effect}CHANCE"),
                effect_type,
                (*effect).to_string(),
                effect,
                "Chance",
                suborder,
            );
        }
    }
    let variable_family = [
        (
            EffectType::Offense,
            OFFENSIVE_EFFECTS,
            OFFENSIVE_EFFECT_VARIABLES,
        ),
        (
            EffectType::OffenseSlow,
            OFFENSIVE_SLOW_EFFECTS,
            OFFENSIVE_SLOW_EFFECT_VARIABLES,
        ),
        (
            EffectType::OffenseSlowModifier,
            OFFENSIVE_SLOW_EFFECTS,
            OFFENSIVE_SLOW_MODIFIER_EFFECT_VARIABLES,
        ),
        (
            EffectType::Retaliation,
            RETALIATION_EFFECTS,
            RETALIATION_EFFECT_VARIABLES,
        ),
        (
            EffectType::RetaliationSlow,
            RETALIATION_SLOW_EFFECTS,
            RETALIATION_SLOW_EFFECT_VARIABLES,
        ),
        (
            EffectType::RetaliationSlowModifier,
            RETALIATION_SLOW_EFFECTS,
            RETALIATION_SLOW_MODIFIER_EFFECT_VARIABLES,
        ),
    ];
    for (effect_type, effects, variables) in variable_family {
        for (suborder, effect) in effects.iter().enumerate() {
            let suborder = suborder as i32;
            for variable in variables {
                add(
                    format!("{effect}{variable}"),
                    effect_type,
                    format!("{effect}{variable}"),
                    effect,
                    variable,
                    suborder,
                );
            }
        }
    }
    for (suborder, effect) in DAMAGE_QUALIFIER_EFFECTS.iter().enumerate() {
        let suborder = suborder as i32;
        add(
            (*effect).to_string(),
            EffectType::DamageQualifier,
            (*effect).to_string(),
            effect,
            "",
            suborder,
        );
    }
    for (suborder, effect) in REAGENTS.iter().enumerate() {
        let suborder = suborder as i32;
        add(
            (*effect).to_string(),
            EffectType::Reagent,
            (*effect).to_string(),
            effect,
            "",
            suborder,
        );
    }
    for (suborder, effect) in SKILL_EFFECTS.iter().enumerate() {
        let suborder = suborder as i32;
        add(
            (*effect).to_string(),
            EffectType::SkillEffect,
            (*effect).to_string(),
            effect,
            "",
            suborder,
        );
    }
    dict
}

/// The text tag naming an effect — the reference's
/// `GetAttributeTextTag` string surgery per effect type.
pub(crate) fn attribute_text_tag(data: &AttributeData) -> String {
    let effect = data.effect.as_str();
    if effect.is_empty() {
        return String::new();
    }
    let upper = effect.to_uppercase();
    let tail = |from: usize| &effect[from.min(effect.len())..];
    match data.effect_type {
        EffectType::ShieldEffect => {
            if upper == "BLOCKRECOVERYTIME" {
                "ShieldBlockRecoveryTime".to_string()
            } else {
                format!("Defense{}", tail(9))
            }
        }
        EffectType::Character => {
            if upper == "CHARACTERGLOBALREQREDUCTION" {
                // The game's own misspelled tag.
                "CharcterItemGlobalReduction".to_string()
            } else if upper == "CHARACTERDEFLECTPROJECTILE" {
                "CharacterDeflectProjectiles".to_string()
            } else {
                effect.to_string()
            }
        }
        EffectType::Defense => match upper.as_str() {
            "DEFENSIVETOTALSPEEDCHANCE" | "DEFENSIVEABSORPTION" => effect.to_string(),
            "DEFENSIVEPROTECTION" => "DefenseAbsorptionProtection".to_string(),
            "DEFENSIVESLEEP" => format!("xtagDefense{}", tail(9)),
            "DEFENSIVETOTALSPEEDRESISTANCE" => "xtagTotalSpeedResistance".to_string(),
            "DAMAGEABSORPTION" => "SkillDamageAbsorption".to_string(),
            "DAMAGEABSORPTIONPERCENT" => "SkillDamageAbsorptionPercent".to_string(),
            _ if upper.starts_with("DEFENSIVESLOW") => format!("Defense{}", tail(13)),
            _ => format!("Defense{}", tail(9)),
        },
        EffectType::Offense => match upper.as_str() {
            _ if upper.starts_with("SKILL") => effect.to_string(),
            "OFFENSIVEPHYSICAL" | "OFFENSIVEBASEPHYSICAL" => "DamageBasePhysical".to_string(),
            "OFFENSIVEPIERCERATIO" => "DamageBasePierceRatio".to_string(),
            "OFFENSIVEMANABURN" => "DamageManaDrain".to_string(),
            "OFFENSIVESLEEP" => "xtagDamageSleep".to_string(),
            "OFFENSIVEFUMBLE" => "DamageDurationFumble".to_string(),
            "OFFENSIVEPROJECTILEFUMBLE" => "DamageDurationProjectileFumble".to_string(),
            _ if upper.starts_with("OFFENSIVEBASE") => format!("Damage{}", tail(13)),
            _ => format!("Damage{}", tail(9)),
        },
        EffectType::OffenseModifier => match upper.as_str() {
            "OFFENSIVEMANABURNRATIOADDER" => "DamageModifierManaBurn".to_string(),
            "OFFENSIVESLEEPMODIFIER" => "xtagDamageModifierSleep".to_string(),
            "OFFENSIVETOTALDAMAGEMODIFIER" => "xtagDamageModifierTotalDamage".to_string(),
            "SPARKMAXNUMBER" => "xtagSparkMaxNumber".to_string(),
            _ if upper.starts_with("SKILL") => effect.to_string(),
            _ => {
                let keep = effect.len().saturating_sub(17);
                format!("DamageModifier{}", &effect[9..(9 + keep).min(effect.len())])
            }
        },
        EffectType::OffenseSlow => format!("DamageDuration{}", tail(13)),
        EffectType::OffenseSlowModifier => format!("DamageDurationModifier{}", tail(13)),
        EffectType::RetaliationModifier => {
            let keep = effect.len().saturating_sub(19);
            format!(
                "RetaliationModifier{}",
                &effect[11..(11 + keep).min(effect.len())]
            )
        }
        EffectType::RetaliationSlow => format!("RetaliationDuration{}", tail(15)),
        EffectType::RetaliationSlowModifier => {
            format!("RetaliationDurationModifier{}", tail(15))
        }
        EffectType::SkillEffect => {
            const UNCHANGED: &[&str] = &[
                "SkillChanceWeight",
                "headVelocity",
                "maxDistance",
                "tailVelocity",
            ];
            if UNCHANGED.iter().any(|tag| tag.eq_ignore_ascii_case(effect)) {
                effect.to_string()
            } else {
                match upper.as_str() {
                    "SKILLCHARGEDURATION" => "SkillChargeDurationMod".to_string(),
                    "SKILLCHARGELEVEL" => "SkillChargeDuration".to_string(),
                    "PIERCINGPROJECTILE" => "ProjectilePiercingChance".to_string(),
                    "REFRESHTIME" => "tagSkillRefreshTime".to_string(),
                    _ if upper.starts_with("PROJECTILE") => tail(10).to_string(),
                    _ => tail(5).to_string(),
                }
            }
        }
        EffectType::Retaliation | EffectType::Other | EffectType::Reagent => effect.to_string(),
        EffectType::DamageQualifier => data.full_attribute.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionary_classifies_grouped_variables() {
        let data = attribute_data("offensiveFireMin").unwrap();
        assert_eq!(data.effect_type, EffectType::Offense);
        assert_eq!(data.effect, "offensiveFire");
        assert_eq!(data.variable, "Min");
        let chance = attribute_data("OFFENSIVEFIRECHANCE").unwrap();
        assert_eq!(chance.variable, "Chance");
        assert_eq!(chance.suborder, data.suborder);
        assert!(attribute_data("no_such_variable").is_none());
    }

    #[test]
    fn slow_effects_have_plain_and_modifier_entries() {
        let plain = attribute_data("offensiveSlowColdMin").unwrap();
        assert_eq!(plain.effect_type, EffectType::OffenseSlow);
        let modifier = attribute_data("offensiveSlowColdModifier").unwrap();
        assert_eq!(modifier.effect_type, EffectType::OffenseSlowModifier);
        assert_eq!(modifier.variable, "Modifier");
    }

    #[test]
    fn text_tags_match_reference_surgery() {
        let tag = |name: &str| attribute_text_tag(attribute_data(name).unwrap());
        assert_eq!(tag("offensiveFireMin"), "DamageFire");
        assert_eq!(tag("offensivePhysicalMin"), "DamageBasePhysical");
        assert_eq!(tag("offensiveBaseColdMin"), "DamageCold");
        assert_eq!(tag("offensivePierceRatioMin"), "DamageBasePierceRatio");
        assert_eq!(tag("offensiveFireModifier"), "DamageModifierFire");
        assert_eq!(tag("offensiveSlowPoisonMin"), "DamageDurationPoison");
        assert_eq!(
            tag("offensiveSlowPoisonModifier"),
            "DamageDurationModifierPoison"
        );
        assert_eq!(tag("retaliationFireMin"), "retaliationFire");
        assert_eq!(tag("retaliationFireModifier"), "RetaliationModifierFire");
        assert_eq!(tag("retaliationSlowColdMin"), "RetaliationDurationCold");
        assert_eq!(tag("defensiveFire"), "DefenseFire");
        assert_eq!(tag("defensiveProtection"), "DefenseAbsorptionProtection");
        assert_eq!(tag("defensiveSlowLifeLeach"), "DefenseLifeLeach");
        assert_eq!(tag("defensiveBlock"), "DefenseBlock");
        assert_eq!(tag("blockRecoveryTime"), "ShieldBlockRecoveryTime");
        assert_eq!(tag("characterStrength"), "characterStrength");
        assert_eq!(
            tag("characterGlobalReqReduction"),
            "CharcterItemGlobalReduction"
        );
        assert_eq!(tag("skillManaCost"), "ManaCost");
        assert_eq!(tag("projectileExplosionRadius"), "ExplosionRadius");
        assert_eq!(tag("skillChargeLevel"), "SkillChargeDuration");
        assert_eq!(
            tag("characterDeflectProjectile"),
            "CharacterDeflectProjectiles"
        );
        assert_eq!(tag("skillChargeDuration"), "SkillChargeDurationMod");
        assert_eq!(tag("piercingProjectile"), "ProjectilePiercingChance");
        assert_eq!(tag("refreshTime"), "tagSkillRefreshTime");
        assert_eq!(tag("headVelocity"), "headVelocity");
        assert_eq!(tag("offensiveManaBurnMin"), "DamageManaDrain");
        assert_eq!(tag("offensiveSleepMin"), "xtagDamageSleep");
        assert_eq!(tag("offensiveFumbleMin"), "DamageDurationFumble");
        assert_eq!(
            tag("offensiveProjectileFumbleMin"),
            "DamageDurationProjectileFumble"
        );
        assert_eq!(tag("offensiveManaBurnRatioAdder"), "DamageModifierManaBurn");
        assert_eq!(tag("offensiveSleepModifier"), "xtagDamageModifierSleep");
        assert_eq!(
            tag("offensiveTotalDamageModifier"),
            "xtagDamageModifierTotalDamage"
        );
        assert_eq!(tag("sparkMaxNumber"), "xtagSparkMaxNumber");
        assert_eq!(
            tag("skillProjectileSpeedModifier"),
            "skillProjectileSpeedModifier"
        );
        assert_eq!(tag("defensiveSleep"), "xtagDefenseSleep");
        assert_eq!(
            tag("defensiveTotalSpeedResistance"),
            "xtagTotalSpeedResistance"
        );
        assert_eq!(
            tag("defensiveTotalSpeedChance"),
            "defensiveTotalSpeedChance"
        );
        assert_eq!(tag("damageAbsorption"), "SkillDamageAbsorption");
        assert_eq!(
            tag("damageAbsorptionPercent"),
            "SkillDamageAbsorptionPercent"
        );
        assert_eq!(
            tag("retaliationSlowColdModifier"),
            "RetaliationDurationModifierCold"
        );
        assert_eq!(tag("physicalDamageQualifier"), "physicalDamageQualifier");
        assert_eq!(tag("reagent1BaseName"), "reagent1BaseName");
        assert_eq!(tag("levelRequirement"), "levelRequirement");
    }
}
