//! Distills mastery skill trees out of the game database: localized
//! names, descriptions, tiers, and per-level effect arrays, with the
//! buff / pet / sub-skill records they reference pulled in
//! transitively. Consumed by the `skilltree` export example and the
//! MCP server; the documents embed extracted game data and stay
//! local per the derived-data rule in ARCHITECTURE.md.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::arz::{ArzFile, DbRecord, DbValues, normalize};
use crate::chr::RecordId;
use crate::gamedata::GameData;

/// The database a skill tree reads: the game's own records, with an
/// optional mod bundle layered over them. A bundle both overrides
/// records it redefines and contributes records vanilla lacks, so a
/// tree built over one reports the skills the game will actually
/// run — the reason mastery output must not be read straight from
/// [`GameData`] when a bundle is installed.
pub struct SkillDb<'a> {
    data: &'a GameData,
    overlay: Option<&'a ArzFile>,
}

impl<'a> SkillDb<'a> {
    #[must_use]
    pub fn vanilla(data: &'a GameData) -> Self {
        Self {
            data,
            overlay: None,
        }
    }

    #[must_use]
    pub fn overlaid(data: &'a GameData, overlay: &'a ArzFile) -> Self {
        Self {
            data,
            overlay: Some(overlay),
        }
    }

    /// The record the game would actually use: the bundle's version
    /// when it carries one, the vanilla record otherwise. An
    /// undecodable record reads as absent — the same treatment every
    /// caller here already gave a decode failure.
    fn record(&self, id: &RecordId) -> Option<DbRecord> {
        if let Some(overlay) = self.overlay
            && let Some(Ok(record)) = overlay.record(id)
        {
            return Some(record);
        }
        self.data.record(id)?.ok()
    }

    fn tag_text(&self, tag: &str) -> Option<&str> {
        self.data.tag_text(tag)
    }

    /// Every record path in play, normalized and deduplicated:
    /// vanilla's plus anything the bundle adds.
    fn record_paths(&self) -> BTreeSet<String> {
        self.data
            .record_ids()
            .chain(self.overlay.into_iter().flat_map(ArzFile::record_ids))
            .map(|id| normalize(id.as_str()))
            .collect()
    }
}

/// How many reference-following rounds `mastery_tree` runs; deep
/// enough for pet skills' own sub-skills, shallow enough to stay out
/// of the quest/loot graph.
const REFERENCE_DEPTH: usize = 4;

/// Where the engine's tier table lives; the expansion copy wins when
/// both exist (they agree in vanilla AE).
const GAME_ENGINE_RECORDS: [&str; 2] = [
    "RECORDS\\XPACK\\GAME\\GAMEENGINE.DBR",
    "RECORDS\\GAME\\GAMEENGINE.DBR",
];

/// The engine's `skillMasteryTierLevel` table: entry N-1 is the
/// mastery points invested before a `skillTier` N skill unlocks
/// (vanilla: 1, 4, 10, 16, 24, 32, 40). Empty when no game-engine
/// record is present.
fn mastery_tier_levels(db: &SkillDb<'_>) -> Vec<i32> {
    GAME_ENGINE_RECORDS
        .iter()
        .find_map(|path| {
            let record = lookup(db, path)?;
            match &record.variable("skillMasteryTierLevel")?.values {
                DbValues::Integers(values) if !values.is_empty() => Some(values.clone()),
                DbValues::Integers(_)
                | DbValues::Floats(_)
                | DbValues::Strings(_)
                | DbValues::Booleans(_) => None,
            }
        })
        .unwrap_or_default()
}

/// How many delegation hops `delegated` follows; enough for the
/// longest vanilla chain (pet modifier → pet skill → buff).
const DELEGATION_DEPTH: usize = 4;

/// Mastery points needed before this skill can be learned: its
/// `skillTier` — resolved through the delegation chain when the
/// skill record carries none — looked up in the engine's tier
/// table. The `skillMasteryLevelRequired` variable is *not*
/// consulted: it is vestigial pre-release data the engine ignores
/// (Nature's Fatigue and Susceptibility both still carry a stale 18).
fn unlock_level(db: &SkillDb<'_>, record: &DbRecord, tiers: &[i32]) -> Option<i32> {
    let tier = delegated(db, record, &skill_tier)?;
    let row = usize::try_from(tier).ok()?.checked_sub(1)?;
    tiers.get(row).copied()
}

fn skill_tier(record: &DbRecord) -> Option<i32> {
    record.integer("skillTier").filter(|&tier| tier > 0)
}

