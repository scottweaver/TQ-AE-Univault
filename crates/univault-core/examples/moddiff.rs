//! Diffs a mod database against the main game database: which
//! records the mod overrides, and which variables changed. A name
//! filter narrows the sweep (e.g. `experience`).
//!
//! Usage: `cargo run --release -p univault-core --example moddiff -- \
//!     "<TQ AE install dir>" <mod .arz> [variable-name filter]`

use std::path::Path;

use univault_core::arz::{ArzFile, DbValues};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(mod_arz)) = (args.next(), args.next()) else {
        eprintln!("usage: moddiff <TQ AE install dir> <mod .arz> [variable filter]");
        std::process::exit(2);
    };
    let filter = args.next().map(|f| f.to_uppercase());
    let main_db = ArzFile::parse(
        std::fs::read(Path::new(&game_dir).join("Database/database.arz")).expect("read database"),
    )
    .expect("parse main database");
    let mod_db =
        ArzFile::parse(std::fs::read(&mod_arz).expect("read mod")).expect("parse mod database");

    let mut overridden = 0;
    let mut added = 0;
    for id in mod_db.record_ids() {
        let Some(Ok(modded)) = mod_db.record(id) else {
            continue;
        };
        let Some(Ok(vanilla)) = main_db.record(id) else {
            added += 1;
            continue;
        };
        overridden += 1;
        for variable in modded.variables() {
            if filter
                .as_ref()
                .is_some_and(|f| !variable.name.to_uppercase().contains(f))
            {
                continue;
            }
            let before = vanilla.variable(&variable.name);
            if before.map(|b| &b.values) != Some(&variable.values) {
                println!(
                    "{}\n  {}: {} -> {}",
                    id.as_str(),
                    variable.name,
                    display(before.map(|b| &b.values)),
                    display(Some(&variable.values))
                );
            }
        }
    }
    println!("\n{overridden} overridden records, {added} added records");
}

fn display(values: Option<&DbValues>) -> String {
    match values {
        None => "<absent>".to_string(),
        Some(DbValues::Integers(v)) => format!("{v:?}"),
        Some(DbValues::Floats(v)) => format!("{v:?}"),
        Some(DbValues::Strings(v)) => format!("{v:?}"),
        Some(DbValues::Booleans(v)) => format!("{v:?}"),
    }
}
