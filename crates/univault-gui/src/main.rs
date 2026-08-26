//! egui/eframe front-end for tq-univault.
//!
//! Usage: `univault-gui [--game <TQ install dir>] [--vault <vault.json>] [file]`
//!
//! Left pane: the character (`Player.chr`) with, discovered
//! automatically beside it, the character's bank (its `winsys.dxb`)
//! and the account's shared bank (`SaveData/Sys/winsys.dxb`). Right
//! pane: a vault — the default vault under the config directory
//! opens (and is created) at launch; `Open vault…` swaps in any
//! other vault file. Click or drag items across, save per file.
//! Saves splice only the item region and go through the backup-first
//! write path; stashes also get their `.dxg` twin rewritten.
//! Drag-and-drop routes files by extension.

mod safe_write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use univault_core::cache::{GameCache, SourceStamp};
use univault_core::chr::{self, Item, PlayerCharacter, RecordId};
use univault_core::gamedata::GameData;
use univault_core::respec;
use univault_core::stash::{self, Stash};
use univault_core::stats;
use univault_core::style;
use univault_core::transfer;
use univault_core::vault::Vault;

fn main() -> eframe::Result {
    let args = CliArgs::parse();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 900.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "TQ UniVault",
        options,
        Box::new(move |cc| {
            cc.egui_ctx
                .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
            Ok(Box::new(App::new(args)))
        }),
    )
}

struct CliArgs {
    game_dir: Option<PathBuf>,
    vault: Option<PathBuf>,
    file: Option<PathBuf>,
}

impl CliArgs {
    fn parse() -> Self {
        Self::from_args(std::env::args_os().skip(1))
    }

    fn from_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Self {
        let mut game_dir = None;
        let mut vault = None;
        let mut file = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--game" {
                game_dir = args.next().map(PathBuf::from);
            } else if arg == "--vault" {
                vault = args.next().map(PathBuf::from);
            } else {
                file = Some(PathBuf::from(arg));
            }
        }
        Self {
            game_dir,
            vault,
            file,
        }
    }
}

struct CharacterPane {
    path: PathBuf,
    original: Vec<u8>,
    character: Box<PlayerCharacter>,
    dirty: bool,
}

struct StashPane {
    path: PathBuf,
    original: Vec<u8>,
    stash: Stash,
    dirty: bool,
}

/// Which of the two stash documents a path or action addresses: the
/// character's own bank or the account-wide shared bank.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StashSlot {
    Bank,
    Shared,
}

struct VaultPane {
    path: PathBuf,
    vault: Vault,
    dirty: bool,
    selected: Option<(GridId, usize)>,
}

enum GameStatus {
    Absent,
    Importing(ImportJob),
    Loaded(GameCache),
    Failed(String),
}

/// A game-data import running on a background thread; the window
/// stays live and shows its progress. The thread finishes with
/// `Done` or `Failed`.
struct ImportJob {
    receiver: std::sync::mpsc::Receiver<ImportEvent>,
    progress: ImportProgress,
}

enum ImportEvent {
    Progress(ImportProgress),
    Done(Box<GameCache>),
    Failed(String),
}

#[derive(Clone)]
struct ImportProgress {
    label: String,
    fraction: Option<f32>,
}

/// Resolved display names, cached per record path — name resolution
/// decompresses database records and must not run per frame.
#[derive(Default)]
struct NameCache {
    names: HashMap<String, String>,
}

impl NameCache {
    fn record_name(&mut self, db: Option<&GameCache>, id: &RecordId) -> String {
        if let Some(cached) = self.names.get(id.as_str()) {
            return cached.clone();
        }
        let resolved = db
            .and_then(|db| db.record_name(id))
            .unwrap_or_else(|| id.file_stem().to_string());
        self.names.insert(id.as_str().to_string(), resolved.clone());
        resolved
    }

    fn item_label(&mut self, db: Option<&GameCache>, item: &Item) -> String {
        let mut parts = Vec::new();
        if let Some(prefix) = &item.prefix {
            parts.push(self.record_name(db, prefix));
        }
        parts.push(self.record_name(db, &item.base));
        if let Some(suffix) = &item.suffix {
            parts.push(self.record_name(db, suffix));
        }
        let mut label = parts.join(" ");
        if item.stack_size > 1 {
            use std::fmt::Write as _;
            let _ = write!(label, " ×{}", item.stack_size);
        }
        label
    }
}

/// Per-record caches for what must never run per frame: record
/// decompression, footprint lookups, texture decodes.
#[derive(Default)]
struct Caches {
    names: NameCache,
    footprints: HashMap<String, (i32, i32)>,
    icons: HashMap<String, Option<egui::TextureHandle>>,
}

impl Caches {
    fn footprint(&mut self, db: Option<&GameCache>, item: &Item) -> (i32, i32) {
        if let Some(cached) = self.footprints.get(item.base.as_str()) {
            return *cached;
        }
        let footprint = db.map_or(univault_core::gamedata::FALLBACK_FOOTPRINT, |db| {
            db.item_footprint(item)
        });
        self.footprints
            .insert(item.base.as_str().to_string(), footprint);
        footprint
    }

    fn icon(
        &mut self,
        ctx: &egui::Context,
        db: Option<&GameCache>,
        item: &Item,
    ) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.icons.get(item.base.as_str()) {
            return cached.clone();
        }
        let handle = db.and_then(|db| db.item_icon(item)).map(|image| {
            let pixels = egui::ColorImage::from_rgba_unmultiplied(
                [image.width, image.height],
                &image.pixels,
            );
            ctx.load_texture(
                item.base.as_str().to_string(),
                pixels,
                egui::TextureOptions::LINEAR,
            )
        });
        self.icons
            .insert(item.base.as_str().to_string(), handle.clone());
        handle
    }
}

/// Recently opened game files, persisted one path per line under the
/// platform config directory.
struct Recents {
    file: Option<PathBuf>,
    entries: Vec<PathBuf>,
}

const RECENTS_CAP: usize = 10;

impl Recents {
    fn load() -> Self {
        let file = univault_core::platform::config_dir().map(|dir| dir.join("recent-files.txt"));
        let entries = file
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| {
                text.lines()
                    .map(PathBuf::from)
                    .filter(|path| path.exists())
                    .take(RECENTS_CAP)
                    .collect()
            })
            .unwrap_or_default();
        Self { file, entries }
    }

    fn remember(&mut self, path: &Path) {
        self.entries.retain(|existing| existing != path);
        self.entries.insert(0, path.to_path_buf());
        self.entries.truncate(RECENTS_CAP);
        if let Some(file) = &self.file {
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let text = self
                .entries
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let _ = std::fs::write(file, text);
        }
    }

    /// "Pally Don" for `.../_Pally Don/Player.chr`, otherwise
    /// "folder — file".
    fn label(path: &Path) -> String {
        let folder = path
            .parent()
            .and_then(std::path::Path::file_name)
            .map(|name| name.to_string_lossy().trim_start_matches('_').to_string())
            .unwrap_or_default();
        let file_name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().to_string(),
        );
        if file_name.eq_ignore_ascii_case("Player.chr") {
            folder
        } else {
            format!("{folder} — {file_name}")
        }
    }
}

struct App {
    game: GameStatus,
    /// Standing advisory about the game cache (e.g. sources changed
    /// since import), distinct from the last-action status line.
    game_note: Option<String>,
    caches: Caches,
    recents: Recents,
    character: Option<CharacterPane>,
    bank: Option<StashPane>,
    shared: Option<StashPane>,
    right: Option<VaultPane>,
    /// The one selected item across all left-side grids (sacks and
    /// both banks); the vault keeps its own so cross-pane moves can
    /// aim at the other pane's last selection.
    left_selected: Option<(GridId, usize)>,
    status: Option<Result<String, String>>,
    pending_respec: Option<PendingRespec>,
    drag: Option<DragState>,
    /// Zoom shown by the slider while dragging; applied on release.
    pending_zoom: f32,
}

