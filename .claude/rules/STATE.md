# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-28 (PR #47 merged + wrapped up)

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #47 (filter overhaul: the all-vaults search
renders in the vault pane's place instead of swapping the whole
window) merged 2026-08-28 and is wrapped up. Awaiting the user's
in-app feel pass on the half-width search table (stats column
starts narrow; columns are draggable — offer to widen defaults or
drop the Rarity column if it feels cramped), plus the still-
outstanding release-build pass over the component chrome
(PRs #43/#44), then the UIX queue. Remaining game-art chrome
surfaces (candidates for future components): nameplates, plate
buttons, grid cells, stone backdrop, tooltip frame. Working-tree
note: `crates/univault-gui/assets/components/gilded-border.png`
is modified but uncommitted — the user's own in-flight art
change; leave it be.

- **egui 0.36 landmines, both user-hit this session:**
  (1) `ui.columns` children share a *stable* id — derive
  per-instance widget ids from an allocated response id, never
  `ui.id()` (tabbed_panel does this); (2)
  `Tooltip::for_widget(...).show()` is unconditionally open — use
  `Tooltip::for_enabled` or gate at the call site.
- **Review loop** (preview harness): launch `cargo run -p
  univault-gui --bin preview -- <component> --review`, keep a
  Monitor on gitignored `review/`, act on exports (annotated PNG
  + component-space JSON). Preview-only by user decision; promote
  into the app behind a debug flag only if an app-look round
  needs it.
- Capture loop (works, keep using it): Screen Recording is granted
  to iTerm; find the window id by pid via a CGWindowList swift
  one-liner, then `screencapture -x -l <id>` (`-o` drops the
  shadow for exact coordinate mapping). Fully occluded windows
  freeze egui repaints (imports look "stuck at 2%"). The user
  often runs their own release build — only ever
  `pkill -f target/debug/…`; each debug relaunch steals focus, so
  the user's in-flight clicks can land in the test instance.
  **Never post synthetic CGEvents into a live window**: this
  session's synthetic-input test interleaved with the user's real
  typing and garbled their annotation.
- Design sources: new-look component art is authored by the user
  in GIMP at `~/Documents/tq-desgins/` (XCF masters; PNG exports
  live beside them). GIMP batch export mostly hangs on this
  machine (script-fu and python-fu both; the save can land before
  the hang) — prefer asking the user to export. Game-art
  reference screenshots live in `/Volumes/scott-games/
  tq-ae-designs` (mount required).
- **Repo setting:** PR auto-merge is disabled
  (`enablePullRequestAutoMerge`) — wrap-up's docs PRs can't
  `gh pr merge --auto`; this session watched CI and merged
  directly. Consider enabling it: repo Settings → General →
  "Allow auto-merge".
- GitHub Actions event delivery was unreliable on 2026-08-26
  (major outage): push/PR webhook events silently dropped several
  times; delivery was prompt again by late 2026-08-26. The lever
  if it recurs: `gh workflow run CI --ref <branch>`
  (workflow_dispatch, added in PR #13), then
  `gh run watch <id> --exit-status`.
- A stale **Travis CI GitHub App** is installed on the repo and
  attaches permanently-queued phantom check suites to commits —
  uninstall advised (repo Settings → Integrations → GitHub Apps),
  not yet confirmed done.
- Cache format is `UVC8` (feat/tq-chrome: chrome textures added;
  import now also reads `InGameUI.arc` + `XPack/UI.arc`): the
  first app launch after pulling re-imports game data in the
  background — over the SMB mount this takes minutes, not the old
  ~8s; the panes work (fallback theme) throughout. Note: a fully
  occluded window stops repainting (normal macOS occlusion) — an
  import can look "stuck at 2%" until the window is uncovered.
- User in-game checks outstanding (mod acceptance): pet Energy
  ×2.5 / regen ×1.75 on Core Dweller (875 / 5.25 at level 20) and
  Call of the Wild wolves (255 / 3.5); vanilla XP restored
  (even-level trash mob on Normal ≈ level×15); target caps ×3 in
  dense packs; 2026-08-26 Core Dweller tune — Provoke 5m radius,
  taunt-max floored at 12, Wildfire OA/movement debuffs 3s. Older
  residual: open one of our vault JSONs in TQVaultAE.
- App checks worth a mention next session: autosave against the
  network mount (one `univault-bak` per file per session). The
  2026-08-25 relic-bank `.dxb` truncation was game-side; the twin
  fallback (PR #10) recovers it — the lost Hecate's Crescent shard
  can be re-duplicated in-app if wanted.
- PROJECT.md bootstrap still deferred (user re-confirmed
  2026-08-25); answers saved in agent memory
  (bootstrap-project-deferred).

## Active workstream

tq-univault: a platform-independent (Windows/macOS/Linux)
reimplementation of TQVaultAE — the item-vault / inventory-manager
companion app for Titan Quest — in Rust (workspace: `univault-core`
pure logic, `univault-gui` egui/eframe shell, `univault-mcp`
read-only MCP stdio server for AI agents). Working today: full
read stack (chr, stash, vault JSON + legacy import, ARZ/ARC/text/
textures) with localized names, real footprints, and icons; grid
rendering; left tab strip (inventory + character/shared/relic banks,
auto-discovered from the character path) ⇄ auto-created default
vault, with drag-and-drop, right-click sends, copy/duplicate,
respec, Reload, gold editing, and autosaved backup-first splice
writes — user-accepted against a real network-mounted save tree.
The mod forge (arz composer + patch specs) tunes the user's live
LootPlus x3 mod. Binding decisions in ARCHITECTURE.md
(autosave + per-load backup-first + targeted-splice writes,
TQVaultAE JSON vault schema,
hand-rolled parsers ported from MIT TQVaultAE — GPL refs eyes-only,
docs/format-references.md; MIT OR Apache-2.0; TQ AE + expansions
only). No issue tracker is bound yet (deliberately deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | PRs #1–#47 all merged; all gates green |

## Next up

Product priority set by the user (2026-08-24): the core feature is
saving items out of a character inventory or transfer stash into
external vault storage, and transferring/copying them back later.
Vault-format decision reconfirmed 2026-08-24: TQVaultAE-style vault
files; a local SQLite DB is a recorded **stretch idea** only, not a
constraint change. The entire read stack is done and validated
against the user's real install + save tree (mounted; path in agent
memory). Sequence:

User acceptance PASSED 2026-08-24: the user saved real transfer
edits through the app on their network-mounted save tree ("It
worked!"). Respec acceptance PASSED 2026-08-25: "I tested it and
it works great!" — both respec buttons verified in-game. Residual
check whenever convenient: confirm a transferred-back item in-game
and open our vault JSON in TQVaultAE.

Default vault + bank tabs + autosave ACCEPTED 2026-08-25 ("Ah,
that's better") and merged as PR #7.

0. **Component-ize remaining chrome surfaces as the user directs:**
   nameplates, plate buttons, grid cells, stone backdrop, tooltip
   frame — each through the preview + review loop first, from new
   art in `~/Documents/tq-desgins/`.

UIX queue (user-listed 2026-08-27; auto-open and per-sack bulk
buttons shipped in PR #44):

- Toolbar order: "Recent" directly right of "Open character…".
- Technical info (file paths etc.) hidden by default; a
  "?-in-a-circle" icon per pane reveals the low-level details —
  this also cures the ugly justified wrap of long paths under the
  nameplates.

1. Game-install auto-discovery (Steam library paths per OS in
   `platform`) to preseed the one-time Import dialog.
2. DXT1/3 decode for the ~7 compressed item bitmaps (currently an
   initial-letter fallback tile).
3. Stretch (unscheduled): local SQLite index across vaults for
   search — would be additive, vault files stay the source of truth.

## Most recent meaningful progress

- **2026-08-28 — Filter overhaul: search in the vault pane (PR #47,
  merged).** User ask: no more full-window swap — the all-vaults
  search now renders in the vault column (one "Search all vaults"
  plate) with the character/bank pane live beside it; ⌘F toggles,
  Esc / "← Vault" returns, and sends, bulk sends, and auto-refresh
  all work while filtering. The search UI's `&mut self` methods
  became split-borrow functions to render inside the columns
  closure; filter combos wrap into sized rows and the stats column
  clips for the half width (screenshot-verified live — the old
  wrapped row silently overflowed the pane). Risk: half-width
  table feel (stats column starts narrow; columns are draggable)
  awaits the user's in-app pass; full-window mode is gone by
  design.
- **2026-08-27 — Components into the app + live-fix round (PR #44,
  merged).** Both panes and the inventory sub-tabs now render
  through `TabbedPanel` (per-tab disabled hints, mid-drag hover
  switching, per-instance widget ids); the gilded border frames
  the window; the caravan/leather game chrome is deleted (net
  −600 lines). Five user-hit fixes in the same round: cross-pane
  tab id collision, the search-view tooltip storm (egui 0.36
  `Tooltip::for_widget` is unconditionally open), unreadable
  disabled plate buttons, `cargo run` default-run, plus UIX:
  auto-open last character, bulk sends moved into each sack's
  pane and scoped per-sack ("Move all → Vault"). Risk: accepted
  live in debug builds; the release-build full pass is still the
  user's to run, and the search view got only a spot check.
- **2026-08-27 — Component workshop + in-app review loop (PR #43,
  merged).** User pivot after the chrome epic: UI parts now grow as
  distinct components (gui lib target, `components/`), previewed in
  isolation (`preview` bin, checkerboard transparency proof) from
  the user's own GIMP art — first two: the gilded border
  (nine-patch, transparent interior) and the tabbed bronze panel
  (3-sliced plates, open plate merging through a rail frame made
  symmetric by flipping the art's good sides). The review overlay
  (armable shape tools, per-shape notes, badge edit/delete, export
  → annotated PNG + component-space JSON in `review/`) replaced
  GIMP markup and caught four real fixes this session. Risk: the
  components are preview-proven but not yet in the app; black
  panel interior is baked from the design — confirm it's wanted
  once composited over the app backdrop.
- **2026-08-27 — Chrome live-review marathon (PR #39, merged).** A
  dozen rounds of screenshot-annotated iteration with the user on
  top of the phase-2 base: two border vocabularies settled (ornate
  frame = pane outermost only; the 15px under-tabs leather strip =
  every tab-owned panel, active tab merging through — found after
  two wrong crops, one slicing the caravan art's own tab plates).
  Reactive layout (`fit_cell_size` by both axes, pane scrollbars
  removed, grids center, doll padded in and out), pointer-attached
  tooltips, sectioned Help modal replacing the intro wall-of-text,
  import-progress modal, vault Filter… button, exclusive inventory
  sub-tabs (UIX queue item retired), Recent as a chrome menu.
  Risk: the final strip fix merged without an in-app pass; the
  whole look now needs one fresh full review.
- **2026-08-27 — Mod: Sylvan Nymph joins the cooldown-free summons
  (PR #41, merged; parallel session).** Her summon is class
  `Skill_AttackProjectileSpawnPet` (thrown as a seed projectile),
  which the zero-cooldown fill rule missed; a new rule covers the
  projectile class (also zeroing Lay Trap, Blood of Ouranos,
  Terracotta Servants — all summons, per the mod's spirit). Both
  bundles recomposed + installed; nymph cooldown dump-verified 0.0.
  Risk: in-game acceptance after a session restart.
- **2026-08-27 — Skilltree: real mastery unlock levels (PR #40,
  merged; parallel session).** The skilltree surface reported
  vestigial unlock-level data; it now reports the real mastery
  unlock levels.
- **2026-08-27 — TQ AE look, phase 2: the game's own chrome
  (PR #39 base).** Design-dialogued (merge #38 first /
  all six surfaces / iron nameplates — all user-picked): the cache
  (UVC8) now carries ~20 UI textures pulled at import from
  `InGameUI.arc` + `XPack/UI.arc` (`CHROME_TEXTURES` manifest;
  whole textures stored, so slice tuning never re-imports); the new
  gui `chrome.rs` owns every slice coordinate and paints the
  caravan window frame (nine-patch), the game's 32×32 beveled grid
  cells (exact CELL_SIZE match, measured from the art), keyhole
  iron nameplates, leather tab plates, 3-state gold plate buttons,
  the borderitem tooltip frame, and a parchment doll backdrop —
  all falling back to the phase-1 painted theme when chrome is
  absent. 224 tests. Session lesson recorded in the cache bullet:
  fully-occluded windows freeze repaints, making imports look
  hung. Risk: look acceptance pending (first reaction positive);
  tooltip/search/modal surfaces still phase-1 styled.
- **2026-08-27 — TQ AE look, phase 1 (PR #38, merged).** User
  ask: the app is feature-rich but looks stock — retheme toward the
  game (reference screenshots in `/Volumes/scott-games/
  tq-ae-designs`). New `theme.rs`: bronze-and-gold palette over dark
  leather/olive surfaces (gold-rimmed plaque buttons, olive sack
  grids, near-black gold-ruled tooltips) and bundled OFL classical
  serifs — Cinzel (headings, gold) + Alegreya (body) under
  `assets/fonts/`; headings route through `theme::heading`. Decided
  in-dialog: phase 2 will pull the game's real border/parchment art
  through the cache for true TQ chrome — the egui restyle is the
  foundation, not the end state. 223 tests. Risk: look acceptance is
  wholly in the user's eyes; expect palette iteration.
- **2026-08-26 — Bulk send: whole tab → vault (PR #37, merged
  2026-08-27).** User ask: move or copy every item in the active game
  storage tab into the vault without per-item clicks. Design
  settled in-dialog: transfers start in the open vault tab and
  spill into the other tabs as each fills (the pre-existing
  `transfer::place_in_vault` cycle, which single sends deliberately
  stopped using in PR #31 — bulk is the sanctioned spill case);
  inventory means all sacks, never the doll. Core:
  `move_all_into_vault`/`copy_all_into_vault` over any item Vec plus
  `BulkOutcome` counts (placed / left-behind / spilled) — an
  unfittable item stays in place while the rest keep moving. GUI:
  "All → Vault" and "Copy all → Vault" buttons on the character and
  bank section headers; one toast summarizes the counts. 221 tests.
  Risk: in-app acceptance pending — header-button feel and toast
  wording; a full vault leaves the remainder behind by design.
- **2026-08-26 — Mod: star heroes actually triple (mod/hero-pools-x3,
  PR open).** User report: quest bosses 3x (Leucus) but star heroes
  never multiplied in the base game. Root cause via record dumps:
  the x3 base duplicates only each hero pool's top-two level
  variants — under the engine's level-band eligibility that adds
  zero heroes on Normal (bosses got every entry duplicated, hence
  visible). New `multiply_hero_pools` modforge rule: every vanilla
  `ProxyPool` under the proxy trees whose entries are hero-evidenced
  (explicit `monsterClassification: Hero`, or `HERO_*` stem when the
  variant omits it — most do) and carry no explicit Boss/Quest entry
  gets its vanilla entry list repeated ×3. 71 pools per bundle; both
  bundles recomposed + installed; dump-verified (Wheedletongue/Hanif
  9 entries; Euryale 6 in x3, 1 in 1xBoss; Thrym untouched). Risks:
  in-game acceptance after session restart; side-quest stars triple
  too (consistent with the mod's spirit); Ragnarök/Atlantis wild
  heroes use champion-slot pools — deliberately skipped, extend when
  the user reaches those acts.

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
