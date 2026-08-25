//! Prints localization tags — a debugging tool for porting the
//! attribute-display format specs.
//!
//! Usage: `cargo run -p univault-core --example dumptag -- <Text_EN.arc> <tag>...`

use univault_core::arc::ArcFile;

fn main() {
    let mut args = std::env::args().skip(1);
    let arc_path = args.next().expect("usage: dumptag <Text_EN.arc> <tag>...");
    let bytes = std::fs::read(&arc_path).expect("read text archive");
    let arc = ArcFile::parse(bytes).expect("parse text archive");
    let mut text = univault_core::text::TextDb::new();
    let mut names: Vec<String> = arc
        .file_names()
        .filter(|name| name.to_lowercase().ends_with(".txt"))
        .map(str::to_string)
        .collect();
    names.sort();
    for name in names {
        if let Some(Ok(bytes)) = arc.file(&name) {
            text.add_file(&bytes);
        }
    }
    for tag in args {
        match text.get(&tag) {
            Some(value) => println!("{tag} = {value:?}"),
            None => println!("{tag} = <missing>"),
        }
    }
}
