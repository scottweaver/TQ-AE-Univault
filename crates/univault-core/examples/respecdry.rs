//! Read-only respec rehearsal against real saves: applies both
//! respec operations in memory and verifies the result still parses
//! with identical inventory, equipment, and money. Never writes.
//!
//! Usage: `cargo run -p univault-core --example respecdry -- <SaveData dir>`

use std::path::{Path, PathBuf};

use univault_core::{chr, respec};

fn main() {
    let save_dir = std::env::args()
        .nth(1)
        .expect("usage: respecdry <SaveData dir>");
    let mut files = Vec::new();
    collect(Path::new(&save_dir), &mut files);
    files.sort();
    let mut failures = 0;
    for path in files {
        if path
            .file_name()
            .is_none_or(|name| !name.eq_ignore_ascii_case("Player.chr"))
        {
            continue;
        }
        let Ok(original) = std::fs::read(&path) else {
            continue;
        };
        let Ok(before) = chr::parse_player(&original) else {
            println!("{}: parse failed, skipping", path.display());
            continue;
        };
        println!("== {}", path.display());
        match respec::attribute_refund(&original) {
            Ok(points) => println!("  attribute refund preview: {points}"),
            Err(error) => println!("  attribute preview error: {error}"),
        }
        match respec::skill_refund(&original) {
            Ok((points, removed)) => {
                println!("  skill refund preview: {points} points, {removed} skills removed");
            }
            Err(error) => println!("  skill preview error: {error}"),
        }
        let respecced = respec::respec_attributes(&original)
            .and_then(|first| respec::respec_skills(&first.bytes));
        match respecced {
            Ok(result) => match chr::parse_player(&result.bytes) {
                Ok(after) => {
                    let items_equal = before.sacks == after.sacks
                        && before.equipment == after.equipment
                        && before.info.money == after.info.money;
                    let second_attr = respec::attribute_refund(&result.bytes);
                    let second_skill = respec::skill_refund(&result.bytes);
                    println!(
                        "  applied both; reparse ok; items+money identical: {items_equal}; \
                         second pass refunds: {second_attr:?} {second_skill:?}"
                    );
                    if !items_equal || second_attr != Ok(0) || !matches!(second_skill, Ok((0, 0))) {
                        failures += 1;
                    }
                }
                Err(error) => {
                    println!("  REPARSE FAILED after respec: {error}");
                    failures += 1;
                }
            },
            Err(error) => println!("  apply error: {error}"),
        }
    }
    if failures > 0 {
        eprintln!("{failures} failure(s)");
        std::process::exit(1);
    }
}

fn collect(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, into);
        } else {
            into.push(path);
        }
    }
}
