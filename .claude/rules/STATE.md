# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-24

## Active workstream

Greenfield start of tq-univault: a platform-independent
(Windows/macOS/Linux) reimplementation of TQVaultAE — the item-vault /
inventory-manager companion app for Titan Quest — in Rust with an
egui/eframe front-end. The rules layer and the Cargo workspace
scaffold are on `main`: `crates/univault-core` (empty, doc-only) and
`crates/univault-gui` (eframe 0.36 window rendering a heading), with
fmt / clippy-pedantic / build gates green. No format parsing exists
yet. Binding decisions from the 2026-08-24
bootstrap and survey dialogs (see ARCHITECTURE.md): Cargo workspace
with a GUI-agnostic core crate plus a thin egui crate; game-owned
files are authoritative, every write to them backup-first and
targeted-splice; native vault format is TQVaultAE's JSON schema
(legacy binary `.vault` import-only); parsers hand-rolled + flate2,
ported from MIT TQVaultAE (GPL references eyes-only — see
docs/format-references.md); dual-licensed MIT OR Apache-2.0; scope
is TQ Anniversary Edition + all expansions (original TQ 2006 out of
scope for now). No issue tracker is bound yet (deliberately
deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | transfer UI merged — the core feature loop is live; all gates green |

## Next up

Product priority set by the user (2026-08-24): the core feature is
saving items out of a character inventory or transfer stash into
external vault storage, and transferring/copying them back later.
Vault-format decision reconfirmed 2026-08-24: TQVaultAE-style vault
files; a local SQLite DB is a recorded **stretch idea** only, not a
constraint change. The entire read stack is done and validated
against the user's real install + save tree (mounted; path in agent
memory). Sequence:

1. User acceptance: run a real transfer (character/stash → vault →
   back) against a copy of the save tree and confirm in-game and in
   TQVaultAE. First real disk writes happen here.
2. Real item footprints: a TEX-header reader for `Resources/*.arc`
   bitmaps so placement stops using conservative upper bounds
   (denser packing); TQVaultAE derives sizes from texture pixels ÷
   cell size.
3. Grid rendering: draw sacks/tabs as actual grids with items at
   their cells (list view today), then drag-and-drop movement.
4. Platform module in core: per-OS discovery of the game install and
   save directories (removes the need for `--game`).
5. Stretch (unscheduled): local SQLite index across vaults for
   search — would be additive, vault files stay the source of truth.

## Most recent meaningful progress

- **2026-08-24 — Transfer UI: the core feature loop is live.**
  Two-pane GUI (game file ⇄ vault) with click-to-select item
  movement, dirty tracking, and explicit saves through the new
  backup-first shell writer (synced timestamped sibling backups;
  stash saves rewrite the `.dxg` twin via the ported
  `EncodeBackupFile`). Pure `transfer` module in core: take/place
  operations using TQVaultAE's column-major grid search with
  conservative per-class footprints (no overlap possible at true
  sizes; real footprints need a TEX reader — next). Legacy `.vault`
  opens import-only, saving as `.json`. Side request landed: gold is
  editable on the character pane (`replace_money`, a 4-byte
  in-place patch). 84 tests. Why: the product's reason to exist now
  functions end to end. Risk: no real disk write has happened yet —
  user acceptance on a save-tree copy is the next gate; placement
  is sparse until real footprints land. First acceptance attempt
  found and fixed a real-world snag: macOS `sync_all` (F_FULLFSYNC)
  is rejected by SMB mounts ("os error 45") — the writer now
  degrades to close-flush on "unsupported", validated by running the
  safe-write tests with TMPDIR on the user's network volume.
- **2026-08-24 — Splice write path lands, byte-identity proven on
  real files.** `writer` module (1252 encoding), `chr::
  replace_inventory` and `stash::replace_items` rebuild only the
  item regions and copy every other byte through; stash CRC ported
  (zero-init, no final complement, verified against real files).
  The model now preserves `tempBool` and per-stack-member
  `seed`/`var2` — the latter found by the byte-identity gate on the
  user's real save (Atlantis-era folded members carry their own
  `var2`; first attempt repeated the head's). Sweep: 4 real
  characters + 3 stashes (one with an item) resplice
  byte-identically. 68 tests. Why: transfers are now a data edit
  plus a file write away. Risk: no writes hit disk yet — the
  backup-first shell function lands with the transfer UI.
- **2026-08-24 — Read stack complete: stash reader + full real-data
  sweep.** All feature branches merged to `main` and deleted. New
  `stash` module (`winsys.dxb`/`.dxg`: CRC header skipped on read,
  explicit `stackCount` = size−1, float grid offsets) reusing the
  chr item parser via a new `Stash` context; GUI opens stashes by
  extension. `examples/smoke.rs` now sweeps a `SaveData` tree: the
  user's real transfer stash + 2 character stashes + 4 characters
  all parse with resolved names. 59 tests. Why: every item source
  (character, stash, vault, game DB) is now readable — the write
  path is all that separates us from working transfers. Risk: the
  real stashes were empty; a stash with items has only synthetic
  coverage until the user stashes something in-game.
