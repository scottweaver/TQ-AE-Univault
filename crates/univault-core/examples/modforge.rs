//! Compiles a record-patch spec into an installable Titan Quest mod
//! bundle, merged onto an existing mod (the game loads one custom
//! quest at a time, so tweaks must ride inside the mod they tune).
//!
//! Only new files are written — the game's own databases and the
//! base mod stay untouched, per ARCHITECTURE.md.
//!
//! Patch spec (JSON):
//! ```json
//! {
//!   "name": "MyTunedMod",
//!   "rules": [
//!     { "kind": "multiply_player_skills", "variable": "skillTargetNumber", "factor": 3.0 },
//!     { "kind": "record", "record": "records\\skills\\earth\\fireenchantmentbuff.dbr",
//!       "multiply": { "skillTargetRadius": 3.0 } }
//!   ]
//! }
//! ```
//!
//! An `"extends": "<sibling spec>.json"` field prepends that spec's
//! rules, so several bundles share one set of tunes and add their own.
//!
//! Usage: `cargo run --release -p univault-core --example modforge -- \
//!     "<TQ AE install dir>" "<base mod dir>" <patch.json> "<CustomMaps dir>"`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use univault_core::arz::{ArzFile, DbRecord, DbValues, DbVariable, compose, normalize};
use univault_core::chr::RecordId;

#[derive(Deserialize)]
struct PatchSpec {
    name: String,
    /// A sibling spec whose rules run first. Shared tunes live in one
    /// file so a bundle added later cannot drift from the others.
    #[serde(default)]
    extends: Option<String>,
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Rule {
    /// Multiplies a variable on every player-side skill record
    /// (`records\skills\…` and expansion equivalents, monster
    /// skills excluded).
    MultiplyPlayerSkills { variable: String, factor: f64 },
    /// Overwrites every value of a variable (array length and type
    /// preserved) on player-side skills of one class.
    FillPlayerSkills {
        class: String,
        variable: String,
        value: f64,
    },
    /// Restores listed variables of one record to their vanilla
    /// (main-database) values, undoing a base mod's change while
    /// keeping the rest of its record edits.
    RevertVariables {
        record: String,
        variables: Vec<String>,
    },
    /// Targeted edits on one record or a list of records.
    Record {
        #[serde(alias = "records")]
        record: RecordRefs,
        #[serde(default)]
        multiply: BTreeMap<String, f64>,
        #[serde(default)]
        set: BTreeMap<String, serde_json::Value>,
    },
    /// Repeats the vanilla entry list of every spawn pool whose
    /// monsters are all Hero-classified (the star-marked ones)
    /// `factor` times — pool entry count is the engine's spawn
    /// multiplier for these records. Pools holding any Boss/Quest
    /// monster are untouched, wherever they live (Ragnarök files
    /// quest bosses under its hero-pool folder). Replaces whatever
    /// the base mod did to a qualifying pool.
    MultiplyHeroPools { factor: usize },
}

/// A single record path or a list — `record` rules accept both.
#[derive(Deserialize)]
#[serde(untagged)]
enum RecordRefs {
    One(String),
    Many(Vec<String>),
}

impl RecordRefs {
    fn iter(&self) -> std::slice::Iter<'_, String> {
        match self {
            Self::One(record) => std::slice::from_ref(record).iter(),
            Self::Many(records) => records.iter(),
        }
    }
}

