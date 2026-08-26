//! Exports every mastery's full skill tree — localized names,
//! descriptions, tiers, and per-level effect arrays — as one JSON
//! document for build theorycrafting with AI tools. The distillation
//! itself lives in `univault_core::skilltree`; this example adds the
//! file layout (one `<mastery-slug>.json` per mastery plus an
//! `index.json`).
//!
//! The output embeds extracted game data: keep it local (the
//! repository ignores `exports/`), per the derived-data rule in
//! ARCHITECTURE.md.
//!
//! Usage: `cargo run --release -p univault-core --example skilltree -- \
//!     "<TQ AE install dir>" <output dir>`

use serde_json::json;
use univault_core::gamedata::GameData;
use univault_core::skilltree;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: skilltree <TQ AE install dir> <output dir>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let game = std::path::Path::new(&game_dir);
    let database = std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    let text = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    let data = GameData::from_bytes(database, text).expect("assemble game data");

    let mut index = Vec::new();
    for mastery in skilltree::masteries(&data) {
        let Some(document) = skilltree::mastery_tree(&data, &mastery) else {
            continue;
        };
        let slug: String = mastery
            .name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let out_path = std::path::Path::new(&out_dir).join(format!("{slug}.json"));
        let rendered = serde_json::to_string_pretty(&document).expect("serialize");
        std::fs::write(&out_path, &rendered).expect("write output");
        println!(
            "{:20} {:3} skills, {:3} referenced, {:4} KB -> {}",
            mastery.name,
            document["skills"].as_array().map_or(0, Vec::len),
            document["referenced"].as_array().map_or(0, Vec::len),
            rendered.len() / 1024,
            out_path.display()
        );
        index.push(json!({
            "mastery": mastery.name,
            "file": format!("{slug}.json"),
            "skills": document["skills"].as_array().map_or(0, Vec::len),
        }));
    }
    let index_path = std::path::Path::new(&out_dir).join("index.json");
    std::fs::write(
        &index_path,
        serde_json::to_string_pretty(&json!({"masteries": index})).unwrap(),
    )
    .expect("write index");
    println!("index -> {}", index_path.display());
}
