//! Real-data check for the stat display engine: imports the game
//! archives into a cache, then prints the assembled tooltip of every
//! item in a save tree, with palette color letters.
//!
//! Usage: `cargo run --release -p univault-core --example tooltips -- \
//!     "<TQ AE install dir>" "<SaveData dir>"`

use std::path::{Path, PathBuf};

use univault_core::cache::GameCache;
use univault_core::chr::{self, Item};
use univault_core::gamedata::GameData;
use univault_core::{stash, stats, style};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(save_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: tooltips <TQ AE install dir> <SaveData dir>");
        std::process::exit(2);
    };
    let game = Path::new(&game_dir);
    let database = std::fs::read(game.join("Database/database.arz")).expect("read database.arz");
    let text = std::fs::read(game.join("Text/Text_EN.arc")).expect("read Text_EN.arc");
    let mut data = GameData::from_bytes(database, text).expect("assemble game data");
    for (label, relative) in [
        ("", "Resources/Items.arc"),
        ("XPACK", "Resources/XPack/Items.arc"),
        ("XPACK2", "Resources/XPack2/Items.arc"),
        ("XPACK3", "Resources/XPack3/Items.arc"),
        ("XPACK4", "Resources/XPack4/Items.arc"),
    ] {
        if let Ok(bytes) = std::fs::read(game.join(relative))
            && let Ok(archive) = univault_core::arc::ArcFile::parse(bytes)
        {
            data.add_items_archive(label, archive);
        }
    }
    let started = std::time::Instant::now();
    let cache = data.build_cache(Vec::new());
    let bytes = cache.to_bytes();
    println!(
        "cache: {} records, {} MB, built in {:.1}s",
        cache.len(),
        bytes.len() / 1_048_576,
        started.elapsed().as_secs_f64()
    );
    let cache = GameCache::from_bytes(&bytes).expect("reload cache");

    let mut files = Vec::new();
    collect_files(Path::new(&save_dir), &mut files);
    files.sort();
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = path.display();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if file_name == "player.chr" {
            match chr::parse_player(&bytes) {
                Ok(player) => {
                    println!("\n######## {name}");
                    for item in player
                        .sacks
                        .iter()
                        .flat_map(|sack| &sack.items)
                        .chain(player.equipment.slots.iter().flatten())
                    {
                        print_tooltip(&cache, item);
                    }
                }
                Err(error) => println!("{name}: chr parse failed: {error}"),
            }
        } else if file_name == "winsys.dxb" {
            match stash::parse_stash(&bytes) {
                Ok(parsed) => {
                    println!("\n######## {name}");
                    for item in &parsed.items {
                        print_tooltip(&cache, item);
                    }
                }
                Err(error) => println!("{name}: stash parse failed: {error}"),
            }
        }
    }
}

fn collect_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, into);
        } else {
            into.push(path);
        }
    }
}

fn print_tooltip(cache: &GameCache, item: &Item) {
    let details = stats::item_details(cache, item);
    let item_style = style::item_style(Some(cache), item);
    let mut title: Vec<String> = Vec::new();
    if let Some(prefix) = &item.prefix {
        title.push(name_of(cache, prefix));
    }
    if let Some(quality) = &details.quality {
        title.push(quality.clone());
    }
    title.push(name_of(cache, &item.base));
    if let Some(style_word) = &details.style_word {
        title.push(style_word.clone());
    }
    if let Some(suffix) = &item.suffix {
        title.push(name_of(cache, suffix));
    }
    println!("\n=== {} [{}]", title.join(" "), item_style.label());
    for block in &details.blocks {
        println!("  ----");
        for line in block {
            println!("  [{}] {}", line.color, line.text);
        }
    }
}

fn name_of(cache: &GameCache, id: &univault_core::chr::RecordId) -> String {
    cache
        .record_name(id)
        .unwrap_or_else(|| id.file_stem().to_string())
}