impl App {
    fn new(args: CliArgs) -> Self {
        // --game forces a (re-)import; otherwise the local cache is
        // the runtime database, imported automatically (in the
        // background) from the remembered game dir when it is
        // missing or in an older format.
        let mut game_note = None;
        let game = if let Some(dir) = args.game_dir.clone() {
            GameStatus::Importing(start_import(dir))
        } else if let Some(cache) = load_cached_game_data() {
            game_note = staleness_warning(&cache);
            GameStatus::Loaded(cache)
        } else if let Some(dir) = stored_game_dir() {
            GameStatus::Importing(start_import(dir))
        } else {
            GameStatus::Absent
        };
        let mut app = Self {
            game,
            game_note,
            caches: Caches::default(),
            recents: Recents::load(),
            character: None,
            bank: None,
            shared: None,
            right: None,
            left_selected: None,
            status: None,
            pending_respec: None,
            drag: None,
            pending_zoom: 1.0,
        };
        app.status = Some(match args.vault {
            Some(path) => app.open(&path),
            None => app.open_default_vault(),
        });
        if let Some(path) = args.file {
            app.status = Some(app.open(&path));
        }
        app
    }

    /// Routes a path into the matching pane by extension.
    fn open(&mut self, path: &Path) -> Result<String, String> {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
        match extension.as_deref() {
            Some("json") => self.open_vault(path),
            Some("vault") => self.import_legacy_vault(path),
            Some("dxb" | "dxg") => {
                let slot = stash_slot_for(path);
                let opened = self.open_stash(slot, path)?;
                self.recents.remember(path);
                Ok(opened)
            }
            _ => self.open_character_file(path),
        }
    }

    /// Opens a character and discovers its companions: the bank
    /// beside it and the shared bank up the save tree. Missing or
    /// unreadable companions never fail the character open — they
    /// are reported in the status line.
    fn open_character_file(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let character = Box::new(chr::parse_player(&bytes).map_err(|error| error.to_string())?);
        self.character = Some(CharacterPane {
            path: path.to_path_buf(),
            original: bytes,
            character,
            dirty: false,
        });
        self.left_selected = None;
        self.recents.remember(path);

        let mut notes = Vec::new();
        match univault_core::platform::personal_stash_path(path) {
            Some(bank) if bank.is_file() => match self.open_stash(StashSlot::Bank, &bank) {
                Ok(_) => notes.push("bank loaded".to_string()),
                Err(error) => notes.push(format!("bank unreadable: {error}")),
            },
            Some(_) | None => {
                self.bank = None;
                notes.push("no bank file yet".to_string());
            }
        }
        let shared = univault_core::platform::transfer_stash_candidates(path)
            .find(|candidate| candidate.is_file());
        match shared {
            Some(shared) => match self.open_stash(StashSlot::Shared, &shared) {
                Ok(_) => notes.push("shared bank loaded".to_string()),
                Err(error) => notes.push(format!("shared bank unreadable: {error}")),
            },
            None => notes.push("no shared bank found".to_string()),
        }
        Ok(format!("opened {} ({})", path.display(), notes.join(", ")))
    }

    /// Parses a stash file into its slot. A dirty pane already
    /// holding the same path is kept as-is — reopening must not
    /// discard unsaved edits.
    fn open_stash(&mut self, slot: StashSlot, path: &Path) -> Result<String, String> {
        let pane = match slot {
            StashSlot::Bank => &mut self.bank,
            StashSlot::Shared => &mut self.shared,
        };
        if pane
            .as_ref()
            .is_some_and(|pane| pane.path == path && pane.dirty)
        {
            return Ok(format!(
                "{} already open with unsaved edits",
                path.display()
            ));
        }
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let stash = stash::parse_stash(&bytes).map_err(|error| error.to_string())?;
        *pane = Some(StashPane {
            path: path.to_path_buf(),
            original: bytes,
            stash,
            dirty: false,
        });
        self.left_selected = None;
        Ok(format!("opened {}", path.display()))
    }

