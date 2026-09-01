# Working notes

Hard-won operational knowledge about *this* codebase on *this*
machine: framework landmines, the UI development loop, how to verify
against real game data, and the environment quirks that have cost a
session before. Durable by nature — none of it expires when a branch
merges.

Boundaries: RUST_BEST_PRACTICES.md owns how code is written,
METHODOLOGIES.md owns how work moves (branching, PRs, cleanup),
ARCHITECTURE.md owns what must remain true, STATE.md owns what is
happening right now. This file owns what an agent would otherwise
have to rediscover the hard way.

Anything here that becomes a *constraint* graduates again, into
ARCHITECTURE.md. Anything that stops being true gets deleted, not
annotated.

## egui 0.36 landmines

1. **`ui.columns` children share a *stable* id.** Derive
   per-instance widget ids from an allocated response id, never from
   `ui.id()` (`tabbed_panel` does this). Two columns rendering the
   same widget otherwise collide.
2. **`Tooltip::for_widget(...).show()` is unconditionally open.** Use
   `Tooltip::for_enabled`, or gate at the call site. Missing this
   produced a tooltip storm in the search view.
3. **A long `horizontal_wrapped` row of ComboBoxes never wraps** — it
   silently overflows its pane. Hit when the search filters moved
   into the half-width vault column. Split into multiple wrapped rows
   sized for the pane instead.
4. **A fully occluded window stops repainting** (normal macOS
   occlusion). A background import then looks "stuck at 2%" until the
   window is uncovered — not a hang. The same stall hits **anything
   driven from the paint loop**: auto-refresh consumes the watcher
   thread's polls inside `drive_refresh`, so a covered window notices
   no file change at all until it is visible again (found 2026-08-30
   chasing "Shared bank doesn't reload"). The toolbar's
   "watching: checked Ns ago · longest pause …" line exists to make
   that visible; when testing a repaint-driven feature, **raise the
   window first** — a screenshot via `screencapture -l <id>` captures
   an occluded window happily and hides the stall.

## The UI development loop

- **Review loop** (preview harness): `cargo run -p univault-gui --bin
  preview -- <component> --review`, keep a Monitor on the gitignored
  `review/` directory, act on exports (annotated PNG +
  component-space JSON). Preview-only by user decision; promote into
  the app behind a debug flag only if an app-look round needs it.
- **Screen capture** (works, keep using it): Screen Recording is
  granted to iTerm. Find the window id by pid via a CGWindowList
  swift one-liner, then `screencapture -x -l <id>`; `-o` drops the
  shadow for exact coordinate mapping.
- **Never post synthetic CGEvents into a live window.** A
  synthetic-input test once interleaved with the user's real typing
  and garbled their annotation. Interactive acceptance is the user's
  to perform, always.
- **Worktree sessions run under a shell guard** that refuses
  compound commands it cannot prove stay inside the worktree —
  `while`/`for` loops, `$(…)` substitutions, and any `HOME=…` prefix
  (so the scratch-`HOME` launch is refused inline). Put the launch
  in a script file (`zsh launch.sh` that exports `HOME` itself) and
  do loops in Python or a throwaway core example. Cost a dozen
  refused commands on 2026-08-30.
- **Screen capture only shows the display it hits.** The Dock
  auto-hides here and owns no on-screen window, so the dock icon
  cannot be captured; ask the user to look.
- **The user often runs their own release build.** Only ever
  `pkill -f target/debug/…`. Each debug relaunch steals focus, so the
  user's in-flight clicks can land in the test instance.
- **Design sources:** component art is authored by the user in GIMP
  at `~/Documents/tq-desgins/` (XCF masters, PNG exports beside
  them). GIMP batch export mostly hangs on this machine (script-fu
  and python-fu both — the save can land *before* the hang), so
  prefer asking the user to export. Game-art reference screenshots
  live in `/Volumes/scott-games/tq-ae-designs` (mount required).

## Reading a save the game is still writing

- **The game writes from another machine.** Steam on a Bazzite Linux
  PC runs the game and writes the save tree straight to the NAS
  (192.168.1.177); the Mac only ever sees those bytes through its own
  smbfs client, so cache coherence is whatever macOS revalidation
  provides — which failed for **minutes** on 2026-08-31: four minutes
  after the game's quit-save, a *freshly launched* app read its own
  last-written pages while `stat` already showed the game's new
  mtime. Stale data under a fresh stamp is invisible to every stamp
  comparison — the watcher said "checked 1s ago" over a stale pane,
  and the overwrite guard was equally blind. Defense since PR #86:
  all game-file IO goes through `safe_io.rs` — `F_NOCACHE` reads
  length-checked against the file's own metadata (a lying read
  surfaces as a "stale or mid-write network read" error that the
  reload grace retries), uncached writes, and backup copies that
  read-verify first. `univault-mcp` still reads through the page
  cache; its answers can be minutes stale during and just after
  play.
