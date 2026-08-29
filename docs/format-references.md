# Titan Quest file-format references

Outcome of the 2026-08-24 buy-vs-build parser survey. Verdict: **no
usable Rust code exists for any TQ format** — all parsers are
hand-written in `univault-core`, ported from the MIT-licensed
references below. Binding rules about provenance and dependencies
live in `.claude/rules/ARCHITECTURE.md` ("Parser provenance and
dependencies"); this file is the working reference map.

## Provenance rules (summary)

- **Primary reference for every format:**
  [TQVaultAE](https://github.com/EtienneLamoureux/TQVaultAE) (C#,
  **MIT**, active). Line-by-line porting is allowed; ported code
  preserves the MIT notice (Brandon Wallace 2006–2012 + TQVaultAE
  contributors).
- **GPL-3.0 references are eyes-only** — read to understand edge
  cases, never transcribe:
  [tqrespec](https://github.com/epinter/tqrespec) (Java, the most
  principled full-block-tree chr model; handles mobile save
  variants),
  [tqdatabase](https://github.com/epinter/tqdatabase) (Java, cleanest
  independent ARZ/ARC reader).
- Grim Dawn tooling is *structural* prior art only — shared Iron
  Lore lineage, but GD ARZ uses LZ4 (TQ uses zlib) and GD saves are
  XOR-obfuscated (TQ's are plaintext):
  [dandels/gdlc](https://github.com/dandels/gdlc) (Rust, MIT, best
  Rust prior art), [gregates/lib-gddb](https://github.com/gregates/lib-gddb)
  (Rust, **GPL** — design only).

## Per-format map

### Player.chr (character save)

- Port from: TQVaultAE `TQVaultAE.Data/PlayerCollectionProvider.cs`,
  `SackCollectionProvider.cs`, `TQDataService.cs`.
- Whole-file record map ("executable documentation"): the bundled
  `src/TQSaveFilesExplorer` — `Entities/TQFile.cs`,
  `TQFileRecord.cs`, `TQFileDataType.cs` (length-prefixed keys;
  types Int, Float, String1252, StringUTF16, ByteArrayVar,
  ByteArray16 UIDs; version-dependent typing).
- Spec cross-check: community field spreadsheet
  <https://docs.google.com/spreadsheets/d/1cdeicva4q0Uqp7t3RK4XFEN4wHNJXfcyG16N_AQCv_I>
  (block magics: `begin_block` = 0xB01DFACE, `end_block` =
  0xDEADC0DE; covers TQ / TQIT / TQAE versions).
- Approach (binding): key-scan + **targeted splice** — locate ASCII
  keys (`numberOfSacks`, `currentlyFocusedSackNumber`,
  `itemPositionsSavedAsGridCoords`, `useAlternate`, `playerLevel`,
  `money`, `masteriesAllowed`), read typed values after them, and on
  write splice only the edited blocks. Never re-serialize the whole
  file.

### Transfer stash (winsys.dxb / .dxg)

- Port from: TQVaultAE `TQVaultAE.Data/StashProvider.cs`.
- Layout: leading 4-byte CRC32 checksum (lookup-table
  implementation), then `begin_block`, `stashVersion` (i32), `fName`
  (length-prefixed), `sackWidth`, `sackHeight`, item sacks. On save
  TQVaultAE writes the `.dxg` backup twin (`EncodeBackupFile()`).

### Item binary encoding (inside saves/stash/legacy vaults)

- Port from: TQVaultAE `TQVaultAE.Data/ItemProvider.Serialization.cs`.
- Field order: `baseName, prefixName, suffixName, relicName,
  relicBonus, seed(i32), var1(i32), [relicName2, relicBonus2, var2 —
  Atlantis], position` (pointX/pointY in sacks, xOffset/yOffset in
  stash), wrapped in begin/end_block.

### ARZ (game record database, read-only)

- Port from: TQVaultAE `TQVaultAE.Data/ArzFileProvider.cs`,
  `RecordInfoProvider.cs`; zlib decompression per record
  (`DeflateDecompressionService.cs`) — use `flate2`.
- Layout: header = 6 × i32 at 0x00–0x14 (unknown/version,
  record-table offset, record-table size, record count, string-table
  offset, string-table size); string table = i32 count +
  null-terminated C-strings; record-table entries are 24 bytes.
- Second opinion + only MIT ARZ *writer* (out of scope for now):
  [ByteSquire/TQArchive-Wrapper](https://github.com/ByteSquire/TQArchive-Wrapper)
  (C#, MIT, archived).
- Record *semantics* (which DBR fields matter for equipment, affixes,
  loot tables): [fonsleenaars/tqdb](https://github.com/fonsleenaars/tqdb)
  (Python, MIT).

### ARC (resource archives, read-only)

- Port from: TQVaultAE `TQVaultAE.Data/ArcFileProvider.cs`.
- Layout: magic "ARC" at 0x00; file-entry count at 0x08;
  compressed-part count at 0x0C; ToC pointer at 0x18. ToC = 12-byte
  part entries (offset, compressed size, uncompressed size), then
  null-terminated ASCII filenames, then 44-byte file records read
  from EOF backward. `storageType` 3 = zlib multi-part, 1 = stored.

### Vault files

- **Import/export interchange = TQVaultAE's JSON schema** (this app's
  own storage is the unified store file; see ARCHITECTURE.md).
  Schema source: `TQVaultAE.Domain/Dto/` —
  `VaultDto` `{disabledtooltip:[int], currentlyFocusedSackNumber,
  currentlySelectedSackNumber, sacks:[SackDto]}`; `ItemDto`
  `{stackSize, seed, baseName, prefixName, suffixName, relicName,
  relicBonus, var1, relicName2, relicBonus2, var2, pointX, pointY,
  width, height}`. Field names are the wire contract.
- Legacy binary `.vault` (same key/block encoding as save sacks):
  **import-only**; TQVaultAE auto-converts on load
  (`VaultService.cs`).
- Share/export JSON (clipboard / file / PasteBin):
  `TQVaultAE.Services/VaultExportDTO.cs`, `TabExportDTO.cs`.

## Behavior worth mirroring

- Backups before every game-file write
  (`TQVaultAE.Services/GameFileService.cs`): copy to a backup dir
  before touching the original; stash backed up as the `.dxb`+`.dxg`
  pair. TQVaultAE does no rotation/pruning.
- TQVaultAE renames the game's own `Backup` folder to stop the game
  re-reading corrupt content (their issue #535) — evaluate whether
  we want this behavior when we get there.

## Ruled out

- `Tamschi/serde_titan-quest` — abandoned unlicensed skeleton, the
  only TQ-Rust repo found.
- `binrw`/`deku` as parsing foundation — declined; poor fit for the
  key-scan chr format and unused write derive under the splice
  strategy.
- `nom`/`winnow` — read-only combinators leave the write path
  unimplemented.
- GD Stash (no public repo/license), `gshearer/tqvaultc`
  (self-described AI-generated, author warns against use).
