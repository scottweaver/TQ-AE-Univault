//! GUI-agnostic core for tq-univault: Titan Quest file formats
//! (saves, stash, ARC/ARZ, vault), the in-memory model, and platform
//! discovery. Pure and sync — IO and async belong to the shell
//! (see `.claude/rules/ARCHITECTURE.md`).

pub mod arc;
pub mod arz;
pub mod cache;
pub mod chr;
pub mod dllpatch;
pub mod gamedata;
pub mod grid;
pub mod platform;
pub mod query;
pub mod reader;
pub mod respec;
pub mod skilltree;
pub mod stash;
pub mod stats;
pub mod store;
pub mod style;
pub mod tex;
pub mod text;
pub mod transfer;
pub mod vault;
mod writer;
