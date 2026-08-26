# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-25

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** nothing in flight — PRs #7 (default vault, bank
tabs, right-click sends, copy/duplicate, Reload, autosave) and #8
(pet Energy tuning in the mod spec) merged 2026-08-25 after user
acceptance ("Ah, that's better"); post-merge cleanup done (all
feature branches deleted local+remote, gates green on merged main:
155 tests, clippy clean). Pick the next item from "Next up".

- User in-game checks outstanding (mod acceptance): pet Energy
  ×2.5 / regen ×1.75 on Core Dweller (875 / 5.25 at level 20) and
  Call of the Wild wolves (255 / 3.5); vanilla XP restored
  (even-level trash mob on Normal ≈ level×15); target caps ×3 in
  dense packs. Older residual: open one of our vault JSONs in
  TQVaultAE.
- App checks worth a mention next session: autosave against the
  network mount (one `univault-bak` per file per session), TQVaultAE
  opening the default vault JSON. Relic-bank `.dxb` truncation
  (game-side partial write over SMB) hit 2026-08-25; recovered via
  the new twin fallback — the truncated write's newest relic (a
  Hecate's Crescent shard) was lost with it and can be re-duplicated
  in-app if wanted.
- PROJECT.md bootstrap still deferred (user re-confirmed 2026-08-25);
  answers saved in agent memory (bootstrap-project-deferred).

## Active workstream

tq-univault: a platform-independent (Windows/macOS/Linux)
reimplementation of TQVaultAE — the item-vault / inventory-manager
companion app for Titan Quest — in Rust (workspace: `univault-core`
pure logic, `univault-gui` egui/eframe shell). Working today: full
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
| `main` | trunk | PRs #1–#8 merged (latest: bank tabs + default vault + autosave #7, pet Energy mod tuning #8); all gates green |

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

1. Game-install auto-discovery (Steam library paths per OS in
   `platform`) to preseed the one-time Import dialog.
2. DXT1/3 decode for the ~7 compressed item bitmaps (currently an
   initial-letter fallback tile).
3. Stretch (unscheduled): local SQLite index across vaults for
   search — would be additive, vault files stay the source of truth.

## Most recent meaningful progress

- **2026-08-26 — Shard icons + in-app charm combining.** Two user
  asks: partial relics/charms now render the game's `shardBitmap`
  art (complete pieces keep `relicBitmap`), so partials read at a
  glance; and dragging a partial onto a matching partial pours
  shards into it (gold drop-highlight; the game's merge rule — the
  remainder stays in the source, nothing destroyed). Completing a
  piece opens a picker modal listing every completion bonus from
  the record's `bonusTableName` table with stats and odds — the
  user chooses (their call, over a game-faithful random roll).
  Cache format `UVC5` (shard icon + bonus tables per relic record;
  next launch re-imports, ~8s). Real-data gate: Boar's Hide level
  5, five bonuses w/ correct weights, distinct partial/complete
  pixels, "+4 Armor" line renders. Same-day follow-up (user
  request): the picker gained "Roll (game odds)" (weighted pick,
  wall-clock entropy — deliberately not a statistical RNG), and
  double-clicking any completed relic/charm re-opens the picker to
  change or remove its bonus (current one marked). 162 tests. Risk:
  combine gesture and modal visually unverified until the user
  drags real shards.
- **2026-08-25 — Stash `.dxg` twin fallback (bug fix).** The user's
  relic bank failed to reload: the game's save over SMB truncated
  `miscsys.dxb` mid-item (stored CRC didn't match the shortened
  bytes). The complete `.dxg` twin sat beside it — the game's own
  recovery path — so `stash::restore_from_twin` (inverse of
  `backup_twin`, shared `patched_name_copy` core) now backs
  `open_stash`: an unreadable `.dxb` loads from its twin, marked
  dirty so autosave writes the repaired file back through
  backup-first (the corrupt original becomes the backup). Proven
  against the real corrupt file: 30 relics recovered, resplice
  byte-identical. 156 tests. Risk: only the truncated write's
  newest item is unrecoverable (it existed nowhere but the cut
  bytes).
