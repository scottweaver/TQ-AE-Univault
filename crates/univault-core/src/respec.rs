//! Respec operations on `Player.chr` bytes: refund attribute points
//! and skill/mastery points, resetting the spent state to a fresh
//! character's. Both follow the targeted-splice rule — only the
//! fields being reset change; every other byte is copied through.
//!
//! Refunds are computed from deltas against the fresh-character
//! baselines (attributes 50/50/50/300/300, +4/+4/+4/+40/+40 per
//! point), so points from any source — level-ups or quest rewards —
//! come back exactly as spent. Baselines validated against real AE
//! saves; `TQVaultAE` has no respec, so there is no ported reference
//! here (`tqrespec` was consulted eyes-only per the provenance
//! rules).

use crate::arz::normalize;
use crate::reader::{ByteReader, ReadError, find_key};

/// str, dex, int, health, energy — the save's five `temp` floats, in
/// file order.
const ATTRIBUTE_COUNT: usize = 5;
const ATTRIBUTE_BASE: [f32; ATTRIBUTE_COUNT] = [50.0, 50.0, 50.0, 300.0, 300.0];
const ATTRIBUTE_INCREMENT: [f32; ATTRIBUTE_COUNT] = [4.0, 4.0, 4.0, 40.0, 40.0];

/// Distance between consecutive `temp` entries: `[len]["temp"][f32]`.
const TEMP_STRIDE: usize = 12;

/// The weapon-set quick-skill slots (`primarySkill1`…, …`5`).
const WEAPON_SET_SLOTS: u32 = 5;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RespecError {
    #[error("key \"{0}\" not found in the save")]
    MissingKey(&'static str),
    #[error(
        "attribute block not found: the five temp entries after skillPoints are missing or not consecutive"
    )]
    AttributesNotFound,
    #[error(
        "attribute {index} is {value} — below the fresh-character base; refusing to respec a modified save"
    )]
    AttributeBelowBase { index: usize, value: f32 },
    #[error(
        "attribute {index} is {value} — not a whole number of points above base; refusing to respec"
    )]
    AttributeNotWhole { index: usize, value: f32 },
    #[error("malformed skill list: {0}")]
    SkillList(#[from] ReadError),
}

/// The outcome of applying a respec: the new file bytes and what was
/// refunded (for the status line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respecced {
    pub bytes: Vec<u8>,
    pub refunded_points: i32,
    /// Skill respec only; 0 for attribute respec.
    pub skills_removed: usize,
}

/// Points an attribute respec would refund, without changing anything.
///
/// # Errors
/// [`RespecError`] when the attribute block cannot be located or the
/// values are not explainable as points spent from base.
pub fn attribute_refund(data: &[u8]) -> Result<i32, RespecError> {
    Ok(locate_attributes(data)?.refund)
}

/// Resets the five attributes to base and refunds the points into
/// `modifierPoints`. In-place patches only — the file length never
/// changes.
///
/// # Errors
/// See [`attribute_refund`].
pub fn respec_attributes(data: &[u8]) -> Result<Respecced, RespecError> {
    let plan = locate_attributes(data)?;
    let mut bytes = data.to_vec();
    for (index, value_at) in plan.temp_value_offsets.iter().enumerate() {
        bytes[*value_at..value_at + 4].copy_from_slice(&ATTRIBUTE_BASE[index].to_le_bytes());
    }
    let refunded = patch_i32_add(&mut bytes, "modifierPoints", plan.refund)?;
    debug_assert!(refunded);
    Ok(Respecced {
        bytes,
        refunded_points: plan.refund,
        skills_removed: 0,
    })
}

/// Points and skill count a skill respec would refund/remove.
///
/// # Errors
/// [`RespecError`] when the skill list cannot be parsed.
pub fn skill_refund(data: &[u8]) -> Result<(i32, usize), RespecError> {
    let list = parse_skill_list(data)?;
    Ok((list.refund(), list.removed_count()))
}

