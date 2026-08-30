# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-29 (PR #61 merged — Psionic Burn radius)

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #58 merged (Phantom Strike blink speed —
`characterRunSpeedModifier` 0→500); both mod bundles are rebuilt and
installed, so it needs only the user's in-game pass after a session
restart. If the blink still feels slow, the next suspect is
`playerRunSpeedCapMax` (166) clamping the 500 — that is a global
affecting all player run speed and needs its own decision, so ask
first. The interactive acceptance pass over the store is still the first
thing next session: **drag/drop into a bucket, bulk sends, ⌘F
search, an export opened in TQVaultAE**, the **▲/▼
sort-direction toggle** (store combo + search headers, PR #52), and
the **Skip duplicates box** (PR #54 — bulk sends only). None of
those have been clicked (synthetic input is banned here). PR #50's
own risk note still stands: it was merged on the user's explicit
call without that pass; a regression there is fixed forward or
reverted (`git revert da227aa`), not re-branched. First launch on
the user's real config will create `vault-store.json` and migrate
their vaults folder once, leaving the vault files untouched.
Older threads still open: the release-build pass over the component
chrome (PRs #43/#44), then the UIX queue. Remaining game-art chrome
surfaces (candidates for future components): nameplates, plate
buttons, grid cells, stone backdrop, tooltip frame. (The user's
gilded-border.png art update is committed to main, 2026-08-28
checkpoint.)

- **Open decision from the 2026-08-29 skilltree review:** whether to
  add a `notes` line to the exported skill-tree document explaining
  when `unlocks_at_mastery_level` is absent. Offered to the user,
  not yet answered. The field itself is correct — see the
  progress entry; do not re-audit it.
- **Known gap, unfixed:** `get_mastery` / `list_masteries` call
  `game_data()` directly, so MCP skill trees are always vanilla.
  Only the record tools (`get_record`, `search_records`, `diff_*`)
  go through `resolve_mod`, yet the server's own instructions claim
  the installed bundle is "overlaid by default". Harmless for unlock
  levels, but the user's LootPlus skill edits are invisible in
  mastery output. Listed under "Next up".
- User in-game checks outstanding (mod acceptance): **Phantom
  Strike blink speed (PR #58) — the freshest one**; pet Energy
  ×2.5 / regen ×1.75 on Core Dweller (875 / 5.25 at level
  20) and Call of the Wild wolves (255 / 3.5); vanilla XP restored
  (even-level trash mob on Normal ≈ level×15); target caps
  ×3 in dense packs; 2026-08-26 Core Dweller tune — Provoke
  5m radius, taunt-max floored at 12, Wildfire OA/movement debuffs
  3s. Older residual: open one of our vault JSONs in TQVaultAE.
- App checks worth a mention next session: autosave against the
  network mount (one `univault-bak` per file per session). The
  2026-08-25 relic-bank `.dxb` truncation was game-side; the twin
  fallback (PR #10) recovers it — the lost Hecate's Crescent
  shard can be re-duplicated in-app if wanted.
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
auto-discovered from the character path) ⇄ the unified item store
with its computed type tabs, with
drag-and-drop, right-click sends, copy/duplicate,
respec, Reload, gold editing, and autosaved backup-first splice
writes — user-accepted against a real network-mounted save tree.
The mod forge (arz composer + patch specs) tunes the user's live
LootPlus x3 mod. Binding decisions in ARCHITECTURE.md
(autosave + per-load backup-first + targeted-splice writes, one
unified store file with computed type buckets and TQVaultAE JSON as
import/export interchange,
hand-rolled parsers ported from MIT TQVaultAE — GPL refs eyes-only,
docs/format-references.md; MIT OR Apache-2.0; TQ AE + expansions
only). No issue tracker is bound yet (deliberately deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | PRs #1–#62 all merged; all gates green |
| `worktree-fix+projectile-speeds-ae` | a parallel session's cast-speed / DPS docs work | pushed, no PR; locked worktree under `.claude/worktrees/` — not the main session's to touch |

The remote is now just these two: every merged branch has been pruned
(2026-08-29). A local `docs/checkpoint-2026-08-29` worktree lingers
whose remote branch is already gone — remove it when convenient.

## Next up

Product priority set by the user (2026-08-24): the core feature is
saving items out of a character inventory or transfer stash into
external storage, and transferring/copying them back later. Storage
decision **renegotiated 2026-08-29** (see the progress entry): one
unified normalized store, own file format, TQVaultAE JSON as
import/export interchange. The read stack is done and validated
against the user's real install + save tree (mounted; path in agent
memory). Sequence:

User acceptance PASSED 2026-08-24: the user saved real transfer
edits through the app on their network-mounted save tree ("It
worked!"). Respec acceptance PASSED 2026-08-25: "I tested it and
it works great!" — both respec buttons verified in-game. Residual
check whenever convenient: confirm a transferred-back item in-game,
and open an exported vault in TQVaultAE (now an export, not the
live store).

Default vault + bank tabs + autosave ACCEPTED 2026-08-25 ("Ah,
that's better") and merged as PR #7.

0. **The store's interactive acceptance pass** (merged, unverified):
   drops into a bucket, bulk sends, ⌘F search, an export opened in
   TQVaultAE, the ▲/▼ sort-direction toggle (store combo + search
   headers, PR #52), and the Skip duplicates box (PR #54) — on the
   user's own config.
0b. **Component-ize remaining chrome surfaces as the user directs:**
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
3. MCP mastery tools honour the mod overlay: `get_mastery` and
   `list_masteries` call `game_data()` directly, so their skill
   trees are always vanilla while the record tools overlay the
   installed bundle. Either route them through `resolve_mod` or say
   plainly in the tool descriptions that mastery output is vanilla.
4. Stretch (unscheduled): an on-disk index over the store for
   search, if a linear scan ever stops being instant. The 2026-08-29
   dialog declined SQLite/redb for the store itself — at ~10⁴ items
   the whole store loads and filters in memory.

## Most recent meaningful progress

- **2026-08-29 — Mod: Psionic Burn radius 3.5 → 6.0 (PR #61,
  merged).** User
  ask, taken literally: `skillTargetRadius` set on the live record
  `records\xpack\skills\dream\psionictouch_psionicburn.dbr`
  (`SkillSecondary_AttackRadius`; the `OLD\` and `11-15-06\` copies
  share the display name but are dev leftovers). Both bundles
  rebuilt + installed, dump-verified at 6.0. Also asked in the same
  round: triple the max targets of Phantom Strike and Dream Stealer —
  **no change was needed and none was made.** The spec's blanket
  `multiply_player_skills` / `skillTargetNumber` ×3 rule already
  takes Dream Stealer from vanilla 3–8 to 9–24, and Phantom Strike
  has no `skillTargetNumber` at all (single-target blink; Dream
  Stealer supplies the 360° multi-hit). User's call after seeing the
  numbers: leave both alone.

- **2026-08-29 — Mod: Phantom Strike blinks at speed (PR #58,
  merged).**
  User ask: make the blink substantially faster, "feels like a
  teleport". The knob is `characterRunSpeedModifier` on
  `records\xpack\skills\dream\phantomstrike.dbr`, shipped at `0.0`;
  set to `500.0` (the user's pick — `absoluteRunSpeedCapMax`),
  per-skill only, no globals touched. Method and the caps/profile
  numbers are now in WORKING_NOTES.md ("Reading the game's record
  semantics") — the short version: a `.tpl` is the editor's schema,
  the engine reads variables by name, and the monster twin
  `HERO_PHANTOMSTRIKE.DBR` ships the same class at `300.0`. Both
  bundles rebuilt + installed, dump-verified. Awaiting the user's
  in-game pass. Risk: `500` exceeds
  `playerRunSpeedCapMax` (166), so the engine may clamp it — if the
  blink still feels slow in game, that global is the next suspect
  (raising it affects all player run speed, so it needs its own
  decision). **Process note:** an earlier attempt this session shipped
  `distanceProfile`/cooldown changes the user had not asked for; they
  were rejected, reverted, and PR #57 closed. Ask before substituting
  scope — especially when the artifact lands in `CustomMaps/`.

- **2026-08-29 — `unlocks_at_mastery_level` reviewed; correct as
  built (no PR).** User report: the field looked missing for a
  number of skills. Audited against the real install — it is absent
  exactly where the game data carries no `skillTier`, and every
  record that has one resolves onto the vanilla ladder
  (1/4/10/16/24/32/40). All 37 absences out of 372 skills are
  legitimate: 11 `Skill_Mastery` records, 11 `SkillTree` index
  records, 12 internal pet sub-skills absent from any SkillTree, and
  the 3 Neidan `DEATHBOMB_{COLD,FIRE,LIGHTNING}` payloads (listed in
  the tree record but `forceHideIconFromQuickSlot`, and their
  `x4tagDeathBomb` name tag does not exist in `Text_EN.arc`). Odd
  names such as `SUNDER.DBR` → "Storm Nimbus" are the game's own
  stale `tagSkillName027`, not our delegation chain. The user's
  LootPlus overlay was cleared too: both bundles keep
  `skillMasteryTierLevel` and all 89 tiered skill records they
  override. PR #40's derivation stands — do not re-audit. Two side
  findings recorded (mastery tools bypass the mod overlay; two
  installed bundles make `mod`-less record calls error). Risk: none,
  nothing changed.

- **2026-08-29 — Skip duplicates: a bulk-send filter on the item
  seed (PR #54, merged).** User ask: a checkbox that keeps a
  move/copy from landing an item already stored — explicitly *not* a
  uniqueness rule on the store, which may still hold duplicates that
  arrived by other routes. Design-dialogued: "item ID" means the
  **item seed** (the user's own clarification), matched within the
  item's type bucket, and the box gates **bulk sends only** (the
  user's pick) — single sends, right-click sends, and drops always
  land. Core gains `ItemIdentity` + `DuplicateGuard`, an accumulator
  so one batch can neither re-add what is stored nor duplicate
  within itself; the guard admits *before* `drain_or_clone` takes
  anything, so a skipped duplicate is never drained out of its sack
  and its save-file bytes stay untouched. An all-skipped send
  returns early without dirtying the source. 255 tests. Risk: the
  box sits in the store pane while the buttons it governs are on
  the left pane's headers, and nothing has been clicked yet.
- **2026-08-29 — Every sort reads both ways (PR #52, merged).**
  User ask, one line: all sorting operations should offer
  ascending/descending. Two surfaces existed — the search table
  could already flip via its headers (spelling direction as
  `ascending: bool`), while the store pane's combo was one-way and
  its rarity key faked descending by negating its own rank. Both
  now share a `SortDirection` (new gui `sort.rs`): the store gains a
  ▲/▼ plate toggle beside the combo, ranks are built ascending
  everywhere and oriented once at the comparison, and a freshly
  picked key opens in its natural direction (names/type A→Z, rarity
  and level best-first) so the store's old rarity view is preserved.
  Two divergent rarity ladders collapsed into
  `ItemStyle::rarity_rank` in core — "by rarity" had meant two
  different orders in one app. 248 tests; toggle rendering confirmed
  in the real chrome under a scratch `HOME`. Risk: no click has
  landed on either control (synthetic input banned), and rarity
  descending now floats artifacts/relics above legendaries in a
  bucket that mixes them.
- **2026-08-29 — Tabs become views onto a normalized store (PR #50,
  merged).** User ask: stop letting tabs be
  arbitrary buckets — dedicate each to an item type — and make
  storage "a true database of sorts" while keeping the app portable.
  Design-dialogued (all four picks the user's): one unified store
  file rather than named vaults; own format over redb/SQLite (a
  DBMS buys nothing at ~10⁴ items and SQLite is the native dep the
  rules gate); TQVaultAE JSON demoted from two-way wire contract to
  import **and** export interchange; family → sub-type tabs. New
  `core/store.rs`: a flat set of `{StoredItemId, Item}` in one
  versioned self-describing file, with buckets *computed* from each
  item's record (`bucket_of`), so misfiling is unrepresentable and
  buckets are unbounded. Addressing unified as `ItemAddr` (game
  containers positional, the store identity-addressed); "no room"
  and the whole bulk-spill path deleted from `transfer`; search
  collapsed onto the one store; MCP's `list_vaults`/`get_vault`
  became `list_buckets`/`get_store`. ARCHITECTURE.md renegotiated in
  the same PR. 241 tests, including an env-gated real-data
  migration check (the user's 295 items: all classified, lossless
  round trip, export repacked into 4 sacks with no overlap).
  Verified by launch against a scratch `HOME`; the family strip
  overflowed on that first run (Misc unreachable) and was fixed,
  as was the content-sized grid (now padded to 18×20, centered).
  Risk: merged on the user's call with **no interactive time** —
  drag/drop, bulk sends, ⌘F, and an export opened in TQVaultAE are
  unexercised on main; grid positions for vaulted items are gone by
  design (the store sorts, the user no longer arranges).
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
  into the appropriate rules doc — durable machine, tooling, and
  framework knowledge goes to WORKING_NOTES.md, binding constraints
  to ARCHITECTURE.md. This file stays small because every agent
  loads it every turn, and because the Session handoff section is
  replaced wholesale at each checkpoint: anything left there that
  still matters next month is in the wrong file.
- **Edit policy:** STATE.md is authored on feature branches,
  propagates through merges, and is refreshed (not deleted) on new
  branches. Never edit it directly on `main`; docs-only diffs under
  `.claude/rules/` ride the METHODOLOGIES.md docs-only carve-out.
- **Keep entries short:** each progress entry is a pointer — date,
  PR #, ticket, a sentence or two of judgment. If you're tempted to
  write more, the detail belongs in the PR, commit, or ticket.