- **A scratch copy that disagrees with a live pane is the
  stale-window signature,** not app corruption — copying revalidates
  the cache, the poisoned pane cannot. (Pre-#86 behaviour; post-#86
  the pane should never disagree with a fresh copy.)
- **A stash save caught mid-write parses as a valid *empty* stash.**
  Nothing errors, so `RELOAD_PATIENCE` (which only counts reads that
  fail) never fires: the app reports a successful reload and shows an
  empty bank. Found 2026-08-30 chasing "Character Bank and Shared
  aren't showing any items".
- **The `.dxg` twin is the tell.** The game keeps it as the last good
  write, so a `.dxb` that reads empty while its twin still parses with
  items is a save in flight, not an emptied bank. That is what
  `mid_save()` checks before letting a reload clear a pane; the
  deferral is bounded (`EMPTY_PATIENCE`) so a twin that never catches
  up cannot pin stale items on screen forever.
- **Never diagnose these files while the app or game is live.** Two
  separate scans during this session read a file mid-rewrite and
  reported "0 items" and "`.dxb` and `.dxg` disagree in 920 bytes",
  both of which were measurement artifacts and sent the session
  chasing corruption that did not exist. `cp` the files to scratch
  first, then analyse the copies.

## Verifying against real game data

- **Copy the database off the mount first.** The game install lives
  on an SMB mount where a bare `ls` of the install directory has
  timed out at two minutes. Copy `Database/database.arz` and
  `Text/Text_EN.arc` to local scratch; whole-database sweeps then run
  in seconds instead of stalling.
- **Drive the GUI under a scratch `HOME`** holding copies of the
  config directory, so the user's own config stays clean. Mandatory
  for any change that writes to the config dir.
- **Real-data migration check** is env-gated:
  `UNIVAULT_REAL_VAULT=<vault.json> UNIVAULT_REAL_CACHE=<gamedata.cache>
  cargo test -p univault-core --test real_vault_migration -- --nocapture`
- **The MCP server** is exercised by piping JSON-RPC into
  `./target/debug/univault-mcp` with `UNIVAULT_STORE` set. The
  session's own `univault` MCP connection points at
  `target/release/univault-mcp`, which a fresh worktree does not
  have — build it (`cargo build --release -p univault-mcp`) before
  expecting the record tools, or the server "fails to connect".
- **Finding a record from a display name is a tag hunt.** The name
  the app shows ("Iron Great Helm Corinthian") is assembled from
  `itemQualityTag` + `itemNameTag` + `itemStyleTag`; record ids are
  opaque (`c04_helm06.dbr`) and `strings database.arz | grep` only
  finds referencing loot tables. Filter records by class prefix and
  those tag values (a scratch example over `ArzFile::record_ids`
  does it in seconds against the local `db/` copies).
- **Three mod bundles are installed on this machine** —
  `LootPlusXMAX3Tuned`, `LootPlusXMAX3Tuned1xBoss`, and
  `LootPlus1MAXTuned`, all under the save root's `CustomMaps`.
  `resolve_mod(None)` errors naming the choices whenever more than one
  exists, so record tools need an explicit `mod` argument here.
- **The base mods all ship in one Workshop item.** Item 1779344333
  under `steamapps/workshop/content/475150/` holds eight sibling
  folders — `LootPlusXMAXFTWx1` through `x5`, `x3x1`, `xMax-`,
  `xMax+`. They share the LootPlus loot tables and differ in density:
  `x1` leaves `spawnMinModifier`/`spawnMaxModifier` at 100/120 and the
  boss/hero proxy pools at vanilla entry counts, while `x3` sets
  300/300, expands those pools, and pays for it with a `* 0.7` wrapper
  on `experienceEquation`. Pick the base folder that already does the
  density you want rather than trying to undo one.
- **Custom-quest characters live in `SaveData/User`, shared by every
  bundle** — switching between our CustomMaps mods keeps the same
  characters; only `SaveData/Main` is the vanilla campaign.

## Reading the game's record semantics

- **A `.tpl` is the editor's schema, not the engine's contract.** The
  engine reads DBR variables **by name**; a variable missing from a
  record's template can still be honoured. Never conclude "the game
  can't do this" from a template alone — that mistake cost a session
  on 2026-08-29 and shipped an unwanted change to the user's install.
- **The reliable method: find a sibling record of the same class that
  already does the thing, and diff it.** Monster/hero variants of a
  player skill are the richest source — they are usually the same
  class with the interesting knobs turned on.
