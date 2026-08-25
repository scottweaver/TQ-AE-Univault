//! Prints the respec-relevant regions of a `Player.chr` — attribute
//! `temp` floats, point pools, class tag, and the skill list — with
//! byte offsets, for porting and validating the respec splices.
//!
//! Usage: `cargo run -p univault-core --example chrprobe -- <Player.chr>`

use std::fmt::Write as _;

use univault_core::reader::{ByteReader, find_key};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: chrprobe <Player.chr>");
    let data = std::fs::read(&path).expect("read save");

    for key in [
        "playerClassTag",
        "playerLevel",
        "currentStats.charLevel",
        "modifierPoints",
        "skillPoints",
        "masteriesAllowed",
        "skillReclamationPointsUsed",
        "equipmentSelection",
        "skillWindowSelection",
        "skillSettingValid",
    ] {
        let mut from = 0;
        while let Some(offset) = find_key(&data, key, from) {
            let mut reader = ByteReader::at(&data, offset);
            if key == "playerClassTag" {
                println!("0x{offset:06X} {key} = {:?}", reader.read_cstring());
            } else {
                println!("0x{offset:06X} {key} = {:?}", reader.read_i32());
            }
            from = offset;
        }
    }

    println!("-- temp floats (attributes live in the first block of five) --");
    let mut from = 0;
    while let Some(offset) = find_key(&data, "temp", from) {
        let mut reader = ByteReader::at(&data, offset);
        println!("0x{offset:06X} temp = {:?}", reader.read_f32());
        from = offset;
    }

    println!("-- skill list --");
    let mut from = 0;
    while let Some(offset) = find_key(&data, "max", from) {
        let mut reader = ByteReader::at(&data, offset);
        let count = reader.read_i32().unwrap_or(-1);
        println!("0x{offset:06X} max = {count}");
        from = offset;
    }
    let mut from = 0;
    let mut total_levels = 0;
    let mut skills = 0;
    while let Some(offset) = find_key(&data, "skillName", from) {
        let mut reader = ByteReader::at(&data, offset);
        let name = reader.read_cstring().unwrap_or_default();
        let mut line = format!("0x{offset:06X} {name}");
        for key in [
            "skillLevel",
            "skillEnabled",
            "skillSubLevel",
            "skillActive",
            "skillTransition",
        ] {
            if let Some(value_at) = find_key(&data, key, reader.pos()) {
                let mut value_reader = ByteReader::at(&data, value_at);
                let value = value_reader.read_i32().unwrap_or(-1);
                if key == "skillLevel" {
                    total_levels += value.max(0);
                }
                let _ = write!(line, " {key}={value}");
                reader = ByteReader::at(&data, value_reader.pos());
            }
        }
        println!("{line}");
        skills += 1;
        from = offset;
    }
    println!("skills: {skills}, total skillLevel sum: {total_levels}");

    println!("-- hotbar --");
    for slot in 1..=5 {
        for key in [
            format!("primarySkill{slot}"),
            format!("secondarySkill{slot}"),
            format!("skillActive{slot}"),
        ] {
            if let Some(offset) = find_key(&data, &key, 0) {
                let mut reader = ByteReader::at(&data, offset);
                println!("0x{offset:06X} {key} = {:?}", reader.read_i32());
            }
        }
    }
}
