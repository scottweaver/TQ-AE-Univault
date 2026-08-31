# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-31 (PR #84 merged — reload-failure grace per-spell)

## Session handoff
<!-- transient; owned by the checkpoint skill -->

**Resume here:** PR #84 **merged to main (f4fad91) on the user's
"merge it"**, branch pruned local and remote, all five CI checks
green. **Fourth user report in the reload area: "byte read errors
again when refreshing the relic bank"** — the `unexpected end of
data … wanted N more bytes` toast recurring during play. **The
files were verified intact first** (snapshotted to scratch per
WORKING_NOTES before analysing: 173 items, CRC valid, `.dxg` twin
in agreement — nothing lost). Cause: the `RELOAD_PATIENCE` failure
counter was cleared only by a successful *auto*-reload; a
successful **manual Reload — the action the toast itself
recommends — kept the count**, so one bad spell spent the ~12 s
grace for the life of the app and every later read racing a game
save toasted immediately. Fix: `reload_succeeded` now sits at the
parse-success point of all three opens (character, stash, store),
covering every route; manual Reload also continues past a
failed bank instead of aborting the rest. 271 tests, clippy clean.
**Honest limits:** the racy trigger is not reproducible on demand,
and the wiring has no unit test (needs a full `App`; none is
constructible in tests). If toasts still recur, the counter is no
longer the suspect — an SMB stale-read window genuinely exceeding
~12 s is, and `RELOAD_PATIENCE` is the next knob; the toolbar's
watcher line is the first thing to read.

Previously: PR #79 (a mid-save read no longer empties a bank — the
`.dxg`-twin `mid_save()` hold) and PR #77 (auto-refresh stall fixes
+ the toolbar watcher line) landed in the same area; the three-fix
arc is summarized in the progress entries. **1MAX ACCEPTED in game
2026-08-30** ("1Max seems to working great!") — the XP ×3 factor is
settled; **do not re-tune the `* 3`** in `mods/1max.json` without a
fresh complaint. Three mod tunes are still live and **unplayed**:
the blanket cooldown tune (PR #81), Phantom Strike
`characterRunSpeedModifier` 0→500 (PR #58), and Psionic Burn
`skillTargetRadius` 3.5→6.0 (PR #61) — all need only an in-game
pass after a session restart. If the blink still feels slow, the
next suspect is `playerRunSpeedCapMax` (166) clamping the 500 — a
global affecting all player run speed, so **ask before changing
it**. The store's interactive acceptance pass is still the first
real task (see "Next up" item 0); nothing there has been clicked
(synthetic input is banned here). From the socket round (PR #75):
the second (Atlantis) socket can still only be *emptied*, never
filled — nothing in the user's save uses `relicName2`, and filling
it stays deliberately unbuilt pending a decision; no right-click
menus, by their choice.

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
- **Parallel sessions are active on this repo** — one owns
  `mod-pet-normal-durability` (**PR #83 open**: Normal-difficulty
  pets carry half of Epic's defenses) in a worktree under
  `.claude/worktrees/`; another owns
  `worktree-mcp-overlay-and-coverage`;
  `worktree-fix+projectile-speeds-ae` survives remote-only (its
  local worktree is gone). None are this session's to touch.
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
- User in-game checks outstanding (mod acceptance): **the blanket
  cooldown tune (PR #81), Phantom Strike blink speed (PR #58), and
  Psionic Burn radius (PR #61) — the freshest three**; pet Energy ×2.5 / regen ×1.75 on Core Dweller (875 /
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
| `main` | trunk | PRs through #84 merged (#83 still open); all gates green |
| `mod-pet-normal-durability` | a parallel session's Normal-difficulty pet-defense mod | **PR #83 open**; checked out in a worktree under `.claude/worktrees/` — not this session's to touch |
| `worktree-mcp-overlay-and-coverage` | a parallel session's MCP mod-overlay / coverage work | pushed, no PR; checked out in its own worktree — not this session's to touch |
| `worktree-fix+projectile-speeds-ae` | a parallel session's cast-speed / DPS docs work | remote-only now (local worktree gone); not this session's to touch |

Everything merged through #84 has been pruned local and remote
(2026-08-31). The three parallel-session branches above belong to
other sessions; leave them alone.

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

- **2026-08-31 — Fix: reload-failure grace is per-spell (PR #84,
  merged).** User report: "byte read errors again when refreshing
  the relic bank" — the byte-error toast recurring during play. The
  files were intact (verified on scratch copies before anything
  else: 173 items, CRCs valid, twin agreement); the toast was the
  defect. The `RELOAD_PATIENCE` counter was cleared only on
  auto-reload success, so a spell ended by the manual Reload the
  toast itself recommends left the grace spent for the app's
  lifetime — the first transient race on any later game save
  reported instantly. `reload_succeeded` moved to the parse-success
  point of all three opens (every open route clears it; failed
  attempts still accumulate), and manual Reload keeps going past a
  failed bank. 271 tests, clippy clean. Risk: the wiring has no
  unit test (needs a full `App`; none constructible in tests) and
  the racy trigger was not reproduced — if toasts recur, the
  suspect is an SMB stale-read window >12 s and `RELOAD_PATIENCE`
  the next knob.

- **2026-08-30 — Mod: blanket cooldown tune, all bundles (PR #81).**
  User ask, three rules: every invested point shortens a skill's
  cooldown meaningfully; baselines >10s cut 20%; >60s halved. One
  new data-driven modforge rule (`tune_cooldowns` appended to
  `xmax3-tuned.json`; 1MAX inherits via extends): the deepest
  matching cut scales the whole array — classified on the
  *effective* rank-1 value, so LootPlus's own 4s shields are not
  re-cut — then a flat array on an investable skill becomes a
  linear per-rank ramp to half the cut baseline at ultimate level.
  The engine honours per-level cooldown arrays (vanilla ships 9
  decreasing ones; those keep their shape, cut only), and the
  zeroed summon classes stay zero. 281 of 300 cooldown-bearing
  player skills change per bundle (Colossus Form 360 → 180→90,
  Death Ward 300 → 150→75, Phantom Strike 16 → 12.8→6.4). 209 core
  tests, clippy clean; scratch and SMB builds byte-identical; all
  three bundles installed. Risk: "meaningful" was read as equal
  steps to 50% at ultimate — the exact curve is the user's to bless
  in game; Renewal's by-design *rising* cooldown kept its shape but
  took the 20% cut.

- **2026-08-30 — A mid-save read no longer empties a bank (PR #79,
  merged).** Third report in the same area: "a change
  on disk and a successful reload, but Character Bank and Shared are
  showing no items." The cause is a gap `RELOAD_PATIENCE` structurally
  cannot see — a stash save caught between truncating and writing its
  items parses as a **valid empty stash**, so nothing fails, the app
  truthfully says "auto-reloaded", and the pane truthfully goes empty.
  The fix reads the evidence the game already keeps: `.dxg` is its
  last good write, so a `.dxb` that comes back empty while its twin
  still parses with items is a save in flight, and the pane is held
  and re-read. Bounded by `EMPTY_PATIENCE` (5 attempts ≈ 20 s) so a
  twin that never catches up cannot pin stale items on screen; a
  genuine emptying, where the twin is empty too, clears with no delay
  at all. Verified live in both directions. 271 tests, clippy clean.
  Risk, stated plainly: **the user's exact trigger was never
  reproduced** — this closes the most plausible mechanism, not a
  proven one. No data was ever at risk; their files were checked and
  found intact throughout.

- **2026-08-30 — Auto-refresh stops stalling silently (PR #77,
  merged).** User report: "Shared bank doesn't
  seem to be reloading when it should." Reproduced against a local
  copy of their save tree, and it was never shared-bank-specific:
  auto-refresh is consumed inside the paint loop, so a fully occluded
  window (macOS stops repainting one) sees no file change at all.
  Two real defects behind that. `drive_refresh` drained every queued
  poll and kept only the newest, so the documented "settled across
  two polls" debounce actually counted *UI frames* and a hidden
  window's backlog of identical observations was discarded — costing
  another poll cycle after the window returned; it now feeds every
  queued poll, so the backlog settles it. And `busy` included
  `memory.focused().is_some()`, which is sticky: a caret left in the
  ⌘F search box disabled refreshing for **every** pane indefinitely.
  Focus now defers only the character, and only for the gold
  `DragValue` — the one focusable widget bound to document state
  (both search fields bind to UI state, and nothing calls
  `request_focus`). Because the user could not say which scenario
  they had hit, the toolbar gained "watching: checked Ns ago ·
  longest pause … (window hidden?)", which records a stall after it
  ends. Verified live occluded → raised: the pane reloads within 2 s
  and a 38 s pause is reported. 270 tests, clippy clean. Risk: while
  the window is *fully covered* nothing refreshes at all and this
  does not change that — moving reloads off the paint loop is a
  bigger change, deliberately not attempted. The toolbar line is a
  first cut; it may belong behind the planned "?" technical-info
  affordance instead.

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
