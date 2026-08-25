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

/// Per-record caches for what must never run per frame: record
/// decompression, footprint lookups, texture decodes.
#[derive(Default)]
struct Caches {
    names: NameCache,
    footprints: HashMap<String, (i32, i32)>,
    icons: HashMap<String, Option<egui::TextureHandle>>,
}

impl Caches {
    fn footprint(&mut self, db: Option<&GameData>, item: &Item) -> (i32, i32) {
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
        db: Option<&GameData>,
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

struct App {
    game: GameStatus,
    caches: Caches,
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
            caches: Caches::default(),
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

/// On-screen size of one grid cell.
const CELL_SIZE: f32 = 24.0;

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
    db: Option<&GameData>,
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

    let mut hovered: Option<String> = None;
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
            hovered = Some(caches.names.item_label(db, item));
            if response.clicked() {
                *selected = Some((container, *index));
            }
        }
    }
    if let Some(label) = hovered {
        response.on_hover_text(label);
    }
}

fn show_file_pane(
    ui: &mut egui::Ui,
    pane: &mut FilePane,
    db: Option<&GameData>,
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
    db: Option<&GameData>,
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

fn first_dropped_path(ctx: &egui::Context) -> Option<PathBuf> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .first()
            .map(|file| file.path().to_path_buf())
    })
}
