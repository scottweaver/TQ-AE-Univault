# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-26

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #35 (star-hero pools ×3) merged 2026-08-26 at
the user's direction; wrap-up done. In-game acceptance still open:
meet a tripled base-game star hero after restarting the game
session. Ragnarök/Atlantis wild heroes spawn via champion-slot
pools the rule deliberately skips — extend when the user reaches
those acts. Now in flight: `feat/move-all-to-vault` — move/copy
every item from the active left tab into the vault in one click
(design settled with the user 2026-08-26: transfers start in the
open vault tab and spill into later tabs when it fills; inventory
scope is all sacks, never the equipped doll). Before that: PR #33 (all-vaults
search view) merged 2026-08-26; wrap-up done. The user already
steered the feature twice in-session (filter bar reworked to dynamic
criteria rows + ranges + suggestions, then a Clear-all button —
"Much nicer!"), so partial acceptance is in hand; the full in-app
pass is still open: "Search vaults…" (⌘F), stack criteria from the
suggestions, act on a row, double-click to jump. Older acceptance
still open, all in-game/in-app: paper doll (drag gear off/onto the doll, then load
the save in-game), auto-refresh feel on the SMB tree, dll patch
(Enable, socket an epic in-game), and the Gorgon rematch on the new
`LootPlusXMAX3Tuned1xBoss` bundle (both bundles installed in
CustomMaps; the original is back to 3x bosses). Otherwise pick from
"Next up".

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
| `main` | trunk | PRs #1–#35 all merged; all gates green |
| `feat/move-all-to-vault` | bulk move/copy of the active left tab into the vault | starting — design settled, code not yet begun |

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
- **2026-08-26 — Equipment paper doll (PR #29).** User ask: the
  character's worn gear was unreachable — no paper doll. The 12
  slots now render as an interactive doll (TQVaultAE geometry) on
  the Inventory tab, wired into every item operation: drag out to
  unequip anywhere, drag in to equip (cache-driven type rules —
  legal empty slots glow; any weapon/shield in any hand slot),
  right-click sends, Shift+Click duplicates, Alt+Click extracts,
  socket-into-worn-gear in place. Core: `chr::EquipSlot`,
  `chr::replace_equipment` (per-slot targeted splice — unchanged
  slots byte-identical incl. the garbage bytes real dummies carry;
  `itemAttached` mirrors the active weapon set) and
  `transfer::{take_equipped, can_equip, equip}`; save path splices
  inventory + equipment + money. Slot naming fixed everywhere: the
  wielded weapon is the *right* hand (real saves put two-handers at
  index 8; MCP's old labels were swapped). New `equipdry` example
  proved both real characters round-trip. 185 tests. Risk: in-app
  acceptance pending — unequip/equip on the doll, then load the
  save in-game.
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
- **2026-08-26 — Game.dll socket-gate patcher (toggle).** User
  supplied the community guide (Steam 2202151189): NOP the two
  conditional jumps after the Epic/Legendary classification
  compares. New core `dllpatch` module (pure bytes: inspect →
  Vanilla/Patched/Mixed/Unrecognized, enable/disable as
  non-overlapping 10-byte signature swaps, self-inverse) + a GUI
  "Socket patch…" header button opening a modal: state, warnings
  (multiplayer; Steam updates/verify replace the dll — re-enable
  after), Enable/Disable. Guardrails: pristine
  `Game.dll.univault-original` written once from a fully vanilla
  file; staging-write + rename; post-write re-read verify;
  unrecognized versions never written. ARCHITECTURE amended in-PR
  (single sanctioned game-binary write). Dry-run against the real
  dll (on a copy): 1 signature site in the current EE build (older
  guides say several — consolidated since), 4 bytes change,
  reverse byte-identical. Risk: the user pressing Enable and
  socketing an epic in-game is the acceptance test.
- **2026-08-26 — Auto-refresh: panes follow the files.** User ask
  (no more Reload button pressing), design-dialogued: prompt on
  conflict, silent reload when clean. A background thread polls the
  open character/banks/vault stamps every 2s (stat can hang on SMB —
  never on the UI thread); a change must hold across two polls
  before acting (never read the game's file mid-write — the relic
  bank truncation lesson); own autosaves are recognized by stamp and
  ignored; reloads defer while a drag/press/text-edit is live.
  Conflicts: an external change to a dirty pane — or an autosave
  about to land on an externally-changed file (every save now
  re-checks freshness first) — suspends autosave and prompts:
  reload-from-disk or keep-mine (keep-mine re-arms backup-first so
  the external bytes are backed up before being overwritten; the
  recorded exception to one-backup-per-load). ARCHITECTURE data-flow
  amended in-PR. The manual Reload button stays. 170 tests. Risk:
  feel unverified against the real SMB tree (poll cost, false
  settles) until the user runs it; a reload that fails mid-conflict
  leaves the pane clean-but-stale (same as manual Reload).
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
