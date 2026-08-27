//! Decodes one texture from an ARC archive to raw RGBA on disk.
//!
//! Usage: `texdump <archive.arc> <entry> <out.rgba>` — prints
//! `<width> <height>` on stdout.
use univault_core::arc::ArcFile;
use univault_core::tex;
fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(arc_path), Some(entry), Some(out)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: texdump <archive.arc> <entry> <out.rgba>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&arc_path).expect("read arc");
    let arc = ArcFile::parse(bytes).expect("parse arc");
    let data = arc
        .file(&entry)
        .expect("entry present")
        .expect("entry reads");
    let image = tex::decode(&data).expect("tex decodes");
    println!("{} {}", image.width, image.height);
    std::fs::write(&out, &image.pixels).expect("write rgba");
}