/// Removes every mastery-tree skill (both masteries included),
/// refunds their levels into `skillPoints`, clears `playerClassTag`
/// back to the fresh-character empty string, empties hotbar slots
/// that referenced removed skills, and resets the weapon-set skill
/// selections. Innate skills (`Skills\Default`, the shared
/// `AllMasteries` actions) and quest-granted skills are kept.
///
/// # Errors
/// See [`skill_refund`].
pub fn respec_skills(data: &[u8]) -> Result<Respecced, RespecError> {
    let list = parse_skill_list(data)?;
    let removed: Vec<String> = list
        .entries
        .iter()
        .filter(|entry| entry.removable)
        .map(|entry| normalize(&entry.name))
        .collect();
    let refund = list.refund();
    let skills_removed = removed.len();

    // Length-changing splices run back-to-front so earlier offsets
    // stay valid: hotbar slots, then the skill list, then the class
    // tag near the head of the file.
    let mut bytes = data.to_vec();
    for span in hotbar_slots_to_clear(&bytes, &removed).into_iter().rev() {
        let mut replacement = Vec::with_capacity(4);
        replacement.extend_from_slice(&(-1_i32).to_le_bytes());
        bytes.splice(span.0..span.1, replacement);
    }
    bytes = splice_skill_list(&bytes, &list);
    clear_player_class_tag(&mut bytes)?;

    // Fixed-size patches on the final layout.
    let patched = patch_i32_add(&mut bytes, "skillPoints", refund)?;
    debug_assert!(patched);
    for slot in 1..=WEAPON_SET_SLOTS {
        for prefix in ["primarySkill", "secondarySkill"] {
            patch_i32_set(&mut bytes, &format!("{prefix}{slot}"), 0);
        }
    }
    Ok(Respecced {
        bytes,
        refunded_points: refund,
        skills_removed,
    })
}

/// The five attribute values as stored in the save, in file order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attributes {
    pub strength: f32,
    pub dexterity: f32,
    pub intelligence: f32,
    pub health: f32,
    pub energy: f32,
}

/// One learned skill: the normalized record path and the points put
/// into it. `mastery` is true for mastery-tree skills — the ones a
/// respec would remove; innate defaults and quest-granted skills
/// carry false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedSkill {
    pub record: String,
    pub level: i32,
    pub mastery: bool,
}

/// Read-only snapshot of a character's spend state: attribute
/// values, unspent point pools, and the learned-skill list.
#[derive(Debug, Clone, PartialEq)]
pub struct Progression {
    pub attributes: Attributes,
    pub unspent_attribute_points: i32,
    pub unspent_skill_points: i32,
    pub skills: Vec<LearnedSkill>,
}

/// Reads the spend state without changing anything.
///
/// # Errors
/// [`RespecError`] when the attribute block, point pools, or skill
/// list cannot be located.
pub fn progression(data: &[u8]) -> Result<Progression, RespecError> {
    let plan = locate_attributes(data)?;
    let mut values = [0.0_f32; ATTRIBUTE_COUNT];
    for (value, offset) in values.iter_mut().zip(plan.temp_value_offsets) {
        *value = ByteReader::at(data, offset)
            .read_f32()
            .map_err(|_| RespecError::AttributesNotFound)?;
    }
    let list = parse_skill_list(data)?;
    Ok(Progression {
        attributes: Attributes {
            strength: values[0],
            dexterity: values[1],
            intelligence: values[2],
            health: values[3],
            energy: values[4],
        },
        unspent_attribute_points: read_i32_value(data, "modifierPoints")?,
        unspent_skill_points: read_i32_value(data, "skillPoints")?,
        skills: list
            .entries
            .into_iter()
            .map(|entry| LearnedSkill {
                record: normalize(&entry.name),
                level: entry.level,
                mastery: entry.removable,
            })
            .collect(),
    })
}

fn read_i32_value(data: &[u8], key: &'static str) -> Result<i32, RespecError> {
    let value_at = find_key(data, key, 0).ok_or(RespecError::MissingKey(key))?;
    ByteReader::at(data, value_at)
        .read_i32()
        .map_err(|_| RespecError::MissingKey(key))
}

