# Architecture constraints

Decided, binding architecture facts. STATE.md answers "what is in
flight"; this file answers "what must remain true." Every constraint
here was decided in a design dialog or by a hard external fact — a
change to any of them is a design decision requiring its own dialog
and a PR that updates this file in the same change (see
METHODOLOGIES.md "Refactors that change documented architecture").

The intuition: STATE.md is a working artifact you update aggressively;
this file is a contract you update deliberately.

Established 2026-08-24 (bootstrap dialog, pre-code — the repo was
empty; constraints derived from TQVaultAE's observed behavior and the
bootstrap Q&A).

## Purpose and scope

- tq-univault is a platform-independent reimplementation of
  TQVaultAE (https://github.com/EtienneLamoureux/TQVaultAE): an
  item-vault / inventory manager for Titan Quest. (2026-08-24)
- Game scope is Titan Quest Anniversary Edition plus all expansions
  (Ragnarök, Atlantis, Eternal Embers). The original 2006 release is
  out of scope until deliberately renegotiated here. (2026-08-24)
- This is **not** a security-conscious application: vault contents
  are game data, stored unencrypted. No crypto layer exists or is
  planned. (2026-08-24)

## Source of truth

- The game's own files — character saves and the transfer stash —
  are the authoritative store for character data. The game owns
  them; this app is a guest editor. (2026-08-24)
- Vault files in this app's own native format are the authoritative
  store for vaulted items. (2026-08-24)
- The game's ARC/ARZ archives (item database, textures, strings) are
  read-only reference data. This app never writes them. (2026-08-24)
- Nothing held in memory is authoritative: a mutation exists only
  once explicitly serialized to disk. (2026-08-24)

## Crate layering

- Cargo workspace with two members: `crates/univault-core` (file
  formats, vault logic, in-memory model — GUI-agnostic) and
  `crates/univault-gui` (egui/eframe front-end). (2026-08-24)
- Dependencies flow `univault-gui` → `univault-core`, never the
  reverse. Falsifiable check: `univault-core` compiles headless with
  no egui, eframe, or winit anywhere in its dependency tree.
  (2026-08-24)
- The GUI framework is egui/eframe; the core/gui split exists
  precisely so this remains swappable without touching core.
  (2026-08-24)

## Data flow

- Load: core parses game/vault files into a typed model. Edit: the
  UI mutates the model only. Save: core serializes and writes on an
  explicit user action. The GUI never reads or writes file bytes
  directly — all format knowledge lives in core. (2026-08-24)
- Every write to a game-owned file (save, stash) goes through a
  backup-first write path: the backup exists on disk before the
  original is touched. Mirrors TQVaultAE's `TQVaultData\Backup`
  behavior. This app mutates people's save files; this constraint is
  non-negotiable. (2026-08-24)

## External boundaries

- File formats are the only external boundaries: TQ save/stash
  format, ARC/ARZ archives, and this app's vault format. Each format
  is guarded by its own module in core with typed read/write
  surfaces. (2026-08-24)
- Vault interop: tq-univault defines its own native vault format and
  provides one-way **import** of TQVaultAE vaults (including their
  JSON export). Writing TQVaultAE's format is a non-goal.
  (2026-08-24)
- No network services, no telemetry, no online features.
  (2026-08-24)

## Platform independence

- Must build and run on Windows, macOS, and Linux. (2026-08-24)
- OS-specific logic (game-directory and save-directory discovery,
  path conventions) is confined to a single platform module in core;
  no `cfg(target_os)` sprawl elsewhere. (2026-08-24)
- Pure-Rust dependencies preferred; a native/C dependency needs a
  reason recorded here. (2026-08-24)

## Audit triggers

Files whose changes warrant re-checking this doc during post-merge
cleanup:

- `Cargo.toml` (workspace root) and `crates/*/Cargo.toml` —
  dependency-direction and native-dep constraints
- `crates/univault-core/src/formats/**` — boundary contracts for
  save/stash, ARC/ARZ, and vault formats
- `crates/univault-core/src/platform*` — the platform-confinement
  rule
- Any module implementing the save/write-back path — the
  backup-first rule
- `crates/univault-gui/src/main.rs` — entry point / framework choice

(Paths are the intended layout; correct them here in the same PR
that scaffolds the workspace if the real layout differs.)

## Structural criteria

Structural (this doc must change in the same PR): a GUI dependency
appearing in core or any reversal of the crate DAG; a new external
boundary (network access, a new file format, telemetry); a change to
who holds authoritative state; writing to ARC/ARZ or to TQVaultAE's
vault format; weakening or bypassing the backup-first write path;
replacing egui/eframe; dropping a supported platform; adding original
TQ 2006 support.

Not structural (no update needed): new UI panels or item operations
behind unchanged file boundaries; parser internals behind an
unchanged typed surface; test changes; new pure-Rust dependencies
that respect the layering.

## Maintenance

Update when a constraint above is deliberately renegotiated (design
dialog + PR updating this file), or when a recorded TBD is resolved.
Never for in-flight status — that's STATE.md. Keep constraints
falsifiable and dated. Secrets never enter this doc.
