# Mod bundles

Personal tuned variants of the LootPlus mod for Titan Quest
Anniversary Edition (all expansions), composed by this repo's mod
forge. Each bundle is a **new folder** written to the save root's
`CustomMaps/` — the game's own databases, the Workshop originals,
and third-party files are never modified, and deleting a bundle
folder removes it without trace (see ARCHITECTURE.md, "Mod bundles
are a sanctioned output boundary").

## The bundles

| Bundle | Base (Workshop 1779344333) | Monster density | Named bosses | XP |
|---|---|---|---|---|
| `LootPlusXMAX3Tuned` | `LootPlusXMAXFTWx3` | ×3 | ×3 | vanilla |
| `LootPlusXMAX3Tuned1xBoss` | `LootPlusXMAXFTWx3x1` | ×3 | ×1 | vanilla |
| `LootPlus1MAXTuned` | `LootPlusXMAXFTWx1` | vanilla | ×1 | vanilla ×3 |

All three share the LootPlus loot tables. The bases differ only in
density: `x1` leaves `spawnMinModifier`/`spawnMaxModifier` at
100/120 and the boss/hero proxy pools at vanilla counts; `x3` sets
300/300, expands those pools, and pays for it with a `* 0.7` wrapper
on `experienceEquation` (which the shared spec reverts — see below).
Pick the base that already does the density you want rather than
undoing one.

Custom-quest characters live in `SaveData/User` and are shared by
every bundle — switching between these mods keeps the same
characters. Only `SaveData/Main` is the vanilla campaign.

## How the specs compose

- `xmax3-tuned.json` is the **one shared rule set**. It names the
  two x3 builds; the 1x-boss variant is the same spec built with the
  bundle-name override.
- `1max.json` is `{name, extends: "xmax3-tuned.json", rules}` — the
  forge prepends the parent's rules, so a tune added to the shared
  spec reaches every bundle and an extending spec states only its
  own difference.
- Rules run in order: a later `set` refines an earlier rule on the
  same variable. That is how 1MAX's XP ×3 overrides the inherited
  revert-to-vanilla.

## Shared tunes (`xmax3-tuned.json`, inherited by all bundles)

### Density and reward

- **Star heroes spawn exactly ×3 on every difficulty**
  (`multiply_hero_pools`, PR #35). Every spawn pool whose entries
  are all Hero-classified has its entry list repeated three times —
  entry count is the engine's spawn multiplier for these records.
  Pools holding any Boss/Quest monster are untouched.
- **XP equation reverted to vanilla** (`revert_variables`, undoing
  the x3 base's `* 0.7` wrapper). A 3× pack should pay 3×, not 2.1×.

### Skill scaling

- **Max targets ×3** on every player skill that has a
  `skillTargetNumber` (10 skills; e.g. Dream Stealer 3–8 → 9–24).
  Phantom Strike carries no target count — its 360° multi-hit is
  Dream Stealer's job — so it is correctly absent here.
- **Fire Enchantment buff radius ×3**
  (`fireenchantmentbuff.dbr`, `skillTargetRadius`).
- **Psionic Burn radius 3.5 → 6.0** (PR #61, set on the live record
  `psionictouch_psionicburn.dbr`; the `OLD\` and `11-15-06\` copies
  are dev leftovers).
- **Phantom Strike blinks at speed** (PR #58):
  `characterRunSpeedModifier` 0 → 500 on the skill record only, no
  globals touched. 500 is `absoluteRunSpeedCapMax`; the monster twin
  `HERO_PHANTOMSTRIKE.DBR` ships the same class at 300. If it still
  feels slow in game, the suspect is `playerRunSpeedCapMax` (166)
  clamping it — a global affecting all player run speed, so raising
  it is its own decision.

### Cooldowns

- **Summons are cooldown-free**: `skillCooldownTime` filled to 0 on
  every player-side `Skill_SpawnPet` and (for Sylvan Nymph, PR #41)
  `Skill_AttackProjectileSpawnPet` record.
- **Every other player-skill cooldown falls with investment**
  (`tune_cooldowns`, PR #81). Three rules, applied to the 281 of 300
  cooldown-bearing player skills the earlier rules don't zero:
  1. a rank-1 baseline **over 60 s is halved**, **over 10 s is cut
     20%** — the deepest matching cut scales the whole per-level
     array, no stacking;
  2. a **flat** cooldown array on an investable skill becomes a
     linear per-rank ramp from the cut baseline down to **50% of it
     at the skill's ultimate level** — every invested point shortens
     the timer by an equal, visible step;
  3. arrays the game already shapes per rank (9 in vanilla, e.g.
     Enslave Spirit 180→45) keep their designed shape and receive
     the cut alone — including Renewal, whose cooldown *rises* by
     design as its refresh grows.

  Classification uses the *effective* record (base-mod override
  included), so LootPlus's own 60→4 s Energy/Heat Shield cooldowns
  are not cut again — they just gain the ramp (4 → 2 at ultimate).
  Samples (rank 1 → ultimate): Colossus Form 360 → 180→90, Death
  Ward 300 → 150→75, Menhir Altar 240 → 120→60, Quick Recovery
  60 → 48→24, Distortion Field 30 → 24→12, Phantom Strike
  16 → 12.8→6.4. The cut table and the 50% floor live in the spec
  JSON — retuning the curve is a data edit, no code change.

### Pets

- **Normal-difficulty pets carry half of Epic's defenses** (PR #83).
  The engine scales every Pet-class summon through one global table,
  `records\game\petgamebalanceattributes.dbr`, whose arrays hold
  3 difficulties × 6 acts — and vanilla's Normal segment grants
  almost nothing (life +0→85% across the acts, zero DA/resists/
  armor, even a −10% attack-speed penalty), which is exactly why
  pets feel fragile until Epic's cliff (+100% life, +350 DA, +12–15%
  resists, +80 armor). The tune rewrites only the Normal segment to
  half of Epic's opening values: life +50→100% (act 6 meets Epic's
  start), DA +175, life regen +25%, resists +6–8% (physical +2.5%),
  armor +40, attack-speed penalty removed. Epic and Legendary are
  byte-identical to vanilla. Known side effect, accepted: enemy-owned
  summons (dark-obelisk raises, broodmother spawns) share the Pet
  class and gain the same Normal-only bump; scroll summons are
  `PetNonScaling` and unaffected.
- **Energy ×2.5, Energy regen ×1.75** across all 20 level records of
  Earth's Core Dweller and Nature's Call of the Wild (Core Dweller
  875 / 5.25 at level 20; wolves 255 / 3.5).
- **Core Dweller Provoke**: `skillTargetRadius` → 5.0 and
  `offensiveTauntMax` raised to 12–18 across its ladder (PR #17).
- **Core Dweller Wildfire**: minimum slow durations
  (offensive-ability and run-speed) → 3.0 s (PR #17).

## 1MAX's own rule (`1max.json`)

- **XP ×3**: the vanilla `experienceEquation` wrapped in `(... * 3)`
  on `gameengine.dbr`, so a mob group at vanilla density is worth
  what a 3× pack pays in `LootPlusXMAX3Tuned` (PR #65).

## Building and installing

The forge merges the patched records onto the base mod's database
and copies its resources; the output lands directly in `CustomMaps/`
as a complete bundle. On this machine:

```sh
GAME='/Volumes/scott-games/steamapps/common/Titan Quest Anniversary Edition'
WORKSHOP='/Volumes/scott-games/steamapps/workshop/content/475150/1779344333'
MAPS='/Volumes/scott-games/tq1-saves/Titan Quest - Immortal Throne/CustomMaps'

cargo run --release -p univault-core --example modforge -- \
  "$GAME" "$WORKSHOP/LootPlusXMAXFTWx3"   mods/xmax3-tuned.json "$MAPS"
cargo run --release -p univault-core --example modforge -- \
  "$GAME" "$WORKSHOP/LootPlusXMAXFTWx3x1" mods/xmax3-tuned.json "$MAPS" \
  LootPlusXMAX3Tuned1xBoss
cargo run --release -p univault-core --example modforge -- \
  "$GAME" "$WORKSHOP/LootPlusXMAXFTWx1"   mods/1max.json        "$MAPS"
```

**Rebuild all three whenever a spec changes** — a rule added to
`xmax3-tuned.json` reaches a bundle only when that bundle is
rebuilt. Builds are deterministic: rebuilding with an unchanged spec
reproduces the installed database byte-for-byte, so `cmp` against
the installed bundle proves whether anything drifted.

Verifying a change:

- the build **report** prints every record edit (before → after);
- `moddiff` diffs an installed bundle against the main database,
  optionally filtered by variable name;
- `dump` prints one record's variables from any `.arz`.

The forge's self-check re-parses the composed database and asserts
every record round-trips before anything is written.

## Play-test status (2026-08-30)

- **Accepted in game**: `LootPlus1MAXTuned` overall pace and XP ×3
  ("1Max seems to working great!", 2026-08-30). The feared hero
  hot-spot (×3 spawns stacking with ×3 XP) did not bite.
- **Awaiting an in-game pass**: Normal pet durability (PR #83), the
  cooldown tune (PR #81), Phantom Strike blink speed (PR #58),
  Psionic Burn radius (PR #61), the pet Energy/regen numbers, and
  target caps ×3 in dense packs.

## Changelog

| Date | PR | Change |
|---|---|---|
| 2026-08-30 | #83 | Normal pets: half-Epic defenses, attack-speed penalty removed (`petgamebalanceattributes.dbr`) |
| 2026-08-30 | #81 | Blanket cooldown tune: per-point ramps; >10 s cut 20%, >60 s halved |
| 2026-08-30 | #65 | `LootPlus1MAXTuned`: vanilla density, XP ×3, `extends` mechanism |
| 2026-08-29 | #61 | Psionic Burn radius 3.5 → 6.0 |
| 2026-08-29 | #58 | Phantom Strike `characterRunSpeedModifier` 0 → 500 |
| 2026-08-27 | #41 | Sylvan Nymph joins the cooldown-free summons |
| 2026-08-26 | #35 | Star heroes exactly ×3 on every difficulty (`multiply_hero_pools`) |
| 2026-08-26 | #28 | Both boss-count variants: x3 restored, 1x-boss bundle added |
| 2026-08-26 | #17 | Core Dweller Provoke reach/taunt + Wildfire slow durations |
| 2026-08-25 | — | Core Dweller & wolves Energy ×2.5, regen ×1.75; summon cooldowns zeroed; XP revert; mod forge itself |
