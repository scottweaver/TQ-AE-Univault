//! The MCP tool surface. Every tool re-reads the underlying files on
//! call and none of them writes anything — the read-only constraint
//! recorded in ARCHITECTURE.md.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use univault_core::arz::{ArzFile, normalize};
use univault_core::cache::GameCache;
use univault_core::chr::{self, AtlantisRelic, Item, ItemSeed, RecordId};
use univault_core::gamedata::GameData;
use univault_core::respec::Progression;
use univault_core::{skilltree, stats, style, vault};

use crate::gamedb::{self, IndexEntry, Provenance};
use crate::view;
use crate::world::{self, BankKind, CharacterEntry, ModEntry, Paths};

pub struct Univault {
    paths: Paths,
    cache: Option<GameCache>,
    game_data: Mutex<Option<Arc<GameData>>>,
    /// Parsed mod bundles by bundle name, loaded on first use.
    mods: Mutex<HashMap<String, Arc<ArzFile>>>,
    /// Search indexes by source (empty key = vanilla, else a bundle
    /// name); each is a one-time full-database decode.
    indexes: Mutex<HashMap<String, Arc<Vec<IndexEntry>>>>,
}

impl Univault {
    #[must_use]
    pub fn from_env() -> Self {
        let paths = Paths::from_env();
        let cache = paths.cache_file.as_deref().and_then(world::load_cache);
        Self {
            paths,
            cache,
            game_data: Mutex::new(None),
            mods: Mutex::new(HashMap::new()),
            indexes: Mutex::new(HashMap::new()),
        }
    }

    fn mod_entries(&self) -> Vec<ModEntry> {
        self.paths
            .custom_maps
            .as_deref()
            .map_or_else(Vec::new, world::list_mod_bundles)
    }