    fn open_vault(&mut self, path: &Path) -> Result<String, String> {
        let vault = if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            Vault::from_json(&text).map_err(|error| error.to_string())?
        } else {
            Vault::new(12)
        };
        let created = !path.exists();
        self.right = Some(VaultPane {
            path: path.to_path_buf(),
            vault,
            dirty: created,
            selected: None,
        });
        Ok(if created {
            format!(
                "new vault (12 tabs) — will be created at {}",
                path.display()
            )
        } else {
            format!("opened {}", path.display())
        })
    }

    /// Legacy vaults are import-only: the pane's save path becomes the
    /// `.json` sibling so the binary original is never written.
    fn import_legacy_vault(&mut self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let vault = Vault::from_legacy_binary(&bytes).map_err(|error| error.to_string())?;
        let json_path = path.with_extension("json");
        self.right = Some(VaultPane {
            path: json_path.clone(),
            vault,
            dirty: true,
            selected: None,
        });
        Ok(format!(
            "imported legacy vault; saving writes {}",
            json_path.display()
        ))
    }

    /// Opens the standing default vault, creating the file on first
    /// launch so a vault exists without any setup. `Open vault…`
    /// still swaps in any other vault file.
    fn open_default_vault(&mut self) -> Result<String, String> {
        let path = default_vault_path().ok_or("no config directory on this platform")?;
        if !path.exists() {
            let empty = Vault::new(12);
            let json = empty.to_json().map_err(|error| error.to_string())?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&path, json).map_err(|error| error.to_string())?;
            self.right = Some(VaultPane {
                path: path.clone(),
                vault: empty,
                dirty: false,
                selected: None,
            });
            return Ok(format!("created default vault at {}", path.display()));
        }
        self.open_vault(&path)
    }

    /// Removes the item at `(grid, index)` from its left-side
    /// document. `Err` when the document is gone or the index stale.
    fn take_from_left(&mut self, grid: GridId, index: usize) -> Result<Item, String> {
        match grid {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or("no character loaded")?;
                transfer::take_from_character(&mut pane.character, sack, index)
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or("no bank loaded")?;
                transfer::take_from_stash(&mut pane.stash, index)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or("no shared bank loaded")?;
                transfer::take_from_stash(&mut pane.stash, index)
            }
            GridId::VaultTab(_) => None,
        }
        .ok_or_else(|| "selection is stale — pick the item again".to_string())
    }

    /// Auto-places an item back into the left-side document it was
    /// taken from; `false` when even that fails.
    fn restore_to_left(&mut self, grid: GridId, item: Item) -> bool {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        match grid {
            GridId::Sack(sack) => self.character.as_mut().is_some_and(|pane| {
                transfer::place_in_character(&mut pane.character, item, sack, db).is_ok()
            }),
            GridId::Bank => self
                .bank
                .as_mut()
                .is_some_and(|pane| transfer::place_in_stash(&mut pane.stash, item, db).is_ok()),
            GridId::Shared => self
                .shared
                .as_mut()
                .is_some_and(|pane| transfer::place_in_stash(&mut pane.stash, item, db).is_ok()),
            GridId::VaultTab(_) => false,
        }
    }

    fn move_left_to_vault(&mut self) -> Result<String, String> {
        let (grid, index) = self.left_selected.ok_or("select an item on the left")?;
        if self.right.is_none() {
            return Err("load a vault first".to_string());
        }
        let item = self.take_from_left(grid, index)?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &item);
        let vault_pane = self.right.as_mut().expect("checked above");
        let preferred = vault_pane.selected.map_or(0, |(target, _)| match target {
            GridId::VaultTab(tab) => tab,
            GridId::Sack(_) | GridId::Bank | GridId::Shared => 0,
        });
        match transfer::place_in_vault(&mut vault_pane.vault, item, preferred, db) {
            Ok(tab) => {
                vault_pane.dirty = true;
                self.mark_dirty(grid);
                self.left_selected = None;
                Ok(format!("{label} → vault tab {}", tab + 1))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                let restored = self.restore_to_left(grid, *rejected.item);
                Err(if restored {
                    format!("{reason}; item returned to its container")
                } else {
                    format!("{reason}; item could not be returned — reload without saving")
                })
            }
        }
    }

    /// The left-side document a vault item lands in: the current
    /// selection's document, else the first loaded one.
    fn left_destination(&self) -> Option<GridId> {
        match self.left_selected {
            Some((grid @ (GridId::Sack(_) | GridId::Bank | GridId::Shared), _)) => Some(grid),
            Some((GridId::VaultTab(_), _)) | None => {
                if self.character.is_some() {
                    Some(GridId::Sack(0))
                } else if self.bank.is_some() {
                    Some(GridId::Bank)
                } else if self.shared.is_some() {
                    Some(GridId::Shared)
                } else {
                    None
                }
            }
        }
    }

    fn move_vault_to_left(&mut self) -> Result<String, String> {
        let destination = self
            .left_destination()
            .ok_or("load a character or bank first")?;
        let vault_pane = self.right.as_mut().ok_or("load a vault first")?;
        let Some((GridId::VaultTab(tab), index)) = vault_pane.selected else {
            return Err("select an item in the vault".to_string());
        };
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let vault_item = transfer::take_from_vault(&mut vault_pane.vault, tab, index)
            .ok_or("selection is stale — pick the item again")?;
        let label = self.caches.names.item_label(db, &vault_item.item);
        let placed = match destination {
            GridId::Sack(preferred) => match self.character.as_mut() {
                Some(pane) => transfer::place_in_character(
                    &mut pane.character,
                    vault_item.item,
                    preferred,
                    db,
                )
                .map(|sack| format!("{label} → sack {}", sack + 1)),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Bank => match self.bank.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, vault_item.item, db)
                    .map(|()| format!("{label} → bank")),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::Shared => match self.shared.as_mut() {
                Some(pane) => transfer::place_in_stash(&mut pane.stash, vault_item.item, db)
                    .map(|()| format!("{label} → shared bank")),
                None => Err(transfer::Rejected {
                    item: Box::new(vault_item.item),
                    reason: transfer::TransferError::BadIndex,
                }),
            },
            GridId::VaultTab(_) => Err(transfer::Rejected {
                item: Box::new(vault_item.item),
                reason: transfer::TransferError::BadIndex,
            }),
        };
        match placed {
            Ok(message) => {
                self.mark_dirty(destination);
                let vault_pane = self.right.as_mut().expect("still loaded");
                vault_pane.dirty = true;
                vault_pane.selected = None;
                Ok(message)
            }
            Err(rejected) => {
                let reason = rejected.reason;
                let vault_pane = self.right.as_mut().expect("still loaded");
                let restored =
                    transfer::place_in_vault(&mut vault_pane.vault, *rejected.item, tab, db)
                        .is_ok();
                Err(if restored {
                    format!("{reason}; item returned to the vault")
                } else {
                    format!("{reason}; item could not be returned — reload without saving")
                })
            }
        }
    }

    fn save_character(&mut self) -> Result<String, String> {
        let pane = self.character.as_mut().ok_or("nothing to save")?;
        let spliced = chr::replace_inventory(&pane.original, &pane.character.sacks)
            .map_err(|error| error.to_string())?;
        let bytes = chr::replace_money(&spliced, pane.character.info.money)
            .map_err(|error| error.to_string())?;
        let backup = safe_write::backup_first_write(&pane.path, &bytes)
            .map_err(|error| error.to_string())?;
        pane.original = bytes;
        pane.dirty = false;
        Ok(saved_message(&pane.path, backup.as_deref()))
    }

    fn save_stash(&mut self, slot: StashSlot) -> Result<String, String> {
        let pane = match slot {
            StashSlot::Bank => self.bank.as_mut(),
            StashSlot::Shared => self.shared.as_mut(),
        }
        .ok_or("nothing to save")?;
        let bytes = stash::replace_items(&pane.original, &pane.stash.items)
            .map_err(|error| error.to_string())?;
        let backup = safe_write::backup_first_write(&pane.path, &bytes)
            .map_err(|error| error.to_string())?;
        let twin = stash::backup_twin(&bytes).map_err(|error| error.to_string())?;
        std::fs::write(pane.path.with_extension("dxg"), twin).map_err(|error| error.to_string())?;
        pane.original = bytes;
        pane.dirty = false;
        Ok(saved_message(&pane.path, backup.as_deref()))
    }

    fn save_vault(&mut self) -> Result<String, String> {
        let pane = self.right.as_mut().ok_or("nothing to save")?;
        let json = pane.vault.to_json().map_err(|error| error.to_string())?;
        safe_write::backup_first_write(&pane.path, json.as_bytes())
            .map_err(|error| error.to_string())?;
        pane.dirty = false;
        Ok(format!("saved {}", pane.path.display()))
    }
}

fn saved_message(path: &Path, backup: Option<&Path>) -> String {
    match backup {
        Some(backup) => format!("saved {} (backup: {})", path.display(), backup.display()),
        None => format!("saved {}", path.display()),
    }
}

/// A stash under a `Sys` folder is the account-wide transfer stash;
/// anywhere else (a character folder) it is that character's bank.
fn stash_slot_for(path: &Path) -> StashSlot {
    let in_sys_folder = path
        .parent()
        .and_then(std::path::Path::file_name)
        .is_some_and(|name| name.eq_ignore_ascii_case("Sys"));
    if in_sys_folder {
        StashSlot::Shared
    } else {
        StashSlot::Bank
    }
}

fn cache_file_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("gamedata.cache"))
}

fn default_vault_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("vaults").join("Main Vault.json"))
}

fn game_dir_file_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("game-dir.txt"))
}

fn stored_game_dir() -> Option<PathBuf> {
    let path = game_dir_file_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let dir = PathBuf::from(text.trim());
    dir.is_dir().then_some(dir)
}

fn load_cached_game_data() -> Option<GameCache> {
    let bytes = std::fs::read(cache_file_path()?).ok()?;
    GameCache::from_bytes(&bytes).ok()
}

fn read_stamped(path: &Path) -> Result<(Vec<u8>, SourceStamp), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok((
        bytes,
        stamp_of(path).unwrap_or(SourceStamp {
            path: path.display().to_string(),
            size: 0,
            mtime_seconds: 0,
        }),
    ))
}

fn stamp_of(path: &Path) -> Option<SourceStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime_seconds = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    Some(SourceStamp {
        path: path.display().to_string(),
        size: i64::try_from(metadata.len()).unwrap_or(0),
        mtime_seconds,
    })
}

/// Kicks off the one-time import on a background thread: the game
/// archives are read and distilled into the cache while the window
/// stays responsive and shows progress.
fn start_import(dir: PathBuf) -> ImportJob {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = match run_import(&dir, &sender) {
            Ok(cache) => ImportEvent::Done(Box::new(cache)),
            Err(message) => ImportEvent::Failed(message),
        };
        let _ = sender.send(outcome);
    });
    ImportJob {
        receiver,
        progress: ImportProgress {
            label: "Preparing game-data import…".to_string(),
            fraction: None,
        },
    }
}

