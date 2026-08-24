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
egui/eframe front-end. The repo currently contains only the
agent-rules layer; no code yet. Binding decisions from the 2026-08-24
bootstrap dialog (see ARCHITECTURE.md): Cargo workspace with a
GUI-agnostic core crate plus a thin egui crate; game-owned files are
authoritative and every write to them is backup-first; own native
vault format with one-way import of TQVaultAE vaults; scope is TQ
Anniversary Edition + all expansions (original TQ 2006 out of scope
for now). No issue tracker is bound yet (deliberately deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | rules layer only, no code yet |

## Next up

1. Scaffold the Cargo workspace (`crates/univault-core`,
   `crates/univault-gui`) with clippy/fmt gates and an empty eframe
   window that builds on all three platforms.
2. First vertical slice: parse a real TQ AE character save file in
   core and render its inventory read-only in the GUI.
3. ARZ/ARC readers so parsed items resolve display names and icons
   from game data.

## Most recent meaningful progress

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
