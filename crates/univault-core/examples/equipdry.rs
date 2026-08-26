//! Dry-runs the equipment splice against a real `Player.chr`
//! without writing anything: proves an unchanged resplice is
//! byte-identical, then simulates unequipping and re-equipping each
//! worn item and re-parses the result.
//!
//! Usage: `cargo run -p univault-core --example equipdry -- <Player.chr>`

use univault_core::chr::{self, EquipSlot};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: equipdry <Player.chr>");
    let data = std::fs::read(&path).expect("read save");
    let character = chr::parse_player(&data).expect("parse save");

    let unchanged = chr::replace_equipment(&data, &character.equipment).expect("resplice");
    assert!(
        unchanged == data,
        "unchanged equipment resplice altered bytes ({} -> {})",
        data.len(),
        unchanged.len()
    );
    println!("unchanged resplice: byte-identical ({} bytes)", data.len());

    for slot in EquipSlot::ALL {
        let Some(worn) = character.equipment.get(slot).cloned() else {
            println!("{:20} —", slot.label());
            continue;
        };
        let mut edited = character.equipment.clone();
        *edited.slot_mut(slot) = None;
        let removed = chr::replace_equipment(&data, &edited).expect("unequip splice");
        let reparsed = chr::parse_player(&removed).expect("reparse after unequip");
        assert!(reparsed.equipment.get(slot).is_none(), "slot not emptied");
        assert_eq!(reparsed.sacks, character.sacks, "inventory disturbed");

        *edited.slot_mut(slot) = Some(worn.clone());
        let restored = chr::replace_equipment(&removed, &edited).expect("re-equip splice");
        let reparsed = chr::parse_player(&restored).expect("reparse after re-equip");
        assert_eq!(
            reparsed.equipment.get(slot),
            Some(&worn),
            "re-equipped item did not round-trip"
        );
        println!(
            "{:20} {} — unequip + re-equip round-trips",
            slot.label(),
            worn.base.file_stem()
        );
    }
}