/// The import itself: reads the game archives, distills the item
/// cache, and persists it (plus the game dir for later refreshes)
/// under the config directory, reporting each phase as it goes.
fn run_import(
    dir: &Path,
    sender: &std::sync::mpsc::Sender<ImportEvent>,
) -> Result<GameCache, String> {
    let report = |label: String, fraction: Option<f32>| {
        let _ = sender.send(ImportEvent::Progress(ImportProgress { label, fraction }));
    };
    report("Reading game database (database.arz)…".to_string(), None);
    let (database, database_stamp) = read_stamped(&dir.join("Database/database.arz"))?;
    report("Reading text archive (Text_EN.arc)…".to_string(), None);
    let (text, text_stamp) = read_stamped(&dir.join("Text/Text_EN.arc"))?;
    let mut stamps = vec![database_stamp, text_stamp];
    report("Parsing game database…".to_string(), None);
    let mut data = GameData::from_bytes(database, text).map_err(|error| error.to_string())?;
    let candidates = [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ];
    for (label, relative) in candidates {
        report(format!("Reading item bitmaps ({relative})…"), None);
        let path = dir.join(relative);
        if let Ok((bytes, stamp)) = read_stamped(&path)
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_items_archive(label, archive);
            stamps.push(stamp);
        }
    }
    let cache = data.build_cache_with_progress(stamps, |scanned, total| {
        report(
            format!("Distilling item records… {scanned} / {total}"),
            Some(fraction(scanned, total)),
        );
    });
    report("Writing the local cache…".to_string(), None);
    if let Some(path) = cache_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, cache.to_bytes())
            .map_err(|error| format!("writing cache: {error}"))?;
    }
    if let Some(path) = game_dir_file_path() {
        let _ = std::fs::write(path, dir.display().to_string());
    }
    Ok(cache)
}

// Record counts sit far below f32's exact-integer range.
#[allow(clippy::cast_precision_loss)]
fn fraction(done: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    }
}

/// `Some(warning)` when any imported source file has changed on disk
/// since the cache was built. Unreachable sources are ignored (the
/// game volume may simply not be mounted).
fn staleness_warning(cache: &GameCache) -> Option<String> {
    let changed = cache.stamps().iter().any(|recorded| {
        stamp_of(Path::new(&recorded.path)).is_some_and(|current| current != *recorded)
    });
    changed.then(|| {
        "Game files changed since the last import — use 'Import game data…' to refresh.".to_string()
    })
}

const EQUIPMENT_SLOT_NAMES: [&str; chr::EQUIPMENT_SLOTS] = [
    "Head",
    "Neck",
    "Torso",
    "Legs",
    "Arms",
    "Ring 1",
    "Ring 2",
    "Weapon 1",
    "Offhand 1",
    "Weapon 2",
    "Offhand 2",
    "Artifact",
];

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_import();
        if let Some(dropped) = first_dropped_path(ui.ctx()) {
            self.status = Some(self.open(&dropped));
        }
        self.show_header(ui);
        ui.separator();
        let (action, drag_frame) = self.show_panes(ui);
        self.update_drag(ui.ctx(), drag_frame);
        match action {
            Some(PaneAction::MoveToVault) => self.status = Some(self.move_left_to_vault()),
            Some(PaneAction::MoveToFile) => self.status = Some(self.move_vault_to_left()),
            Some(PaneAction::SaveCharacter) => self.status = Some(self.save_character()),
            Some(PaneAction::SaveStash(slot)) => self.status = Some(self.save_stash(slot)),
            Some(PaneAction::SaveVault) => self.status = Some(self.save_vault()),
            Some(PaneAction::PreviewRespec(kind)) => self.preview_respec(kind),
            None => {}
        }
        self.show_respec_modal(ui.ctx());
    }
}

enum PaneAction {
    MoveToVault,
    MoveToFile,
    SaveCharacter,
    SaveStash(StashSlot),
    SaveVault,
    PreviewRespec(RespecKind),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RespecKind {
    Attributes,
    Skills,
}

/// A respec awaiting the user's confirmation, with its previewed
/// refund.
struct PendingRespec {
    kind: RespecKind,
    points: i32,
    skills_removed: usize,
}

impl App {
    /// Applies whatever a running background import has produced:
    /// progress for the header, or its final cache/failure.
    fn poll_import(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let GameStatus::Importing(job) = &mut self.game else {
            return;
        };
        let mut outcome = None;
        loop {
            match job.receiver.try_recv() {
                Ok(ImportEvent::Progress(progress)) => job.progress = progress,
                Ok(ImportEvent::Done(cache)) => {
                    outcome = Some((
                        GameStatus::Loaded(*cache),
                        Ok("game data imported".to_string()),
                    ));
                    break;
                }
                Ok(ImportEvent::Failed(message)) => {
                    outcome = Some((GameStatus::Failed(message.clone()), Err(message)));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let message = "game-data import stopped unexpectedly".to_string();
                    outcome = Some((GameStatus::Failed(message.clone()), Err(message)));
                    break;
                }
            }
        }
        if let Some((game, status)) = outcome {
            let count = match &game {
                GameStatus::Loaded(cache) => Some(cache.len()),
                GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
            };
            self.game = game;
            self.game_note = None;
            self.caches = Caches::default();
            self.status = Some(match (status, count) {
                (Ok(_), Some(count)) => Ok(format!("imported {count} item records")),
                (status, _) => status,
            });
        }
    }

