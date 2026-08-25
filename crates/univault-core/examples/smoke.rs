//! Real-data smoke check for the ARZ/ARC/text pipeline.
//!
//! Usage: `cargo run --release -p univault-core --example smoke -- "<TQ AE install dir>"`
//!
//! Parses `Database/database.arz`, decompresses **every** record,
//! loads `Text/Text_EN.arc` into a tag table, and resolves item names
//! end to end. Read-only against the install; exits non-zero on any
//! structural failure.

use univault_core::arc::ArcFile;
use univault_core::arz::ArzFile;
use univault_core::text::TextDb;

fn main() {
    let game_dir = std::env::args()
        .nth(1)
        .expect("usage: smoke <TQ AE install dir>");
    let game = std::path::Path::new(&game_dir);

    let database_bytes =
        std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    println!("database.arz: {} bytes", database_bytes.len());
    let arz = ArzFile::parse(database_bytes).expect("parse database.arz");
    println!("database.arz: {} records indexed", arz.record_ids().count());

    let archive_bytes = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    let arc = ArcFile::parse(archive_bytes).expect("parse Text_EN.arc");
    let mut names: Vec<String> = arc.file_names().map(str::to_string).collect();
    names.sort_unstable();
    println!("Text_EN.arc: {} files: {names:?}", names.len());

    let mut text = TextDb::new();
    for name in &names {
        if name.to_lowercase().ends_with(".txt") {
            let bytes = arc.file(name).unwrap().expect("extract text file");
            text.add_file(&bytes);
        }
    }
    println!("text db: {} tags", text.len());

    let ids: Vec<_> = arz.record_ids().cloned().collect();
    let mut record_errors = 0_usize;
    let mut resolved = 0_usize;
    let mut examples: Vec<String> = Vec::new();
    let mut dumped_one = false;
    for id in &ids {
        match arz.record(id).expect("indexed id must resolve") {
            Ok(record) => {
                let is_gear = record.record_type.starts_with("Weapon")
                    || record.record_type.starts_with("Armor");
                if is_gear && !dumped_one {
                    dumped_one = true;
                    println!(
                        "--- sample record {} [{}] ---",
                        id.as_str(),
                        record.record_type
                    );
                    let mut variables: Vec<String> = record
                        .variables()
                        .map(|variable| format!("  {} = {:?}", variable.name, variable.values))
                        .collect();
                    variables.sort_unstable();
                    for line in variables.iter().take(30) {
                        println!("{line}");
                    }
                }
                if let Some(label) = record.string("description").and_then(|tag| text.get(tag)) {
                    resolved += 1;
                    if is_gear && examples.len() < 10 {
                        examples.push(format!("  {} -> {label}", id.file_stem()));
                    }
                }
            }
            Err(error) => {
                record_errors += 1;
                if record_errors <= 3 {
                    eprintln!("record error: {error}");
                }
            }
        }
    }
    println!(
        "decompressed {} records: {} errors, {resolved} resolved to localized names",
        ids.len(),
        record_errors
    );
    println!("sample names:");
    for line in &examples {
        println!("{line}");
    }
    assert_eq!(record_errors, 0, "some records failed to decompress");
}
