//! egui/eframe front-end for tq-univault.
//!
//! Usage: `univault-gui [--game <TQ install dir>] [file]`
//!
//! Opens a Titan Quest `Player.chr`, a vault `.json`, or a legacy
//! `.vault` (argument or drag-and-drop) and renders it read-only.
//! With `--game`, items show their localized names resolved through
//! the game's database; without it, record file stems.

use std::path::{Path, PathBuf};

use eframe::egui;
use univault_core::chr::{self, Item, PlayerCharacter};
use univault_core::gamedata::GameData;
use univault_core::stash::{self, Stash};
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
    file: Option<PathBuf>,
}

impl CliArgs {
    fn parse() -> Self {
        let mut game_dir = None;
        let mut file = None;
        let mut args = std::env::args_os().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--game" {
                game_dir = args.next().map(PathBuf::from);
            } else {
                file = Some(PathBuf::from(arg));
            }
        }
        Self { game_dir, file }
    }
}

/// Everything below is display-ready: views are built once at load
/// time (name resolution decompresses database records, which must
/// not happen per frame).
enum View {
    Idle,
    File(FileView),
    Failed { path: String, message: String },
}

struct FileView {
    heading: String,
    subtitle: String,
    path: String,
    sections: Vec<Section>,
}

struct Section {
    title: String,
    rows: Vec<Row>,
}

struct Row {
    text: String,
    dimmed: bool,
}

enum GameStatus {
    Absent,
    Loaded(GameData),
    Failed(String),
}

struct App {
    game: GameStatus,
    view: View,
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
            view: View::Idle,
        };
        if let Some(file) = args.file {
            app.view = app.load(&file);
        }
        app
    }

    fn db(&self) -> Option<&GameData> {
        match &self.game {
            GameStatus::Loaded(data) => Some(data),
            GameStatus::Absent | GameStatus::Failed(_) => None,
        }
    }

    fn load(&self, path: &Path) -> View {
        let parsed = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| parse_by_extension(path, &bytes));
        match parsed {
            Ok(Parsed::Character(character)) => {
                View::File(character_view(path, &character, self.db()))
            }
            Ok(Parsed::Vault(vault)) => View::File(vault_view(path, &vault, self.db())),
            Ok(Parsed::Stash(stash)) => View::File(stash_view(path, &stash, self.db())),
            Err(message) => View::Failed {
                path: path.display().to_string(),
                message,
            },
        }
    }
}

fn load_game_data(dir: &Path) -> Result<GameData, String> {
    let database = std::fs::read(dir.join("Database/database.arz"))
        .map_err(|error| format!("Database/database.arz: {error}"))?;
    let text = std::fs::read(dir.join("Text/Text_EN.arc"))
        .map_err(|error| format!("Text/Text_EN.arc: {error}"))?;
    GameData::from_bytes(database, text).map_err(|error| error.to_string())
}

enum Parsed {
    Character(Box<PlayerCharacter>),
    Vault(Box<Vault>),
    Stash(Box<Stash>),
}