    fn show_header(&mut self, ui: &mut egui::Ui) {
        let mut requested: Option<PathBuf> = None;
        ui.horizontal(|ui| {
            if ui.button("Open character…").clicked() {
                requested = pick_file(
                    "Character / stash",
                    &["chr", "dxb", "dxg"],
                    self.dialog_start_dir(),
                );
            }
            if ui.button("Open vault…").clicked() {
                requested = pick_file("Vault", &["json", "vault"], self.dialog_start_dir());
            }
            let importing = matches!(self.game, GameStatus::Importing(_));
            if ui
                .add_enabled(!importing, egui::Button::new("Import game data…"))
                .clicked()
            {
                let start = stored_game_dir();
                let mut dialog = rfd::FileDialog::new();
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.game = GameStatus::Importing(start_import(dir));
                    self.game_note = None;
                }
            }
            ui.menu_button("Recent", |ui| {
                if self.recents.entries.is_empty() {
                    ui.weak("nothing yet");
                }
                for path in &self.recents.entries {
                    if ui.button(Recents::label(path)).clicked() {
                        requested = Some(path.clone());
                        ui.close();
                    }
                }
            });
        });
        if let Some(path) = requested {
            self.status = Some(self.open(&path));
        }
        self.show_zoom_control(ui);
        if let Some(note) = &self.game_note {
            ui.colored_label(ui.visuals().warn_fg_color, note);
        }
        match &self.game {
            GameStatus::Loaded(_) => {}
            GameStatus::Importing(job) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&job.progress.label);
                });
                ui.add(
                    egui::ProgressBar::new(job.progress.fraction.unwrap_or(0.0))
                        .desired_width(360.0)
                        .show_percentage(),
                );
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            GameStatus::Absent => {
                ui.weak(
                    "No game data imported yet — use 'Import game data…' and pick your \
                     Titan Quest install (one time; names, icons and sizes come from it).",
                );
            }
            GameStatus::Failed(message) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Game data failed to load: {message}"),
                );
            }
        }
        if self.character.is_none() && self.bank.is_none() && self.shared.is_none() {
            ui.heading("TQ UniVault");
            ui.label(
                "Open (or drop) a Player.chr — its bank and the shared bank load with it. \
                 A stash (.dxb/.dxg) or another vault (.json / legacy .vault) opens alone too.",
            );
        }
        match &self.status {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
            None => {}
        }
    }

    /// Applying zoom mid-drag rescales the slider out from under the
    /// pointer, so the slider tracks a pending value while dragging
    /// and the zoom only takes effect on release.
    fn show_zoom_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            let mut value = self.pending_zoom;
            let response = ui.add(
                egui::Slider::new(&mut value, 0.75..=2.5)
                    .step_by(0.05)
                    .custom_formatter(|zoom, _| format!("{:.0}%", zoom * 100.0)),
            );
            if response.dragged() {
                self.pending_zoom = value;
            } else if response.drag_stopped() {
                self.pending_zoom = value;
                ui.ctx().set_zoom_factor(value);
            } else {
                // Stay in step with the ⌘+/⌘−/⌘0 shortcuts.
                self.pending_zoom = ui.ctx().zoom_factor();
            }
            ui.weak("(⌘+ / ⌘− / ⌘0 work too)");
        });
    }

    /// Advances the drag: adopts a newly started one, paints the item
    /// at the pointer, and commits or cancels on release.
    fn update_drag(&mut self, ctx: &egui::Context, frame: DragFrame) {
        if self.drag.is_none() {
            self.drag = frame.begin;
        }
        let Some(state) = self.drag.clone() else {
            return;
        };

        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let footprint = self.caches.footprint(db, &state.item);
        if let Some(pointer) = ctx.pointer_latest_pos() {
            let rect = egui::Rect::from_min_size(
                pointer - state.grab,
                egui::vec2(cells_to_points(footprint.0), cells_to_points(footprint.1)),
            );
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("drag-cursor"),
            ));
            if let Some(texture) = self.caches.icon(ctx, db, &state.item) {
                painter.image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                );
            } else {
                painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(120));
            }
        }

        if ctx.input(|input| input.pointer.any_released()) {
            if let Some(candidate) = frame.candidate.filter(|candidate| candidate.fits) {
                let same_spot =
                    candidate.grid == state.source && candidate.cell == state.item.position;
                if !same_spot {
                    self.status = Some(self.perform_drop(&state, candidate));
                }
            }
            self.drag = None;
            ctx.request_repaint();
        }
    }

    /// Moves the dragged item to the drop cell; on a failed placement
    /// the item goes back where it came from.
    fn perform_drop(&mut self, state: &DragState, target: DropCandidate) -> Result<String, String> {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let label = self.caches.names.item_label(db, &state.item);

        let taken = match state.source {
            GridId::Sack(_) | GridId::Bank | GridId::Shared => {
                self.take_from_left(state.source, state.index)?
            }
            GridId::VaultTab(tab) => {
                let pane = self.right.as_mut().ok_or("no vault loaded")?;
                transfer::take_from_vault(&mut pane.vault, tab, state.index)
                    .map(|entry| entry.item)
                    .ok_or("item moved under the drag — drop ignored")?
            }
        };

        if taken.base != state.item.base {
            let origin = state.item.position;
            self.restore_dropped(state.source, taken, origin)?;
            return Err("item moved under the drag — drop ignored".to_string());
        }
        let origin = taken.position;

        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let placed = match target.grid {
            GridId::Sack(sack) => {
                let Some(pane) = self.character.as_mut() else {
                    return Err("no character loaded".to_string());
                };
                transfer::place_in_character_at(&mut pane.character, taken, sack, target.cell, db)
            }
            GridId::Bank => {
                let Some(pane) = self.bank.as_mut() else {
                    return Err("no bank loaded".to_string());
                };
                transfer::place_in_stash_at(&mut pane.stash, taken, target.cell, db)
            }
            GridId::Shared => {
                let Some(pane) = self.shared.as_mut() else {
                    return Err("no shared bank loaded".to_string());
                };
                transfer::place_in_stash_at(&mut pane.stash, taken, target.cell, db)
            }
            GridId::VaultTab(tab) => {
                let Some(pane) = self.right.as_mut() else {
                    return Err("no vault loaded".to_string());
                };
                transfer::place_in_vault_at(&mut pane.vault, taken, tab, target.cell, db)
            }
        };

        match placed {
            Ok(()) => {
                self.mark_dirty(state.source);
                self.mark_dirty(target.grid);
                self.left_selected = None;
                if let Some(pane) = &mut self.right {
                    pane.selected = None;
                }
                let destination = match target.grid {
                    GridId::Sack(sack) => format!("sack {}", sack + 1),
                    GridId::Bank => "bank".to_string(),
                    GridId::Shared => "shared bank".to_string(),
                    GridId::VaultTab(tab) => format!("vault tab {}", tab + 1),
                };
                Ok(format!(
                    "{label} → {destination} ({}, {})",
                    target.cell.x, target.cell.y
                ))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                self.restore_dropped(state.source, *rejected.item, origin)?;
                Err(format!("{reason}; item returned"))
            }
        }
    }

    /// Puts a taken item back at its original cell (guaranteed free),
    /// falling back to any open spot.
    fn restore_dropped(
        &mut self,
        source: GridId,
        item: Item,
        position: univault_core::chr::GridPos,
    ) -> Result<(), String> {
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let lost = "item could not be returned — reload without saving".to_string();
        match source {
            GridId::Sack(sack) => {
                let pane = self.character.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_character_at(&mut pane.character, item, sack, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_character(&mut pane.character, *rejected.item, sack, db)
                            .map(|_| ())
                    })
                    .map_err(|_| lost)
            }
            GridId::Bank => {
                let pane = self.bank.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_stash_at(&mut pane.stash, item, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_stash(&mut pane.stash, *rejected.item, db)
                    })
                    .map_err(|_| lost)
            }
            GridId::Shared => {
                let pane = self.shared.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_stash_at(&mut pane.stash, item, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_stash(&mut pane.stash, *rejected.item, db)
                    })
                    .map_err(|_| lost)
            }
            GridId::VaultTab(tab) => {
                let pane = self.right.as_mut().ok_or_else(|| lost.clone())?;
                transfer::place_in_vault_at(&mut pane.vault, item, tab, position, db)
                    .or_else(|rejected| {
                        transfer::place_in_vault(&mut pane.vault, *rejected.item, tab, db)
                            .map(|_| ())
                    })
                    .map_err(|_| lost)
            }
        }
    }

    fn mark_dirty(&mut self, grid: GridId) {
        let dirty = match grid {
            GridId::Sack(_) => self.character.as_mut().map(|pane| &mut pane.dirty),
            GridId::Bank => self.bank.as_mut().map(|pane| &mut pane.dirty),
            GridId::Shared => self.shared.as_mut().map(|pane| &mut pane.dirty),
            GridId::VaultTab(_) => self.right.as_mut().map(|pane| &mut pane.dirty),
        };
        if let Some(dirty) = dirty {
            *dirty = true;
        }
    }

    /// Computes a respec's refund from the pane's baseline bytes and
    /// opens the confirmation modal.
    fn preview_respec(&mut self, kind: RespecKind) {
        let Some(pane) = &self.character else { return };
        let preview = match kind {
            RespecKind::Attributes => {
                respec::attribute_refund(&pane.original).map(|points| (points, 0))
            }
            RespecKind::Skills => respec::skill_refund(&pane.original),
        };
        match preview {
            Ok((points, skills_removed)) => {
                self.pending_respec = Some(PendingRespec {
                    kind,
                    points,
                    skills_removed,
                });
            }
            Err(error) => self.status = Some(Err(format!("respec unavailable: {error}"))),
        }
    }

    fn show_respec_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_respec else {
            return;
        };
        let (title, body) = match pending.kind {
            RespecKind::Attributes => (
                "Respec attributes?",
                format!(
                    "Attributes return to base values; {} attribute points will be refunded.",
                    pending.points
                ),
            ),
            RespecKind::Skills => (
                "Respec skills & masteries?",
                format!(
                    "{} skills and both masteries will be removed; {} skill points will be refunded. \
                     The class resets so both masteries can be picked again.",
                    pending.skills_removed, pending.points
                ),
            ),
        };
        let nothing_to_do = pending.points == 0 && pending.skills_removed == 0;
        let kind = pending.kind;
        let mut close = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("respec-modal")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.heading(title);
            if nothing_to_do {
                ui.label("Nothing to refund — this character is already respecced.");
            } else {
                ui.label(body);
                ui.weak("Applies in memory; nothing is written until you press Save.");
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if nothing_to_do {
                    close = ui.button("Close").clicked();
                } else {
                    close = ui.button("Cancel").clicked();
                    confirm = ui.button("Respec").clicked();
                }
            });
        });
        if confirm {
            self.status = Some(self.apply_respec(kind));
        }
        if close || confirm || modal.should_close() {
            self.pending_respec = None;
        }
    }

    fn apply_respec(&mut self, kind: RespecKind) -> Result<String, String> {
        let pane = self.character.as_mut().ok_or("no character loaded")?;
        let result = match kind {
            RespecKind::Attributes => respec::respec_attributes(&pane.original),
            RespecKind::Skills => respec::respec_skills(&pane.original),
        }
        .map_err(|error| error.to_string())?;
        pane.original = result.bytes;
        pane.dirty = true;
        Ok(match kind {
            RespecKind::Attributes => format!(
                "refunded {} attribute points — press Save to write",
                result.refunded_points
            ),
            RespecKind::Skills => format!(
                "removed {} skills, refunded {} skill points — press Save to write",
                result.skills_removed, result.refunded_points
            ),
        })
    }

    /// Where file dialogs start: near what the user last touched.
    fn dialog_start_dir(&self) -> Option<PathBuf> {
        self.character
            .as_ref()
            .map(|pane| pane.path.clone())
            .or_else(|| self.bank.as_ref().map(|pane| pane.path.clone()))
            .or_else(|| self.shared.as_ref().map(|pane| pane.path.clone()))
            .or_else(|| self.recents.entries.first().cloned())
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    }

    fn show_panes(&mut self, ui: &mut egui::Ui) -> (Option<PaneAction>, DragFrame) {
        let caches = &mut self.caches;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Importing(_) | GameStatus::Failed(_) => None,
        };
        let drag = self.drag.clone();
        let mut frame = DragFrame::default();
        let mut action = None;
        let has_left = self.character.is_some() || self.bank.is_some() || self.shared.is_some();
        let can_move = has_left && self.right.is_some();
        ui.columns(2, |columns| {
            if has_left {
                let character = &mut self.character;
                let bank = &mut self.bank;
                let shared = &mut self.shared;
                let selected = &mut self.left_selected;
                egui::ScrollArea::vertical()
                    .id_salt("file-pane")
                    .show(&mut columns[0], |ui| {
                        if let Some(pane) = character
                            && let Some(chosen) = show_character_section(
                                ui,
                                pane,
                                db,
                                caches,
                                can_move,
                                selected,
                                drag.as_ref(),
                                &mut frame,
                            )
                        {
                            action = Some(chosen);
                        }
                        let has_character = character.is_some();
                        for (pane, slot) in [(bank, StashSlot::Bank), (shared, StashSlot::Shared)] {
                            if let Some(chosen) = show_stash_slot(
                                ui,
                                pane,
                                slot,
                                has_character,
                                db,
                                caches,
                                can_move,
                                selected,
                                drag.as_ref(),
                                &mut frame,
                            ) {
                                action = Some(chosen);
                            }
                        }
                    });
            } else {
                columns[0].weak("No game file loaded.");
            }
            if let Some(pane) = &mut self.right {
                if let Some(chosen) = show_vault_pane(
                    &mut columns[1],
                    pane,
                    db,
                    caches,
                    can_move,
                    drag.as_ref(),
                    &mut frame,
                ) {
                    action = Some(chosen);
                }
            } else {
                columns[1].weak("No vault loaded.");
            }
        });
        (action, frame)
    }
}

