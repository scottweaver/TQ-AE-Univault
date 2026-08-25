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
| `main` | trunk | read stack + transfers + grid rendering merged; all gates green |
| `feat/item-hover-details` | full item tooltips: rarity colors + ported stats engine | implemented + real-data validated; awaiting user visual check + merge |

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
worked!"). Residual check whenever convenient: confirm a
transferred-back item in-game and open our vault JSON in TQVaultAE.

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
  record-local (deviation noted in `stats::render` docs).
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
- **2026-08-24 — Splice write path lands, byte-identity proven on
  real files.** `writer` module (1252 encoding), `chr::
  replace_inventory` and `stash::replace_items` rebuild only the
  item regions and copy every other byte through; stash CRC ported
  (zero-init, no final complement, verified against real files).
  The model now preserves `tempBool` and per-stack-member
  `seed`/`var2` — the latter found by the byte-identity gate on the
  user's real save (Atlantis-era folded members carry their own
  `var2`; first attempt repeated the head's). Sweep: 4 real
  characters + 3 stashes (one with an item) resplice
  byte-identically. 68 tests. Why: transfers are now a data edit
  plus a file write away. Risk: no writes hit disk yet — the
  backup-first shell function lands with the transfer UI.
- **2026-08-24 — Read stack complete: stash reader + full real-data
  sweep.** All feature branches merged to `main` and deleted. New
  `stash` module (`winsys.dxb`/`.dxg`: CRC header skipped on read,
  explicit `stackCount` = size−1, float grid offsets) reusing the
  chr item parser via a new `Stash` context; GUI opens stashes by
  extension. `examples/smoke.rs` now sweeps a `SaveData` tree: the
  user's real transfer stash + 2 character stashes + 4 characters
  all parse with resolved names. 59 tests. Why: every item source
  (character, stash, vault, game DB) is now readable — the write
  path is all that separates us from working transfers. Risk: the
  real stashes were empty; a stash with items has only synthetic
  coverage until the user stashes something in-game.
- **2026-08-24 — Name resolution + GUI wiring; items have real
  names.** `gamedata` module combines ARZ + text: per-type name-tag
  dispatch ported from TQVaultAE's `Info.AssignVariableNames`
  (gear → `itemNameTag`, affixes → `lootRandomizerName`,
  relics/quest/artifacts → `description`), expansion-ordered text
  loading, color-code stripping (`{^l}`), affix+base+suffix
  assembly with stem fallback. GUI takes `--game <dir>`, loads the
  DB once, and precomputes display rows (no per-frame record
  decompression). Real-install smoke: 18,783 names resolved
  (up from 11,006 description-only), clean output. 53 tests. Why:
  the item-DB feature the user asked for is live end-to-end. Risk:
  quality/style tags ("Ancient", "Fine") not yet part of names;
  formula items show the generic formula name.
- **2026-08-24 — ARC + text readers; real-install validation
  passed.** `arc` module (part-table/directory/names parse, stored +
  multi-part zlib extraction) and `text` module (BOM-sniffed
  `tag=label` table with `TQVaultAE`'s gendered-label cleanup). The
  user mounted a real TQ AE install; `examples/smoke.rs` decompressed
  **all 74,013 records of database.arz with 0 errors**, extracted 57
  files from Text_EN.arc, loaded 17,540 tags, and resolved item
  names end-to-end ("Bow of Upis"). Why: the whole read stack is now
  proven against reality. Risk: `description` on quest items is
  flavor text, not a name — the naming layer needs per-type variable
  selection.

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