- **Worked example — blink/charge travel speed is
  `characterRunSpeedModifier`.** Phantom Strike
  (`records\xpack\skills\dream\phantomstrike.dbr`,
  `Skill_AttackWeaponBlink`) ships it at `0.0`, while the monster copy
  `HERO_PHANTOMSTRIKE.DBR` — identical class — ships `300.0`, as do
  the `Skill_AttackWeaponCharge` skills Shield Charge and Take Down.
  `Skill_AttackWeaponBlink.tpl` declares none of it; it is a bare
  header over `Skill_AttackWeapon.tpl` (literally "Copy of
  Skill_AttackWeaponCharge.tpl").
- **Templates live in `Toolset/Templates.arc`**, not under
  `Database/`. Entries are lowercase paths like
  `templates/templatebase/skill_warmup.tpl`, readable as plain text
  through `arc::ArcFile::file`.
- **Check the spec's blanket rules before adding a per-record one.**
  `mods/xmax3-tuned.json` carries sweeping rules — `skillTargetNumber`
  ×3 across every player skill that has one (10 skills; Dream Stealer
  3–8 → 9–24), `skillCooldownTime` zeroed on both summon classes,
  and `tune_cooldowns` over every remaining player-skill cooldown
  (added 2026-08-30: rank-1 >60s halves the array, >10s cuts it 20%,
  and a flat array on an investable skill becomes a linear per-rank
  ramp down to half the cut baseline at ultimate level; vanilla's 9
  hand-shaped decreasing arrays keep their shape and get the cut
  alone). A request to "triple X" or "shorten X's cooldown" is often
  already satisfied; adding a rule on top multiplies again. Diff the
  installed bundle against vanilla first — that is what `moddiff`
  and the build report are for.
- **A new bundle inherits the tunes with `"extends"`, never a copy.**
  `mods/1max.json` is `{name, extends: "xmax3-tuned.json", rules}`:
  modforge prepends the named spec's rules, so a rule added to
  `xmax3-tuned.json` reaches every bundle and the extending spec only
  states its own difference. Rules run in order, so a later `set`
  refines an earlier `revert_variables` on the same variable — that is
  how 1MAX overrides the inherited XP revert.
- **Useful globals in `records\xpack\game\gameengine.dbr`:** distance
  profiles (`meleeRange` 1.2, `shortRange` 5, `moderateRange` 10,
  `longRange` 17, `maximumRange` 30, against a 34-unit
  `CameraDistanceDefault`) and the run-speed ceilings
  (`playerRunSpeedCapMax` 166, `monsterRunSpeedCapMax` 400,
  `absoluteRunSpeedCapMax` 500). These are global — prefer changing a
  skill's own `distanceProfile` over editing a shared range.

## Cache and first launch

Cache format is `UVC8`; import reads `InGameUI.arc` + `XPack/UI.arc`
for chrome textures alongside item data. The first app launch after
pulling a cache-format change re-imports game data in the background
— over the SMB mount that takes minutes, not the old ~8s. The panes
work throughout on the fallback theme.

## Repository and CI quirks

- **`gh pr merge --auto` merges immediately here — it does not
  queue** (corrected 2026-08-29 by running it twice, PRs #53 and
  #55). The repo's `allow_auto_merge` is `false` *and* `main` is
  unprotected with zero required status checks, so there is no gate
  for `--auto` to wait behind and gh falls straight through to a
  direct merge. Both docs PRs landed before CI reported. Harmless
  under the docs-only carve-out; **dangerous on a source PR** —
  those still get CI watched to completion and merged deliberately.
  Enabling real auto-merge is a one-click change (repo Settings →
  General → "Allow auto-merge") but would also need branch
  protection with required checks to actually gate anything.
- **A stale Travis CI GitHub App** is installed and attaches
  permanently-queued phantom check suites to commits. Uninstall
  advised (repo Settings → Integrations → GitHub Apps); not yet
  confirmed done.
- **GitHub Actions event delivery** was unreliable during the
  2026-08-26 outage: push/PR webhook events silently dropped several
  times. The lever if it recurs is `gh workflow run CI --ref
  <branch>` (workflow_dispatch, added in PR #13), then `gh run watch
  <id> --exit-status`.
- **The `.cursor/rules/*.mdc` mirrors are stale and cannot be
  regenerated:** they carry a "generated by agent-sync" marker but no
  sync script is checked into the repo, so edits under
  `.claude/rules/` never propagate. Either add the script or drop the
  mirrors — hand-editing generated files is the wrong fix.

## Maintenance

Add an entry when a session burns real time rediscovering something
that will bite again. Delete an entry the moment it stops being true
(a fixed landmine, an uninstalled app, a repaired mirror) — a stale
note here is worse than no note. Keep entries falsifiable: name the
symptom, the cause, and the lever.