/// On-screen size of one grid cell — the textures' native 32 pixels.
const CELL_SIZE: f32 = 32.0;

// Grid coordinates are small integers; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn cells_to_points(cells: i32) -> f32 {
    cells as f32 * CELL_SIZE
}

/// Which on-screen grid an item lives in — the address space of
/// selection and drag-and-drop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GridId {
    Sack(usize),
    Bank,
    Shared,
    VaultTab(usize),
}

/// An in-flight drag: where the item came from and how it was
/// grabbed. The item stays in its container (painted dimmed) until
/// the drop commits.
#[derive(Clone)]
struct DragState {
    source: GridId,
    index: usize,
    item: Item,
    /// Pointer offset from the item's top-left, in points, so the
    /// item hangs where it was grabbed.
    grab: egui::Vec2,
}

/// The cell a drop would land in, computed by whichever grid the
/// pointer is over this frame.
#[derive(Clone, Copy)]
struct DropCandidate {
    grid: GridId,
    cell: univault_core::chr::GridPos,
    fits: bool,
}

/// What the grids reported back this frame.
#[derive(Default)]
struct DragFrame {
    begin: Option<DragState>,
    candidate: Option<DropCandidate>,
}

/// Paints a container as its actual cell grid, with items at their
/// positions (icon when decodable, initial letter otherwise), click
/// selection, a name tooltip on hover, and drag-and-drop with a
/// green/red footprint preview.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn grid_view(
    ui: &mut egui::Ui,
    dims: (i32, i32),
    entries: &[(usize, &Item)],
    grid: GridId,
    selected: &mut Option<(GridId, usize)>,
    db: Option<&GameCache>,
    caches: &mut Caches,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) {
    let size = egui::vec2(cells_to_points(dims.0), cells_to_points(dims.1));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let grid_stroke = egui::Stroke::new(0.5, visuals.widgets.noninteractive.bg_stroke.color);
    for column in 0..=dims.0 {
        let x = rect.min.x + cells_to_points(column);
        painter.line_segment(
            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
            grid_stroke,
        );
    }
    for row in 0..=dims.1 {
        let y = rect.min.y + cells_to_points(row);
        painter.line_segment(
            [egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)],
            grid_stroke,
        );
    }

    // The grab position decides which item a starting drag lifts —
    // the pointer may already have moved past egui's drag threshold.
    let press_origin = ui.ctx().input(|input| input.pointer.press_origin());

    let mut hovered: Option<&Item> = None;
    for (index, item) in entries {
        let (width, height) = caches.footprint(db, item);
        let item_rect = egui::Rect::from_min_size(
            rect.min
                + egui::vec2(
                    cells_to_points(item.position.x),
                    cells_to_points(item.position.y),
                ),
            egui::vec2(cells_to_points(width), cells_to_points(height)),
        )
        .shrink(1.0);
        let is_selected = *selected == Some((grid, *index));
        paint_item_tile(
            ui,
            &painter,
            item_rect,
            item,
            is_selected,
            &visuals,
            db,
            caches,
        );

        // The lifted item stays put but fades until the drop lands.
        if drag.is_some_and(|state| state.source == grid && state.index == *index) {
            painter.rect_filled(item_rect, 2.0, egui::Color32::from_black_alpha(140));
        }

        if response.drag_started()
            && drag.is_none()
            && frame.begin.is_none()
            && press_origin.is_some_and(|origin| item_rect.contains(origin))
        {
            frame.begin = Some(DragState {
                source: grid,
                index: *index,
                item: (*item).clone(),
                grab: press_origin.map_or(egui::Vec2::ZERO, |origin| origin - item_rect.min),
            });
        }

        if drag.is_none()
            && let Some(pointer) = response.hover_pos()
            && item_rect.contains(pointer)
        {
            hovered = Some(item);
            if response.clicked() {
                *selected = Some((grid, *index));
            }
        }
    }
    if let Some(item) = hovered {
        response.on_hover_ui(|ui| item_tooltip(ui, item, db, caches));
    }

    if let Some(state) = drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && rect.contains(pointer)
    {
        frame.candidate = Some(paint_drop_preview(
            &painter, rect, dims, entries, grid, state, pointer, db, caches,
        ));
    }
}