struct AttributePlan {
    temp_value_offsets: [usize; ATTRIBUTE_COUNT],
    refund: i32,
}

fn locate_attributes(data: &[u8]) -> Result<AttributePlan, RespecError> {
    let anchor = find_key(data, "skillPoints", 0).ok_or(RespecError::MissingKey("skillPoints"))?;
    let mut offsets = [0_usize; ATTRIBUTE_COUNT];
    let mut from = anchor;
    for offset in &mut offsets {
        *offset = find_key(data, "temp", from).ok_or(RespecError::AttributesNotFound)?;
        from = *offset;
    }
    let consecutive = offsets
        .windows(2)
        .all(|pair| pair[1] - pair[0] == TEMP_STRIDE);
    if !consecutive {
        return Err(RespecError::AttributesNotFound);
    }
    let mut refund = 0_i32;
    for (index, value_at) in offsets.iter().enumerate() {
        let mut reader = ByteReader::at(data, *value_at);
        let value = reader
            .read_f32()
            .map_err(|_| RespecError::AttributesNotFound)?;
        let delta = value - ATTRIBUTE_BASE[index];
        if delta < -f32::EPSILON {
            return Err(RespecError::AttributeBelowBase { index, value });
        }
        #[allow(clippy::cast_possible_truncation)] // points fit in i32 by construction
        let points = (delta / ATTRIBUTE_INCREMENT[index]).round() as i32;
        #[allow(clippy::cast_precision_loss)] // small point counts
        let reconstructed =
            ATTRIBUTE_INCREMENT[index].mul_add(points as f32, ATTRIBUTE_BASE[index]);
        if (reconstructed - value).abs() > 0.01 {
            return Err(RespecError::AttributeNotWhole { index, value });
        }
        refund += points;
    }
    Ok(AttributePlan {
        temp_value_offsets: offsets,
        refund,
    })
}

struct SkillEntry {
    /// Byte span of the whole block, `begin_block` length prefix
    /// through the `end_block` value.
    span: (usize, usize),
    name: String,
    level: i32,
    removable: bool,
}

struct SkillList {
    /// Offset of the `max` count value.
    count_value_at: usize,
    entries: Vec<SkillEntry>,
}

impl SkillList {
    fn refund(&self) -> i32 {
        self.entries
            .iter()
            .filter(|entry| entry.removable)
            .map(|entry| entry.level)
            .sum()
    }

    fn removed_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.removable).count()
    }
}

/// Innate skills every character keeps: the default weapon actions
/// and the shared pet/taunt actions; quest-granted skills live
/// outside the skills directories entirely.
fn removable_skill(normalized: &str) -> bool {
    normalized.contains(r"\SKILLS\")
        && !normalized.contains(r"\SKILLS\DEFAULT\")
        && !normalized.contains("ALLMASTERIES")
}

fn parse_skill_list(data: &[u8]) -> Result<SkillList, RespecError> {
    let anchor =
        find_key(data, "playerClassTag", 0).ok_or(RespecError::MissingKey("playerClassTag"))?;
    let count_value_at = find_key(data, "max", anchor).ok_or(RespecError::MissingKey("max"))?;
    let mut reader = ByteReader::at(data, count_value_at);
    let count = reader.read_i32()?;
    let mut entries = Vec::new();
    for _ in 0..count.max(0) {
        let start = reader.pos();
        reader.expect_key("begin_block")?;
        let _ = reader.read_i32()?;
        reader.expect_key("skillName")?;
        let name = reader.read_cstring()?;
        reader.expect_key("skillLevel")?;
        let level = reader.read_i32()?;
        reader.expect_key("skillEnabled")?;
        let _ = reader.read_i32()?;
        reader.expect_key("skillSubLevel")?;
        let _ = reader.read_i32()?;
        reader.expect_key("skillActive")?;
        let _ = reader.read_i32()?;
        reader.expect_key("skillTransition")?;
        let _ = reader.read_i32()?;
        reader.expect_key("end_block")?;
        let _ = reader.read_i32()?;
        let removable = removable_skill(&normalize(&name));
        entries.push(SkillEntry {
            span: (start, reader.pos()),
            name,
            level,
            removable,
        });
    }
    Ok(SkillList {
        count_value_at,
        entries,
    })
}

fn splice_skill_list(data: &[u8], list: &SkillList) -> Vec<u8> {
    let kept: Vec<&SkillEntry> = list
        .entries
        .iter()
        .filter(|entry| !entry.removable)
        .collect();
    let list_end = list
        .entries
        .last()
        .map_or(list.count_value_at + 4, |entry| entry.span.1);
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..list.count_value_at]);
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    // the list held `count` entries already
    out.extend_from_slice(&(kept.len() as i32).to_le_bytes());
    for entry in &kept {
        out.extend_from_slice(&data[entry.span.0..entry.span.1]);
    }
    out.extend_from_slice(&data[list_end..]);
    out
}

