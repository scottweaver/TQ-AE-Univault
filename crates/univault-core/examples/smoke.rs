//! Real-data smoke check for the ARZ/ARC/text/naming pipeline.
//!
//! Usage: `cargo run --release -p univault-core --example smoke -- "<TQ AE install dir>"`
//!
//! Builds the combined [`GameData`] from `Database/database.arz` and
//! `Text/Text_EN.arc`, decompresses **every** record, and resolves
//! localized names through the per-type dispatch. Read-only against
//! the install; exits non-zero on any structural failure.

use univault_core::gamedata::GameData;

fn main() {
    let game_dir = std::env::args()
        .nth(1)
        .expect("usage: smoke <TQ AE install dir>");
    let game = std::path::Path::new(&game_dir);

    let database_bytes =
        std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    let archive_bytes = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    println!(
        "database.arz: {} bytes, Text_EN.arc: {} bytes",
        database_bytes.len(),
        archive_bytes.len()
    );
    let db = GameData::from_bytes(database_bytes, archive_bytes).expect("assemble game data");

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
}
