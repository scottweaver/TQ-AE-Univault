# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-25

## Active workstream

tq-univault: a platform-independent (Windows/macOS/Linux)
reimplementation of TQVaultAE — the item-vault / inventory-manager
companion app for Titan Quest — in Rust (workspace: `univault-core`
pure logic, `univault-gui` egui/eframe shell). Working today: full
read stack (chr, stash, vault JSON + legacy import, ARZ/ARC/text/
textures) with localized names, real footprints, and icons; grid
rendering; two-pane vault ⇄ character/stash transfers; gold editing;
backup-first splice saves — user-accepted against a real
network-mounted save tree. Binding decisions in ARCHITECTURE.md
(backup-first + targeted-splice writes, TQVaultAE JSON vault schema,
hand-rolled parsers ported from MIT TQVaultAE — GPL refs eyes-only,
docs/format-references.md; MIT OR Apache-2.0; TQ AE + expansions
only). No issue tracker is bound yet (deliberately deferred).

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | read stack + transfers + grids + full item tooltips merged; pushed to the new GitHub origin |
| `feat/mod-forge` | ARZ composer + mod patch compiler; first tuned mod installed | PR open |

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

User-inserted priorities (2026-08-25, before drag-and-drop):
backup rotation (5 newest per file — in PR) and two respec buttons
("Respec attributes", "Respec skills & masteries"), each behind a
confirm dialog, refunds computed from deltas. Respec needs new chr
parsing (attributes, skill list, hotbar) + targeted splices;
provenance note: TQVaultAE has no respec — tqrespec (GPL) is
eyes-only reference, implementation independent.

1. Drag-and-drop item movement on the grids (click-select + buttons
   today), including cross-pane drags and an occupied-cell drop
   preview.
2. Game-install auto-discovery (Steam library paths per OS in
   `platform`) to preseed the one-time Import dialog.
3. DXT1/3 decode for the ~7 compressed item bitmaps (currently an
   initial-letter fallback tile).
4. Stretch (unscheduled): local SQLite index across vaults for
   search — would be additive, vault files stay the source of truth.

## Most recent meaningful progress

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
- **2026-08-24 — Local game-data cache: import once, launch from
  cache.** User-directed pivot: instead of pointing at the game dir
  every launch, one Import pass distills all 13,188 item records
  (names, footprints, zlib'd RGBA icons) into a 50MB UTF-8/binary
  cache under the config dir (`cache` module; ARCHITECTURE gained
  the derived-cache constraint). `GameCache` replaced `GameData` as
  the runtime DB everywhere (GameData is now import-time only);
  launches need no game volume, staleness is fingerprint-detected
  (size+mtime), `--game` now just forces a re-import, and the GUI
  gained "Import game data…". Real-data gate caught a 1252-encoding
  bug: "Jǫrmungandr" doesn't fit Windows-1252 — cache strings are
  UTF-8. 95+ tests; cache answers verified identical to live data
  across all real characters. Risk: cache format is versioned only
  by magic — bump `UVC1` on layout changes.
- **2026-08-24 — Grid rendering with item icons.** `tex::decode`
  turns the game's uncompressed 32-bit BGRA / 24-bit BGR textures
  into RGBA (a probe of the real install put those at 99.9% of item
  bitmaps; DXT stragglers fall back to an initial-letter tile);
  `GameData::item_icon` serves decoded images. The GUI paints
  sacks/stash/vault tabs as real cell grids — items at their
  positions with true footprints, icon textures, stack badges,
  selection highlight, hover-name tooltips — via per-record caches
  (names/footprints/icons) so nothing expensive runs per frame.
  96 tests. Why: it finally looks like a vault, not a debugger.
  Risk: rendering is visually unverified until the user runs it;
  egui texture memory grows unbounded with the icon cache (fine at
  item-bitmap sizes).
- **2026-08-24 — Real item footprints from game textures.** `tex`
  module reads TEX/DDS headers (magic variants incl. the
  Atlantis-era pad byte; cells = px ÷ 32 per TQVaultAE);
  `GameData::resource` resolves bitmap ids across all `Items.arc`
  archives (XPACK prefix mapping + the records-lie cross-archive
  fallback); `item_footprint` now returns true grid sizes with the
  conservative class bounds as fallback. Real-install proof: bow
  1×4, torso 2×3, rings 1×1 across 4 loaded archives. 90+ tests.
  Why: dense, game-accurate placement; also unlocks item icons for
  grid rendering. Risk: footprint lookups are uncached (record +
  tex decompress per query) — fine for click-driven moves, revisit
  before drag-and-drop hover previews.
- **2026-08-24 — Transfer UI: the core feature loop is live.**
  Two-pane GUI (game file ⇄ vault) with click-to-select item
  movement, dirty tracking, and explicit saves through the new
  backup-first shell writer (synced timestamped sibling backups;
  stash saves rewrite the `.dxg` twin via the ported
  `EncodeBackupFile`). Pure `transfer` module in core: take/place
  operations using TQVaultAE's column-major grid search with
  conservative per-class footprints (no overlap possible at true
  sizes; real footprints need a TEX reader — next). Legacy `.vault`
  opens import-only, saving as `.json`. Side request landed: gold is
  editable on the character pane (`replace_money`, a 4-byte
  in-place patch). 84 tests. Why: the product's reason to exist now
  functions end to end. Risk: no real disk write has happened yet —
  user acceptance on a save-tree copy is the next gate; placement
  is sparse until real footprints land. First acceptance attempt
  found and fixed a real-world snag: macOS `sync_all` (F_FULLFSYNC)
  is rejected by SMB mounts ("os error 45") — the writer now
  degrades to close-flush on "unsupported", validated by running the
  safe-write tests with TMPDIR on the user's network volume.

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