/// Spans of hotbar slot payloads whose `skillName` is in `removed`:
/// from the `storedType` value through the `itemName` value, to be
/// replaced by the empty-slot marker `-1`.
fn hotbar_slots_to_clear(data: &[u8], removed: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(value_at) = find_key(data, "storedType", from) {
        from = value_at;
        let mut reader = ByteReader::at(data, value_at);
        let Ok(stored_type) = reader.read_i32() else {
            break;
        };
        if stored_type != 0 {
            continue;
        }
        if !reader.next_key_is("skillName") {
            continue;
        }
        let parsed: Result<(String, ()), ReadError> = (|| {
            reader.expect_key("skillName")?;
            let name = reader.read_cstring()?;
            reader.expect_key("isItemSkill")?;
            let _ = reader.read_i32()?;
            reader.expect_key("itemName")?;
            let _ = reader.read_cstring()?;
            Ok((name, ()))
        })();
        let Ok((name, ())) = parsed else { continue };
        if removed.contains(&normalize(&name)) {
            spans.push((value_at, reader.pos()));
        }
    }
    spans
}

/// Rewrites `playerClassTag` to the empty string a fresh character
/// carries.
fn clear_player_class_tag(bytes: &mut Vec<u8>) -> Result<(), RespecError> {
    let value_at =
        find_key(bytes, "playerClassTag", 0).ok_or(RespecError::MissingKey("playerClassTag"))?;
    let mut reader = ByteReader::at(bytes, value_at);
    let _ = reader
        .read_cstring()
        .map_err(|_| RespecError::MissingKey("playerClassTag"))?;
    bytes.splice(value_at..reader.pos(), 0_i32.to_le_bytes());
    Ok(())
}

/// Adds `delta` to the i32 at `key`; `Ok(true)` when patched.
fn patch_i32_add(bytes: &mut [u8], key: &'static str, delta: i32) -> Result<bool, RespecError> {
    let value_at = find_key(bytes, key, 0).ok_or(RespecError::MissingKey(key))?;
    let mut reader = ByteReader::at(bytes, value_at);
    let current = reader
        .read_i32()
        .map_err(|_| RespecError::MissingKey(key))?;
    let updated = current.saturating_add(delta);
    bytes[value_at..value_at + 4].copy_from_slice(&updated.to_le_bytes());
    Ok(true)
}