    /// Resolves the `mod` tool parameter to a loaded bundle. Omitted
    /// means "what the game is playing": the single installed bundle
    /// when there is exactly one, vanilla when there are none, and
    /// an error naming the choices when there are several.
    /// `"vanilla"` always disables the overlay.
    fn resolve_mod(&self, wanted: Option<&str>) -> Result<Option<(String, Arc<ArzFile>)>, String> {
        let bundles = self.mod_entries();
        let entry = match wanted {
            Some(name) if name.eq_ignore_ascii_case("vanilla") => return Ok(None),
            Some(name) => bundles
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    let names: Vec<&str> =
                        bundles.iter().map(|entry| entry.name.as_str()).collect();
                    format!(
                        "no mod bundle named '{name}' (installed: {})",
                        names.join(", ")
                    )
                })?,
            None => match bundles.len() {
                0 => return Ok(None),
                1 => &bundles[0],
                _ => {
                    let names: Vec<&str> =
                        bundles.iter().map(|entry| entry.name.as_str()).collect();
                    return Err(format!(
                        "several mod bundles installed ({}); pass mod: <name> or mod: \"vanilla\"",
                        names.join(", ")
                    ));
                }
            },
        };
        let mut loaded = self
            .mods
            .lock()
            .expect("lock poisoned only if a loader panicked");
        if let Some(db) = loaded.get(&entry.name) {
            return Ok(Some((entry.name.clone(), Arc::clone(db))));
        }
        let db = Arc::new(world::load_mod_db(&entry.arz_path)?);
        loaded.insert(entry.name.clone(), Arc::clone(&db));
        Ok(Some((entry.name.clone(), db)))
    }

    /// The search index for one source, built on first use (a full
    /// database decode, a few seconds for vanilla).
    fn index_for(
        &self,
        source: Option<&(String, Arc<ArzFile>)>,
    ) -> Result<Arc<Vec<IndexEntry>>, String> {
        let key = source.map_or(String::new(), |(name, _)| name.clone());
        {
            let indexes = self
                .indexes
                .lock()
                .expect("lock poisoned only if a builder panicked");
            if let Some(index) = indexes.get(&key) {
                return Ok(Arc::clone(index));
            }
        }
        let data = self.game_data()?;
        let built = Arc::new(match source {
            Some((_, mod_db)) => gamedb::mod_index(&data, mod_db),
            None => gamedb::vanilla_index(&data),
        });
        self.indexes
            .lock()
            .expect("lock poisoned only if a builder panicked")
            .insert(key, Arc::clone(&built));
        Ok(built)
    }

    fn characters(&self) -> Vec<CharacterEntry> {
        world::discover_characters(&self.paths.save_roots)
    }

    fn resolve_character(&self, character: &str) -> Result<CharacterEntry, String> {
        let as_path = Path::new(character);
        if as_path.is_file() {
            return Ok(CharacterEntry {
                name: as_path.parent().map_or_else(
                    || character.to_string(),
                    |dir| {
                        dir.file_name().map_or_else(
                            || character.to_string(),
                            |name| name.to_string_lossy().trim_start_matches('_').to_string(),
                        )
                    },
                ),
                path: as_path.to_path_buf(),
            });
        }
        let all = self.characters();
        let wanted = character.to_lowercase();
        all.iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(character))
            .or_else(|| {
                all.iter()
                    .find(|entry| entry.name.to_lowercase().contains(&wanted))
            })
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = all.iter().map(|entry| entry.name.as_str()).collect();
                format!(
                    "no character matching '{character}' (known: {})",
                    if names.is_empty() {
                        "none — set UNIVAULT_SAVE_ROOT or open a character in the GUI once"
                            .to_string()
                    } else {
                        names.join(", ")
                    }
                )
            })
    }

    fn resolve_vault(&self, wanted: &str) -> Result<world::VaultEntry, String> {
        let as_path = Path::new(wanted);
        if as_path.is_file() {
            return Ok(world::VaultEntry {
                name: as_path.file_stem().map_or_else(
                    || wanted.to_string(),
                    |stem| stem.to_string_lossy().into_owned(),
                ),
                path: as_path.to_path_buf(),
            });
        }
        let vaults = self.vaults();
        vaults
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(wanted))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = vaults.iter().map(|entry| entry.name.as_str()).collect();
                format!("no vault named '{wanted}' (known: {})", names.join(", "))
            })
    }

    fn vaults(&self) -> Vec<world::VaultEntry> {
        self.paths
            .vaults_dir
            .as_deref()
            .map_or_else(Vec::new, world::list_vaults)
    }

    fn game_data(&self) -> Result<Arc<GameData>, String> {
        let mut guard = self
            .game_data
            .lock()
            .expect("lock poisoned only if a loader panicked");
        if let Some(data) = guard.as_ref() {
            return Ok(Arc::clone(data));
        }
        let dir = self.paths.game_dir.as_deref().ok_or_else(|| {
            "no game install configured: set UNIVAULT_GAME_DIR or run the GUI import once"
                .to_string()
        })?;
        let data = Arc::new(world::load_game_data(dir)?);
        *guard = Some(Arc::clone(&data));
        Ok(data)
    }

    fn db(&self) -> Option<&GameCache> {
        self.cache.as_ref()
    }

    fn character_json(&self, entry: &CharacterEntry) -> Result<Value, String> {
        let loaded = world::load_character(&entry.path)?;
        let equipment: Vec<Value> = view::equipment_slot_names()
            .iter()
            .zip(&loaded.player.equipment.slots)
            .map(|(slot, item)| {
                json!({
                    "slot": slot,
                    "item": item.as_ref().map(|item| view::item_view(self.db(), item)),
                })
            })
            .collect();
        let sacks: Vec<Value> = loaded
            .player
            .sacks
            .iter()
            .enumerate()
            .map(|(index, sack)| {
                let (width, height) = chr::sack_dimensions(index);
                json!({
                    "sack": index,
                    "width": width,
                    "height": height,
                    "items": sack
                        .items
                        .iter()
                        .map(|item| view::item_view(self.db(), item))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({
            "name": loaded.player.info.name.clone().unwrap_or_else(|| entry.name.clone()),
            "path": entry.path.display().to_string(),
            "level": loaded.player.info.level,
            "class_tag": loaded.player.info.class_tag,
            "gold": loaded.player.info.money,
            "build": loaded.progression.as_ref().map(progression_json),
            "equipment": equipment,
            "inventory": sacks,
        }))
    }

    fn unique_bank_paths(&self, kind: BankKind) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for entry in self.characters() {
            if let Some(path) = world::bank_path(&entry.path, kind)
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
        paths
    }
}

fn progression_json(progression: &Progression) -> Value {
    json!({
        "attributes": {
            "strength": progression.attributes.strength,
            "dexterity": progression.attributes.dexterity,
            "intelligence": progression.attributes.intelligence,
            "health": progression.attributes.health,
            "energy": progression.attributes.energy,
        },
        "unspent_attribute_points": progression.unspent_attribute_points,
        "unspent_skill_points": progression.unspent_skill_points,
        "masteries": view::masteries_of(&progression.skills),
        "skills": progression
            .skills
            .iter()
            .map(|skill| json!({
                "record": skill.record,
                "level": skill.level,
                "mastery": skill.mastery,
            }))
            .collect::<Vec<_>>(),
    })
}

fn ok_json<T: serde::Serialize>(value: &T) -> CallToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => fail(format!("serialize response: {error}")),
    }
}