/// Vaults are `.json` (modern) or `.vault` (legacy binary, imported
/// read-only); stashes are `.dxb` / `.dxg`; anything else is treated
/// as a `Player.chr`.
fn parse_by_extension(path: &Path, bytes: &[u8]) -> Result<Parsed, String> {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("json") => Vault::from_json(&String::from_utf8_lossy(bytes))
            .map(|vault| Parsed::Vault(Box::new(vault)))
            .map_err(|error| error.to_string()),
        Some("vault") => Vault::from_legacy_binary(bytes)
            .map(|vault| Parsed::Vault(Box::new(vault)))
            .map_err(|error| error.to_string()),
        Some("dxb" | "dxg") => stash::parse_stash(bytes)
            .map(|stash| Parsed::Stash(Box::new(stash)))
            .map_err(|error| error.to_string()),
        _ => chr::parse_player(bytes)
            .map(|character| Parsed::Character(Box::new(character)))
            .map_err(|error| error.to_string()),
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

/// Localized name when game data is loaded, record file stems
/// otherwise.
fn item_label(item: &Item, db: Option<&GameData>) -> String {
    db.map_or_else(|| stem_label(item), |db| db.item_name(item))
}

fn stem_label(item: &Item) -> String {
    let parts = [
        item.prefix.as_ref().map(chr::RecordId::file_stem),
        Some(item.base.file_stem()),
        item.suffix.as_ref().map(chr::RecordId::file_stem),
    ];
    parts.into_iter().flatten().collect::<Vec<_>>().join(" ")
}

fn item_row(item: &Item, db: Option<&GameData>) -> Row {
    let stack = if item.stack_size > 1 {
        format!(" ×{}", item.stack_size)
    } else {
        String::new()
    };
    Row {
        text: format!(
            "({}, {})  {}{stack}",
            item.position.x,
            item.position.y,
            item_label(item, db)
        ),
        dimmed: false,
    }
}

fn character_view(path: &Path, character: &PlayerCharacter, db: Option<&GameData>) -> FileView {
    let info = &character.info;
    let equipment = Section {
        title: "Equipment".to_string(),
        rows: EQUIPMENT_SLOT_NAMES
            .iter()
            .zip(&character.equipment.slots)
            .map(|(name, slot)| match slot {
                Some(item) => Row {
                    text: format!("{name}: {}", item_label(item, db)),
                    dimmed: false,
                },
                None => Row {
                    text: format!("{name}: —"),
                    dimmed: true,
                },
            })
            .collect(),
    };
    let sacks = character.sacks.iter().enumerate().map(|(index, sack)| {
        item_section(
            format!("Sack {} ({} items)", index + 1, sack.items.len()),
            &sack.items,
            db,
        )
    });
    FileView {
        heading: info
            .name
            .clone()
            .unwrap_or_else(|| "Unnamed character".to_string()),
        subtitle: format!(
            "Level {} — {} — {} gold",
            info.level,
            info.class_tag.as_deref().unwrap_or("no class"),
            info.money
        ),
        path: path.display().to_string(),
        sections: std::iter::once(equipment).chain(sacks).collect(),
    }
}

fn vault_view(path: &Path, vault: &Vault, db: Option<&GameData>) -> FileView {
    let item_count: usize = vault.sacks.iter().map(|sack| sack.items.len()).sum();
    let name = path.file_stem().map_or_else(
        || "Vault".to_string(),
        |stem| stem.to_string_lossy().to_string(),
    );
    FileView {
        heading: format!("Vault — {name}"),
        subtitle: format!("{} tabs — {item_count} items", vault.sacks.len()),
        path: path.display().to_string(),
        sections: vault
            .sacks
            .iter()
            .enumerate()
            .map(|(index, sack)| {
                let items: Vec<Item> = sack
                    .items
                    .iter()
                    .map(|vault_item| vault_item.item.clone())
                    .collect();
                item_section(
                    format!("Tab {} ({} items)", index + 1, items.len()),
                    &items,
                    db,
                )
            })
            .collect(),
    }
}

fn stash_view(path: &Path, stash: &Stash, db: Option<&GameData>) -> FileView {
    let name = path.file_stem().map_or_else(
        || "Stash".to_string(),
        |stem| stem.to_string_lossy().to_string(),
    );
    FileView {
        heading: format!("Stash — {name}"),
        subtitle: format!(
            "{}×{} grid — {} items",
            stash.width,
            stash.height,
            stash.items.len()
        ),
        path: path.display().to_string(),
        sections: vec![item_section(
            format!("Items ({})", stash.items.len()),
            &stash.items,
            db,
        )],
    }
}

fn item_section(title: String, items: &[Item], db: Option<&GameData>) -> Section {
    let rows = if items.is_empty() {
        vec![Row {
            text: "empty".to_string(),
            dimmed: true,
        }]
    } else {
        items.iter().map(|item| item_row(item, db)).collect()
    };
    Section { title, rows }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(dropped) = first_dropped_path(ui.ctx()) {
            self.view = self.load(&dropped);
        }
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
        match &self.view {
            View::Idle => {
                ui.heading("TQ UniVault");
                ui.label(
                    "Drop a Player.chr, a stash (.dxb/.dxg), or a vault (.json / \
                     legacy .vault) here, or pass a path as the first argument.",
                );
            }
            View::Failed { path, message } => {
                ui.heading("Could not load file");
                ui.monospace(path);
                ui.colored_label(ui.visuals().error_fg_color, message);
                ui.label("Drop another file to retry.");
            }
            View::File(view) => show_file(ui, view),
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

fn show_file(ui: &mut egui::Ui, view: &FileView) {
    ui.heading(&view.heading);
    ui.label(&view.subtitle);
    ui.monospace(&view.path);
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for section in &view.sections {
            ui.collapsing(&section.title, |ui| {
                for row in &section.rows {
                    if row.dimmed {
                        ui.weak(&row.text);
                    } else {
                        ui.label(&row.text);
                    }
                }
            });
        }
    });
}
