//! Full-database access behind the record tools: effective-record
//! resolution against an installed mod bundle, lazy record indexes
//! for search, and JSON shaping of raw record variables. Pure over
//! already-loaded data — no IO here.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use univault_core::arz::{ArzFile, DbRecord, DbValues, DbVariable};
use univault_core::chr::RecordId;
use univault_core::gamedata::GameData;

/// Where an effective record's bytes came from once a mod overlay
/// is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Vanilla,
    ModOverride,
    ModAdded,
}

impl Provenance {
    pub fn label(self, mod_name: Option<&str>) -> String {
        let name = mod_name.unwrap_or("mod");
        match self {
            Self::Vanilla => "vanilla".to_string(),
            Self::ModOverride => format!("{name} (overrides vanilla)"),
            Self::ModAdded => format!("{name} (new record, not in vanilla)"),
        }
    }
}

/// The record the game would actually use: the mod's version when
/// the bundle carries one, the vanilla record otherwise.
pub fn effective_record(
    data: &GameData,
    mod_db: Option<&ArzFile>,
    id: &RecordId,
) -> Option<(DbRecord, Provenance)> {
    if let Some(mod_db) = mod_db
        && let Some(Ok(record)) = mod_db.record(id)
    {
        let provenance = if data.record(id).is_some() {
            Provenance::ModOverride
        } else {
            Provenance::ModAdded
        };
        return Some((record, provenance));
    }
    match data.record(id)? {
        Ok(record) => Some((record, Provenance::Vanilla)),
        Err(_) => None,
    }
}

/// One searchable record: its normalized path, class, and localized
/// display name where the record carries one.
pub struct IndexEntry {
    pub path: String,
    pub class: String,
    pub name: Option<String>,
}

/// Variables whose translated value serves as a record's display
/// name, in lookup order (monsters use `description`, skills their
/// display tag).
const NAME_VARIABLES: [&str; 3] = ["description", "skillDisplayName", "itemNameTag"];

fn display_name(data: &GameData, record: &DbRecord) -> Option<String> {
    NAME_VARIABLES.iter().find_map(|variable| {
        let tag = record.string(variable)?;
        if tag.is_empty() {
            return None;
        }
        data.tag_text(tag).map(str::to_string)
    })
}

/// Decodes every record in the vanilla database into search-index
/// entries. Expensive (a few seconds) — build once and cache.
pub fn vanilla_index(data: &GameData) -> Vec<IndexEntry> {
    data.record_ids()
        .filter_map(|id| {
            let record = data.record(id)?.ok()?;
            Some(index_entry(data, id, &record))
        })
        .collect()
}

/// Same, over one mod bundle's records; translations still resolve
/// against the vanilla text archive.
pub fn mod_index(data: &GameData, mod_db: &ArzFile) -> Vec<IndexEntry> {
    mod_db
        .record_ids()
        .filter_map(|id| {
            let record = mod_db.record(id)?.ok()?;
            Some(index_entry(data, id, &record))
        })
        .collect()
}

fn index_entry(data: &GameData, id: &RecordId, record: &DbRecord) -> IndexEntry {
    IndexEntry {
        path: univault_core::arz::normalize(id.as_str()),
        class: record.record_type.clone(),
        name: display_name(data, record),
    }
}

/// A whole record as JSON. `everything: false` drops the template's
/// resting defaults (all-zero, all-false, all-empty variables) that
/// dominate raw records; `true` is the byte-faithful dump.
pub fn record_json(data: &GameData, record: &DbRecord, source: &str, everything: bool) -> Value {
    let mut variables: BTreeMap<String, Value> = BTreeMap::new();
    let mut translated: BTreeMap<String, String> = BTreeMap::new();
    for variable in record.variables() {
        if !everything && default_valued(&variable.values) {
            continue;
        }
        variables.insert(variable.name.clone(), values_json(&variable.values));
        if let DbValues::Strings(values) = &variable.values
            && let Some(text) = values
                .iter()
                .find(|value| !value.is_empty())
                .and_then(|tag| data.tag_text(tag))
        {
            translated.insert(variable.name.clone(), text.to_string());
        }
    }
    let mut out = json!({
        "record": univault_core::arz::normalize(record.id.as_str()),
        "class": record.record_type,
        "source": source,
        "variables": variables,
    });
    if !translated.is_empty() {
        out["translated"] = json!(translated);
    }
    if !everything {
        out["note"] = json!(
            "template-default variables (all zero/false/empty) omitted; \
             pass everything: true for the byte-faithful dump"
        );
    }
    out
}

fn default_valued(values: &DbValues) -> bool {
    match values {
        DbValues::Integers(values) => values.iter().all(|&value| value == 0),
        DbValues::Floats(values) => values.iter().all(|&value| value == 0.0),
        DbValues::Strings(values) => values.iter().all(String::is_empty),
        DbValues::Booleans(values) => values.iter().all(|&value| !value),
    }
}

