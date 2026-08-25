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
use univault_core::chr::{self, Item, PlayerCharacter, RecordId};
use univault_core::gamedata::GameData;
use univault_core::stash::{self, Stash};
use univault_core::transfer;
use univault_core::vault::Vault;

fn main() -> eframe::Result {
    let args = CliArgs::parse();
    eframe::run_native(
        "TQ UniVault",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(App::new(args)))),
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
    Loaded(GameData),
    Failed(String),
}

/// Resolved display names, cached per record path — name resolution
/// decompresses database records and must not run per frame.
#[derive(Default)]
struct NameCache {
    names: HashMap<String, String>,
}

impl NameCache {
    fn record_name(&mut self, db: Option<&GameData>, id: &RecordId) -> String {
        if let Some(cached) = self.names.get(id.as_str()) {
            return cached.clone();
        }
        let resolved = db
            .and_then(|db| db.record_name(id))
            .unwrap_or_else(|| id.file_stem().to_string());
        self.names.insert(id.as_str().to_string(), resolved.clone());
        resolved
    }

    fn item_label(&mut self, db: Option<&GameData>, item: &Item) -> String {
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

struct App {
    game: GameStatus,
    names: NameCache,
    left: Option<FilePane>,
    right: Option<VaultPane>,
    status: Option<Result<String, String>>,
}

impl App {
    fn new(args: CliArgs) -> Self {
        let game = args
            .game_dir
            .as_deref()
            .map_or(GameStatus::Absent, |dir| match load_game_data(dir) {
                Ok(data) => GameStatus::Loaded(data),
                Err(message) => GameStatus::Failed(message),
            });
        let mut app = Self {
            game,
            names: NameCache::default(),
            left: None,
            right: None,
            status: None,
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
        let label = self.names.item_label(db, &item);
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
        let label = self.names.item_label(db, &vault_item.item);
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

fn load_game_data(dir: &Path) -> Result<GameData, String> {
    let database = std::fs::read(dir.join("Database/database.arz"))
        .map_err(|error| format!("Database/database.arz: {error}"))?;
    let text = std::fs::read(dir.join("Text/Text_EN.arc"))
        .map_err(|error| format!("Text/Text_EN.arc: {error}"))?;
    let mut data = GameData::from_bytes(database, text).map_err(|error| error.to_string())?;
    load_item_archives(dir, &mut data);
    Ok(data)
}

/// Registers every present `Items.arc` (base + expansions) for real
/// item footprints; missing ones just leave the conservative
/// fallback in place.
fn load_item_archives(dir: &Path, data: &mut GameData) {
    let candidates = [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ];
    for (label, relative) in candidates {
        if let Ok(bytes) = std::fs::read(dir.join(relative))
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_items_archive(label, archive);
        }
    }
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
        match &self.game {
            GameStatus::Loaded(_) => {}
            GameStatus::Absent => {
                ui.weak("No --game <dir> given — showing record ids instead of item names.");
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

    fn show_panes(&mut self, ui: &mut egui::Ui) -> Option<PaneAction> {
        let db_names = &mut self.names;
        let db = match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Failed(_) => None,
        };
        let mut action = None;
        let can_move = self.left.is_some() && self.right.is_some();
        ui.columns(2, |columns| {
            if let Some(pane) = &mut self.left {
                if let Some(chosen) = show_file_pane(&mut columns[0], pane, db, db_names, can_move)
                {
                    action = Some(chosen);
                }
            } else {
                columns[0].weak("No game file loaded.");
            }
            if let Some(pane) = &mut self.right {
                if let Some(chosen) = show_vault_pane(&mut columns[1], pane, db, db_names, can_move)
                {
                    action = Some(chosen);
                }
            } else {
                columns[1].weak("No vault loaded.");
            }
        });
        action
    }
}

fn show_file_pane(
    ui: &mut egui::Ui,
    pane: &mut FilePane,
    db: Option<&GameData>,
    names: &mut NameCache,
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
                                ui.label(format!("{name}: {}", names.item_label(db, item)))
                            }
                            None => ui.weak(format!("{name}: —")),
                        };
                    }
                });
                for (index, sack) in character.sacks.iter().enumerate() {
                    let title = format!("Sack {} ({} items)", index + 1, sack.items.len());
                    ui.collapsing(title, |ui| {
                        item_rows(ui, &sack.items, index, &mut pane.selected, db, names);
                    });
                }
            }
            GameFile::Stash(stash) => {
                item_rows(ui, &stash.items, 0, &mut pane.selected, db, names);
            }
        });
    action
}

fn show_vault_pane(
    ui: &mut egui::Ui,
    pane: &mut VaultPane,
    db: Option<&GameData>,
    names: &mut NameCache,
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
                ui.collapsing(title, |ui| {
                    if sack.items.is_empty() {
                        ui.weak("empty");
                    }
                    for (index, entry) in sack.items.iter().enumerate() {
                        let selected = pane.selected == Some((tab, index));
                        let label = format!(
                            "({}, {})  {}",
                            entry.item.position.x,
                            entry.item.position.y,
                            names.item_label(db, &entry.item)
                        );
                        if ui.selectable_label(selected, label).clicked() {
                            pane.selected = Some((tab, index));
                        }
                    }
                });
            }
        });
    action
}

fn item_rows(
    ui: &mut egui::Ui,
    items: &[Item],
    container: usize,
    selected: &mut Option<(usize, usize)>,
    db: Option<&GameData>,
    names: &mut NameCache,
) {
    if items.is_empty() {
        ui.weak("empty");
    }
    for (index, item) in items.iter().enumerate() {
        let is_selected = *selected == Some((container, index));
        let label = format!(
            "({}, {})  {}",
            item.position.x,
            item.position.y,
            names.item_label(db, item)
        );
        if ui.selectable_label(is_selected, label).clicked() {
            *selected = Some((container, index));
        }
    }
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