- **2026-08-25 — Default vault + bank/shared/relic panes +
  Shift+Click duplicate.** The user's prompt asks after real use: a
  vault file now exists without setup (`<config>/vaults/Main
  Vault.json`, created and auto-opened at launch; `Open vault…`
  still swaps in any other file); opening a `Player.chr`
  auto-discovers and loads its private bank (`winsys.dxb` beside
  it), the shared bank (`Sys/winsys.dxb`), and the relic bank
  (`Sys/miscsys.dxb`, Atlantis+) as grid sections in the left pane,
  each with its own Save (stash splice + `.dxg` twin,
  backup-first); right-click sends an item straight to the other
  pane (vault items land in the active left tab); Shift+Right-click
  sends a copy across (original stays); Shift+Click duplicates an
  item in place (same seed = exact copy, auto-placed, spilling to
  sibling sacks/tabs); a Reload button re-reads the character and
  all banks from disk (confirm modal when unsaved edits would be
  lost). Same-day UX rework (user request): the left pane became a
  tab strip — Inventory / Character bank / Shared bank / Relic bank
  — one document on screen at a time, absent documents greyed out
  with the reason on hover; cross-bank drags now route via the
  vault or right-click since only one left grid is visible. Then
  autosave (user request): all Save buttons removed — edits flush
  600ms after the interaction quiets (drag/pointer/text focus
  postpone), with a 5s retry on failure and a "Saving…" header
  note. Backup policy refined to one-backup-per-load so per-edit
  writes can't churn the 5-slot rotation; ARCHITECTURE.md's
  data-flow constraint renegotiated in the same change.
  Underneath: the left pane became independent documents
  (`CharacterPane` + three `StashPane`s), `GridId` gained
  `Bank`/`Shared`/`Relic`, selection is `(GridId, index)`
  everywhere, and re-discovery never silently reloads a dirty
  stash pane. 155 tests. Risk: layout and discovery unverified
  against the real network-mounted save tree until the user runs
  it; a tree whose Sys stashes live outside any `Sys/` ancestor
  reports "no shared/relic bank found"; duplication is a deliberate
  cheat feature — the game has no such operation.
- **2026-08-25 — Drag-and-drop item movement.** The top "Next up"
  item: items now drag between cells, sacks, panes, and vault tabs
  with a live drop preview (green = fits, red = blocked), the
  ghosted source dimmed and the item riding the cursor at true
  footprint scale. Core gained exact-position placement
  (`grid::fits`, `transfer::place_*_at` / `fits_at` / `occupancy`
  with a skip index so the dragged item's own cells read as free);
  the GUI's `grid_view` switched to `Sense::click_and_drag` (click
  select and hover tooltips unchanged — egui only reports drag
  after a movement threshold). Failed drops restore the item at its
  origin, falling back to auto-place. 150 tests. Risk: drop-feel
  (snapping, grab offset) unverified until the user drags for real;
  footprint lookups during hover are cached per record, so the old
  "uncached before drag-and-drop" worry is closed.
