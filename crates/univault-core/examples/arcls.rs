//! Lists every entry in an ARC archive.
use univault_core::arc::ArcFile;
fn main() {
    let path = std::env::args().nth(1).expect("arc path");
    let bytes = std::fs::read(&path).expect("read arc");
    let arc = ArcFile::parse(bytes).expect("parse arc");
    for name in arc.file_names() {
        println!("{name}");
    }
}