fn fail(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn or_fail<T: serde::Serialize>(result: Result<T, String>) -> CallToolResult {
    match result {
        Ok(value) => ok_json(&value),
        Err(message) => fail(message),
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CharacterParams {
    /// Character name (case-insensitive, substrings accepted) or a
    /// full path to a Player.chr file.
    pub character: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BankParams {
    /// Character name or Player.chr path the bank belongs to.
    pub character: String,
    /// Which bank: the character's personal bank, the account-wide
    /// shared bank, or the relic bank (Atlantis+).
    pub kind: BankKindParam,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BankKindParam {
    Personal,
    Shared,
    Relic,
}

impl From<BankKindParam> for BankKind {
    fn from(kind: BankKindParam) -> Self {
        match kind {
            BankKindParam::Personal => Self::Personal,
            BankKindParam::Shared => Self::Shared,
            BankKindParam::Relic => Self::Relic,
        }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct VaultParams {
    /// Vault name (file stem, case-insensitive) or a full path to a
    /// vault .json file.
    pub vault: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Case-insensitive substring matched against item names and
    /// record paths.
    pub query: String,
    /// Maximum hits to return (default 100).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MasteryParams {
    /// Mastery name (e.g. "Earth") or its record path.
    pub mastery: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ItemDetailsParams {
    /// The item's base record path (as returned by other tools).
    pub base_record: String,
    pub prefix_record: Option<String>,
    pub suffix_record: Option<String>,
    pub relic_record: Option<String>,
    pub relic_bonus_record: Option<String>,
    pub relic2_record: Option<String>,
    pub relic2_bonus_record: Option<String>,
    /// The item's seed; affects rolled stat values within ranges.
    pub seed: Option<i32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordParams {
    /// The record's database path, e.g.
    /// `records\creature\monster\ratman\ratman_01.dbr`.
    pub record: String,
    /// Mod bundle to overlay (omit = the single installed bundle;
    /// "vanilla" = no overlay).
    #[serde(rename = "mod")]
    pub mod_bundle: Option<String>,
    /// true = byte-faithful dump including template-default
    /// (all-zero) variables; default omits them.
    pub everything: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordSearchParams {
    /// Case-insensitive substring matched against record paths and
    /// localized display names (monsters, skills, items).
    pub query: String,
    /// Optional record class filter, e.g. "Monster",
    /// `Skill_AttackProjectile`, `LootTableDynWeight`.
    pub class: Option<String>,
    /// Mod bundle to overlay (omit = the single installed bundle;
    /// "vanilla" = no overlay). Overlay search also surfaces
    /// mod-added records and marks overridden ones.
    #[serde(rename = "mod")]
    pub mod_bundle: Option<String>,
    /// Maximum hits to return (default 50).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DiffRecordParams {
    /// The record's database path.
    pub record: String,
    /// Mod bundle to diff against vanilla (omit = the single
    /// installed bundle).
    #[serde(rename = "mod")]
    pub mod_bundle: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DiffModParams {
    /// Mod bundle to sweep (omit = the single installed bundle).
    #[serde(rename = "mod")]
    pub mod_bundle: Option<String>,
    /// Only report records where a changed variable's name contains
    /// this (case-insensitive), e.g. "cooldown".
    pub variable: Option<String>,
    /// Maximum changed records to list (default 100).
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct TagParams {
    /// A localization tag, e.g. "tagSkillName115".
    pub tag: String,
}

const DEFAULT_SEARCH_LIMIT: usize = 100;
const DEFAULT_RECORD_SEARCH_LIMIT: usize = 50;
const DEFAULT_DIFF_LIMIT: usize = 100;

// The rmcp macros generate `async fn`s that only await when a tool
// is itself async; ours are sync, so the generated bodies trip the
// unused-async lints.
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
#[tool_router]
impl Univault {
    #[tool(
        description = "What this server can see right now: game cache state, save roots, characters, banks, vaults, and the game install. Call this first to orient."
    )]
    fn overview(&self) -> CallToolResult {
        let characters = self.characters();
        let vaults = self.vaults();
        ok_json(&json!({
            "cache": self.cache.as_ref().map_or_else(
                || json!({"loaded": false, "note": "item names/stats degrade to record stems; run the GUI import once to build it"}),
                |cache| json!({"loaded": true, "records": cache.len()}),
            ),
            "save_roots": self.paths.save_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "characters": characters.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
            "shared_banks": self.unique_bank_paths(BankKind::Shared).len(),
            "relic_banks": self.unique_bank_paths(BankKind::Relic).len(),
            "vaults": vaults.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
            "game_dir": self.paths.game_dir.as_ref().map(|p| p.display().to_string()),
            "mods": self
                .mod_entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>(),
            "read_only": true,
        }))
    }

    #[tool(
        description = "List every Titan Quest character found under the save roots, with level, gold, masteries, and save path."
    )]
    fn list_characters(&self) -> CallToolResult {
        let mut out = Vec::new();
        for entry in self.characters() {
            match world::load_character(&entry.path) {
                Ok(loaded) => out.push(json!({
                    "name": loaded.player.info.name.clone().unwrap_or_else(|| entry.name.clone()),
                    "folder": entry.name,
                    "level": loaded.player.info.level,
                    "gold": loaded.player.info.money,
                    "masteries": loaded
                        .progression
                        .as_ref()
                        .map(|p| view::masteries_of(&p.skills)),
                    "path": entry.path.display().to_string(),
                })),
                Err(error) => out.push(json!({
                    "folder": entry.name,
                    "path": entry.path.display().to_string(),
                    "error": error,
                })),
            }
        }
        ok_json(&out)
    }

    #[tool(
        description = "Full detail for one character: stats, attributes, unspent points, masteries and per-skill levels (the build), all 12 equipment slots, and every inventory sack with items."
    )]
    fn get_character(&self, Parameters(params): Parameters<CharacterParams>) -> CallToolResult {
        or_fail(
            self.resolve_character(&params.character)
                .and_then(|entry| self.character_json(&entry)),
        )
    }

    #[tool(
        description = "Items in a character's personal bank, the shared bank, or the relic bank — the game's stash files."
    )]
    fn get_bank(&self, Parameters(params): Parameters<BankParams>) -> CallToolResult {
        let result = self.resolve_character(&params.character).and_then(|entry| {
            let kind = BankKind::from(params.kind);
            let path = world::bank_path(&entry.path, kind)
                .ok_or_else(|| format!("no {kind:?} bank found near {}", entry.path.display()))?;
            let stash = world::load_stash(&path)?;
            Ok(json!({
                "path": path.display().to_string(),
                "width": stash.width,
                "height": stash.height,
                "items": stash
                    .items
                    .iter()
                    .map(|item| view::item_view(self.db(), item))
                    .collect::<Vec<_>>(),
            }))
        });
        or_fail(result)
    }

    #[tool(description = "List the vault files (external item storage) this server can see.")]
    fn list_vaults(&self) -> CallToolResult {
        ok_json(
            &self
                .vaults()
                .iter()
                .map(|entry| {
                    json!({
                        "name": entry.name,
                        "path": entry.path.display().to_string(),
                    })
                })
                .collect::<Vec<_>>(),
        )
    }

    #[tool(description = "Every item in one vault, tab by tab.")]
    fn get_vault(&self, Parameters(params): Parameters<VaultParams>) -> CallToolResult {
        let result = self.resolve_vault(&params.vault).and_then(|entry| {
            let loaded = world::load_vault(&entry.path)?;
            Ok(json!({
                "name": entry.name,
                "path": entry.path.display().to_string(),
                "tab_width": vault::TAB_WIDTH,
                "tab_height": vault::TAB_HEIGHT,
                "tabs": loaded
                    .sacks
                    .iter()
                    .enumerate()
                    .map(|(index, sack)| json!({
                        "tab": index,
                        "items": sack
                            .items
                            .iter()
                            .map(|entry| view::item_view(self.db(), &entry.item))
                            .collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>(),
            }))
        });
        or_fail(result)
    }

    #[tool(
        description = "Search every possession — all characters' equipment, inventories and personal banks, the shared and relic banks, and every vault — by item name or record path. Each hit carries its exact location."
    )]
    fn search_items(&self, Parameters(params): Parameters<SearchParams>) -> CallToolResult {
        let wanted = params.query.to_lowercase();
        let limit = params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        let db = self.db();
        let mut hits: Vec<Value> = Vec::new();
        let mut total = 0_usize;
        let mut consider = |location: String, item: &Item| {
            let name = view::item_name(db, item);
            if !name.to_lowercase().contains(&wanted)
                && !item.base.as_str().to_lowercase().contains(&wanted)
            {
                return;
            }
            total += 1;
            if hits.len() < limit {
                let mut hit = json!(view::item_view(db, item));
                hit["location"] = json!(location);
                hits.push(hit);
            }
        };

        for entry in self.characters() {
            let Ok(loaded) = world::load_character(&entry.path) else {
                continue;
            };
            for (slot, item) in view::equipment_slot_names()
                .iter()
                .zip(&loaded.player.equipment.slots)
            {
                if let Some(item) = item {
                    consider(
                        format!("character '{}' › equipment › {slot}", entry.name),
                        item,
                    );
                }
            }
            for (index, sack) in loaded.player.sacks.iter().enumerate() {
                for item in &sack.items {
                    consider(format!("character '{}' › sack {index}", entry.name), item);
                }
            }
            if let Some(path) = world::bank_path(&entry.path, BankKind::Personal)
                && let Ok(stash) = world::load_stash(&path)
            {
                for item in &stash.items {
                    consider(format!("character '{}' › personal bank", entry.name), item);
                }
            }
        }
        for (kind, label) in [
            (BankKind::Shared, "shared bank"),
            (BankKind::Relic, "relic bank"),
        ] {
            for path in self.unique_bank_paths(kind) {
                if let Ok(stash) = world::load_stash(&path) {
                    for item in &stash.items {
                        consider(label.to_string(), item);
                    }
                }
            }
        }
        for entry in self.vaults() {
            let Ok(loaded) = world::load_vault(&entry.path) else {
                continue;
            };
            for (index, sack) in loaded.sacks.iter().enumerate() {
                for vault_item in &sack.items {
                    consider(
                        format!("vault '{}' › tab {index}", entry.name),
                        &vault_item.item,
                    );
                }
            }
        }

        ok_json(&json!({
            "query": params.query,
            "total_matches": total,
            "shown": hits.len(),
            "hits": hits,
        }))
    }

    #[tool(
        description = "Tooltip-grade stat lines for an item, given its record paths and seed (as returned by the other tools). Requires the game cache."
    )]
    fn get_item_details(
        &self,
        Parameters(params): Parameters<ItemDetailsParams>,
    ) -> CallToolResult {
        let Some(db) = self.db() else {
            return fail("no game cache loaded: run the GUI import once so item stats exist");
        };
        let parse = |field: &str, value: Option<String>| -> Result<Option<RecordId>, String> {
            match value {
                None => Ok(None),
                Some(raw) => RecordId::parse(raw)
                    .map(Some)
                    .ok_or_else(|| format!("{field} is empty")),
            }
        };
        let result = (|| {
            let base = parse("base_record", Some(params.base_record))?
                .ok_or_else(|| "base_record is empty".to_string())?;
            let mut item = Item::bare(base, ItemSeed::new(params.seed.unwrap_or(0)));
            item.prefix = parse("prefix_record", params.prefix_record)?;
            item.suffix = parse("suffix_record", params.suffix_record)?;
            item.relic = parse("relic_record", params.relic_record)?;
            item.relic_bonus = parse("relic_bonus_record", params.relic_bonus_record)?;
            let relic2 = parse("relic2_record", params.relic2_record)?;
            let relic2_bonus = parse("relic2_bonus_record", params.relic2_bonus_record)?;
            if relic2.is_some() || relic2_bonus.is_some() {
                item.atlantis = Some(AtlantisRelic {
                    relic: relic2,
                    bonus: relic2_bonus,
                    var2: vault::VAR2_DEFAULT,
                });
            }
            let details = stats::item_details(db, &item);
            Ok(json!({
                "name": view::item_name(Some(db), &item),
                "style": style::item_style(Some(db), &item).label(),
                "shards": style::relic_shards(Some(db), &item)
                    .map(|shards| json!({"have": shards.have, "needed": shards.needed})),
                "blocks": view::item_stat_blocks(db, &item),
                "quality": details.quality,
            }))
        })();
        or_fail(result)
    }

    #[tool(
        description = "List the playable masteries discovered in the game database. First call loads the database (a few seconds)."
    )]
    fn list_masteries(&self) -> CallToolResult {
        let result = self.game_data().map(|data| {
            skilltree::masteries(&data)
                .into_iter()
                .map(|mastery| {
                    json!({
                        "name": mastery.name,
                        "record": mastery.record,
                    })
                })
                .collect::<Vec<_>>()
        });
        or_fail(result)
    }

    #[tool(
        description = "One mastery's full skill tree: every skill with localized names, tiers, caps, per-level effect arrays, and the buff/pet records they reference. Large response (~100-500 KB)."
    )]
    fn get_mastery(&self, Parameters(params): Parameters<MasteryParams>) -> CallToolResult {
        let result = self.game_data().and_then(|data| {
            let masteries = skilltree::masteries(&data);
            let wanted = params.mastery.to_lowercase();
            let mastery = masteries
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(&params.mastery))
                .or_else(|| {
                    masteries
                        .iter()
                        .find(|m| m.record.to_lowercase().contains(&wanted))
                })
                .ok_or_else(|| {
                    let names: Vec<&str> = masteries.iter().map(|m| m.name.as_str()).collect();
                    format!(
                        "no mastery matching '{}' (known: {})",
                        params.mastery,
                        names.join(", ")
                    )
                })?;
            skilltree::mastery_tree(&data, mastery)
                .ok_or_else(|| format!("mastery '{}' has no skill directory", mastery.name))
        });
        or_fail(result)
    }

    #[tool(
        description = "List the mod bundles installed in the game's CustomMaps directory. Record tools overlay the single installed bundle by default, so mod changes are visible everywhere."
    )]
    fn list_mods(&self) -> CallToolResult {
        let bundles = self.mod_entries();
        ok_json(&json!({
            "custom_maps": self
                .paths
                .custom_maps
                .as_ref()
                .map(|dir| dir.display().to_string()),
            "bundles": bundles
                .iter()
                .map(|entry| json!({
                    "name": entry.name,
                    "database": entry.arz_path.display().to_string(),
                }))
                .collect::<Vec<_>>(),
            "default": match bundles.len() {
                0 => "vanilla (no bundles installed)".to_string(),
                1 => format!("'{}' overlays every record tool unless mod: \"vanilla\"", bundles[0].name),
                _ => "several bundles — record tools need an explicit mod: <name>".to_string(),
            },
        }))
    }

    #[tool(
        description = "Search the entire game database (monsters, skills, loot tables, equations — every record) by path substring or localized name, optionally filtered by record class. First call builds an index (a few seconds)."
    )]
    fn search_records(&self, Parameters(params): Parameters<RecordSearchParams>) -> CallToolResult {
        let result = (|| {
            let overlay = self.resolve_mod(params.mod_bundle.as_deref())?;
            let vanilla_index = self.index_for(None)?;
            let mod_index = match &overlay {
                Some(source) => Some(self.index_for(Some(source))?),
                None => None,
            };
            let query = params.query.to_uppercase();
            let class = params.class.as_deref();
            let limit = params.limit.unwrap_or(DEFAULT_RECORD_SEARCH_LIMIT);
            let matches = |entry: &IndexEntry| {
                (entry.path.contains(&query)
                    || entry
                        .name
                        .as_ref()
                        .is_some_and(|name| name.to_uppercase().contains(&query)))
                    && class.is_none_or(|class| entry.class.eq_ignore_ascii_case(class))
            };
            let overridden: std::collections::HashSet<&str> = mod_index
                .as_deref()
                .map(|index| index.iter().map(|entry| entry.path.as_str()).collect())
                .unwrap_or_default();
            let mut hits = Vec::new();
            let mut total = 0_usize;
            let mut push = |entry: &IndexEntry, source: &str| {
                total += 1;
                if hits.len() < limit {
                    let mut hit = json!({
                        "record": entry.path,
                        "class": entry.class,
                        "source": source,
                    });
                    if let Some(name) = &entry.name {
                        hit["name"] = json!(name);
                    }
                    hits.push(hit);
                }
            };
            let mod_name = overlay.as_ref().map(|(name, _)| name.as_str());
            for entry in vanilla_index.iter().filter(|entry| matches(entry)) {
                let source = if overridden.contains(entry.path.as_str()) {
                    Provenance::ModOverride.label(mod_name)
                } else {
                    Provenance::Vanilla.label(mod_name)
                };
                push(entry, &source);
            }
            if let Some(index) = mod_index.as_deref() {
                let vanilla_paths: std::collections::HashSet<&str> = vanilla_index
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect();
                for entry in index
                    .iter()
                    .filter(|entry| matches(entry) && !vanilla_paths.contains(entry.path.as_str()))
                {
                    push(entry, &Provenance::ModAdded.label(mod_name));
                }
            }
            Ok(json!({
                "query": params.query,
                "total_matches": total,
                "shown": hits.len(),
                "hits": hits,
            }))
        })();
        or_fail(result)
    }

    #[tool(
        description = "One database record in full — every variable with values (per-difficulty arrays included) and translated text where tags resolve. Reflects the installed mod's version by default; source says where the bytes came from."
    )]
    fn get_record(&self, Parameters(params): Parameters<RecordParams>) -> CallToolResult {
        let result = (|| {
            let overlay = self.resolve_mod(params.mod_bundle.as_deref())?;
            let data = self.game_data()?;
            let id = RecordId::parse(params.record.clone())
                .ok_or_else(|| "record path is empty".to_string())?;
            let mod_name = overlay.as_ref().map(|(name, _)| name.as_str());
            let (record, provenance) =
                gamedb::effective_record(&data, overlay.as_ref().map(|(_, db)| db.as_ref()), &id)
                    .ok_or_else(|| {
                    format!(
                        "no record at '{}' — search_records finds exact paths",
                        params.record
                    )
                })?;
            Ok(gamedb::record_json(
                &data,
                &record,
                &provenance.label(mod_name),
                params.everything.unwrap_or(false),
            ))
        })();
        or_fail(result)
    }

    #[tool(
        description = "How the installed mod changes one record vs vanilla: changed variables side by side, plus variables only one side has."
    )]
    fn diff_record(&self, Parameters(params): Parameters<DiffRecordParams>) -> CallToolResult {
        let result = (|| {
            let (mod_name, mod_db) = self
                .resolve_mod(params.mod_bundle.as_deref())?
                .ok_or_else(|| "no mod bundle installed — nothing to diff".to_string())?;
            let data = self.game_data()?;
            let id = RecordId::parse(params.record.clone())
                .ok_or_else(|| "record path is empty".to_string())?;
            let modded = match mod_db.record(&id) {
                Some(Ok(record)) => record,
                Some(Err(error)) => return Err(format!("mod record unreadable: {error}")),
                None => {
                    return Ok(json!({
                        "record": normalize(&params.record),
                        "mod": mod_name,
                        "verdict": "the mod does not touch this record — the vanilla version is effective",
                    }));
                }
            };
            let vanilla = match data.record(&id) {
                Some(Ok(record)) => record,
                Some(Err(error)) => return Err(format!("vanilla record unreadable: {error}")),
                None => {
                    return Ok(json!({
                        "record": normalize(&params.record),
                        "mod": mod_name,
                        "verdict": "mod-added record with no vanilla counterpart — get_record shows it in full",
                    }));
                }
            };
            let mut out = gamedb::diff_json(vanilla.variables(), modded.variables());
            out["record"] = json!(normalize(&params.record));
            out["mod"] = json!(mod_name);
            Ok(out)
        })();
        or_fail(result)
    }

    #[tool(
        description = "Everything the installed mod changes vs vanilla: every overridden record with the names of its changed variables, and every mod-added record. Filter by variable name to hunt one kind of change."
    )]
    fn diff_mod(&self, Parameters(params): Parameters<DiffModParams>) -> CallToolResult {
        let result = (|| {
            let (mod_name, mod_db) = self
                .resolve_mod(params.mod_bundle.as_deref())?
                .ok_or_else(|| "no mod bundle installed — nothing to diff".to_string())?;
            let data = self.game_data()?;
            let filter = params.variable.map(|variable| variable.to_uppercase());
            let limit = params.limit.unwrap_or(DEFAULT_DIFF_LIMIT);
            let mut changed = Vec::new();
            let mut changed_total = 0_usize;
            let mut added = Vec::new();
            let mut identical = 0_usize;
            for id in mod_db.record_ids() {
                let Some(Ok(modded)) = mod_db.record(id) else {
                    continue;
                };
                let Some(Ok(vanilla)) = data.record(id) else {
                    added.push(normalize(id.as_str()));
                    continue;
                };
                let mut names = gamedb::changed_variable_names(&vanilla, &modded);
                if let Some(filter) = &filter {
                    names.retain(|name| name.to_uppercase().contains(filter));
                }
                if names.is_empty() {
                    identical += 1;
                    continue;
                }
                changed_total += 1;
                if changed.len() < limit {
                    changed.push(json!({
                        "record": normalize(id.as_str()),
                        "changed_variables": names,
                    }));
                }
            }
            Ok(json!({
                "mod": mod_name,
                "records_changed": changed_total,
                "records_added": added.len(),
                "records_carried_unchanged": identical,
                "shown": changed.len(),
                "changes": changed,
                "added": added,
                "note": "diff_record shows any record's before/after values",
            }))
        })();
        or_fail(result)
    }

    #[tool(
        description = "Translate one localization tag (e.g. tagSkillName115) to its English text."
    )]
    fn translate_tag(&self, Parameters(params): Parameters<TagParams>) -> CallToolResult {
        let result = self.game_data().map(|data| {
            json!({
                "tag": params.tag,
                "text": data.tag_text(&params.tag),
            })
        });
        or_fail(result)
    }
}

// Same as above: the generated `call_tool`/`list_tools` are async
// with nothing to await because every tool here is sync.
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
#[tool_handler]
impl ServerHandler for Univault {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::default();
        info.server_info.name = "univault-mcp".into();
        info.server_info.version = env!("CARGO_PKG_VERSION").into();
        info.instructions = Some(
            "Read-only access to a Titan Quest Anniversary Edition install: \
             characters (stats, builds, equipment, inventory), the personal/shared/relic \
             banks, TQVaultAE-compatible vault files, item stat sheets, mastery skill \
             trees, and the entire game database — every record (monsters, skills, loot \
             tables, equations) via search_records/get_record, with the installed \
             CustomMaps mod bundle overlaid by default and diffable against vanilla \
             (diff_record/diff_mod). Nothing is ever written. Call `overview` first to \
             see what is configured; paths come from the tq-univault GUI's config and \
             the UNIVAULT_SAVE_ROOT / UNIVAULT_VAULTS_DIR / UNIVAULT_GAME_DIR / \
             UNIVAULT_CUSTOMMAPS environment variables."
                .into(),
        );
        info
    }
}