#[allow(clippy::too_many_lines)] // linear build script
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(base_dir), Some(patch_path), Some(out_dir)) =
        (args.next(), args.next(), args.next(), args.next())
    else {
        eprintln!(
            "usage: modforge <TQ AE install dir> <base mod dir> <patch.json> <CustomMaps dir> \
             [bundle name]"
        );
        std::process::exit(2);
    };
    let mut spec = load_spec(Path::new(&patch_path), &mut Vec::new());
    // One rules file, several bundles: an override names this build
    // (e.g. the same tunes onto the x3 and x3x1 bases).
    if let Some(name) = args.next() {
        spec.name = name;
    }

    let main_db = ArzFile::parse(
        std::fs::read(Path::new(&game_dir).join("Database/database.arz")).expect("read database"),
    )
    .expect("parse main database");
    let base_path = Path::new(&base_dir);
    let base_name = base_path
        .file_name()
        .expect("base mod folder name")
        .to_string_lossy()
        .to_string();
    let base_db = ArzFile::parse(
        std::fs::read(base_path.join("database").join(format!("{base_name}.arz")))
            .expect("read base mod database"),
    )
    .expect("parse base mod database");

    // Effective record: the base mod's override when it has one.
    let effective = |id: &RecordId| -> Option<DbRecord> {
        base_db
            .record(id)
            .or_else(|| main_db.record(id))
            .and_then(Result::ok)
    };
    let timestamp_of = |id: &RecordId| -> i64 {
        base_db
            .record_timestamp(id)
            .or_else(|| main_db.record_timestamp(id))
            .unwrap_or(0)
    };

    // Apply rules, collecting patched records by normalized id.
    let mut patched: BTreeMap<String, DbRecord> = BTreeMap::new();
    let mut report = Vec::new();
    for rule in &spec.rules {
        match rule {
            Rule::MultiplyPlayerSkills { variable, factor } => {
                let targets: Vec<RecordId> = main_db
                    .record_ids()
                    .filter(|id| player_skill(&normalize(id.as_str())))
                    .cloned()
                    .collect();
                for id in targets {
                    let key = normalize(id.as_str());
                    let record = patched.get(&key).cloned().or_else(|| effective(&id));
                    let Some(mut record) = record else { continue };
                    let Some(changed) = multiply_variable(&mut record, variable, *factor) else {
                        continue;
                    };
                    report.push(format!("{key}: {variable} {changed}"));
                    patched.insert(key, record);
                }
            }
            Rule::FillPlayerSkills {
                class,
                variable,
                value,
            } => {
                let targets: Vec<RecordId> = main_db
                    .record_ids()
                    .filter(|id| player_skill(&normalize(id.as_str())))
                    .cloned()
                    .collect();
                for id in targets {
                    let key = normalize(id.as_str());
                    let record = patched.get(&key).cloned().or_else(|| effective(&id));
                    let Some(mut record) = record else { continue };
                    if !record.record_type.eq_ignore_ascii_case(class) {
                        continue;
                    }
                    let Some(changed) = fill_variable(&mut record, variable, *value) else {
                        continue;
                    };
                    report.push(format!("{key}: {variable} {changed}"));
                    patched.insert(key, record);
                }
            }
            Rule::RevertVariables { record, variables } => {
                let id = RecordId::parse(record.clone()).expect("record id in patch");
                let key = normalize(id.as_str());
                let vanilla = main_db
                    .record(&id)
                    .and_then(Result::ok)
                    .unwrap_or_else(|| panic!("vanilla record not found: {record}"));
                let mut target = patched
                    .get(&key)
                    .cloned()
                    .or_else(|| effective(&id))
                    .unwrap_or_else(|| panic!("record not found: {record}"));
                for name in variables {
                    let Some(original) = vanilla.variable(name) else {
                        report.push(format!("{key}: {name} has no vanilla value; left as-is"));
                        continue;
                    };
                    if target.variable(name).map(|v| &v.values) == Some(&original.values) {
                        continue;
                    }
                    report.push(format!("{key}: {name} reverted to vanilla"));
                    target.set_variable(original.clone());
                }
                patched.insert(key, target);
            }
            Rule::MultiplyHeroPools { factor } => {
                // A hero's higher-level variants often omit
                // monsterClassification, so the test is: at least one
                // entry explicitly Hero, none explicitly anything
                // else (Boss/Quest pools stay untouched).
                let mut class_cache: BTreeMap<String, Option<String>> = BTreeMap::new();
                let mut classification = |raw: &str| -> Option<String> {
                    let key = normalize(raw);
                    if let Some(known) = class_cache.get(&key) {
                        return known.clone();
                    }
                    let class = RecordId::parse(raw.to_string())
                        .and_then(|id| main_db.record(&id).and_then(Result::ok))
                        .and_then(|monster| {
                            monster.string("monsterClassification").map(str::to_string)
                        });
                    class_cache.insert(key, class.clone());
                    class
                };
                let targets: Vec<RecordId> = main_db
                    .record_ids()
                    .filter(|id| normalize(id.as_str()).contains("PROXIES"))
                    .cloned()
                    .collect();
                for id in targets {
                    let key = normalize(id.as_str());
                    let Some(vanilla) = main_db.record(&id).and_then(Result::ok) else {
                        continue;
                    };
                    let pool = vanilla
                        .string("templateName")
                        .is_some_and(|template| template.to_uppercase().ends_with("PROXYPOOL.TPL"));
                    if !pool {
                        continue;
                    }
                    let entries = pool_entries(&vanilla);
                    if entries.is_empty() {
                        continue;
                    }
                    let classes: Vec<Option<String>> = entries
                        .iter()
                        .map(|(name, _)| classification(name))
                        .collect();
                    // Some heroes (all Slobberjaw variants, higher
                    // levels of most others) omit the classification;
                    // the database's HERO_* stem convention fills the
                    // gap.
                    let hero_stem = |name: &str| {
                        Path::new(&name.replace('\\', "/"))
                            .file_stem()
                            .is_some_and(|stem| {
                                stem.to_string_lossy().to_uppercase().contains("HERO")
                            })
                    };
                    let any_hero =
                        classes
                            .iter()
                            .zip(&entries)
                            .any(|(class, (name, _))| match class {
                                Some(class) => class.eq_ignore_ascii_case("Hero"),
                                None => hero_stem(name),
                            });
                    let any_other = classes
                        .iter()
                        .flatten()
                        .any(|class| !class.eq_ignore_ascii_case("Hero"));
                    if !any_hero || any_other {
                        continue;
                    }
                    let mut record = vanilla.clone();
                    for (slot, (name, weight)) in entries
                        .iter()
                        .cycle()
                        .take(entries.len() * factor)
                        .enumerate()
                    {
                        let n = slot + 1;
                        record.set_variable(DbVariable {
                            name: format!("name{n}"),
                            values: DbValues::Strings(vec![name.clone()]),
                        });
                        if let Some(weight) = weight {
                            record.set_variable(DbVariable {
                                name: format!("weight{n}"),
                                values: DbValues::Integers(vec![*weight]),
                            });
                        }
                    }
                    report.push(format!(
                        "{key}: hero pool {} -> {} entries",
                        entries.len(),
                        entries.len() * factor
                    ));
                    patched.insert(key, record);
                }
            }
            Rule::Record {
                record,
                multiply,
                set,
            } => {
                for record in record.iter() {
                    let id = RecordId::parse(record.clone()).expect("record id in patch");
                    let key = normalize(id.as_str());
                    let mut target = patched
                        .get(&key)
                        .cloned()
                        .or_else(|| effective(&id))
                        .unwrap_or_else(|| panic!("record not found: {record}"));
                    for (variable, factor) in multiply {
                        if let Some(changed) = multiply_variable(&mut target, variable, *factor) {
                            report.push(format!("{key}: {variable} {changed}"));
                        }
                    }
                    for (variable, value) in set {
                        let values = json_values(value);
                        report.push(format!("{key}: {variable} set to {values:?}"));
                        target.set_variable(DbVariable {
                            name: variable.clone(),
                            values,
                        });
                    }
                    patched.insert(key, target);
                }
            }
        }
    }

    // Merged database: the base mod's records in their original
    // order (patched where targeted), then patched records the base
    // did not carry, in main-database order.
    let mut records: Vec<(DbRecord, i64)> = Vec::new();
    let mut appended = 0_usize;
    for id in base_db.record_ids() {
        let key = normalize(id.as_str());
        let record = patched
            .remove(&key)
            .unwrap_or_else(|| base_db.record(id).unwrap().expect("base record"));
        records.push((record, timestamp_of(id)));
    }
    for id in main_db.record_ids() {
        let key = normalize(id.as_str());
        if let Some(record) = patched.remove(&key) {
            records.push((record, timestamp_of(id)));
            appended += 1;
        }
    }
    assert!(patched.is_empty(), "patched records left unplaced");

    let image = compose(&records);

    // Self-check: the composed database must re-parse with every
    // record intact.
    let check = ArzFile::parse(image.clone()).expect("re-parse composed database");
    assert_eq!(check.record_ids().count(), records.len());
    for (record, _) in &records {
        let reread = check
            .record(&record.id)
            .expect("composed record present")
            .expect("composed record decodes");
        assert_eq!(&reread, record, "{:?} round-trip", record.id);
    }

    // Emit the bundle: database + the base mod's resources.
    let bundle = Path::new(&out_dir).join(&spec.name);
    let database_dir = bundle.join("database");
    std::fs::create_dir_all(&database_dir).expect("create bundle dirs");
    std::fs::write(database_dir.join(format!("{}.arz", spec.name)), &image)
        .expect("write mod database");
    let resources_out = bundle.join("resources");
    std::fs::create_dir_all(&resources_out).expect("create resources dir");
    let mut copied = 0_usize;
    for entry in std::fs::read_dir(base_path.join("resources"))
        .expect("base resources")
        .flatten()
    {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), resources_out.join(entry.file_name()))
                .expect("copy resource");
            copied += 1;
        }
    }

    for line in &report {
        println!("{line}");
    }
    println!(
        "\n{}: {} records ({} patched, {} appended), {} resource files, {} KB database -> {}",
        spec.name,
        records.len(),
        report.len(),
        appended,
        copied,
        image.len() / 1024,
        bundle.display()
    );
}