- **2026-08-25 — Mod forge: from reader to mod maker.** User pivot
  into modding, design-dialogued: `arz::compose` (writer half of the
  format; layout from the MIT `TQArchive-Wrapper` reference — 24B
  header, zlib payloads, record table w/ timestamps, string table)
  with parser upgrades it needed: record-table order + timestamps
  preserved, `DbRecord` variables now an ordered Vec (also fixes
  latent HashMap-iteration nondeterminism), `set_variable` edit op.
  ARCHITECTURE amended: mod bundles are a sanctioned output
  boundary; the game's own archives stay read-only. New `modforge`
  example compiles a JSON patch spec into a CustomMaps bundle
  merged onto a base mod (game loads one custom quest at a time —
  the user plays LootPlus XMAX x3): effective-record logic patches
  the base mod's version when it overrides vanilla (proven: x3's
  own Earth Enchantment radii got scaled, not vanilla's). First
  real mod built + installed: `LootPlusXMAX3Tuned` — all
  player-side `skillTargetNumber` ×3 (11 records; monster/boss/
  hero/quest-script skills and dev leftovers excluded) and Earth
  Enchantment aura radius ×3 (15→60m at ultimate). Patch spec
  committed at `mods/xmax3-tuned.json`. Composed db self-checks:
  full re-parse + every record decode-equal. 146 tests. Two
  follow-up rules same-day (user request): all summon cooldowns
  (`Skill_SpawnPet` × `skillCooldownTime`) filled to 0 — 28 skills
  incl. item/artifact summons — and the x3 mod's global 30% XP cut
  (`* 0.7` wrapper on gameengine.dbr `experienceEquation`, found
  with the new `moddiff` example) reverted to vanilla via a
  variable-level revert rule. In-game acceptance STARTED
  2026-08-25: the engine loads the composed database — zero summon
  cooldowns verified in play, which also clears the record-table
  ordering worry; Earth Enchantment radius verified (5m → 15m at
  one point — the game's own tooltip reads the merged record).
  Remaining spot checks: target caps ×3; vanilla XP (predicted:
  an even-level trash mob on Normal awards level×15).
  Follow-up 2026-08-25 (user request): Core Dweller Energy ×2.5 —
  `characterMana` on all 20 pet levels (327.5 → 875 at 20) and
  Energy regen ×1.75 (the x3 mod's 3.0/s → 5.25/s); same factors
  on Call of the Wild's 20 wolf records (energy 112.5 → 255,
  regen 2.0 → 3.5/s) — via a
  new list form of the modforge `record` rule; bundle rebuilt +
  reinstalled, installed arz verified; in-game check pending.

- **2026-08-25 — Mastery skill-tree export for AI theorycrafting.**
  PR #2 (rotation + respec) merged. New `skilltree` example distills
  `database.arz` into one JSON per mastery (all 11 discovered via
  `Skill_Mastery` records; dev leftovers — `OLD\`/`REV`/dated dirs,
  the cut Medicine mastery — filtered; skills swept flat per mastery
  dir): localized names/descriptions, tiers
  (`skillMasteryLevelRequired`), caps, per-level effect arrays, and
  transitively the referenced buffs/pets/sub-skills, with cosmetic
  variables (anim/sound/particle) stripped — 83–462 KB per file
  plus an index. Output goes to gitignored `exports/` (derived game
  data stays local per ARCHITECTURE's never-distribute posture);
  `arz::normalize` and `GameData::tag_text` went public for
  example use. Spot-checked against known values (Warfare mastery
  +2 str/level, Onslaught tier 1 max 8/12, Wolf_16 612 life).
- **2026-08-25 — Backup rotation + full respec.** Two user-requested
  features before drag-and-drop. Rotation: after each synced backup
  the oldest `univault-bak` siblings beyond 5 are pruned per file;
  new backup names floor above the newest existing so rotation
  (which ages by name) can never eat the newest. Respec: new core
  `respec` module — attribute reset (five `temp` floats after
  `skillPoints`, refund from deltas vs 50/50/50/300/300 at
  4/4/4/40/40 per point) and skill/mastery reset (skill-list splice
  keeping Default/AllMasteries/quest skills, `playerClassTag`
  cleared to fresh-character empty, hotbar slots of removed skills
  emptied to storedType -1, weapon-set selections zeroed, refund
  into `skillPoints`). Layout mapped from TQVaultAE's MIT
  TQSaveFilesExplorer + probes of real saves (new `chrprobe`
  example); `tqrespec` eyes-only. GUI: two buttons on the character
  pane behind a confirm modal showing the previewed refund; applies
  to the pane's baseline bytes, Save still explicit +
  backup-first. Read-only dry run (`respecdry` example) across all
  6 real saves: previews match hand-computed spend exactly (16 attr
  / 25 skill pts, 7 skills on Pally Don), reparse identical
  items/equipment/money, second pass refunds zero. 144 tests.
  Risk: GUI modal visually unverified; in-game acceptance (load a
  respecced save, re-pick masteries) still pending.

- **2026-08-25 — GitHub repo + CI + test-coverage push.** The user
  created the GitHub remote and `main` is pushed. New CI workflow:
  rustfmt, clippy `-D warnings` + tests across
  ubuntu/macos/windows (platform independence is now checked, not
  assumed), and a cargo-llvm-cov job publishing an lcov artifact.
  Coverage rose 70%→80% of lines: fixture tests now cover the stats
  engine's dark corners (granted skills incl. triggered levels, pet
  summons, augments, racial bonuses, global XOR chance groups,
  damage qualifiers, duration-scaled slow damage, socketed-relic and
  artifact bonus assembly, expansion origin), cache corrupt-file
  errors, dictionary/style branches, and the GUI's pure helpers.
  The origin test caught a real bug: DBR paths start with
  `records\`, so the XPACK check never matched and expansion lines
  never rendered. `Item::bare` joined core's public API. Risk:
  `stats/render.rs` sits at ~71% lines (formula reagents, buff
  redirects, scroll effects untested); GUI shell logic beyond the
  helpers is uncovered; CI is unproven until the first Actions run.
- **2026-08-25 — Full item statistics in tooltips: the attribute
  engine lands.** User-directed follow-up to the rarity tooltip: a
  full-fidelity port of TQVaultAE's display engine (~5,900 reference
  lines studied; `ItemAttributeProvider` dictionary,
  `ConvertOffenseAttributesToString`, requirements incl. the
  itemcost.dbr equation evaluator, granted skills/pets, sets,
  formulae, racial bonuses, global XOR chance groups). Architecture
  per design dialog: stats pre-render once at import into per-record
  line blocks in the cache (`UVC3`; 50→56 MB, 8s import);
  `stats::item_details` assembles per-item tooltips at display time
  (relic shard slots, max-merged requirements + equations with
  totalAttCount). Real-data gate: 80 items across the user's save
  tree render with zero unresolved tags — set lists, XOR chance
  groups, flavor text (via Info's per-kind `itemText`/`itemStyleTag`
  dispatch) all correct. New `tooltips`/`dump`/`dumptag` examples
  are the validation harness. 115 tests. Risk: granted-skill and
  socketed-relic sections are unit-tested but absent from the real
  saves swept; `attributeScalePercent` deliberately stays
  record-local (deviation noted in `stats::render` docs). Follow-up
  after the user hit two silent slow launches (each format bump
  forces a re-import): imports now run on a background thread with
  a startup progress bar (phase labels + record fraction via
  `build_cache_with_progress`); verified live against the real
  install — window responsive from launch, cache rewritten. Also
  fixed after the user spotted "shieldbucklerwood03a_01": TQ's
  `default\` template records store literal text where others store
  tags — name resolution now accepts literal descriptions (space =
  not a tag) and quality/style fall back to the raw word per
  TQVaultAE, giving "Light Pine Buckler Ornate of Strength"; magic
  bumped to `UVC4` (content bumps use the magic too) so caches
  rebuild themselves.
- **2026-08-25 — Rarity-colored item tooltips.** New core `style`
  module ports TQVaultAE's `Item.ItemStyle` decision order and the
  game's exact text-palette RGB values (MIT refs fetched from the
  repo — no local checkout). Hovering a grid item now shows a
  dark game-style tooltip: name in its rarity color, style caption,
  relic/charm piece progress (`completedRelicLevel` now cached),
  and socketed-relic names. Cache entries gained classification +
  kind — magic bumped `UVC1`→`UVC2`, so the next launch re-imports
  (game volume must be reachable once). Unknown records fall back
  to TQVaultAE's record-path heuristics, so items color sensibly
  even with no game data. 105 tests. Risk: tooltip visuals
  unverified until the user hovers a real grid; path fallback skips
  TQVaultAE's Eternal Embers special-case relic lists.
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
