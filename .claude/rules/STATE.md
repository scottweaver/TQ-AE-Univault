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
| `main` | trunk | chr read slice merged; all gates green |
| `feat/chr-read-slice` | chr parser + read-only inventory GUI | merged into `main`; local deletion pending user confirmation |

## Next up

Product priority set by the user (2026-08-24): the core feature is
saving items out of a character inventory or transfer stash into
external vault storage, and transferring/copying them back later.
Sequence toward that:

1. Vault model + JSON read/write in core — TQVaultAE `VaultDto`
   schema per ARCHITECTURE.md (plan: adopt `serde`/`serde_json` for
   this boundary; flag at implementation time).
2. Transfer stash (`winsys.dxb`) parsing — the other item source.
3. The write path: targeted-splice item-block re-encode for chr and
   stash plus the backup-first infrastructure, enabling actual
   vault ↔ character transfers.
4. ARZ/ARC readers for display names and icons (demoted below the
   transfer loop; this is where `flate2` enters).
5. Platform module in core: per-OS discovery of the game install and
   save directories.

## Most recent meaningful progress

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
- **2026-08-24 — Workspace scaffolded.** Cargo workspace with
  `univault-core` (lib, empty) and `univault-gui` (eframe 0.36
  window); pedantic clippy wired workspace-wide; fmt/clippy/build
  green. Why: locks the core/gui layering from the first line of
  code. Risk: cross-platform claim unverified — only macOS has
  actually built it so far.
- **2026-08-24 — Rules layer bootstrapped.** git init on `main`;
  installed RUST_BEST_PRACTICES.md + METHODOLOGIES.md; held the
  architecture dialog and wrote ARCHITECTURE.md, CLAUDE.md, and this
  file. Why: agents get durable memory and binding constraints from
  the first commit, before any code exists. Risk: constraints were
  set pre-code from TQVaultAE's observed behavior — revisit once the
  first parsers meet real save files.

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
