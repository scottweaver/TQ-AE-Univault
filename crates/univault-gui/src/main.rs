//! egui/eframe front-end for tq-univault.
//!
//! Usage: `univault-gui [--game <TQ install dir>] [--vault <vault.json>] [file]`
//!
//! Two panes: a game file (a `Player.chr` or a stash `.dxb`/`.dxg`)
//! on the left, a vault on the right. Click an item, move it across,
//! save. Saves splice only the item region and go through the
//! backup-first write path; stashes also get their `.dxg` twin
//! rewritten. Drag-and-drop routes files by extension.

mod safe_write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use univault_core::cache::{GameCache, SourceStamp};
use univault_core::chr::{self, Item, PlayerCharacter, RecordId};
use univault_core::gamedata::GameData;
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
        let mut game_dir = None;
        let mut vault = None;
        let mut file = None;
        let mut args = std::env::args_os().skip(1);
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

enum GameFile {
    Character(Box<PlayerCharacter>),
    Stash(Stash),
}

struct FilePane {
    path: PathBuf,
    original: Vec<u8>,
    file: GameFile,
    dirty: bool,
    selected: Option<(usize, usize)>,
}

struct VaultPane {
    path: PathBuf,
    vault: Vault,
    dirty: bool,
    selected: Option<(usize, usize)>,
}

enum GameStatus {
    Absent,
    Loaded(GameCache),
    Failed(String),
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
    left: Option<FilePane>,
    right: Option<VaultPane>,
    status: Option<Result<String, String>>,
    /// Zoom shown by the slider while dragging; applied on release.
    pending_zoom: f32,
}

