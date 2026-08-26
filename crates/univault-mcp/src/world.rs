//! File discovery and loading — the IO half of the server. All
//! format knowledge stays in `univault-core`; this module only
//! decides which files to read and reads them.

use std::path::{Path, PathBuf};

use univault_core::cache::GameCache;
use univault_core::chr::{self, PlayerCharacter};
use univault_core::gamedata::GameData;
use univault_core::platform;
use univault_core::respec::{self, Progression};
use univault_core::stash::{self, Stash};
use univault_core::vault::Vault;

/// Where the server looks for everything. Environment variables
/// override; defaults come from the same config directory the GUI
/// writes (`game-dir.txt`, `recent-files.txt`, `vaults/`,
/// `gamedata.cache`).
#[derive(Debug, Clone)]
pub struct Paths {
    pub save_roots: Vec<PathBuf>,
    pub vaults_dir: Option<PathBuf>,
    pub cache_file: Option<PathBuf>,
    pub game_dir: Option<PathBuf>,
    pub custom_maps: Option<PathBuf>,
}

impl Paths {
    pub fn from_env() -> Self {
        let config = platform::config_dir();
        let save_roots = match std::env::var_os("UNIVAULT_SAVE_ROOT") {
            Some(root) => vec![PathBuf::from(root)],
            None => recent_save_roots(config.as_deref()),
        };
        let vaults_dir = std::env::var_os("UNIVAULT_VAULTS_DIR")
            .map(PathBuf::from)
            .or_else(|| config.as_ref().map(|dir| dir.join("vaults")));
        let cache_file = config.as_ref().map(|dir| dir.join("gamedata.cache"));
        let game_dir = std::env::var_os("UNIVAULT_GAME_DIR")
            .map(PathBuf::from)
            .or_else(|| stored_game_dir(config.as_deref()));
        let custom_maps = std::env::var_os("UNIVAULT_CUSTOMMAPS")
            .map(PathBuf::from)
            .or_else(|| custom_maps_near(&save_roots));
        Self {
            save_roots,
            vaults_dir,
            cache_file,
            game_dir,
            custom_maps,
        }
    }
}

/// The game's `CustomMaps` directory (installed mod bundles), which
/// sits beside the `SaveData` tree the save roots point into.
fn custom_maps_near(save_roots: &[PathBuf]) -> Option<PathBuf> {
    save_roots.iter().find_map(|root| {
        root.ancestors()
            .map(|ancestor| ancestor.join("CustomMaps"))
            .find(|candidate| candidate.is_dir())
    })
}

/// One installed mod bundle: the bundle folder's name and its
/// database file.
#[derive(Debug, Clone)]
pub struct ModEntry {
    pub name: String,
    pub arz_path: PathBuf,
}

pub fn list_mod_bundles(dir: &Path) -> Vec<ModEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut bundles: Vec<ModEntry> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|bundle| {
            let name = bundle.file_name()?.to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let arz_path = std::fs::read_dir(bundle.join("database"))
                .ok()?
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("arz"))
                })?;
            Some(ModEntry { name, arz_path })
        })
        .collect();
    bundles.sort_by(|a, b| a.name.cmp(&b.name));
    bundles
}

pub fn load_mod_db(path: &Path) -> Result<univault_core::arz::ArzFile, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    univault_core::arz::ArzFile::parse(bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Save roots implied by the GUI's recent-files list: each recent
/// `Player.chr` sits in a character folder inside a root.
fn recent_save_roots(config: Option<&Path>) -> Vec<PathBuf> {
    let Some(config) = config else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(config.join("recent-files.txt")) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    for line in text.lines() {
        let path = Path::new(line.trim());
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("chr"))
        {
            continue;
        }
        let Some(root) = path.parent().and_then(Path::parent) else {
            continue;
        };
        if root.is_dir() && !roots.contains(&root.to_path_buf()) {
            roots.push(root.to_path_buf());
        }
    }
    roots
}

fn stored_game_dir(config: Option<&Path>) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config?.join("game-dir.txt")).ok()?;
    let dir = PathBuf::from(text.trim());
    dir.is_dir().then_some(dir)
}

/// One discovered character: the folder's display name (the game
/// prefixes folders with `_`) and the `Player.chr` path.
#[derive(Debug, Clone)]
pub struct CharacterEntry {
    pub name: String,
    pub path: PathBuf,
}