/// One item's tile: fill, icon (or initial letter), outline, and
/// stack badge.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn paint_item_tile(
    ui: &egui::Ui,
    painter: &egui::Painter,
    item_rect: egui::Rect,
    item: &Item,
    is_selected: bool,
    visuals: &egui::Visuals,
    db: Option<&GameCache>,
    caches: &mut Caches,
) {
    let fill = if is_selected {
        visuals.selection.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    painter.rect_filled(item_rect, 2.0, fill);
    if let Some(texture) = caches.icon(ui.ctx(), db, item) {
        painter.image(
            texture.id(),
            item_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        let initial = caches
            .names
            .record_name(db, &item.base)
            .chars()
            .next()
            .unwrap_or('?');
        painter.text(
            item_rect.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            egui::FontId::proportional(12.0),
            visuals.strong_text_color(),
        );
    }
    let outline = if is_selected {
        egui::Stroke::new(2.0, visuals.selection.stroke.color)
    } else {
        egui::Stroke::new(1.0, visuals.widgets.inactive.fg_stroke.color)
    };
    painter.rect_stroke(item_rect, 2.0, outline, egui::StrokeKind::Inside);
    if item.stack_size > 1 {
        painter.text(
            item_rect.right_bottom() - egui::vec2(2.0, 1.0),
            egui::Align2::RIGHT_BOTTOM,
            item.stack_size.to_string(),
            egui::FontId::proportional(10.0),
            visuals.strong_text_color(),
        );
    }
}

/// Snaps the dragged footprint to the hovered cell, paints it green
/// (fits) or red (blocked), and returns the drop candidate.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn paint_drop_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    dims: (i32, i32),
    entries: &[(usize, &Item)],
    grid: GridId,
    state: &DragState,
    cursor: egui::Pos2,
    db: Option<&GameCache>,
    caches: &mut Caches,
) -> DropCandidate {
    let footprint = caches.footprint(db, &state.item);
    let relative = cursor - state.grab - rect.min.to_vec2();
    let cell = univault_core::chr::GridPos {
        x: point_to_cell(relative.x, dims.0, footprint.0),
        y: point_to_cell(relative.y, dims.1, footprint.1),
    };
    let skip = (state.source == grid).then_some(state.index);
    let occupied: Vec<univault_core::grid::CellRect> = entries
        .iter()
        .filter(|(index, _)| Some(*index) != skip)
        .map(|(_, item)| {
            let (width, height) = caches.footprint(db, item);
            univault_core::grid::CellRect {
                x: item.position.x,
                y: item.position.y,
                width,
                height,
            }
        })
        .collect();
    let fits = transfer::fits_at(&occupied, footprint, cell, dims);
    let preview = egui::Rect::from_min_size(
        rect.min + egui::vec2(cells_to_points(cell.x), cells_to_points(cell.y)),
        egui::vec2(cells_to_points(footprint.0), cells_to_points(footprint.1)),
    )
    .shrink(1.0);
    let (fill, stroke) = if fits {
        (
            egui::Color32::from_rgba_unmultiplied(64, 255, 64, 50),
            egui::Color32::from_rgb(64, 255, 64),
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(255, 64, 64, 50),
            egui::Color32::from_rgb(255, 64, 64),
        )
    };
    painter.rect_filled(preview, 2.0, fill);
    painter.rect_stroke(
        preview,
        2.0,
        egui::Stroke::new(2.0, stroke),
        egui::StrokeKind::Inside,
    );
    DropCandidate { grid, cell, fits }
}

/// The grid cell a point lands in, clamped so the footprint stays
/// inside the container.
#[allow(clippy::cast_possible_truncation)] // grid coordinates are tiny
fn point_to_cell(point: f32, grid_cells: i32, footprint_cells: i32) -> i32 {
    let cell = (point / CELL_SIZE).round() as i32;
    cell.clamp(0, (grid_cells - footprint_cells).max(0))
}

/// Item details on hover, name colored by rarity. The game's palette
/// assumes its dark backdrop, so the tooltip paints its own instead
/// of inheriting the theme.
fn item_tooltip(ui: &mut egui::Ui, item: &Item, db: Option<&GameCache>, caches: &mut Caches) {
    let item_style = style::item_style(db, item);
    let details = db.map(|db| stats::item_details(db, item));
    egui::Frame::NONE
        .fill(egui::Color32::from_rgb(24, 20, 16))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.style_mut().visuals.override_text_color = Some(egui::Color32::from_gray(190));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                egui::RichText::new(tooltip_title(item, db, details.as_ref(), caches))
                    .color(game_color(style::style_color(item_style)))
                    .size(15.0),
            );
            ui.label(
                egui::RichText::new(item_style.label())
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
            let Some(details) = details else { return };
            for block in &details.blocks {
                ui.add(egui::Separator::default().spacing(6.0));
                for line in block {
                    if line.text.trim().is_empty() {
                        ui.add_space(4.0);
                    } else {
                        ui.label(
                            egui::RichText::new(&line.text)
                                .color(game_color(stats::palette_color(line.color)))
                                .size(12.0),
                        );
                    }
                }
            }
        });
}

/// The game's full item name: prefix, quality, base, style, suffix,
/// and the stack count.
fn tooltip_title(
    item: &Item,
    db: Option<&GameCache>,
    details: Option<&stats::ItemDetails>,
    caches: &mut Caches,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(prefix) = &item.prefix {
        parts.push(caches.names.record_name(db, prefix));
    }
    if let Some(quality) = details.and_then(|details| details.quality.clone()) {
        parts.push(quality);
    }
    parts.push(caches.names.record_name(db, &item.base));
    if let Some(style_word) = details.and_then(|details| details.style_word.clone()) {
        parts.push(style_word);
    }
    if let Some(suffix) = &item.suffix {
        parts.push(caches.names.record_name(db, suffix));
    }
    if item.stack_size > 1 {
        parts.push(format!("×{}", item.stack_size));
    }
    parts.join(" ")
}

fn game_color(rgb: style::Rgb) -> egui::Color32 {
    egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

fn show_equipment(
    ui: &mut egui::Ui,
    character: &PlayerCharacter,
    db: Option<&GameCache>,
    caches: &mut Caches,
) {
    ui.collapsing("Equipment (read-only)", |ui| {
        for (name, slot) in EQUIPMENT_SLOT_NAMES.iter().zip(&character.equipment.slots) {
            match slot {
                Some(item) => ui.label(format!("{name}: {}", caches.names.item_label(db, item))),
                None => ui.weak(format!("{name}: —")),
            };
        }
    });
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_character_section(
    ui: &mut egui::Ui,
    pane: &mut CharacterPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(
            pane.character
                .info
                .name
                .as_deref()
                .unwrap_or("Unnamed character"),
        );
        let gold = ui.add(
            egui::DragValue::new(&mut pane.character.info.money)
                .range(0..=i32::MAX)
                .prefix("gold: "),
        );
        if gold.changed() {
            pane.dirty = true;
        }
        if ui
            .add_enabled(pane.dirty, egui::Button::new("Save"))
            .clicked()
        {
            action = Some(PaneAction::SaveCharacter);
        }
        let selection_here = matches!(*selected, Some((GridId::Sack(_), _)));
        if ui
            .add_enabled(can_move && selection_here, egui::Button::new("→ Vault"))
            .clicked()
        {
            action = Some(PaneAction::MoveToVault);
        }
        if ui.button("Respec attributes").clicked() {
            action = Some(PaneAction::PreviewRespec(RespecKind::Attributes));
        }
        if ui.button("Respec skills & masteries").clicked() {
            action = Some(PaneAction::PreviewRespec(RespecKind::Skills));
        }
    });
    ui.monospace(pane.path.display().to_string());
    show_equipment(ui, &pane.character, db, caches);
    for (index, sack) in pane.character.sacks.iter().enumerate() {
        let title = format!("Sack {} ({} items)", index + 1, sack.items.len());
        egui::CollapsingHeader::new(title)
            .default_open(true)
            .show(ui, |ui| {
                let entries: Vec<(usize, &Item)> = sack.items.iter().enumerate().collect();
                grid_view(
                    ui,
                    chr::sack_dimensions(index),
                    &entries,
                    GridId::Sack(index),
                    selected,
                    db,
                    caches,
                    drag,
                    frame,
                );
            });
    }
    action
}

