# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-26

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #22 (MCP full-database + mod overlays, 16
tools) merged 2026-08-26, wrap-up done, release binary rebuilt from
main (Claude Desktop/Code serve the new tools after a restart).
Still open: **PR #21 (GUI auto-refresh + conflict prompts)** —
awaiting the user's in-app acceptance run (play, save in-game,
watch panes update; conflict modal on simultaneous edits). Its
branch `feat/auto-refresh` will conflict with main's STATE.md on
merge — keep both progress entries, re-trim to 10. Otherwise pick
from "Next up".

- GitHub Actions event delivery was unreliable all 2026-08-26
  (major outage + slow recovery): push/PR webhook events silently
  dropped several times. The lever: `gh workflow run CI --ref
  <branch>` (workflow_dispatch, added in PR #13), then
  `gh run watch <id> --exit-status`.
- A stale **Travis CI GitHub App** is installed on the repo and
  attaches permanently-queued phantom check suites to commits —
  uninstall advised (repo Settings → Integrations → GitHub Apps),
  not yet confirmed done.
- Cache format is `UVC6` (PRs #12/#14): the first app launch after
  pulling re-imports game data (~8s, background; game volume must
  be mounted). The user's acceptance suggests this already ran.
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
| `main` | trunk | PRs #1–#20, #22 merged (latest: MCP full database + mod overlays #22); all gates green |
| `feat/auto-refresh` | GUI auto-refresh + conflict prompts (PR #21) | CI green; awaiting user in-app acceptance |

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

- **2026-08-26 — Mod: a second bundle with single bosses.** User
  hit an unwinnable tripled Gorgon-sisters fight (three queens
  cross-healing via Regrowth), then refined the ask: keep the
  original 3x-boss mod AND a 1x-boss variant, side by side. Root
  cause mapped via the MCP record tools: the x3 base multiplies
  named boss/hero *pools* (extra entries, limits stripped) on top
  of the global 300% spawn modifier; the author's own x3x1 base
  keeps trash x3 but collapses those pools to one spawn. Now one
  rules file builds both: `modforge` gained a bundle-name override,
  and `mods/xmax3-tuned.json` documents the two builds —
  `LootPlusXMAX3Tuned` (base x3, unchanged behavior) and
  `LootPlusXMAX3Tuned1xBoss` (base x3x1). Both composed,
  dump-verified (Euryale pool 6 entries vs 1; identical tunes:
  vanilla XP, Provoke 5m, spawnModifier 300), and installed to
  CustomMaps. Risk: user switches to the 1xBoss mod in-game and
  replays the Gorgon fight — that is the acceptance test; the x3x1
  base also singles named heroes.
- **2026-08-26 — Socket into any rarity (type rules kept).** User
  ask, refined: relics/charms socket into epics, legendaries, and
  set pieces in-app — the game's *type* rules stay (a ring relic
  fits only rings), only the rarity gate is lifted (that gate is
  Game.dll code, confirmed via the Enchanting Unlimited findings).
  Cache `UVC7`: entries now carry each gear record's equipment
  family (15 classes) and each relic's allow-flag bitmask (the 15
  `helmet`/`bodyArmor`/…/`rangedOneHand` template booleans), so the
  GUI enforces type rules from the cache alone — next launch
  re-imports (~8s). New `transfer::can_socket`/`socket_relic`;
  drag a standalone relic/charm onto allowed gear (violet
  highlight, vs gold for combining) and it sockets: record, shard
  count, bonus. Pairs with Alt+Click extraction for full
  socket/unsocket freedom. Tests cover type-rule enforcement,
  rarity indifference, and zero-encoded counts. Risk: in-game load
  of a relic-on-epic item forged here is the acceptance test; a
  Game.dll patcher (guide-based, backup + toggle) is the agreed
  next feature.
- **2026-08-26 — Shard encoding fix + Enchanter-free extraction.**
  User-reported bug: fresh single-shard drops refused to combine —
  the game encodes one shard as `var1 = 0`, a rule the stats
  renderer already carried but `can_combine` did not. New
  `transfer::shard_count` (= `var1.max(1)`) is the one home for the
  encoding; combine works on effective counts, display (GUI + MCP)
  now shows 1/N like the game, and completed pieces are no longer
  valid pour sources (PR #24, diagnosed from live data via the MCP
  record tools). Then the user ask "keep both at the Enchanter":
  the database interrogation proved the destroy-one-side rule is
  engine code (two fixed template slots, no data hook — only
  `enchanterRecoveryFactor` scales cost), so the app does it
  instead: Alt+Click an item with a socketed relic/charm extracts
  the piece — shard count and bonus preserved, Atlantis second
  socket handled (its var2 stores a completed-sentinel, clamped),
  piece auto-placed, gear committed only after the piece has a
  home. 175 tests. Risk: in-app acceptance pending; gesture is
  Alt+Click (documented in the header hint).
- **2026-08-26 — MCP: the whole database, mods included.** User
  ask ("expose ALL internal game file data, monster info, my mod
  changes"): six new tools — search_records (path or localized-name
  search over every record, class filter, lazy full-decode index),
  get_record (any record, template-default noise omitted unless
  everything: true, tags translated inline), list_mods,
  diff_record / diff_mod (vanilla vs bundle, per-variable), and
  translate_tag. The installed CustomMaps bundle (auto-discovered
  beside the save tree, UNIVAULT_CUSTOMMAPS override) overlays
  every record tool by default — "what the game actually plays" —
  with mod: "vanilla" opting out and provenance on every response.
  Verified live: 58 Ratman monsters with names and per-difficulty
  arrays; Provoke reads radius 5.0 as mod-override vs 3.0 vanilla;
  diff_mod sweeps 8,866 bundle records (290 added) in ~2s. 173
  tests. Risk: none new — read-only reads of already-sanctioned
  formats; user acceptance from their buildcrafting project
  pending.
- **2026-08-26 — MCP server: game data for AI agents.** User ask
  ("a true MCP server, not just exports"), design-dialogued:
  read-only v1, official `rmcp` SDK, stdio-only — ARCHITECTURE
  amended in the same PR (third workspace member `univault-mcp`, a
  sanctioned read-only MCP boundary; write tools or network
  transports need a new dialog). Ten tools: overview,
  list/get_character (via new core `respec::progression` read API —
  attributes, unspent pools, per-skill levels), get_bank
  (personal/shared/relic, `.dxg` twin fallback), list/get_vault,
  search_items across every possession with location provenance,
  get_item_details (tooltip blocks), list/get_mastery (skilltree
  distillation promoted from the example into `core::skilltree`).
  Paths from the GUI's config (recent-files → save roots,
  game-dir.txt, vaults/) with `UNIVAULT_*` env overrides; `.mcp.json`
  registers it for Claude Code. Verified over real JSON-RPC against
  the live tree: builds/equipment resolve, 45 relic-bank items,
  Hecate hits across bank + vault, Earth tree 34 skills. 167 tests.
  ACCEPTED 2026-08-26: driven from the user's "Titan Quest AE
  Buildcrafting" Claude project — "able to use just fine".
- **2026-08-26 — Core Dweller Provoke/Wildfire tune.** Three user
  asks via two new `record` rules in `mods/xmax3-tuned.json`:
  Provoke `skillTargetRadius` 3 → 5m (user confirmed 5m total)
  and `offensiveTauntMax` floored at 12 (user identified the
  variable; levels already above 12 keep their higher values);
  Wildfire's OA and movement-slow debuff durations 1s → 3s (burn
  duration untouched per the user's clarification). Bundle
  rebuilt + reinstalled, installed arz dump-verified. Risk:
  in-game check pending.
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
  change or remove its bonus (current one marked); artifacts too —
  their bonus table lives on the formula record
  (`artifactBonusTableName`), attached to the artifact entry at
  import (`UVC6`; probe: Thunderfist 7 bonuses, "of Annihilation"
  +25% Physical Damage). 162 tests. ACCEPTED
  2026-08-26: "everything seems to be working" — shard art,
  combining, picker, roll, re-pick, and artifacts verified in use.
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