- **2026-08-24 — Name resolution + GUI wiring; items have real
  names.** `gamedata` module combines ARZ + text: per-type name-tag
  dispatch ported from TQVaultAE's `Info.AssignVariableNames`
  (gear → `itemNameTag`, affixes → `lootRandomizerName`,
  relics/quest/artifacts → `description`), expansion-ordered text
  loading, color-code stripping (`{^l}`), affix+base+suffix
  assembly with stem fallback. GUI takes `--game <dir>`, loads the
  DB once, and precomputes display rows (no per-frame record
  decompression). Real-install smoke: 18,783 names resolved
  (up from 11,006 description-only), clean output. 53 tests. Why:
  the item-DB feature the user asked for is live end-to-end. Risk:
  quality/style tags ("Ancient", "Fine") not yet part of names;
  formula items show the generic formula name.
- **2026-08-24 — ARC + text readers; real-install validation
  passed.** `arc` module (part-table/directory/names parse, stored +
  multi-part zlib extraction) and `text` module (BOM-sniffed
  `tag=label` table with `TQVaultAE`'s gendered-label cleanup). The
  user mounted a real TQ AE install; `examples/smoke.rs` decompressed
  **all 74,013 records of database.arz with 0 errors**, extracted 57
  files from Text_EN.arc, loaded 17,540 tags, and resolved item
  names end-to-end ("Bow of Upis"). Why: the whole read stack is now
  proven against reality. Risk: `description` on quest items is
  flavor text, not a name — the naming layer needs per-type variable
  selection.
- **2026-08-24 — ARZ database reader (`feat/arz-reader`).** Core
  `arz` module: header/string-table/record-index parse, lazy per-
  record zlib decompression (`flate2` enters as planned), typed
  variables (int/float/string/bool incl. arrays), record lookup via
  TQVaultAE's normalization (uppercase, `/`→`\`). Port fixed a
  survey error: record-table entries are variable-length; 24 is the
  payload base offset. 31 tests green. Why: unblocks proper item
  names/descriptions and vault grid sizes. Risk: not yet run against
  a real `database.arz` — needs a game install; ARC text reader
  still missing before names show in the GUI.
- **2026-08-24 — Vault files land (`feat/vault-files`).** Core
  `vault` module: TQVaultAE JSON schema read/write (verbatim member
  names, `""` for empty ids, `var2Default` 2035248, unknown-field
  preservation for forward compat), legacy binary `.vault` import
  reusing the chr sacks-block parser, `serde`/`serde_json` adopted
  for the JSON boundary. GUI opens vaults by extension. 23 tests
  green. Why: first half of the core save-to-vault feature; wire
  schema verified against TQVaultAE sources (DTOs +
  `JsonSerializerOptions`). Risk: not yet validated against a vault
  file written by real TQVaultAE — ask the user for one.
- **2026-08-24 — Real-save validation passed; slice merged.** The
  user ran the GUI against their actual TQ AE save and it read the
  inventory correctly — the synthetic-fixture-only risk is retired.
  `feat/chr-read-slice` fast-forwarded into `main`. Product
  priority recorded: external vault storage with transfer-back is
  the core feature (see "Next up"). Risk: vault work adds the first
  serde dependency and the first write path — both flagged for
  design attention.
- **2026-08-24 — chr read slice implemented
  (`feat/chr-read-slice`).** `univault-core` gained `reader`
  (typed LE reader, Windows-1252/UTF-16 strings, key scan) and `chr`
  (sacks, stack folding, 12-slot equipment, header info), ported
  from TQVaultAE's providers; GUI loads a chr via arg or drag-drop
  and renders it read-only. 16 unit tests against a synthetic
  fixture in TQVaultAE's exact layout. Why: first end-to-end proof
  of the port-from-reference approach. Risk: validated only against
  the synthetic fixture — a real save may expose key-order or
  version quirks; eframe 0.36's `App::ui`/`DroppedFile` APIs differ
  from published examples.
- **2026-08-24 — Parser survey done; four decisions locked.** No
  Rust prior art exists for any TQ format; TQVaultAE (C#, MIT) is
  the port reference for all of them (map: docs/format-references.md).
  Dialog outcomes: TQVaultAE JSON as native vault schema (two-way
  compat — renegotiated from import-only), targeted-splice writes,
  flate2 + hand-rolled reader (binrw declined), MIT OR Apache-2.0.
  Why: unblocks the vertical slice with license-clean references.
  Risk: the vault schema is now an external contract tracked from a
  live upstream project.
## Blocked / waiting

- *(nothing)*

## Maintenance

- **Refresh trigger:** any merge or milestone that changes what an
  incoming agent needs to know: workstream shifts, a branch opens or
  closes, "Next up" changes, something lands. Wired into
  METHODOLOGIES.md's post-merge routine (the "refresh STATE.md"
  step).
- **Always update:** "Last updated"; "Branches in flight"; prepend a
  progress entry (what / why / risk voice — a judgment edit, not a
  paste of the PR description).
- **As applicable:** "Active workstream" paragraph, "Next up",
  "Blocked / waiting".
- **Trim policy:** progress log holds at most 10 entries — drop the
  oldest when adding. Anything stable graduates out of this file
  into the appropriate rules doc; this file stays small because
  every agent loads it every turn.
- **Edit policy:** STATE.md is authored on feature branches,
  propagates through merges, and is refreshed (not deleted) on new
  branches. Never edit it directly on `main`; docs-only diffs under
  `.claude/rules/` ride the METHODOLOGIES.md docs-only carve-out.
- **Keep entries short:** each progress entry is a pointer — date,
  PR #, ticket, a sentence or two of judgment. If you're tempted to
  write more, the detail belongs in the PR, commit, or ticket.
