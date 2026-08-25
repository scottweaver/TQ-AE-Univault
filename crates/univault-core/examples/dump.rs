//! Prints every variable of one database record — a debugging tool
//! for porting record semantics.
//!
//! Usage: `cargo run -p univault-core --example dump -- <database.arz> <record path>`

use univault_core::arz::{ArzFile, DbValues};
use univault_core::chr::RecordId;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(db_path), Some(record_path)) = (args.next(), args.next()) else {
        eprintln!("usage: dump <database.arz> <record path>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&db_path).expect("read database");
    let arz = ArzFile::parse(bytes).expect("parse database");
    let id = RecordId::parse(record_path).expect("record id");
    let Some(record) = arz.record(&id) else {
        eprintln!("record not found");
        std::process::exit(1);
    };
    let record = record.expect("record decompress");
    println!("class: {}", record.record_type);
    for variable in record.variables() {
        let rendered = match &variable.values {
            DbValues::Integers(values) => format!("{values:?}"),
            DbValues::Floats(values) => format!("{values:?}"),
            DbValues::Strings(values) => format!("{values:?}"),
            DbValues::Booleans(values) => format!("{values:?}"),
        };
        println!("{} = {}", variable.name, rendered);
    }
}
