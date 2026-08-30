# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-30 (PR #75 merged and pruned — socketing now
announces itself)

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #75 **merged to main (477cd0e) on the user's
"merge it"**, branch pruned local and remote, all five CI checks
green. The socket-discoverability round — **the user asked to "add
the ability to slot/unslot charms and relics from gear" and the
answer was that it already shipped** (drag a piece onto gear to
socket; Alt+Click gear to extract, PRs #25/#26), invisibly. Their
call after seeing that: keep every gesture exactly as it is, make it
announce itself. So: a tooltip footer listing every relic/charm
gesture the hovered item accepts, and one relic-orange pip per
filled socket at a tile's lower-left. Core gained `Affordance` +
`affordances()` and a typed `BonusPick` (which also de-duplicates
the double-click gate that `request_bonus_edit` had inlined). 268
tests, clippy clean; the footer and pips were **seen live** under a
scratch `HOME` — the tooltip by temporarily forcing `hovered`
(synthetic input stays banned), a scratch patch reverted before
commit. Merged **unclicked**, like #69 before it: what the user's
own pass from `main` decides is whether the pip is findable without
being loud, and whether the footer earns its two lines on partial
pieces. Regressions are fixed forward.
**Deliberately not built, and worth a decision:** the second
(Atlantis) socket can still only be *emptied*, never filled —
`can_socket` refuses any target whose first socket is full. A scan
of both characters and all banks found 15 socketed items and **zero**
using `relicName2`, so nothing in the user's save exercises it and
the game's acceptance of an app-made two-relic item is unverified.
Also unbuilt by their choice: any right-click/menu affordance.
**1MAX ACCEPTED in game 2026-08-30** ("1Max seems to working
great!"): `LootPlus1MAXTuned` (PR #65) — vanilla density, XP ×3 —
plays at the pace it was built for, so the XP factor is settled.
**Do not re-tune the `* 3`** in `mods/1max.json` without a fresh
complaint; the feared hero-pack hot spot (×3 spawns × ×3 XP = nine
times vanilla hero XP) did not bite. Two older mod
tunes are still live and **unplayed**: Phantom Strike
`characterRunSpeedModifier` 0→500
(PR #58) and Psionic Burn `skillTargetRadius` 3.5→6.0 (PR #61). Both
need only an in-game pass after a session restart. If the blink still
feels slow, the next suspect is `playerRunSpeedCapMax` (166) clamping
the 500 — a global affecting all player run speed, so **ask before
changing it**. Otherwise the interactive acceptance pass over the
store is still the first real task: drag/drop into a bucket, bulk
sends, ⌘F search, an export opened in TQVaultAE, the ▲/▼
sort-direction toggle (PR #52) and the Skip duplicates box (PR #54).
None have been clicked (synthetic input is banned here). PR #50's
risk note stands: merged without that pass; regressions are fixed
forward or reverted (`git revert da227aa`), not re-branched.

- **Standing instruction from 2026-08-29, learned the hard way:**
  when the exact change the user asked for looks impossible, **stop
  and ask** — do not ship an adjacent change you judge to be "in the
  spirit of" it. This session concluded Phantom Strike's blink speed
  wasn't data-tunable, substituted `distanceProfile`+cooldown edits,
  and installed them to `CustomMaps/`. The user rejected them
  outright; they were reverted and PR #57 closed. The conclusion was
  also simply wrong — see WORKING_NOTES.md "Reading the game's record
  semantics". Extra force when the artifact lands in the user's live
  game rather than the repo.
- **Answered, do not re-raise:** tripling max targets on Phantom
  Strike / Dream Stealer needs no change. The spec's blanket
  `multiply_player_skills` ×3 already takes Dream Stealer from
  vanilla 3–8 to 9–24, and Phantom Strike has no `skillTargetNumber`
  at all. User's call after seeing the numbers: leave both alone.
- **Dangling UI feedback, never resolved:** early this session the
  user reported the gold triangular-cornered inner frame fighting the
  tab strips ("the decorative border should be the most outer one"),
  and active-tab borders too thick / offset down into the border
  texture rather than meeting it seamlessly — with a screenshot. That
  was against `feat/tq-chrome`/PR #39, whose chrome PRs #43/#44 have
  since deleted. Flagged as superseded; the user did not confirm
  either way. **Re-ask before acting** — the complaint may still
  apply to the current gilded border + `TabbedPanel`.
- **A parallel session is active on this repo** — it merged PR #62
  and owns `worktree-fix+projectile-speeds-ae` (cast-speed / DPS
  docs, pushed, no PR). Not this session's to touch.
- **Skill/binding conflict worth fixing:** `wrap-up` and `checkpoint`
  both defer to `bootstrap-project` when PROJECT.md is missing, which
  contradicts the standing deferral. Both were run this session with
  METHODOLOGIES.md conventions substituted by hand; the conflict will
  recur every time until PROJECT.md is bound or the deferral is
  recorded where the skills look.
- **Open decision from the 2026-08-29 skilltree review:** whether to
  add a `notes` line to the exported skill-tree document explaining
  when `unlocks_at_mastery_level` is absent. Offered, unanswered. The
  field itself is correct — do not re-audit it.
- **Known gap, unfixed:** `get_mastery` / `list_masteries` call
  `game_data()` directly, so MCP skill trees are always vanilla while
  the record tools overlay the installed bundle. Listed under "Next up".
- User in-game checks outstanding (mod acceptance): **Phantom Strike
  blink speed (PR #58) and Psionic Burn radius (PR #61) — now the
  freshest two**; pet Energy ×2.5 / regen ×1.75 on Core Dweller (875 /
  5.25 at level 20) and Call of the Wild wolves (255 / 3.5); vanilla
  XP restored (even-level trash mob on Normal ≈ level×15); target
  caps ×3 in dense packs; 2026-08-26 Core Dweller tune. Older
  residual: open one of our vault JSONs in TQVaultAE.
- App checks worth a mention next session: autosave against the
  network mount (one `univault-bak` per file per session). The
  2026-08-25 relic-bank `.dxb` truncation was game-side; the twin
  fallback (PR #10) recovers it.
- PROJECT.md bootstrap still deferred (user re-confirmed 2026-08-25);
  answers saved in agent memory (`bootstrap-project-deferred`).

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
The mod forge (arz composer + patch specs, one shared rule set that
bundles extend) builds the user's three live LootPlus bundles: x3, x3
with 1× bosses, and 1MAX at vanilla density. Binding decisions in ARCHITECTURE.md
(autosave + per-load backup-first + targeted-splice writes, one
unified store file with computed type buckets and TQVaultAE JSON as
import/export interchange,
hand-rolled parsers ported from MIT TQVaultAE — GPL refs eyes-only,
docs/format-references.md; MIT OR Apache-2.0; TQ AE + expansions
only). No issue tracker is bound yet (deliberately deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | PRs #1–#75 all merged; all gates green |
| `worktree-fix+projectile-speeds-ae` | a parallel session's cast-speed / DPS docs work | pushed, no PR; locked worktree under `.claude/worktrees/` — not the main session's to touch |
| `worktree-mcp-overlay-and-coverage` | a parallel session's MCP mod-overlay / coverage work | pushed, no PR; checked out in its own worktree — not this session's to touch |

Everything merged through #75 has been pruned local and remote
(2026-08-30). The two `worktree-*` branches belong to parallel
sessions; leave them alone.

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
   headers, PR #52), the Skip duplicates box (PR #54), and now the
   socket pip + tooltip gesture footer — on the user's own config.
   The socketing gestures the footer advertises (drag a piece onto
   gear, Alt+Click to pull it back) have never been clicked either.
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

- **2026-08-30 — Socketing announces itself: tooltip gesture footer +
  socket pips (PR #75, merged).** The ask was "add the
  ability to slot/unslot charms and relics from gear"; the honest
  finding was that both have shipped since PRs #25/#26 — drag a piece
  onto gear, Alt+Click gear to pull it back out, on every surface —
  and that nothing in the app ever said so. Scope the user chose after
  seeing that: change no gesture, make them visible. Every item
  tooltip now ends with the relic/charm gestures that item actually
  accepts ("Alt+Click to remove Essence of the Golden Fleece — both
  are kept", socket, pour shards, pick bonus), and gear wears one
  relic-orange pip per filled socket at its lower-left, so a socketed
  piece is findable while scanning a bank. Core owns the rules, the
  shell owns the phrasing: new `Affordance` (carrying the socketed
  record, so the shell cannot name a piece that isn't there) and a
  typed `BonusPick`, which also collapses the double-click gate
  `request_bonus_edit` had inlined. 268 tests, clippy clean; both
  surfaces seen live under a scratch `HOME`. Risk: no pointer has
  touched either — the tooltip was captured by temporarily forcing
  `hovered` (reverted before commit), since synthetic input stays
  banned, so pip legibility and footer length are the user's call.
  The second (Atlantis) socket is still fill-proof by design, and no
  right-click menu was added — both the user's explicit choices.

- **2026-08-30 — Persisted view state, relic/charm slot filter, helm
  app icon (PR #69, merged).** Three asks in one round. (1) A
  `ui-state.json` beside the store (format tag `univault-ui-state`)
  carries the left tab, inventory sub-tab, store surface, the store
  pane's bucket/sort/Skip-duplicates/slot filter, and the whole
  search bar + sort; snapshotted per frame, written after 1 s of
  quiet, flushed on exit, ignored when foreign or newer. The pane's
  view fields became `StoreView` with the family derived from the
  bucket. (2) Relic and Charm buckets get a "Fits into:" chip row —
  any number of equipment families lit, OR semantics, from the
  cache's existing `socket_targets`; "n of m fit" under the chips.
  (3) The icon is `c04_helm06` (Iron + Corinthian + Great Helm tags)
  read from the game cache at launch — not committed, per the
  never-distribute posture — so the platform default shows until the
  first import. 264 tests, clippy clean, restore + chips seen live
  under a scratch `HOME`. Also fixes the user's mid-session report —
  transfer-stash auto-reload toasting "unexpected end of data …
  wanted 4 more bytes" while the game's SMB write was still landing:
  failed reloads now stay silent for two attempts (~12 s) and report
  once when persisting; a failed reload restores `dirty`. Risk: no
  click has landed on the chips; the dock icon was not captured; a
  bundled `.app` would still need an embedded icon; the reload fix
  is reasoned from the code, not reproduced — the game-side timing
  is the user's to exercise.

- **2026-08-30 — 1MAX: the same mod at vanilla density, XP ×3 (PR
  #65, merged).** User
  ask: a third bundle with no enemy-density increase, where a mob
  group is still worth what a 3× pack is worth. Two things made it
  cheap. The Workshop item already ships `LootPlusXMAXFTWx1` — same
  LootPlus loot tables, `spawnMin/MaxModifier` 100/120 instead of
  300/300, boss and hero pools left at vanilla counts — so the base
  supplies "no density" without undoing anything. And the x3 base pays
  for its density with a `* 0.7` wrapper on `experienceEquation`,
  which `xmax3-tuned` already reverts; 1MAX writes the same vanilla
  string wrapped `* 3` instead. Rather than fork the rules, modforge
  gained `"extends"`: `mods/1max.json` is a name, a parent, and one
  rule, and every tune added to `xmax3-tuned.json` from now on reaches
  all three bundles. Proof the mechanism is inert on what exists:
  rebuilding `LootPlusXMAX3Tuned` with the new binary is **byte-identical**
  to the installed bundle. Built and installed; 255 tests, clippy
  clean. Three user decisions, all theirs: star heroes keep their ×3
  (a density bump, but one they want), XP ×3.0 over the gentler
  options, name `LootPlus1MAXTuned`. **ACCEPTED in game 2026-08-30**
  ("1Max seems to working great!") — the XP factor was the one thing
  no test could settle, and it landed first try. The flagged risk,
  heroes stacking ×3 spawns with ×3 XP for nine times vanilla hero XP,
  did not materialise in play. Risk now retired.

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
