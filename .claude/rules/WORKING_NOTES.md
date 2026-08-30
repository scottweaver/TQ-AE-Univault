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
   window is uncovered — not a hang.

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
- **The user often runs their own release build.** Only ever
  `pkill -f target/debug/…`. Each debug relaunch steals focus, so the
  user's in-flight clicks can land in the test instance.
- **Design sources:** component art is authored by the user in GIMP
  at `~/Documents/tq-desgins/` (XCF masters, PNG exports beside
  them). GIMP batch export mostly hangs on this machine (script-fu
  and python-fu both — the save can land *before* the hang), so
  prefer asking the user to export. Game-art reference screenshots
  live in `/Volumes/scott-games/tq-ae-designs` (mount required).

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
  `./target/debug/univault-mcp` with `UNIVAULT_STORE` set.
- **Two mod bundles are installed on this machine** —
  `LootPlusXMAX3Tuned` and `LootPlusXMAX3Tuned1xBoss`, both under the
  save root's `CustomMaps`. `resolve_mod(None)` errors naming the
  choices whenever more than one exists, so record tools need an
  explicit `mod` argument here.

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
