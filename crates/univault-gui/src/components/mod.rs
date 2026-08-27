//! Self-contained visual components, each drawn from its own
//! bundled art under `assets/components/` and previewable in
//! isolation: `cargo run -p univault-gui --bin preview -- <name>`.
//! A component owns its slice geometry and texture upload; call
//! sites only hand it a `Ui` or a `Painter` plus a rect.

pub mod gilded_border;
