//! egui/eframe front-end for tq-univault.
//!
//! Read-only vertical slice: opens a Titan Quest `Player.chr` (first
//! command-line argument, or drag-and-drop onto the window) and renders
//! the character's inventory sacks and equipment.

use std::path::PathBuf;

use eframe::egui;
use univault_core::chr::{self, Item, PlayerCharacter};
use univault_core::vault::Vault;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    eframe::run_native(
        "TQ UniVault",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(App::new(initial)))),
    )
}

enum View {
    Idle,
    Loaded {
        path: PathBuf,
        character: Box<PlayerCharacter>,
    },
    LoadedVault {
        path: PathBuf,
        vault: Box<Vault>,
    },
    Failed {
        path: PathBuf,
        message: String,
    },
}

struct App {
    view: View,
}

impl App {
    fn new(initial: Option<PathBuf>) -> Self {
        Self {
            view: initial.map_or(View::Idle, load),
        }
    }
}

enum Parsed {
    Character(Box<PlayerCharacter>),
    Vault(Box<Vault>),
}

fn load(path: PathBuf) -> View {
    let parsed = std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| parse_by_extension(&path, &bytes));
    match parsed {
        Ok(Parsed::Character(character)) => View::Loaded { path, character },
        Ok(Parsed::Vault(vault)) => View::LoadedVault { path, vault },
        Err(message) => View::Failed { path, message },
    }
}

/// Vaults are `.json` (modern) or `.vault` (legacy binary, imported
/// read-only); anything else is treated as a `Player.chr`.
fn parse_by_extension(path: &std::path::Path, bytes: &[u8]) -> Result<Parsed, String> {
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

/// Best display name available without ARZ text resources: the record
/// file stems of affixes and base, e.g. "sharp `sword_01` of quality".
fn display_name(item: &Item) -> String {
    let parts = [
        item.prefix.as_ref().map(chr::RecordId::file_stem),
        Some(item.base.file_stem()),
        item.suffix.as_ref().map(chr::RecordId::file_stem),
    ];
    parts.into_iter().flatten().collect::<Vec<_>>().join(" ")
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(dropped) = first_dropped_path(ui.ctx()) {
            self.view = load(dropped);
        }
        match &self.view {
            View::Idle => {
                ui.heading("TQ UniVault");
                ui.label(
                    "Drop a Player.chr or a vault (.json / legacy .vault) here, \
                     or pass a path as the first argument.",
                );
            }
            View::Failed { path, message } => {
                ui.heading("Could not load file");
                ui.monospace(path.display().to_string());
                ui.colored_label(ui.visuals().error_fg_color, message);
                ui.label("Drop another file to retry.");
            }
            View::Loaded { path, character } => show_character(ui, path, character),
            View::LoadedVault { path, vault } => show_vault(ui, path, vault),
        }
    }
}

fn show_vault(ui: &mut egui::Ui, path: &std::path::Path, vault: &Vault) {
    let name = path.file_stem().map_or_else(
        || "Vault".to_string(),
        |stem| stem.to_string_lossy().to_string(),
    );
    let item_count: usize = vault.sacks.iter().map(|sack| sack.items.len()).sum();
    ui.heading(format!("Vault — {name}"));
    ui.label(format!("{} tabs — {item_count} items", vault.sacks.len()));
    ui.monospace(path.display().to_string());
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, sack) in vault.sacks.iter().enumerate() {
            let title = format!("Tab {} ({} items)", index + 1, sack.items.len());
            ui.collapsing(title, |ui| {
                if sack.items.is_empty() {
                    ui.weak("empty");
                }
                for vault_item in &sack.items {
                    let item = &vault_item.item;
                    let stack = if item.stack_size > 1 {
                        format!(" ×{}", item.stack_size)
                    } else {
                        String::new()
                    };
                    ui.label(format!(
                        "({}, {})  {}{stack}",
                        item.position.x,
                        item.position.y,
                        display_name(item)
                    ));
                }
            });
        }
    });
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

fn show_character(ui: &mut egui::Ui, path: &std::path::Path, character: &PlayerCharacter) {
    let info = &character.info;
    ui.heading(info.name.as_deref().unwrap_or("Unnamed character"));
    ui.label(format!(
        "Level {} — {} — {} gold",
        info.level,
        info.class_tag.as_deref().unwrap_or("no class"),
        info.money
    ));
    ui.monospace(path.display().to_string());
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.collapsing("Equipment", |ui| {
            for (name, slot) in EQUIPMENT_SLOT_NAMES.iter().zip(&character.equipment.slots) {
                match slot {
                    Some(item) => ui.label(format!("{name}: {}", display_name(item))),
                    None => ui.weak(format!("{name}: —")),
                };
            }
        });
        for (index, sack) in character.sacks.iter().enumerate() {
            let title = format!("Sack {} ({} items)", index + 1, sack.items.len());
            ui.collapsing(title, |ui| {
                if sack.items.is_empty() {
                    ui.weak("empty");
                }
                for item in &sack.items {
                    let stack = if item.stack_size > 1 {
                        format!(" ×{}", item.stack_size)
                    } else {
                        String::new()
                    };
                    ui.label(format!(
                        "({}, {})  {}{stack}",
                        item.position.x,
                        item.position.y,
                        display_name(item)
                    ));
                }
            });
        }
    });
}
