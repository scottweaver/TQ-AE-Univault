# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-27

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** everything merged 2026-08-27 evening at the
user's direction: PR #39 (the TQ chrome epic, ~20 commits of
live-review iteration), PR #40 (skilltree unlock levels, parallel
session), PR #41 (nymph cooldown mod, parallel session; its STATE
conflict was resolved to main's and superseded by this refresh).
No branches in flight. First items next session: the user rebuilds
release and gives the merged chrome a fresh full pass (the final
leather-strip border fix landed without an in-app review), then
pick from the UIX queue in "Next up". Border law, settled after
many annotated rounds: the ornate caravan frame belongs to a
pane's outermost edge ONLY; tab-owned panels wear the flat
under-tabs leather strip (art rows 108–122) on all four sides,
the active tab merging into it (`chrome::tab_anchor`). Method
lesson, twice-earned: measure game art programmatically AND view
every crop — single-column profiles lie. All slice coordinates
live in gui `chrome.rs`; tuning them never forces a re-import.

- Capture loop (works, keep using it): Screen Recording is granted
  to iTerm; find the window id by pid via a CGWindowList swift
  one-liner, then `screencapture -x -l <id>`. Fully occluded
  windows freeze egui repaints (imports look "stuck at 2%"). The
  user often runs their own release build — only ever
  `pkill -f target/debug/…`; each debug relaunch steals focus, so
  the user's in-flight clicks can land in the test instance.
- Reference screenshots from the user live in
  `/Volumes/scott-games/tq-ae-designs` (tabs.png,
  annotated-stone.png, in-game shots) — ask for annotated images
  when a look note is ambiguous; they resolved every stall.
- Parallel-session leftovers under `.claude/worktrees/`:
  `mod-nymph-cooldown` and `skilltree-unlock` (locked, their
  sessions may still be open) and `chrome-outer-frame` (stale at
  0bd7ff4) — clean up once those sessions are done.
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
| `main` | trunk | PRs #1–#41 all merged; all gates green |

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

UIX queue (user-listed 2026-08-27, to address after the theme work;
order not yet prioritized):

- On launch with no file argument, auto-open the last viewed
  character (recents already persist; open the newest).
- Toolbar order: "Recent" directly right of "Open character…".
- Technical info (file paths etc.) hidden by default; a
  "?-in-a-circle" icon per pane reveals the low-level details.

1. Game-install auto-discovery (Steam library paths per OS in
   `platform`) to preseed the one-time Import dialog.
2. DXT1/3 decode for the ~7 compressed item bitmaps (currently an
   initial-letter fallback tile).
3. Stretch (unscheduled): local SQLite index across vaults for
   search — would be additive, vault files stay the source of truth.

## Most recent meaningful progress

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
- **2026-08-26 — All-vaults search view (PR #33).** User ask,
  design-dialogued (full-window mode / all vault
  files / full row actions): "Search vaults…" (⌘F) swaps the panes
  for one filtered, sortable table over every vault file — icon,
  rarity-colored name, requirement, vault, full colored stat lines.
  Filters: name, set, wearable-at req caps, rarity, type, socketed,
  expansion, per-vault — plus dynamic stat/affix criteria rows
  (add/remove; any-stat / affix-stat / affix-name; min–max value
  windows; autocomplete fed by the vaults' own stat templates and
  affix names — reworked mid-PR after user feedback that single
  fixed fields couldn't express "pierce res AND poison res AND burn
  damage"). Core got `query` (Filter conjunction over the cache;
  `ValueBounds` windows; `stat_template` vocabulary; typed
  `stats::item_requirements`; `item_name` promoted from MCP);
  non-open vaults load as docs riding the existing autosave/
  refresh/conflict rails (`DocId::SearchVault`), and rows are
  gesture targets via `GridId::SearchDoc` (send/duplicate/copy/
  extract); double-click adopts the vault into the pane by model
  handoff — no disk round-trip. A Clear-all button resets every
  filter (user ask; "Much nicer!" on the criteria rework). 217
  tests. Risk: table feel (row heights, gestures, autocomplete
  popup) unverified until the user runs the full in-app pass.
- **2026-08-26 — Status messages became toasts (UX fix).** User
  report: the header status line reflowed the panes on every action
  ("jittering" while moving items), and "Saving…" flickered its own
  line. Outcomes now surface as bottom-right toasts — an egui
  `Area` overlay, `interactable(false)` so it is click-through,
  auto-expiring (4s, errors 8s, stack capped at 6) — and "Saving…"
  rides the fixed-height zoom row. The `status` channel is
  unchanged (every set-site untouched); it drains into the toast
  stack at frame end. Hand-rolled (~70 lines) over `egui-notify`
  deliberately: its toasts are clickable widgets, which would
  obstruct drops underneath. Risk: in-app feel — drop an item and
  watch nothing move but the toast.
- **2026-08-26 — Vault sends land where you look (bug fix).** User
  report: right-click sent items to the earliest vault tab with
  room, not the one on screen — and the stacked collapsible tab
  list let many tabs look "open" at once. The vault pane is now a
  tab strip like the left pane: exactly one tab visible
  (`VaultPane::open_tab`, kept across auto-refresh reloads), and
  every send/copy/duplicate/extract lands in that tab via the new
  `transfer::place_in_vault_tab` — a full tab reports "no free
  space" instead of silently spilling elsewhere. Mid-drag, pointing
  at a tab button switches to it, so drag-into-any-tab still works.
  192 tests. Risk: in-app acceptance pending — right-click a bank
  item with tab 3 open and watch it land there.
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