/// One stash slot in the left column: the section when loaded, a
/// placeholder explaining where the file was expected when not.
#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_stash_slot(
    ui: &mut egui::Ui,
    pane: &mut Option<StashPane>,
    slot: StashSlot,
    has_character: bool,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    if let Some(pane) = pane {
        return show_stash_section(ui, pane, slot, db, caches, can_move, selected, drag, frame);
    }
    if has_character {
        let (title, hint) = match slot {
            StashSlot::Bank => (
                "Character bank",
                "No bank file yet — the game creates winsys.dxb the first time \
                 this character opens the caravan stash.",
            ),
            StashSlot::Shared => (
                "Shared bank",
                "No shared bank found — expected Sys/winsys.dxb up the save tree.",
            ),
        };
        ui.separator();
        ui.heading(title);
        ui.weak(hint);
    }
    None
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_stash_section(
    ui: &mut egui::Ui,
    pane: &mut StashPane,
    slot: StashSlot,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    selected: &mut Option<(GridId, usize)>,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    let (title, grid) = match slot {
        StashSlot::Bank => ("Character bank", GridId::Bank),
        StashSlot::Shared => ("Shared bank", GridId::Shared),
    };
    ui.separator();
    ui.horizontal(|ui| {
        ui.heading(format!(
            "{title} {}×{}",
            pane.stash.width, pane.stash.height
        ));
        if ui
            .add_enabled(pane.dirty, egui::Button::new("Save"))
            .clicked()
        {
            action = Some(PaneAction::SaveStash(slot));
        }
        let selection_here = matches!(*selected, Some((current, _)) if current == grid);
        if ui
            .add_enabled(can_move && selection_here, egui::Button::new("→ Vault"))
            .clicked()
        {
            action = Some(PaneAction::MoveToVault);
        }
    });
    ui.monospace(pane.path.display().to_string());
    let entries: Vec<(usize, &Item)> = pane.stash.items.iter().enumerate().collect();
    grid_view(
        ui,
        (pane.stash.width, pane.stash.height),
        &entries,
        grid,
        selected,
        db,
        caches,
        drag,
        frame,
    );
    action
}

#[allow(clippy::too_many_arguments)] // one call surface, shell-internal
fn show_vault_pane(
    ui: &mut egui::Ui,
    pane: &mut VaultPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
    drag: Option<&DragState>,
    frame: &mut DragFrame,
) -> Option<PaneAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading("Vault");
        if ui
            .add_enabled(pane.dirty, egui::Button::new("Save"))
            .clicked()
        {
            action = Some(PaneAction::SaveVault);
        }
        if ui
            .add_enabled(
                can_move && pane.selected.is_some(),
                egui::Button::new("← To file"),
            )
            .clicked()
        {
            action = Some(PaneAction::MoveToFile);
        }
    });
    ui.monospace(pane.path.display().to_string());
    egui::ScrollArea::vertical()
        .id_salt("vault-pane")
        .show(ui, |ui| {
            for (tab, sack) in pane.vault.sacks.iter().enumerate() {
                let title = format!("Tab {} ({} items)", tab + 1, sack.items.len());
                egui::CollapsingHeader::new(title)
                    .default_open(tab == 0 || !sack.items.is_empty())
                    .show(ui, |ui| {
                        let entries: Vec<(usize, &Item)> = sack
                            .items
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| (index, &entry.item))
                            .collect();
                        grid_view(
                            ui,
                            (
                                univault_core::vault::TAB_WIDTH,
                                univault_core::vault::TAB_HEIGHT,
                            ),
                            &entries,
                            GridId::VaultTab(tab),
                            &mut pane.selected,
                            db,
                            caches,
                            drag,
                            frame,
                        );
                    });
            }
        });
    action
}

fn pick_file(description: &str, extensions: &[&str], start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter(description, extensions);
    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_file()
}

fn first_dropped_path(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .first()
            .map(|file| file.path().to_path_buf())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> CliArgs {
        CliArgs::from_args(raw.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn cli_args_route_flags_and_file() {
        let parsed = args(&["--game", "/tq", "--vault", "v.json", "save/Player.chr"]);
        assert_eq!(parsed.game_dir, Some(PathBuf::from("/tq")));
        assert_eq!(parsed.vault, Some(PathBuf::from("v.json")));
        assert_eq!(parsed.file, Some(PathBuf::from("save/Player.chr")));
        let empty = args(&[]);
        assert_eq!(empty.game_dir, None);
        assert_eq!(empty.vault, None);
        assert_eq!(empty.file, None);
        // A dangling flag consumes nothing and breaks nothing.
        assert_eq!(args(&["--game"]).game_dir, None);
    }

    #[test]
    fn stash_files_route_by_their_folder() {
        assert!(matches!(
            stash_slot_for(Path::new("/saves/SaveData/Sys/winsys.dxb")),
            StashSlot::Shared
        ));
        assert!(matches!(
            stash_slot_for(Path::new("/saves/SaveData/Main/_Pally Don/winsys.dxb")),
            StashSlot::Bank
        ));
        assert!(matches!(
            stash_slot_for(Path::new("winsys.dxb")),
            StashSlot::Bank
        ));
    }

    #[test]
    fn default_vault_lives_under_the_config_dir() {
        let path = default_vault_path().expect("a config dir on a supported platform");
        assert!(path.ends_with("vaults/Main Vault.json"), "{path:?}");
        assert!(
            path.starts_with(univault_core::platform::config_dir().unwrap()),
            "{path:?}"
        );
    }

    #[test]
    fn recents_labels_hide_the_player_chr_boilerplate() {
        let label = |raw: &str| Recents::label(Path::new(raw));
        assert_eq!(label("/saves/_Pally Don/Player.chr"), "Pally Don");
        assert_eq!(
            label("/saves/_Pally Don/winsys.dxb"),
            "Pally Don — winsys.dxb"
        );
        assert_eq!(label("vault.json"), " — vault.json");
    }

    #[test]
    fn tooltip_title_assembles_name_particles() {
        let mut caches = Caches::default();
        let mut item = Item::bare(
            RecordId::parse("records\\item\\sword.dbr".to_string()).unwrap(),
            univault_core::chr::ItemSeed::new(1),
        );
        item.prefix = Some(RecordId::parse("records\\item\\sharp.dbr".to_string()).unwrap());
        item.stack_size = 3;
        // Without game data, names fall back to record file stems.
        assert_eq!(
            tooltip_title(&item, None, None, &mut caches),
            "sharp sword ×3"
        );
        item.prefix = None;
        item.stack_size = 1;
        assert_eq!(tooltip_title(&item, None, None, &mut caches), "sword");
    }

    #[test]
    #[allow(clippy::float_cmp)] // quarters are exact in f32
    fn fraction_is_zero_safe() {
        assert_eq!(fraction(0, 0), 0.0);
        assert_eq!(fraction(1, 4), 0.25);
        assert_eq!(fraction(4, 4), 1.0);
    }
}