impl App {
    fn new(args: CliArgs) -> Self {
        // --game forces a (re-)import; otherwise the local cache is
        // the runtime database, imported automatically from the
        // remembered game dir the first time.
        let mut game_note = None;
        let game = if let Some(dir) = args.game_dir.as_deref() {
            match import_game_data(dir) {
                Ok(cache) => GameStatus::Loaded(cache),
                Err(message) => GameStatus::Failed(message),
            }
        } else if let Some(cache) = load_cached_game_data() {
            game_note = staleness_warning(&cache);
            GameStatus::Loaded(cache)
        } else if let Some(dir) = stored_game_dir() {
            match import_game_data(&dir) {
                Ok(cache) => GameStatus::Loaded(cache),
                Err(message) => GameStatus::Failed(message),
            }
        } else {
            GameStatus::Absent
        };
        let mut app = Self {
            game,
            game_note,
            caches: Caches::default(),
            recents: Recents::load(),
            left: None,
            right: None,
            status: None,
            pending_zoom: 1.0,
        };
        if let Some(path) = args.vault {
            app.status = Some(app.open(&path));
        }
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
            Some("dxb" | "dxg") => self.open_game_file(path, true),
            _ => self.open_game_file(path, false),
        }
    }

    fn open_game_file(&mut self, path: &Path, is_stash: bool) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let file = if is_stash {
            GameFile::Stash(stash::parse_stash(&bytes).map_err(|error| error.to_string())?)
        } else {
            GameFile::Character(Box::new(
                chr::parse_player(&bytes).map_err(|error| error.to_string())?,
            ))
        };
        self.left = Some(FilePane {
            path: path.to_path_buf(),
            original: bytes,
            file,
            dirty: false,
            selected: None,
        });
        self.recents.remember(path);
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

    fn move_left_to_vault(&mut self) -> Result<String, String> {
        let (Some(pane), Some(vault_pane)) = (self.left.as_mut(), self.right.as_mut()) else {
            return Err("load a game file and a vault first".to_string());
        };
        let (container, index) = pane.selected.ok_or("select an item on the left")?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Failed(_) => None,
        };
        let item = match &mut pane.file {
            GameFile::Character(character) => {
                transfer::take_from_character(character, container, index)
            }
            GameFile::Stash(stash) => transfer::take_from_stash(stash, index),
        }
        .ok_or("selection is stale — pick the item again")?;
        let label = self.caches.names.item_label(db, &item);
        let preferred = vault_pane.selected.map_or(0, |(tab, _)| tab);
        match transfer::place_in_vault(&mut vault_pane.vault, item, preferred, db) {
            Ok(tab) => {
                pane.dirty = true;
                pane.selected = None;
                vault_pane.dirty = true;
                Ok(format!("{label} → vault tab {}", tab + 1))
            }
            Err(rejected) => {
                let reason = rejected.reason;
                let item = *rejected.item;
                let restored = match &mut pane.file {
                    GameFile::Character(character) => {
                        transfer::place_in_character(character, item, container, db).is_ok()
                    }
                    GameFile::Stash(stash) => transfer::place_in_stash(stash, item, db).is_ok(),
                };
                Err(if restored {
                    format!("{reason}; item returned to its container")
                } else {
                    format!("{reason}; item could not be returned — reload without saving")
                })
            }
        }
    }

    fn move_vault_to_left(&mut self) -> Result<String, String> {
        let (Some(pane), Some(vault_pane)) = (self.left.as_mut(), self.right.as_mut()) else {
            return Err("load a game file and a vault first".to_string());
        };
        let (tab, index) = vault_pane.selected.ok_or("select an item in the vault")?;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Failed(_) => None,
        };
        let vault_item = transfer::take_from_vault(&mut vault_pane.vault, tab, index)
            .ok_or("selection is stale — pick the item again")?;
        let label = self.caches.names.item_label(db, &vault_item.item);
        let preferred = pane.selected.map_or(0, |(container, _)| container);
        let placed = match &mut pane.file {
            GameFile::Character(character) => {
                transfer::place_in_character(character, vault_item.item, preferred, db)
                    .map(|sack| format!("{label} → sack {}", sack + 1))
            }
            GameFile::Stash(stash) => transfer::place_in_stash(stash, vault_item.item, db)
                .map(|()| format!("{label} → stash")),
        };
        match placed {
            Ok(message) => {
                pane.dirty = true;
                vault_pane.dirty = true;
                vault_pane.selected = None;
                Ok(message)
            }
            Err(rejected) => {
                let reason = rejected.reason;
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

    fn save_left(&mut self) -> Result<String, String> {
        let pane = self.left.as_mut().ok_or("nothing to save")?;
        let bytes = match &pane.file {
            GameFile::Character(character) => {
                let spliced = chr::replace_inventory(&pane.original, &character.sacks)
                    .map_err(|error| error.to_string())?;
                chr::replace_money(&spliced, character.info.money)
                    .map_err(|error| error.to_string())?
            }
            GameFile::Stash(stash) => stash::replace_items(&pane.original, &stash.items)
                .map_err(|error| error.to_string())?,
        };
        let backup = safe_write::backup_first_write(&pane.path, &bytes)
            .map_err(|error| error.to_string())?;
        if matches!(pane.file, GameFile::Stash(_)) {
            let twin = stash::backup_twin(&bytes).map_err(|error| error.to_string())?;
            std::fs::write(pane.path.with_extension("dxg"), twin)
                .map_err(|error| error.to_string())?;
        }
        pane.original = bytes;
        pane.dirty = false;
        Ok(match backup {
            Some(backup) => format!(
                "saved {} (backup: {})",
                pane.path.display(),
                backup.display()
            ),
            None => format!("saved {}", pane.path.display()),
        })
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

fn cache_file_path() -> Option<PathBuf> {
    univault_core::platform::config_dir().map(|dir| dir.join("gamedata.cache"))
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

/// One-time import: reads the game archives, distills the item cache,
/// and persists it (plus the game dir for later refreshes) under the
/// config directory.
fn import_game_data(dir: &Path) -> Result<GameCache, String> {
    let (database, database_stamp) = read_stamped(&dir.join("Database/database.arz"))?;
    let (text, text_stamp) = read_stamped(&dir.join("Text/Text_EN.arc"))?;
    let mut stamps = vec![database_stamp, text_stamp];
    let mut data = GameData::from_bytes(database, text).map_err(|error| error.to_string())?;
    let candidates = [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ];
    for (label, relative) in candidates {
        let path = dir.join(relative);
        if let Ok((bytes, stamp)) = read_stamped(&path)
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_items_archive(label, archive);
            stamps.push(stamp);
        }
    }
    let cache = data.build_cache(stamps);
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
        if let Some(dropped) = first_dropped_path(ui.ctx()) {
            self.status = Some(self.open(&dropped));
        }
        self.show_header(ui);
        ui.separator();
        let action = self.show_panes(ui);
        match action {
            Some(PaneAction::MoveToVault) => self.status = Some(self.move_left_to_vault()),
            Some(PaneAction::MoveToFile) => self.status = Some(self.move_vault_to_left()),
            Some(PaneAction::SaveFile) => self.status = Some(self.save_left()),
            Some(PaneAction::SaveVault) => self.status = Some(self.save_vault()),
            None => {}
        }
    }
}

enum PaneAction {
    MoveToVault,
    MoveToFile,
    SaveFile,
    SaveVault,
}

impl App {
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
            if ui.button("Import game data…").clicked() {
                let start = stored_game_dir();
                let mut dialog = rfd::FileDialog::new();
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                if let Some(dir) = dialog.pick_folder() {
                    match import_game_data(&dir) {
                        Ok(cache) => {
                            self.status = Some(Ok(format!(
                                "imported {} item records from {}",
                                cache.len(),
                                dir.display()
                            )));
                            self.game = GameStatus::Loaded(cache);
                            self.game_note = None;
                            self.caches = Caches::default();
                        }
                        Err(message) => self.status = Some(Err(message)),
                    }
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
        if self.left.is_none() && self.right.is_none() {
            ui.heading("TQ UniVault");
            ui.label(
                "Drop a Player.chr or stash (.dxb/.dxg) for the left pane and a vault \
                 (.json / legacy .vault) for the right — or pass --vault and a file path.",
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

    /// Where file dialogs start: near what the user last touched.
    fn dialog_start_dir(&self) -> Option<PathBuf> {
        self.left
            .as_ref()
            .map(|pane| pane.path.clone())
            .or_else(|| self.recents.entries.first().cloned())
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    }

    fn show_panes(&mut self, ui: &mut egui::Ui) -> Option<PaneAction> {
        let caches = &mut self.caches;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Failed(_) => None,
        };
        let mut action = None;
        let can_move = self.left.is_some() && self.right.is_some();
        ui.columns(2, |columns| {
            if let Some(pane) = &mut self.left {
                if let Some(chosen) = show_file_pane(&mut columns[0], pane, db, caches, can_move) {
                    action = Some(chosen);
                }
            } else {
                columns[0].weak("No game file loaded.");
            }
            if let Some(pane) = &mut self.right {
                if let Some(chosen) = show_vault_pane(&mut columns[1], pane, db, caches, can_move) {
                    action = Some(chosen);
                }
            } else {
                columns[1].weak("No vault loaded.");
            }
        });
        action
    }
}

/// On-screen size of one grid cell — the textures' native 32 pixels.
const CELL_SIZE: f32 = 32.0;

// Grid coordinates are small integers; f32 represents them exactly.
#[allow(clippy::cast_precision_loss)]
fn cells_to_points(cells: i32) -> f32 {
    cells as f32 * CELL_SIZE
}

/// Paints a container as its actual cell grid, with items at their
/// positions (icon when decodable, initial letter otherwise), click
/// selection, and a name tooltip on hover.
fn grid_view(
    ui: &mut egui::Ui,
    dims: (i32, i32),
    entries: &[(usize, &Item)],
    container: usize,
    selected: &mut Option<(usize, usize)>,
    db: Option<&GameCache>,
    caches: &mut Caches,
) {
    let size = egui::vec2(cells_to_points(dims.0), cells_to_points(dims.1));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
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
        let is_selected = *selected == Some((container, *index));
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

        if let Some(pointer) = response.hover_pos()
            && item_rect.contains(pointer)
        {
            hovered = Some(item);
            if response.clicked() {
                *selected = Some((container, *index));
            }
        }
    }
    if let Some(item) = hovered {
        response.on_hover_ui(|ui| item_tooltip(ui, item, db, caches));
    }
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

fn show_file_pane(
    ui: &mut egui::Ui,
    pane: &mut FilePane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
) -> Option<PaneAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        match &mut pane.file {
            GameFile::Character(character) => {
                ui.heading(
                    character
                        .info
                        .name
                        .as_deref()
                        .unwrap_or("Unnamed character"),
                );
                let gold = ui.add(
                    egui::DragValue::new(&mut character.info.money)
                        .range(0..=i32::MAX)
                        .prefix("gold: "),
                );
                if gold.changed() {
                    pane.dirty = true;
                }
            }
            GameFile::Stash(stash) => {
                ui.heading(format!("Stash {}×{}", stash.width, stash.height));
            }
        }
        if ui
            .add_enabled(pane.dirty, egui::Button::new("Save"))
            .clicked()
        {
            action = Some(PaneAction::SaveFile);
        }
        if ui
            .add_enabled(
                can_move && pane.selected.is_some(),
                egui::Button::new("→ Vault"),
            )
            .clicked()
        {
            action = Some(PaneAction::MoveToVault);
        }
    });
    ui.monospace(pane.path.display().to_string());
    egui::ScrollArea::vertical()
        .id_salt("file-pane")
        .show(ui, |ui| match &pane.file {
            GameFile::Character(character) => {
                ui.collapsing("Equipment (read-only)", |ui| {
                    for (name, slot) in EQUIPMENT_SLOT_NAMES.iter().zip(&character.equipment.slots)
                    {
                        match slot {
                            Some(item) => {
                                ui.label(format!("{name}: {}", caches.names.item_label(db, item)))
                            }
                            None => ui.weak(format!("{name}: —")),
                        };
                    }
                });
                for (index, sack) in character.sacks.iter().enumerate() {
                    let title = format!("Sack {} ({} items)", index + 1, sack.items.len());
                    egui::CollapsingHeader::new(title)
                        .default_open(true)
                        .show(ui, |ui| {
                            let entries: Vec<(usize, &Item)> =
                                sack.items.iter().enumerate().collect();
                            grid_view(
                                ui,
                                chr::sack_dimensions(index),
                                &entries,
                                index,
                                &mut pane.selected,
                                db,
                                caches,
                            );
                        });
                }
            }
            GameFile::Stash(stash) => {
                let entries: Vec<(usize, &Item)> = stash.items.iter().enumerate().collect();
                grid_view(
                    ui,
                    (stash.width, stash.height),
                    &entries,
                    0,
                    &mut pane.selected,
                    db,
                    caches,
                );
            }
        });
    action
}

fn show_vault_pane(
    ui: &mut egui::Ui,
    pane: &mut VaultPane,
    db: Option<&GameCache>,
    caches: &mut Caches,
    can_move: bool,
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
                            tab,
                            &mut pane.selected,
                            db,
                            caches,
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
