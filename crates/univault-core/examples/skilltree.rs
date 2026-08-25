//! Exports every mastery's full skill tree — localized names,
//! descriptions, tiers, and per-level effect arrays — as one JSON
//! document for build theorycrafting with AI tools.
//!
//! Masteries are discovered from `Skill_Mastery` records; each
//! mastery's directory is swept for skill records, and buff / pet /
//! sub-skill records they reference are pulled in transitively so
//! the numbers behind summons and buffs are present too.
//!
//! The output embeds extracted game data: keep it local (the
//! repository ignores `exports/`), per the derived-data rule in
//! ARCHITECTURE.md.
//!
//! Usage: `cargo run --release -p univault-core --example skilltree -- \
//!     "<TQ AE install dir>" <output dir>`

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use univault_core::arz::{DbRecord, DbValues, normalize};
use univault_core::chr::RecordId;
use univault_core::gamedata::GameData;

#[allow(clippy::too_many_lines)] // linear export script
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: skilltree <TQ AE install dir> <output dir>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let game = std::path::Path::new(&game_dir);
    let database = std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    let text = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    let data = GameData::from_bytes(database, text).expect("assemble game data");

    let mut mastery_records: Vec<(String, DbRecord)> = data
        .record_ids()
        .filter_map(|id| {
            let record = data.record(id)?.ok()?;
            record
                .record_type
                .eq_ignore_ascii_case("Skill_Mastery")
                .then(|| (normalize(id.as_str()), record))
        })
        .collect();
    mastery_records.sort_by(|a, b| a.0.cmp(&b.0));

    let mut index = Vec::new();
    for (mastery_path, mastery_record) in mastery_records {
        // The database ships development leftovers; skip them and
        // anything whose name never got a localization entry.
        if ["\\OLD\\", "\\REV", "11-15-06"]
            .iter()
            .any(|junk| mastery_path.contains(junk))
        {
            continue;
        }
        let Some(name) = translated(&data, &mastery_record, "skillDisplayName") else {
            continue;
        };
        let directory = match mastery_path.rsplit_once('\\') {
            Some((directory, _)) => format!("{directory}\\"),
            None => continue,
        };
        // Real skills sit directly in the mastery directory; dev
        // junk hides in subdirectories.
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
            let Some(Ok(record)) = lookup(&data, &path) else {
                continue;
            };
            if !record.record_type.to_uppercase().starts_with("SKILL") {
                continue;
            }
            exported.insert(path.clone());
            skills.push(export_record(&data, &path, &record, &mut queue));
        }

        // Pull in the buffs, pets, and pet skills this tree references.
        let mut referenced = Vec::new();
        let mut depth = 0;
        while !queue.is_empty() && depth < 4 {
            let batch: Vec<String> = std::mem::take(&mut queue);
            for path in batch {
                if !exported.insert(path.clone()) {
                    continue;
                }
                let Some(Ok(record)) = lookup(&data, &path) else {
                    continue;
                };
                referenced.push(export_record(&data, &path, &record, &mut queue));
            }
            depth += 1;
        }
        referenced.sort_by_key(|entry| entry["record"].as_str().unwrap_or_default().to_string());

        let document = json!({
            "source": "Titan Quest Anniversary Edition database.arz (+ expansions)",
            "mastery": name,
            "record": mastery_path,
            "purpose": "One mastery's skill tree with per-level effect arrays; array index = skill level - 1.",
            "notes": [
                "skillMasteryLevelRequired is the tier: mastery points needed before the skill unlocks.",
                "skillMaxLevel is the buyable cap; skillUltimateLevel the cap with +skill gear.",
                "references contains record paths (buffs, pets, sub-skills); their data is under 'referenced'.",
                "Attribute variable names follow Titan Quest's DBR conventions (offensive*/defensive*/character*/retaliation*).",
            ],
            "skills": skills,
            "referenced": referenced,
        });
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let out_path = std::path::Path::new(&out_dir).join(format!("{slug}.json"));
        let rendered = serde_json::to_string_pretty(&document).expect("serialize");
        std::fs::write(&out_path, &rendered).expect("write output");
        println!(
            "{name:20} {:3} skills, {:3} referenced, {:4} KB -> {}",
            document["skills"].as_array().map_or(0, Vec::len),
            document["referenced"].as_array().map_or(0, Vec::len),
            rendered.len() / 1024,
            out_path.display()
        );
        index.push(json!({
            "mastery": name,
            "file": format!("{slug}.json"),
            "skills": document["skills"].as_array().map_or(0, Vec::len),
        }));
    }
    let index_path = std::path::Path::new(&out_dir).join("index.json");
    std::fs::write(
        &index_path,
        serde_json::to_string_pretty(&json!({"masteries": index})).unwrap(),
    )
    .expect("write index");
    println!("index -> {}", index_path.display());
}

fn lookup(data: &GameData, path: &str) -> Option<Result<DbRecord, univault_core::arz::ArzError>> {
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