/// Resolves a UI-identity attribute (name, tier), following the
/// delegation chain when the skill record itself lacks it: a skill
/// may keep its identity on its buff (Nature's Plague →
/// `PlagueBuff.dbr`), and a pet modifier points at the pet skill it
/// modifies, which may itself wrap a buff (Wolf's Strength of the
/// Pack is modifier → pet skill → buff).
fn delegated<T>(
    db: &SkillDb<'_>,
    record: &DbRecord,
    extract: &impl Fn(&DbRecord) -> Option<T>,
) -> Option<T> {
    let mut current = record.clone();
    for _ in 0..DELEGATION_DEPTH {
        if let Some(found) = extract(&current) {
            return Some(found);
        }
        current = delegate_record(db, &current)?;
    }
    extract(&current)
}

/// The record a skill delegates its UI identity to: its buff or the
/// pet skill it modifies.
fn delegate_record(db: &SkillDb<'_>, record: &DbRecord) -> Option<DbRecord> {
    ["buffSkillName", "petSkillName"]
        .iter()
        .find_map(|variable| {
            let path = normalize(record.string(variable)?);
            lookup(db, &path)
        })
}

/// One discovered mastery: its localized name and normalized record
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mastery {
    pub name: String,
    pub record: String,
}

/// Discovers the playable masteries: `Skill_Mastery` records with a
/// localized display name, development leftovers filtered, sorted by
/// record path.
#[must_use]
pub fn masteries(db: &SkillDb<'_>) -> Vec<Mastery> {
    db.record_paths()
        .into_iter()
        .filter(|path| {
            !["\\OLD\\", "\\REV", "11-15-06"]
                .iter()
                .any(|junk| path.contains(junk))
        })
        .filter_map(|path| {
            let record = lookup(db, &path)?;
            if !record.record_type.eq_ignore_ascii_case("Skill_Mastery") {
                return None;
            }
            let name = translated(db, &record, "skillDisplayName")?;
            Some(Mastery { name, record: path })
        })
        .collect()
}

/// One mastery's full skill tree as a JSON document: the skills that
/// sit directly in the mastery's directory plus everything they
/// reference, per-level effect arrays included. `None` when the
/// mastery record has no directory to sweep.
#[must_use]
pub fn mastery_tree(db: &SkillDb<'_>, mastery: &Mastery) -> Option<Value> {
    let directory = format!("{}\\", mastery.record.rsplit_once('\\')?.0);
    let skill_paths: Vec<String> = db
        .record_paths()
        .into_iter()
        .filter(|path| {
            path.strip_prefix(&directory)
                .is_some_and(|rest| !rest.contains('\\'))
        })
        .collect();

    let tiers = mastery_tier_levels(db);
    let mut exported: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = Vec::new();
    let mut skills = Vec::new();
    for path in skill_paths {
        let Some(record) = lookup(db, &path) else {
            continue;
        };
        if !record.record_type.to_uppercase().starts_with("SKILL") {
            continue;
        }
        exported.insert(path.clone());
        skills.push(export_record(db, &path, &record, &tiers, &mut queue));
    }

    let mut referenced = Vec::new();
    let mut depth = 0;
    while !queue.is_empty() && depth < REFERENCE_DEPTH {
        let batch: Vec<String> = std::mem::take(&mut queue);
        for path in batch {
            if !exported.insert(path.clone()) {
                continue;
            }
            let Some(record) = lookup(db, &path) else {
                continue;
            };
            referenced.push(export_record(db, &path, &record, &tiers, &mut queue));
        }
        depth += 1;
    }
    referenced.sort_by_key(|entry| entry["record"].as_str().unwrap_or_default().to_string());

    Some(json!({
        "source": "Titan Quest Anniversary Edition database.arz (+ expansions)",
        "mastery": mastery.name,
        "record": mastery.record,
        "purpose": "One mastery's skill tree with per-level effect arrays; array index = skill level - 1.",
        "notes": [
            "unlocks_at_mastery_level: mastery points invested before the skill can be learned — its skillTier row looked up in gameengine.dbr's skillMasteryTierLevel table (vanilla: 1, 4, 10, 16, 24, 32, 40).",
            "skillMasteryLevelRequired is omitted: vestigial pre-release data the engine ignores.",
            "skillMaxLevel is the buyable cap; skillUltimateLevel the cap with +skill gear.",
            "references contains record paths (buffs, pets, sub-skills); their data is under 'referenced'.",
            "Attribute variable names follow Titan Quest's DBR conventions (offensive*/defensive*/character*/retaliation*).",
        ],
        "skills": skills,
        "referenced": referenced,
    }))
}

fn lookup(db: &SkillDb<'_>, path: &str) -> Option<DbRecord> {
    db.record(&RecordId::parse(path.to_string())?)
}