/// Sets the i32 at `key`, silently skipping absent keys (older save
/// variants may lack some weapon-set slots).
fn patch_i32_set(bytes: &mut [u8], key: &str, value: i32) {
    if let Some(value_at) = find_key(bytes, key, 0)
        && value_at + 4 <= bytes.len()
    {
        bytes[value_at..value_at + 4].copy_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chr::fixture::Fixture;

    fn skill_block(fixture: Fixture, name: &str, level: i32) -> Fixture {
        fixture
            .begin_block()
            .cstr("skillName", name)
            .keyed_int("skillLevel", level)
            .keyed_int("skillEnabled", 1)
            .keyed_int("skillSubLevel", 0)
            .keyed_int("skillActive", 0)
            .keyed_int("skillTransition", 0)
            .end_block()
    }

    fn hot_slot(fixture: Fixture, skill: &str) -> Fixture {
        fixture
            .keyed_int("storedType", 0)
            .cstr("skillName", skill)
            .keyed_int("isItemSkill", 0)
            .cstr("itemName", "")
    }

    /// Mirrors the layout probed from real AE saves: class tag, a
    /// stray non-attribute `temp`, the skill list, the point pools,
    /// the five attribute temps, and hotbar slots.
    fn sample_save() -> Vec<u8> {
        let mut fixture = Fixture::default()
            .cstr("playerClassTag", "tagCClass23")
            .keyed_int("playerLevel", 9)
            .keyed_f32("temp", 0.25)
            .keyed_int("max", 5);
        fixture = skill_block(
            fixture,
            "Records/Skills/Default/DefaultWPBasicAttack.dbr",
            1,
        );
        fixture = skill_block(
            fixture,
            "Records\\XPack3\\Skills\\AllMasteries\\All_Taunt.dbr",
            1,
        );
        fixture = skill_block(fixture, "Records\\Skills\\Earth\\EarthMastery.dbr", 11);
        fixture = skill_block(fixture, "Records\\Skills\\Earth\\VolcanicOrb.dbr", 6);
        fixture = skill_block(fixture, "Records\\Quests\\Rewards\\ChironsReward.dbr", 1);
        fixture = fixture
            .keyed_int("masteriesAllowed", 2)
            .keyed_int("primarySkill1", 3)
            .keyed_int("secondarySkill1", 0)
            .keyed_int("skillActive1", 1)
            .keyed_int("modifierPoints", 2)
            .keyed_int("skillPoints", 1)
            .keyed_f32("temp", 82.0)
            .keyed_f32("temp", 58.0)
            .keyed_f32("temp", 50.0)
            .keyed_f32("temp", 540.0)
            .keyed_f32("temp", 300.0);
        fixture = hot_slot(fixture, "Records\\Skills\\Earth\\VolcanicOrb.dbr");
        fixture = fixture.keyed_int("storedType", -1);
        fixture = hot_slot(fixture, "Records/Skills/Default/DefaultWeaponAttack.dbr");
        fixture.bytes
    }

    fn read_i32_at_key(data: &[u8], key: &str) -> i32 {
        let at = find_key(data, key, 0).unwrap();
        ByteReader::at(data, at).read_i32().unwrap()
    }

    #[test]
    fn attribute_refund_counts_points_from_deltas() {
        assert_eq!(attribute_refund(&sample_save()), Ok(16));
    }

    #[test]
    #[allow(clippy::float_cmp)] // fixture values are exact
    fn progression_reads_attributes_pools_and_skills() {
        let snapshot = progression(&sample_save()).unwrap();
        assert_eq!(snapshot.attributes.strength, 82.0);
        assert_eq!(snapshot.attributes.dexterity, 58.0);
        assert_eq!(snapshot.attributes.intelligence, 50.0);
        assert_eq!(snapshot.attributes.health, 540.0);
        assert_eq!(snapshot.attributes.energy, 300.0);
        assert_eq!(snapshot.unspent_attribute_points, 2);
        assert_eq!(snapshot.unspent_skill_points, 1);
        assert_eq!(snapshot.skills.len(), 5);
        let mastery: Vec<(&str, i32)> = snapshot
            .skills
            .iter()
            .filter(|skill| skill.mastery)
            .map(|skill| (skill.record.as_str(), skill.level))
            .collect();
        assert_eq!(
            mastery,
            [
                (r"RECORDS\SKILLS\EARTH\EARTHMASTERY.DBR", 11),
                (r"RECORDS\SKILLS\EARTH\VOLCANICORB.DBR", 6),
            ]
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // fixture values are exact
    fn attribute_respec_resets_to_base_and_refunds_in_place() {
        let original = sample_save();
        let respecced = respec_attributes(&original).unwrap();
        assert_eq!(respecced.refunded_points, 16);
        assert_eq!(respecced.bytes.len(), original.len());
        assert_eq!(read_i32_at_key(&respecced.bytes, "modifierPoints"), 18);
        let plan = locate_attributes(&respecced.bytes).unwrap();
        assert_eq!(plan.refund, 0);
        for (index, at) in plan.temp_value_offsets.iter().enumerate() {
            let value = ByteReader::at(&respecced.bytes, *at).read_f32().unwrap();
            assert_eq!(value, ATTRIBUTE_BASE[index]);
        }
        // The stray temp before the skill list is untouched.
        let stray_at = find_key(&respecced.bytes, "temp", 0).unwrap();
        let stray = ByteReader::at(&respecced.bytes, stray_at)
            .read_f32()
            .unwrap();
        assert_eq!(stray, 0.25);
        // Everything outside the six patched i32/f32 cells is identical.
        let differing = original
            .iter()
            .zip(&respecced.bytes)
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing <= 24, "{differing} bytes changed");
    }

    #[test]
    fn attribute_respec_refuses_modified_saves() {
        let below = Fixture::default()
            .keyed_int("skillPoints", 0)
            .keyed_f32("temp", 42.0)
            .keyed_f32("temp", 50.0)
            .keyed_f32("temp", 50.0)
            .keyed_f32("temp", 300.0)
            .keyed_f32("temp", 300.0)
            .bytes;
        assert_eq!(
            attribute_refund(&below),
            Err(RespecError::AttributeBelowBase {
                index: 0,
                value: 42.0
            })
        );
        let uneven = Fixture::default()
            .keyed_int("skillPoints", 0)
            .keyed_f32("temp", 51.0)
            .keyed_f32("temp", 50.0)
            .keyed_f32("temp", 50.0)
            .keyed_f32("temp", 300.0)
            .keyed_f32("temp", 300.0)
            .bytes;
        assert_eq!(
            attribute_refund(&uneven),
            Err(RespecError::AttributeNotWhole {
                index: 0,
                value: 51.0
            })
        );
    }

    #[test]
    fn skill_refund_counts_only_mastery_tree_skills() {
        assert_eq!(skill_refund(&sample_save()), Ok((17, 2)));
    }

    #[test]
    fn skill_respec_removes_trees_and_keeps_innate_and_quest_skills() {
        let respecced = respec_skills(&sample_save()).unwrap();
        assert_eq!(respecced.refunded_points, 17);
        assert_eq!(respecced.skills_removed, 2);
        let bytes = &respecced.bytes;

        let class_at = find_key(bytes, "playerClassTag", 0).unwrap();
        assert_eq!(ByteReader::at(bytes, class_at).read_cstring().unwrap(), "");

        let list = parse_skill_list(bytes).unwrap();
        let names: Vec<&str> = list
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "Records/Skills/Default/DefaultWPBasicAttack.dbr",
                "Records\\XPack3\\Skills\\AllMasteries\\All_Taunt.dbr",
                "Records\\Quests\\Rewards\\ChironsReward.dbr",
            ]
        );
        assert_eq!(read_i32_at_key(bytes, "max"), 3);
        assert_eq!(read_i32_at_key(bytes, "skillPoints"), 18);
        assert_eq!(read_i32_at_key(bytes, "primarySkill1"), 0);
        assert_eq!(read_i32_at_key(bytes, "skillActive1"), 1);

        // The VolcanicOrb hotbar slot is emptied; the kept default
        // skill's slot survives.
        let mut stored_types = Vec::new();
        let mut from = 0;
        while let Some(at) = find_key(bytes, "storedType", from) {
            stored_types.push(ByteReader::at(bytes, at).read_i32().unwrap());
            from = at;
        }
        assert_eq!(stored_types, vec![-1, -1, 0]);
        assert!(
            !String::from_utf8_lossy(bytes).contains("VolcanicOrb"),
            "removed skill still referenced"
        );

        // Respec is idempotent: a second pass finds nothing to refund.
        let again = respec_skills(bytes).unwrap();
        assert_eq!(again.refunded_points, 0);
        assert_eq!(again.skills_removed, 0);
        assert_eq!(again.bytes.len(), bytes.len());
    }
}
