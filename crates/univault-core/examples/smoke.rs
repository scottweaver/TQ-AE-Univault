//! Real-data smoke check for the whole read stack.
//!
//! Usage: `cargo run --release -p univault-core --example smoke -- "<TQ AE install dir>" ["<SaveData dir>"]`
//!
//! Builds the combined [`GameData`] from `Database/database.arz` and
//! `Text/Text_EN.arc`, decompresses **every** record, and resolves
//! localized names through the per-type dispatch. With a `SaveData`
//! directory as second argument it also parses every `Player.chr`
//! and stash (`winsys.dxb`) in the tree and prints their contents
//! with resolved names. Read-only; exits non-zero on any structural
//! failure.

use std::path::Path;

use univault_core::gamedata::GameData;

fn main() {
    let game_dir = std::env::args()
        .nth(1)
        .expect("usage: smoke <TQ AE install dir> [SaveData dir]");
    let game = std::path::Path::new(&game_dir);

    let database_bytes =
        std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    let archive_bytes = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    println!(
        "database.arz: {} bytes, Text_EN.arc: {} bytes",
        database_bytes.len(),
        archive_bytes.len()
    );
    let mut db = GameData::from_bytes(database_bytes, archive_bytes).expect("assemble game data");
    let mut archives = 0_usize;
    for (label, relative) in [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ] {
        if let Ok(bytes) = std::fs::read(game.join(relative)) {
            let archive = univault_core::arc::ArcFile::parse(bytes)
                .unwrap_or_else(|error| panic!("{relative}: {error}"));
            db.add_items_archive(label, archive);
            archives += 1;
        }
    }
    println!("item archives loaded: {archives}");
    let db = db;

    let ids: Vec<_> = db.record_ids().cloned().collect();
    println!("{} records indexed", ids.len());

    let mut record_errors = 0_usize;
    let mut resolved = 0_usize;
    let mut gear_samples: Vec<String> = Vec::new();
    let mut affix_samples: Vec<String> = Vec::new();
    for id in &ids {
        let record = match db.record(id).expect("indexed id must resolve") {
            Ok(record) => record,
            Err(error) => {
                record_errors += 1;
                if record_errors <= 3 {
                    eprintln!("record error: {error}");
                }
                continue;
            }
        };
        let Some(name) = db.record_name(id) else {
            continue;
        };
        resolved += 1;
        let is_gear =
            record.record_type.starts_with("Weapon") || record.record_type.starts_with("Armor");
        if is_gear && gear_samples.len() < 8 {
            gear_samples.push(format!(
                "  {} [{}] -> {name}",
                id.file_stem(),
                record.record_type
            ));
        }
        if record.record_type.starts_with("LootRandomizer") && affix_samples.len() < 8 {
            affix_samples.push(format!("  {} -> {name}", id.file_stem()));
        }
    }
    println!(
        "decompressed {} records: {record_errors} errors, {resolved} resolved to localized names",
        ids.len()
    );
    println!("gear samples:");
    for line in &gear_samples {
        println!("{line}");
    }
    println!("affix samples:");
    for line in &affix_samples {
        println!("{line}");
    }
    assert_eq!(record_errors, 0, "some records failed to decompress");
    assert!(resolved > 10_000, "suspiciously few names resolved");

    if let Some(saves) = std::env::args().nth(2) {
        sweep_saves(Path::new(&saves), &db);
    }
}

/// Parses every character and stash in a `SaveData` tree, printing
/// contents with resolved names. Panics on any parse failure so the
/// sweep is a hard validation gate.
fn sweep_saves(saves: &Path, db: &GameData) {
    let mut characters = 0_usize;
    let mut stashes = 0_usize;

    let transfer = saves.join("Sys/winsys.dxb");
    if transfer.is_file() {
        stashes += 1;
        report_stash(&transfer, db);
    }

    for area in ["Main", "User"] {
        let Ok(entries) = std::fs::read_dir(saves.join(area)) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let chr_path = dir.join("Player.chr");
            if chr_path.is_file() {
                characters += 1;
                report_character(&chr_path, db);
            }
            let stash_path = dir.join("winsys.dxb");
            if stash_path.is_file() {
                stashes += 1;
                report_stash(&stash_path, db);
            }
        }
    }
    println!("save sweep: {characters} characters, {stashes} stashes — all parsed");
    assert!(characters > 0, "no characters found in {}", saves.display());
}

fn report_character(path: &Path, db: &GameData) {
    let bytes = std::fs::read(path).expect("read Player.chr");
    let character = univault_core::chr::parse_player(&bytes)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let respliced = univault_core::chr::replace_inventory(&bytes, &character.sacks)
        .unwrap_or_else(|error| panic!("{}: resplice: {error}", path.display()));
    assert_identical(path, &bytes, &respliced);
    let info = &character.info;
    let equipped = character
        .equipment
        .slots
        .iter()
        .flatten()
        .map(|item| {
            let (width, height) = db.item_footprint(item);
            format!("{} [{width}x{height}]", db.item_name(item))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let sack_items: usize = character.sacks.iter().map(|sack| sack.items.len()).sum();
    println!(
        "character {:?} (level {}, {} sack items): wearing [{equipped}]",
        info.name.as_deref().unwrap_or("?"),
        info.level,
        sack_items
    );
}

/// Byte-identity gate with a diagnostic dump of the first difference.
fn assert_identical(path: &Path, original: &[u8], respliced: &[u8]) {
    if original == respliced {
        return;
    }
    let diff_at = original
        .iter()
        .zip(respliced)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| original.len().min(respliced.len()));
    let window = |data: &[u8]| {
        let start = diff_at.saturating_sub(24);
        let end = (diff_at + 24).min(data.len());
        (
            data[start..end]
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            String::from_utf8_lossy(&data[start..end])
                .chars()
                .map(|c| if c.is_ascii_graphic() { c } else { '.' })
                .collect::<String>(),
        )
    };
    let (original_hex, original_ascii) = window(original);
    let (respliced_hex, respliced_ascii) = window(respliced);
    panic!(
        "{}: resplice differs at offset {diff_at} ({} -> {} bytes)\n\
         original:  {original_hex}\n           {original_ascii}\n\
         respliced: {respliced_hex}\n           {respliced_ascii}",
        path.display(),
        original.len(),
        respliced.len()
    );
}

fn report_stash(path: &Path, db: &GameData) {
    let bytes = std::fs::read(path).expect("read stash");
    let stash = univault_core::stash::parse_stash(&bytes)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let respliced = univault_core::stash::replace_items(&bytes, &stash.items)
        .unwrap_or_else(|error| panic!("{}: resplice: {error}", path.display()));
    assert_identical(path, &bytes, &respliced);
    let names = stash
        .items
        .iter()
        .take(6)
        .map(|item| db.item_name(item))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "stash {} ({}x{}, {} items): [{names}{}]",
        path.display(),
        stash.width,
        stash.height,
        stash.items.len(),
        if stash.items.len() > 6 { ", …" } else { "" }
    );
}