fn translated(db: &SkillDb<'_>, record: &DbRecord, variable: &str) -> Option<String> {
    let tag = record.string(variable).filter(|tag| !tag.is_empty())?;
    db.tag_text(tag).map(str::to_string)
}

/// One record as JSON: identity, translated tags, per-level effect
/// arrays (all-zero variables, cosmetic resources, and the vestigial
/// `skillMasteryLevelRequired` dropped), and the `.dbr` references
/// (queued for transitive export).
fn export_record(
    db: &SkillDb<'_>,
    path: &str,
    record: &DbRecord,
    tiers: &[i32],
    queue: &mut Vec<String>,
) -> Value {
    let mut effects: BTreeMap<String, Value> = BTreeMap::new();
    let mut references: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut strings: BTreeMap<String, String> = BTreeMap::new();

    for variable in record.variables() {
        let name = variable.name.as_str();
        if matches!(
            name,
            "Class"
                | "templateName"
                | "ActorName"
                | "FileDescription"
                | "skillMasteryLevelRequired"
        ) || cosmetic_variable(name)
        {
            continue;
        }
        match &variable.values {
            DbValues::Strings(values) => {
                let meaningful: Vec<&String> =
                    values.iter().filter(|value| !value.is_empty()).collect();
                if meaningful.is_empty() {
                    continue;
                }
                if meaningful
                    .iter()
                    .all(|value| value.to_uppercase().ends_with(".DBR"))
                {
                    let paths: Vec<String> =
                        meaningful.iter().map(|value| normalize(value)).collect();
                    for target in &paths {
                        if !boring_reference(target) {
                            queue.push(target.clone());
                        }
                    }
                    references.insert(name.to_string(), paths);
                } else if !meaningful.iter().any(|value| resource_path(value)) {
                    let rendered = meaningful
                        .iter()
                        .map(|value| {
                            db.tag_text(value)
                                .map_or_else(|| (*value).clone(), str::to_string)
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    strings.insert(name.to_string(), rendered);
                }
            }
            DbValues::Integers(values) => {
                if values.iter().any(|&value| value != 0) {
                    effects.insert(name.to_string(), json!(values));
                }
            }
            DbValues::Floats(values) => {
                if values.iter().any(|&value| value != 0.0) {
                    let rounded: Vec<f64> = values
                        .iter()
                        .map(|&value| (f64::from(value) * 1000.0).round() / 1000.0)
                        .collect();
                    effects.insert(name.to_string(), json!(rounded));
                }
            }
            DbValues::Booleans(values) => {
                if values.iter().any(|&value| value) {
                    effects.insert(name.to_string(), json!(values));
                }
            }
        }
    }

    let mut entry = Map::new();
    entry.insert("record".to_string(), json!(path));
    entry.insert("class".to_string(), json!(record.record_type));
    if let Some(name) = delegated(db, record, &|r| {
        translated(db, r, "skillDisplayName").or_else(|| translated(db, r, "description"))
    }) {
        entry.insert("name".to_string(), json!(name));
    }
    if let Some(level) = unlock_level(db, record, tiers) {
        entry.insert("unlocks_at_mastery_level".to_string(), json!(level));
    }
    if let Some(description) = translated(db, record, "skillBaseDescription") {
        entry.insert("description".to_string(), json!(description));
    }
    strings.remove("skillDisplayName");
    strings.remove("skillBaseDescription");
    strings.remove("description");
    if !strings.is_empty() {
        entry.insert("strings".to_string(), json!(strings));
    }
    if !references.is_empty() {
        entry.insert("references".to_string(), json!(references));
    }
    entry.insert("effects".to_string(), json!(effects));
    Value::Object(entry)
}

/// References not worth following for build data.
fn boring_reference(normalized: &str) -> bool {
    normalized.contains("\\LOOT")
        || normalized.contains("\\QUEST")
        || normalized.contains("\\FX")
        || normalized.contains("EFFECT")
        || normalized.contains("SOUND")
}

/// Animation, audio, and rendering variables — noise for
/// theorycrafting, and the bulk of pet records' size.
fn cosmetic_variable(name: &str) -> bool {
    let upper = name.to_uppercase();
    [
        "ANIM",
        "SOUND",
        "VOICE",
        "CAMERASHAKE",
        "RAGDOLL",
        "MESH",
        "BITMAP",
        "SHADER",
        "SHADOW",
        "TRANSPARENCY",
        "FOOTSTEP",
        "TINT",
        "FX",
    ]
    .iter()
    .any(|noise| upper.contains(noise))
}

/// Art/audio resource strings — noise for theorycrafting.
fn resource_path(value: &str) -> bool {
    let upper = value.to_uppercase();
    [
        ".TEX", ".MSH", ".ANM", ".WAV", ".MP3", ".PFX", ".SSH", ".TXT", ".QST",
    ]
    .iter()
    .any(|extension| upper.ends_with(extension))
        || upper.contains('/') && upper.contains("SOUND")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arz::ArzFile;
    use crate::arz::fixture::{ArzBuilder, Values};
    use crate::text::TextDb;

    fn text_file(content: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    fn sample_db(engine_record: &str) -> GameData {
        let mut builder = ArzBuilder::default();
        builder.record(
            engine_record,
            "GameEngine",
            &[(
                "skillMasteryTierLevel",
                Values::Ints(&[1, 4, 10, 16, 24, 32, 40]),
            )],
        );
        builder.record(
            "records\\skills\\nature\\naturemastery.dbr",
            "Skill_Mastery",
            &[("skillDisplayName", Values::Strings(&["tagNature"]))],
        );
        builder.record(
            "records\\skills\\nature\\fatigue.dbr",
            "Skill_Modifier",
            &[
                ("skillDisplayName", Values::Strings(&["tagFatigue"])),
                ("skillTier", Values::Ints(&[3])),
                ("skillMasteryLevelRequired", Values::Ints(&[18])),
            ],
        );
        builder.record(
            "records\\skills\\nature\\plague.dbr",
            "Skill_AttackBuff",
            &[(
                "buffSkillName",
                Values::Strings(&["records\\skills\\nature\\plaguebuff.dbr"]),
            )],
        );
        builder.record(
            "records\\skills\\nature\\plaguebuff.dbr",
            "SkillBuff_Contageous",
            &[
                ("skillDisplayName", Values::Strings(&["tagPlague"])),
                ("skillTier", Values::Ints(&[5])),
            ],
        );
        builder.record(
            "records\\skills\\nature\\maul.dbr",
            "SkillSecondary_PetModifier",
            &[(
                "petSkillName",
                Values::Strings(&["records\\skills\\nature\\petskill_maul.dbr"]),
            )],
        );
        builder.record(
            "records\\skills\\nature\\petskill_maul.dbr",
            "Skill_AttackMelee",
            &[
                ("skillDisplayName", Values::Strings(&["tagMaul"])),
                ("skillTier", Values::Ints(&[4])),
            ],
        );
        builder.record(
            "records\\skills\\nature\\pack.dbr",
            "SkillSecondary_PetModifier",
            &[(
                "petSkillName",
                Values::Strings(&["records\\skills\\nature\\petskill_pack.dbr"]),
            )],
        );
        builder.record(
            "records\\skills\\nature\\petskill_pack.dbr",
            "Skill_BuffRadius",
            &[(
                "buffSkillName",
                Values::Strings(&["records\\skills\\nature\\petskill_packbuff.dbr"]),
            )],
        );
        builder.record(
            "records\\skills\\nature\\petskill_packbuff.dbr",
            "SkillBuff_Passive",
            &[
                ("skillDisplayName", Values::Strings(&["tagPack"])),
                ("skillTier", Values::Ints(&[6])),
            ],
        );
        let arz = ArzFile::parse(builder.build()).unwrap();
        let mut text = TextDb::new();
        text.add_file(&text_file(
            "tagNature=Nature Mastery\ntagFatigue=Fatigue\ntagPlague=Plague\ntagMaul=Maul\n\
             tagPack=Strength of the Pack\n",
        ));
        GameData::from_parts(arz, text)
    }

    fn nature_tree(data: &GameData) -> Value {
        let db = SkillDb::vanilla(data);
        let mastery = masteries(&db).into_iter().next().unwrap();
        mastery_tree(&db, &mastery).unwrap()
    }

    fn nature_tree_over(db: &SkillDb<'_>) -> Value {
        let mastery = masteries(db)
            .into_iter()
            .find(|mastery| mastery.record.contains("NATURE"))
            .unwrap();
        mastery_tree(db, &mastery).unwrap()
    }

    /// A bundle that retunes Fatigue's tier and adds a skill of its
    /// own — the shape of a live `LootPlus` tune.
    fn tuning_bundle() -> ArzFile {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\skills\\nature\\fatigue.dbr",
            "Skill_Modifier",
            &[
                ("skillDisplayName", Values::Strings(&["tagFatigue"])),
                ("skillTier", Values::Ints(&[6])),
            ],
        );
        builder.record(
            "records\\skills\\nature\\blink.dbr",
            "Skill_Modifier",
            &[
                ("skillDisplayName", Values::Strings(&["tagFatigue"])),
                ("skillTier", Values::Ints(&[1])),
                ("characterRunSpeedModifier", Values::Ints(&[500])),
            ],
        );
        ArzFile::parse(builder.build()).unwrap()
    }

    #[test]
    fn an_overlaid_bundle_wins_over_the_vanilla_record() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let overlay = tuning_bundle();

        let vanilla = skill(&nature_tree(&data), "RECORDS\\SKILLS\\NATURE\\FATIGUE.DBR").clone();
        let modded = nature_tree_over(&SkillDb::overlaid(&data, &overlay));
        let modded = skill(&modded, "RECORDS\\SKILLS\\NATURE\\FATIGUE.DBR");

        assert_eq!(vanilla["unlocks_at_mastery_level"], 10);
        assert_eq!(modded["unlocks_at_mastery_level"], 32);
    }

    #[test]
    fn a_bundles_own_skill_joins_the_tree_vanilla_never_sees_it() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let overlay = tuning_bundle();

        let tree = nature_tree_over(&SkillDb::overlaid(&data, &overlay));
        let blink = skill(&tree, "RECORDS\\SKILLS\\NATURE\\BLINK.DBR");
        assert_eq!(blink["effects"]["characterRunSpeedModifier"][0], 500);

        assert!(
            nature_tree(&data)["skills"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["record"] != "RECORDS\\SKILLS\\NATURE\\BLINK.DBR")
        );
    }

    #[test]
    fn a_bundle_adding_a_mastery_makes_it_discoverable() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\skills\\runes\\runemastery.dbr",
            "Skill_Mastery",
            &[("skillDisplayName", Values::Strings(&["tagNature"]))],
        );
        let overlay = ArzFile::parse(builder.build()).unwrap();

        let vanilla: Vec<String> = masteries(&SkillDb::vanilla(&data))
            .into_iter()
            .map(|mastery| mastery.record)
            .collect();
        let overlaid: Vec<String> = masteries(&SkillDb::overlaid(&data, &overlay))
            .into_iter()
            .map(|mastery| mastery.record)
            .collect();

        assert!(!vanilla.contains(&"RECORDS\\SKILLS\\RUNES\\RUNEMASTERY.DBR".to_string()));
        assert!(overlaid.contains(&"RECORDS\\SKILLS\\RUNES\\RUNEMASTERY.DBR".to_string()));
    }

    fn skill<'a>(tree: &'a Value, record: &str) -> &'a Value {
        tree["skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["record"] == record)
            .unwrap()
    }

    #[test]
    fn unlock_level_comes_from_tier_row_not_the_vestigial_variable() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let fatigue = skill(&tree, "RECORDS\\SKILLS\\NATURE\\FATIGUE.DBR");
        assert_eq!(fatigue["unlocks_at_mastery_level"], 10);
        assert!(
            fatigue["effects"]
                .get("skillMasteryLevelRequired")
                .is_none()
        );
    }

    #[test]
    fn buff_delegating_skill_takes_name_and_tier_from_its_buff() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let plague = skill(&tree, "RECORDS\\SKILLS\\NATURE\\PLAGUE.DBR");
        assert_eq!(plague["name"], "Plague");
        assert_eq!(plague["unlocks_at_mastery_level"], 24);
    }

    #[test]
    fn pet_modifier_takes_name_and_tier_from_the_pet_skill() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let maul = skill(&tree, "RECORDS\\SKILLS\\NATURE\\MAUL.DBR");
        assert_eq!(maul["name"], "Maul");
        assert_eq!(maul["unlocks_at_mastery_level"], 16);
    }

    #[test]
    fn delegation_follows_the_chain_through_pet_skill_to_its_buff() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let pack = skill(&tree, "RECORDS\\SKILLS\\NATURE\\PACK.DBR");
        assert_eq!(pack["name"], "Strength of the Pack");
        assert_eq!(pack["unlocks_at_mastery_level"], 32);
    }

    #[test]
    fn base_game_engine_record_supplies_the_table_when_no_expansion() {
        let data = sample_db("records\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let fatigue = skill(&tree, "RECORDS\\SKILLS\\NATURE\\FATIGUE.DBR");
        assert_eq!(fatigue["unlocks_at_mastery_level"], 10);
    }

    #[test]
    fn tierless_records_get_no_unlock_level() {
        let data = sample_db("records\\xpack\\game\\gameengine.dbr");
        let tree = nature_tree(&data);
        let mastery = skill(&tree, "RECORDS\\SKILLS\\NATURE\\NATUREMASTERY.DBR");
        assert!(mastery.get("unlocks_at_mastery_level").is_none());
    }
}