/// Reads a spec and the chain it extends, base rules first — a rule
/// in the extending spec therefore refines the one it follows.
fn load_spec(path: &Path, chain: &mut Vec<PathBuf>) -> PatchSpec {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    assert!(
        !chain.contains(&canonical),
        "extends cycle at {}",
        path.display()
    );
    chain.push(canonical);
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read patch spec {}: {error}", path.display()));
    let mut spec: PatchSpec = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse patch spec {}: {error}", path.display()));
    if let Some(base) = spec.extends.take() {
        let base_path = path.parent().unwrap_or(Path::new(".")).join(base);
        let mut rules = load_spec(&base_path, chain).rules;
        rules.append(&mut spec.rules);
        spec.rules = rules;
    }
    spec
}

/// Player-side skill records: the mastery trees, shared actions,
/// scrolls, and pet skills. Enemy skills (monster/boss/hero dirs)
/// and the database's development leftovers are excluded.
fn player_skill(normalized: &str) -> bool {
    let in_skills = normalized.starts_with(r"RECORDS\SKILLS\")
        || (normalized.starts_with(r"RECORDS\XPACK") && normalized.contains(r"\SKILLS\"));
    // "QUEST SKILLS" holds quest-scripted enemy skills; player
    // quest rewards live under RECORDS\QUESTS\ instead.
    let enemy = ["MONSTER", "BOSS", "HERO", "QUEST SKILLS"]
        .iter()
        .any(|dir| normalized.contains(dir));
    // MEDICINE is a cut mastery the database still ships; \OUT\ and
    // _OLD-suffixed records are other development leftovers.
    let leftover = [
        r"\OLD\",
        r"\REV",
        "11-15-06",
        r"\MEDICINE\",
        r"\OUT\",
        "_OLD.",
    ]
    .iter()
    .any(|junk| normalized.contains(junk));
    in_skills && !enemy && !leftover
}

/// A spawn pool's entry list: consecutive `nameN` record paths with
/// their `weightN` (when present), stopping at the first missing or
/// empty slot.
fn pool_entries(record: &DbRecord) -> Vec<(String, Option<i32>)> {
    let mut entries = Vec::new();
    for n in 1.. {
        let Some(name) = record.string(&format!("name{n}")) else {
            break;
        };
        if name.trim().is_empty() {
            break;
        }
        let weight =
            record
                .variable(&format!("weight{n}"))
                .and_then(|variable| match &variable.values {
                    DbValues::Integers(values) => values.first().copied(),
                    DbValues::Floats(_) | DbValues::Strings(_) | DbValues::Booleans(_) => None,
                });
        entries.push((name.to_string(), weight));
    }
    entries
}

/// Multiplies every value of `variable`; `None` when the record has
/// no such variable or nothing changes. Returns a before → after
/// description.
fn multiply_variable(record: &mut DbRecord, variable: &str, factor: f64) -> Option<String> {
    let current = record.variable(variable)?;
    let (values, description) = match &current.values {
        DbValues::Integers(values) => {
            #[allow(clippy::cast_possible_truncation)] // game-scale ints
            let scaled: Vec<i32> = values
                .iter()
                .map(|&value| (f64::from(value) * factor).round() as i32)
                .collect();
            if scaled == *values {
                return None;
            }
            let text = format!("{values:?} -> {scaled:?}");
            (DbValues::Integers(scaled), text)
        }
        DbValues::Floats(values) => {
            #[allow(clippy::cast_possible_truncation)] // display data
            let scaled: Vec<f32> = values
                .iter()
                .map(|&value| (f64::from(value) * factor) as f32)
                .collect();
            if scaled == *values {
                return None;
            }
            let text = format!("{values:?} -> {scaled:?}");
            (DbValues::Floats(scaled), text)
        }
        DbValues::Strings(_) | DbValues::Booleans(_) => return None,
    };
    record.set_variable(DbVariable {
        name: variable.to_string(),
        values,
    });
    Some(description)
}

/// Overwrites every value of `variable` with `value`, preserving the
/// array's length and type; `None` when absent or unchanged.
fn fill_variable(record: &mut DbRecord, variable: &str, value: f64) -> Option<String> {
    let current = record.variable(variable)?;
    let (values, description) = match &current.values {
        DbValues::Integers(existing) => {
            #[allow(clippy::cast_possible_truncation)] // game-scale ints
            let filled = vec![value.round() as i32; existing.len()];
            if filled == *existing {
                return None;
            }
            let text = format!("{existing:?} -> {filled:?}");
            (DbValues::Integers(filled), text)
        }
        DbValues::Floats(existing) => {
            #[allow(clippy::cast_possible_truncation)] // patch data
            let filled = vec![value as f32; existing.len()];
            if filled == *existing {
                return None;
            }
            let text = format!("{existing:?} -> {filled:?}");
            (DbValues::Floats(filled), text)
        }
        DbValues::Strings(_) | DbValues::Booleans(_) => return None,
    };
    record.set_variable(DbVariable {
        name: variable.to_string(),
        values,
    });
    Some(description)
}

fn json_values(value: &serde_json::Value) -> DbValues {
    let items: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    if items.iter().all(serde_json::Value::is_i64) {
        DbValues::Integers(
            items
                .iter()
                .filter_map(serde_json::Value::as_i64)
                .filter_map(|item| i32::try_from(item).ok())
                .collect(),
        )
    } else if items.iter().all(serde_json::Value::is_number) {
        #[allow(clippy::cast_possible_truncation)] // patch data
        DbValues::Floats(
            items
                .iter()
                .filter_map(serde_json::Value::as_f64)
                .map(|item| item as f32)
                .collect(),
        )
    } else if items.iter().all(serde_json::Value::is_boolean) {
        DbValues::Booleans(
            items
                .iter()
                .filter_map(serde_json::Value::as_bool)
                .collect(),
        )
    } else {
        DbValues::Strings(
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_string())
                .collect(),
        )
    }
}
