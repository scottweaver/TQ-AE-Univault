//! Parses a cache file and reports its shape — a debugging tool for
//! cache-format changes.
//!
//! Usage: `cacheprobe <gamedata.cache> [chrome-key]`
use univault_core::cache::GameCache;
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("cache path");
    let bytes = std::fs::read(&path).expect("read cache");
    match GameCache::from_bytes(&bytes) {
        Ok(cache) => {
            println!("ok: {} entries", cache.len());
            for key in ["caravan/caravanwindow01.tex", "borderitemtl01.tex"] {
                match cache.chrome(key) {
                    Some(image) => println!("chrome {key}: {}x{}", image.width, image.height),
                    None => println!("chrome {key}: MISSING"),
                }
            }
        }
        Err(error) => println!("parse FAILED: {error}"),
    }
}