/// `Player.chr` files one folder below each root (the game's
/// `SaveData/Main/_<name>/Player.chr` layout), plus a root that is
/// itself a character folder.
pub fn discover_characters(roots: &[PathBuf]) -> Vec<CharacterEntry> {
    let mut found: Vec<CharacterEntry> = Vec::new();
    let push = |dir: &Path, found: &mut Vec<CharacterEntry>| {
        let chr = dir.join("Player.chr");
        if chr.is_file() && !found.iter().any(|entry| entry.path == chr) {
            found.push(CharacterEntry {
                name: folder_display_name(dir),
                path: chr,
            });
        }
    };
    for root in roots {
        push(root, &mut found);
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if dir.is_dir() {
                push(&dir, &mut found);
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn folder_display_name(dir: &Path) -> String {
    let raw = dir
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    raw.strip_prefix('_').map_or(raw.clone(), str::to_string)
}

pub struct LoadedCharacter {
    pub player: PlayerCharacter,
    /// `None` when the save predates the probed layout — the rest of
    /// the character still loads.
    pub progression: Option<Progression>,
}

pub fn load_character(path: &Path) -> Result<LoadedCharacter, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let player = chr::parse_player(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let progression = respec::progression(&bytes).ok();
    Ok(LoadedCharacter {
        player,
        progression,
    })
}

/// The three banks reachable from a character's save file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankKind {
    Personal,
    Shared,
    Relic,
}

pub fn bank_path(chr_path: &Path, kind: BankKind) -> Option<PathBuf> {
    match kind {
        BankKind::Personal => platform::personal_stash_path(chr_path).filter(|path| path.is_file()),
        BankKind::Shared => {
            platform::transfer_stash_candidates(chr_path).find(|path| path.is_file())
        }
        BankKind::Relic => platform::relic_bank_candidates(chr_path).find(|path| path.is_file()),
    }
}

/// Reads and parses a stash, falling back to the game's complete
/// `.dxg` twin when the `.dxb` is unreadable (the game's own
/// recovery path). Read-only: the repaired bytes are never written.
pub fn load_stash(path: &Path) -> Result<Stash, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match stash::parse_stash(&bytes) {
        Ok(parsed) => Ok(parsed),
        Err(primary) => {
            let twin = path.with_extension("dxg");
            let repaired = std::fs::read(&twin)
                .ok()
                .and_then(|dxg| stash::restore_from_twin(&dxg).ok())
                .and_then(|dxb| stash::parse_stash(&dxb).ok());
            repaired.ok_or_else(|| format!("parse {}: {primary}", path.display()))
        }
    }
}

/// One vault file: the file stem and full path.
#[derive(Debug, Clone)]
pub struct VaultEntry {
    pub name: String,
    pub path: PathBuf,
}

pub fn list_vaults(dir: &Path) -> Vec<VaultEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut vaults: Vec<VaultEntry> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .map(|path| VaultEntry {
            name: path
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
            path,
        })
        .collect();
    vaults.sort_by(|a, b| a.name.cmp(&b.name));
    vaults
}

pub fn load_vault(path: &Path) -> Result<Vault, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Vault::from_json(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn load_cache(path: &Path) -> Option<GameCache> {
    GameCache::from_bytes(&std::fs::read(path).ok()?).ok()
}

/// Assembles `GameData` from the install's database and English
/// text — the inputs the mastery tools need.
pub fn load_game_data(game_dir: &Path) -> Result<GameData, String> {
    let database = std::fs::read(game_dir.join("Database/database.arz"))
        .map_err(|e| format!("read database.arz under {}: {e}", game_dir.display()))?;
    let text = std::fs::read(game_dir.join("Text/Text_EN.arc"))
        .map_err(|e| format!("read Text_EN.arc under {}: {e}", game_dir.display()))?;
    GameData::from_bytes(database, text).map_err(|e| format!("parse game data: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_characters_one_level_below_roots() {
        let base = std::env::temp_dir().join(format!("univault-mcp-test-{}", std::process::id()));
        let root = base.join("Main");
        for folder in ["_Alice", "_Bob", "junk"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join("_Alice/Player.chr"), b"x").unwrap();
        std::fs::write(root.join("_Bob/Player.chr"), b"x").unwrap();

        let found = discover_characters(std::slice::from_ref(&root));
        let names: Vec<&str> = found.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Alice", "Bob"]);
        assert!(found.iter().all(|entry| entry.path.ends_with("Player.chr")));

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn lists_vaults_sorted_by_stem() {
        let base = std::env::temp_dir().join(format!("univault-mcp-vaults-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("Zeta.json"), b"{}").unwrap();
        std::fs::write(base.join("Main Vault.json"), b"{}").unwrap();
        std::fs::write(base.join("notes.txt"), b"x").unwrap();

        let names: Vec<String> = list_vaults(&base).into_iter().map(|v| v.name).collect();
        assert_eq!(names, ["Main Vault", "Zeta"]);

        std::fs::remove_dir_all(&base).unwrap();
    }
}
