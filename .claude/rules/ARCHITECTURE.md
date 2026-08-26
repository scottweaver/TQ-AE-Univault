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
- Mod bundles are a sanctioned **output** boundary: `arz::compose`
  serializes this app's own mod databases into new CustomMaps
  bundles (optionally merged onto a base mod's records). The game's
  databases and third-party mod files are never modified — a
  composed bundle is always a new folder, deletable without trace.
  (2026-08-25, mod-forge design dialog)
- Nothing held in memory is authoritative: a mutation exists only
  once explicitly serialized to disk. (2026-08-24)
- A derived local cache of item reference data (names, footprints,
  icons) lives under the platform config directory: regenerable at
  any time, fingerprint-keyed to the archives it was built from,
  never authoritative, and never distributed (it embeds extracted
  game assets for local personal use only). (2026-08-24)

## Crate layering

- Cargo workspace with three members: `crates/univault-core` (file
  formats, vault logic, in-memory model — GUI-agnostic),
  `crates/univault-gui` (egui/eframe front-end), and
  `crates/univault-mcp` (read-only MCP server shell). (2026-08-24,
  third member added 2026-08-26, MCP design dialog)
- Dependencies flow shell → core (`univault-gui` → `univault-core`,
  `univault-mcp` → `univault-core`), never the reverse, and never
  shell → shell. Falsifiable check: `univault-core` compiles
  headless with no egui, eframe, winit, rmcp, or tokio anywhere in
  its dependency tree. (2026-08-24, extended 2026-08-26)
- The GUI framework is egui/eframe; the core/gui split exists
  precisely so this remains swappable without touching core.
  (2026-08-24)
- Async is confined to shell crates; core stays sync and pure.
  `univault-mcp` carries tokio because its SDK demands it — that is
  a shell concern, not license for async in core. (2026-08-26)

## Data flow

- Load: core parses game/vault files into a typed model. Edit: the
  UI mutates the model only. Save: core serializes and the shell
  writes **automatically** once an edit has been quiet briefly
  (autosave); there are no manual save buttons. All format knowledge
  still lives in core — the shell only decides *when* to write.
  Renegotiated 2026-08-25 (user request: multiple per-pane save
  buttons were error-prone) from the original explicit-save rule.
  (2026-08-25)
- Every write to a game-owned file (save, stash) goes through a
  backup-first write path: a backup of the file exists on disk
  before the original is touched. Under autosave this is **one
  backup per load**: the first write since the file was last loaded
  takes the backup; subsequent autosaves of the same loaded baseline
  reuse it, so per-edit writes cannot churn the rotation into
  discarding the pre-session state. (Re)loading a file — including
  via Reload or auto-refresh — re-arms the backup. Mirrors
  TQVaultAE's `TQVaultData\Backup` behavior. This app mutates
  people's save files; this constraint is non-negotiable.
  (2026-08-24, autosave refinement 2026-08-25)
- The shell watches the open files (character, banks, vault) by
  polling and keeps panes current: an external change reloads a
  clean pane automatically, but only after the file's stamp holds
  stable across two polls (never read a file mid-write). **The app
  never knowingly overwrites an externally-changed file without the
  user choosing to**: every save first re-checks the file against
  the stamp taken at load/last write, and a mismatch — or an
  external change to a dirty pane — suspends autosave and prompts.
  Choosing "keep mine" re-arms backup-first so the external version
  is backed up before being overwritten; a deliberate exception to
  one-backup-per-load, since those bytes never existed in this
  session. (2026-08-26, auto-refresh design dialog)
- Writes to game-owned files are targeted splices: parsing locates
  the blocks being edited and only those bytes change; every other
  byte is copied through untouched. Full-file re-serialization of a
  game-owned file is prohibited — TQVaultAE's proven approach, and
  the safest posture for a guest editor. (2026-08-24)

## External boundaries

- External boundaries are the file formats — TQ save/stash format,
  ARC/ARZ archives, and this app's vault format, each guarded by its
  own module in core with typed read/write surfaces — plus the
  read-only MCP stdio surface recorded under "Source of truth".
  (2026-08-24, MCP added 2026-08-26)
- The MCP surface (`univault-mcp`) is a sanctioned **read-only**
  boundary: an MCP server speaking JSON-RPC over **stdio only** — the
  client spawns it as a child process; no listening sockets, ever.
  It exposes game data (characters, banks, vaults, skill trees, item
  stats) to AI agents and never writes any file. Adding write tools
  or a network transport (HTTP/SSE) is a structural change requiring
  its own design dialog. The "no network services" constraint below
  stands — stdio IPC is not a network service. (2026-08-26, MCP
  design dialog)
- The native vault format is TQVaultAE's JSON vault schema — its
  `VaultDto`/`SackDto`/`ItemDto` field names are the wire contract —
  giving full two-way compatibility so both tools can open the same
  vaults. Legacy binary `.vault` files are import-only. Renegotiated
  2026-08-24 (from "own format + one-way import") after the parser
  survey found modern TQVaultAE vaults are plain JSON. (2026-08-24)
- No network services, no telemetry, no online features.
  (2026-08-24)

## Platform independence

- Must build and run on Windows, macOS, and Linux. (2026-08-24)
- OS-specific logic (game-directory and save-directory discovery,
  path conventions) is confined to a single platform module in core;
  no `cfg(target_os)` sprawl elsewhere. (2026-08-24)
- Pure-Rust dependencies preferred; a native/C dependency needs a
  reason recorded here. (2026-08-24)

## Parser provenance and dependencies

- Core's parsing foundation is a hand-rolled typed little-endian
  reader plus `flate2` for zlib. No parser-derive proc-macros
  (binrw evaluated and declined in the 2026-08-24 survey dialog).
  (2026-08-24)
- The porting reference for every format is TQVaultAE (MIT); ported
  code preserves its MIT attribution. GPL-3.0 references (tqrespec,
  tqdatabase) are eyes-only for edge cases — their code is never
  transcribed. Full reference map: `docs/format-references.md`.
  (2026-08-24)
- The project is dual-licensed MIT OR Apache-2.0. (2026-08-24)

## Audit triggers

Files whose changes warrant re-checking this doc during post-merge
cleanup:

- `Cargo.toml` (workspace root) and `crates/*/Cargo.toml` —
  dependency-direction and native-dep constraints
- `crates/univault-core/src/*.rs` format modules (`reader`, `chr`,
  and future `stash`/`vault`/`arz`/`arc`/`platform`) — boundary
  contracts and the platform-confinement rule
- Any module implementing the save/write-back path — the
  backup-first and targeted-splice rules
- `crates/univault-gui/src/main.rs` — entry point / framework choice
- `crates/univault-mcp/src/*.rs` — the read-only and stdio-only MCP
  constraints

## Structural criteria

Structural (this doc must change in the same PR): a GUI dependency
appearing in core or any reversal of the crate DAG; a new external
boundary (network access, a new file format, telemetry); a change to
who holds authoritative state; writing to the game's own ARC/ARZ
files (composing new mod bundles is the sanctioned exception,
recorded above); a change to the
vault JSON schema contract; weakening or bypassing the backup-first
or targeted-splice write rules; adding write tools or a network
transport to the MCP surface; adopting a parser-derive dependency;
a license change; replacing egui/eframe; dropping a supported
platform; adding original TQ 2006 support.

Not structural (no update needed): new UI panels or item operations
behind unchanged file boundaries; parser internals behind an
unchanged typed surface; test changes; new pure-Rust dependencies
that respect the layering.

## Maintenance

Update when a constraint above is deliberately renegotiated (design
dialog + PR updating this file), or when a recorded TBD is resolved.
Never for in-flight status — that's STATE.md. Keep constraints
falsifiable and dated. Secrets never enter this doc.