/// Values as JSON: a one-element array unwraps to its scalar (the
/// overwhelmingly common shape), floats lose their f32 noise.
pub fn values_json(values: &DbValues) -> Value {
    fn unwrap_single(mut rendered: Vec<Value>) -> Value {
        if rendered.len() == 1 {
            rendered.remove(0)
        } else {
            Value::Array(rendered)
        }
    }
    match values {
        DbValues::Integers(values) => unwrap_single(values.iter().map(|&v| json!(v)).collect()),
        DbValues::Floats(values) => unwrap_single(values.iter().map(|&v| float_json(v)).collect()),
        DbValues::Strings(values) => unwrap_single(values.iter().map(|v| json!(v)).collect()),
        DbValues::Booleans(values) => unwrap_single(values.iter().map(|&v| json!(v)).collect()),
    }
}

fn float_json(value: f32) -> Value {
    json!((f64::from(value) * 10_000.0).round() / 10_000.0)
}

/// Per-variable diff of a vanilla record against a mod's override:
/// changed values side by side, plus variables only one side has.
pub fn diff_json<'a>(
    vanilla: impl IntoIterator<Item = &'a DbVariable>,
    modded: impl IntoIterator<Item = &'a DbVariable>,
) -> Value {
    let vanilla: BTreeMap<&str, &DbValues> = vanilla
        .into_iter()
        .map(|variable| (variable.name.as_str(), &variable.values))
        .collect();
    let modded: BTreeMap<&str, &DbValues> = modded
        .into_iter()
        .map(|variable| (variable.name.as_str(), &variable.values))
        .collect();
    let mut changed: BTreeMap<&str, Value> = BTreeMap::new();
    let mut added: BTreeMap<&str, Value> = BTreeMap::new();
    let mut removed: BTreeMap<&str, Value> = BTreeMap::new();
    for (name, values) in &modded {
        match vanilla.get(name) {
            None => {
                added.insert(name, values_json(values));
            }
            Some(before) if *before != *values => {
                changed.insert(
                    name,
                    json!({"vanilla": values_json(before), "mod": values_json(values)}),
                );
            }
            Some(_) => {}
        }
    }
    for (name, values) in &vanilla {
        if !modded.contains_key(name) {
            removed.insert(name, values_json(values));
        }
    }
    json!({
        "changed": changed,
        "added_by_mod": added,
        "removed_by_mod": removed,
        "identical": changed.is_empty() && added.is_empty() && removed.is_empty(),
    })
}

/// The names of variables whose values differ between two records —
/// the cheap summary the whole-mod sweep reports per record.
pub fn changed_variable_names(vanilla: &DbRecord, modded: &DbRecord) -> Vec<String> {
    let before: BTreeMap<&str, &DbValues> = vanilla
        .variables()
        .map(|variable| (variable.name.as_str(), &variable.values))
        .collect();
    let mut names: Vec<String> = modded
        .variables()
        .filter(|variable| before.get(variable.name.as_str()) != Some(&&variable.values))
        .map(|variable| variable.name.clone())
        .collect();
    for (name, _) in before {
        if modded.variable(name).is_none() {
            names.push(format!("{name} (removed)"));
        }
    }
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variable(name: &str, values: DbValues) -> DbVariable {
        DbVariable {
            name: name.to_string(),
            values,
        }
    }

    #[test]
    fn values_unwrap_singles_and_round_float_noise() {
        assert_eq!(values_json(&DbValues::Integers(vec![7])), json!(7));
        assert_eq!(values_json(&DbValues::Integers(vec![1, 2])), json!([1, 2]));
        // 0.1f32 carries binary noise; rounding restores the value
        // the mod author wrote.
        assert_eq!(values_json(&DbValues::Floats(vec![0.1])), json!(0.1));
    }

    #[test]
    fn diff_reports_changed_added_and_removed_variables() {
        let vanilla = [
            variable("skillTargetRadius", DbValues::Floats(vec![3.0])),
            variable("onlyVanilla", DbValues::Integers(vec![1])),
            variable("same", DbValues::Integers(vec![5])),
        ];
        let modded = [
            variable("skillTargetRadius", DbValues::Floats(vec![5.0])),
            variable("onlyMod", DbValues::Booleans(vec![true])),
            variable("same", DbValues::Integers(vec![5])),
        ];
        let diff = diff_json(vanilla.iter(), modded.iter());
        assert_eq!(
            diff["changed"]["skillTargetRadius"],
            json!({"vanilla": 3.0, "mod": 5.0})
        );
        assert_eq!(diff["added_by_mod"]["onlyMod"], json!(true));
        assert_eq!(diff["removed_by_mod"]["onlyVanilla"], json!(1));
        assert_eq!(diff["identical"], json!(false));
        assert!(diff["changed"].get("same").is_none());
    }

    #[test]
    fn template_defaults_are_recognized() {
        assert!(default_valued(&DbValues::Floats(vec![0.0, 0.0])));
        assert!(default_valued(&DbValues::Strings(vec![String::new()])));
        assert!(!default_valued(&DbValues::Floats(vec![0.0, 2.5])));
        assert!(!default_valued(&DbValues::Booleans(vec![true])));
    }
}
