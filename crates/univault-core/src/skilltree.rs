//! Distills mastery skill trees out of the game database: localized
//! names, descriptions, tiers, and per-level effect arrays, with the
//! buff / pet / sub-skill records they reference pulled in
//! transitively. Consumed by the `skilltree` export example and the
//! MCP server; the documents embed extracted game data and stay
//! local per the derived-data rule in ARCHITECTURE.md.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::arz::{DbRecord, DbValues, normalize};
use crate::chr::RecordId;
use crate::gamedata::GameData;

/// How many reference-following rounds `mastery_tree` runs; deep
/// enough for pet skills' own sub-skills, shallow enough to stay out
/// of the quest/loot graph.
const REFERENCE_DEPTH: usize = 4;

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
pub fn masteries(data: &GameData) -> Vec<Mastery> {
    let mut found: Vec<Mastery> = data
        .record_ids()
        .filter_map(|id| {
            let record = data.record(id)?.ok()?;
            if !record.record_type.eq_ignore_ascii_case("Skill_Mastery") {
                return None;
            }
            let path = normalize(id.as_str());
            if ["\\OLD\\", "\\REV", "11-15-06"]
                .iter()
                .any(|junk| path.contains(junk))
            {
                return None;
            }
            let name = translated(data, &record, "skillDisplayName")?;
            Some(Mastery { name, record: path })
        })
        .collect();
    found.sort_by(|a, b| a.record.cmp(&b.record));
    found
}

/// One mastery's full skill tree as a JSON document: the skills that
/// sit directly in the mastery's directory plus everything they
/// reference, per-level effect arrays included. `None` when the
/// mastery record has no directory to sweep.
#[must_use]
pub fn mastery_tree(data: &GameData, mastery: &Mastery) -> Option<Value> {
    let directory = format!("{}\\", mastery.record.rsplit_once('\\')?.0);
    let mut skill_paths: Vec<String> = data
        .record_ids()
        .map(|id| normalize(id.as_str()))
        .filter(|path| {
            path.strip_prefix(&directory)
                .is_some_and(|rest| !rest.contains('\\'))
        })
        .collect();
    skill_paths.sort();

    let mut exported: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = Vec::new();
    let mut skills = Vec::new();
    for path in skill_paths {
        let Some(Ok(record)) = lookup(data, &path) else {
            continue;
        };
        if !record.record_type.to_uppercase().starts_with("SKILL") {
            continue;
        }
        exported.insert(path.clone());
        skills.push(export_record(data, &path, &record, &mut queue));
    }

    let mut referenced = Vec::new();
    let mut depth = 0;
    while !queue.is_empty() && depth < REFERENCE_DEPTH {
        let batch: Vec<String> = std::mem::take(&mut queue);
        for path in batch {
            if !exported.insert(path.clone()) {
                continue;
            }
            let Some(Ok(record)) = lookup(data, &path) else {
                continue;
            };
            referenced.push(export_record(data, &path, &record, &mut queue));
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
            "skillMasteryLevelRequired is the tier: mastery points needed before the skill unlocks.",
            "skillMaxLevel is the buyable cap; skillUltimateLevel the cap with +skill gear.",
            "references contains record paths (buffs, pets, sub-skills); their data is under 'referenced'.",
            "Attribute variable names follow Titan Quest's DBR conventions (offensive*/defensive*/character*/retaliation*).",
        ],
        "skills": skills,
        "referenced": referenced,
    }))
}

fn lookup(data: &GameData, path: &str) -> Option<Result<DbRecord, crate::arz::ArzError>> {
    data.record(&RecordId::parse(path.to_string())?)
}

fn translated(data: &GameData, record: &DbRecord, variable: &str) -> Option<String> {
    let tag = record.string(variable).filter(|tag| !tag.is_empty())?;
    data.tag_text(tag).map(str::to_string)
}

/// One record as JSON: identity, translated tags, per-level effect
/// arrays (all-zero variables and cosmetic resources dropped), and
/// the `.dbr` references (queued for transitive export).
fn export_record(data: &GameData, path: &str, record: &DbRecord, queue: &mut Vec<String>) -> Value {
    let mut effects: BTreeMap<String, Value> = BTreeMap::new();
    let mut references: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut strings: BTreeMap<String, String> = BTreeMap::new();

    for variable in record.variables() {
        let name = variable.name.as_str();
        if matches!(
            name,
            "Class" | "templateName" | "ActorName" | "FileDescription"
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
                            data.tag_text(value)
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
    if let Some(name) = translated(data, record, "skillDisplayName")
        .or_else(|| translated(data, record, "description"))
    {
        entry.insert("name".to_string(), json!(name));
    }
    if let Some(description) = translated(data, record, "skillBaseDescription") {
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
