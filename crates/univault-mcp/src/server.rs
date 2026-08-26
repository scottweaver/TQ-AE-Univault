//! The MCP tool surface. Every tool re-reads the underlying files on
//! call and none of them writes anything — the read-only constraint
//! recorded in ARCHITECTURE.md.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use univault_core::cache::GameCache;
use univault_core::chr::{self, AtlantisRelic, Item, ItemSeed, RecordId};
use univault_core::gamedata::GameData;
use univault_core::respec::Progression;
use univault_core::{skilltree, stats, style, vault};

use crate::view;
use crate::world::{self, BankKind, CharacterEntry, Paths};

pub struct Univault {
    paths: Paths,
    cache: Option<GameCache>,
    game_data: Mutex<Option<Arc<GameData>>>,
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
        }
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
        let equipment: Vec<Value> = view::EQUIPMENT_SLOT_NAMES
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

const DEFAULT_SEARCH_LIMIT: usize = 100;

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
            for (slot, item) in view::EQUIPMENT_SLOT_NAMES
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
}

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
             banks, TQVaultAE-compatible vault files, item stat sheets, and mastery \
             skill trees. Nothing is ever written. Call `overview` first to see what is \
             configured; paths come from the tq-univault GUI's config and the \
             UNIVAULT_SAVE_ROOT / UNIVAULT_VAULTS_DIR / UNIVAULT_GAME_DIR environment \
             variables."
                .into(),
        );
        info
    }
}
