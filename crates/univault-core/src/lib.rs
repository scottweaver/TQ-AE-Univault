//! GUI-agnostic core for tq-univault: Titan Quest file formats
//! (saves, stash, ARC/ARZ, vault), the in-memory model, and platform
//! discovery. Pure and sync — IO and async belong to the shell
//! (see `.claude/rules/ARCHITECTURE.md`).

pub mod arc;
pub mod arz;
pub mod chr;
pub mod gamedata;
pub mod grid;
pub mod reader;
pub mod stash;
pub mod tex;
pub mod text;
pub mod transfer;
pub mod vault;
mod writer;
