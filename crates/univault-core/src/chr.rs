//! Read-only parser for Titan Quest `Player.chr` character saves
//! (Anniversary Edition). Ported from `TQVaultAE`'s
//! `PlayerCollectionProvider.cs`, `SackCollectionProvider.cs` and
//! `ItemProvider.Serialization.cs` (MIT).
//!
//! The file is a key/value stream (see [`crate::reader`]) with two
//! landmark keys: `itemPositionsSavedAsGridCoords` opens the inventory
//! block, `useAlternate` opens the equipment block. Stacked items are
//! stored as repeated full item entries at grid position (-1,-1) and are
//! folded into the preceding item's stack count on parse.

use crate::reader::{ByteReader, Offset, ReadError, find_key};
use crate::writer::{write_cstring, write_keyed_i32};

/// The `i32` value stored after every `begin_block` key.
pub(crate) const BEGIN_BLOCK_VALUE: i32 = -1_340_212_530;
/// The `i32` value stored after every `end_block` key.
pub(crate) const END_BLOCK_VALUE: i32 = -559_038_242;

/// Path of a game database record (`records\...\something.dbr`), the
/// game's identifier for an item base, affix, or relic. Never empty —
/// absence is `Option<RecordId>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordId(String);

impl RecordId {
    /// `None` when `raw` is empty or whitespace-only, which is how the
    /// save formats spell "no record here". Surrounding whitespace is
    /// trimmed, matching `TQVaultAE`'s `RecordId` constructor.
    #[must_use]
    pub fn parse(raw: String) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else if trimmed.len() == raw.len() {
            Some(Self(raw))
        } else {
            Some(Self(trimmed.to_string()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Final path segment without the `.dbr` extension — the closest
    /// thing to a display name until ARZ text resources are wired up.
    #[must_use]
    pub fn file_stem(&self) -> &str {
        let name = self.0.rsplit(['\\', '/']).next().unwrap_or(self.0.as_str());
        name.strip_suffix(".dbr").unwrap_or(name)
    }
}

/// The RNG seed rolled when an item dropped; fixes its stat rolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemSeed(i32);

impl ItemSeed {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self(seed)
    }

    #[must_use]
    pub fn value(self) -> i32 {
        self.0
    }
}

/// Item position in a sack's grid. (-1,-1) never survives parsing — it
/// marks a stack continuation entry and is folded away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

/// Second relic slot, present only on items forged after the Atlantis
/// expansion; its three fields travel together in the save format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtlantisRelic {
    pub relic: Option<RecordId>,
    pub bonus: Option<RecordId>,
    pub var2: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub base: RecordId,
    pub prefix: Option<RecordId>,
    pub suffix: Option<RecordId>,
    pub relic: Option<RecordId>,
    pub relic_bonus: Option<RecordId>,
    pub seed: ItemSeed,
    pub var1: i32,
    pub atlantis: Option<AtlantisRelic>,
    pub position: GridPos,
    pub stack_size: u32,
    /// Per-member data of the folded stack members after the first
    /// (player-sack stacks store each member as a full item entry with
    /// its own `seed` and, on Atlantis-era files, its own `var2`).
    /// Preserved so an unchanged sack re-encodes byte-identically;
    /// items born outside a chr parse leave it empty and encode by
    /// repeating the first member's values.
    pub(crate) folded_members: Vec<FoldedMember>,
}

impl Item {
    /// A bare, unstacked item at the grid origin — the shape items
    /// have when born outside a parsed file (shells, tests).
    #[must_use]
    pub fn bare(base: RecordId, seed: ItemSeed) -> Self {
        Self {
            base,
            prefix: None,
            suffix: None,
            relic: None,
            relic_bonus: None,
            seed,
            var1: 0,
            atlantis: None,
            position: GridPos { x: 0, y: 0 },
            stack_size: 1,
            folded_members: Vec::new(),
        }
    }
}

/// The varying fields of one folded stack member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FoldedMember {
    pub(crate) seed: i32,
    /// `None` when the member entry had no Atlantis fields.
    pub(crate) var2: Option<i32>,
}

/// One inventory bag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sack {
    pub items: Vec<Item>,
    /// The `tempBool` header value, preserved verbatim for
    /// byte-identical re-encoding.
    pub(crate) temp_bool: i32,
}

/// Fixed equipment slot count for Anniversary Edition (Immortal Throne
/// layout: 11 gear slots plus the artifact).
pub const EQUIPMENT_SLOTS: usize = 12;

/// Grid size of inventory sack `index` (`TQVaultAE`'s `PlayerPanel`:
/// the main sack is 12×5, the extra bags 8×5).
#[must_use]
pub fn sack_dimensions(index: usize) -> (i32, i32) {
    if index == 0 { (12, 5) } else { (8, 5) }
}

/// Worn equipment; a `None` slot is empty (stored as a dummy item with
/// an empty `baseName`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equipment {
    pub slots: [Option<Item>; EQUIPMENT_SLOTS],
}

/// Header facts read for display. `name`/`class_tag` are optional in
/// the format (a class reset blanks the tag); level and money are
/// always present in real saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub name: Option<String>,
    pub class_tag: Option<String>,
    pub level: i32,
    pub money: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerCharacter {
    pub info: PlayerInfo,
    pub sacks: Vec<Sack>,
    pub equipment: Equipment,
}

/// Errors from parsing a `Player.chr` file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("not a player save: landmark key \"{0}\" not found")]
    MissingSection(&'static str),
    #[error("invalid {what} count {count}")]
    InvalidCount { what: &'static str, count: i32 },
    #[error("sack item at {at} has an empty baseName")]
    EmptyBaseName { at: Offset },
    #[error("too many {what} to encode: {count}")]
    Overflow { what: &'static str, count: usize },
    #[error(transparent)]
    Read(#[from] ReadError),
}

/// Parses a `Player.chr` byte image into a read-only model.
///
/// # Errors
/// [`ParseError::MissingSection`] when the landmark keys are absent
/// (not a player save), otherwise the shape errors of the underlying
/// stream.
pub fn parse_player(data: &[u8]) -> Result<PlayerCharacter, ParseError> {
    Ok(PlayerCharacter {
        info: parse_info(data)?,
        sacks: parse_inventory(data)?,
        equipment: parse_equipment(data)?,
    })
}

fn parse_info(data: &[u8]) -> Result<PlayerInfo, ParseError> {
    let name = find_key(data, "myPlayerName", 0)
        .map(|at| ByteReader::at(data, at).read_utf16_string())
        .transpose()?;
    let class_tag = find_key(data, "playerClassTag", 0)
        .map(|at| ByteReader::at(data, at).read_cstring())
        .transpose()?
        .filter(|tag| !tag.is_empty());
    let level = read_i32_at_key(data, "playerLevel")?;
    // Anchor past the class tag as TQVaultAE does: "money" is short
    // enough to collide with unrelated bytes earlier in the header.
    let money_from = find_key(data, "playerClassTag", 0).unwrap_or(0);
    let money_at =
        find_key(data, "money", money_from).ok_or(ParseError::MissingSection("money"))?;
    let money = ByteReader::at(data, money_at).read_i32()?;
    Ok(PlayerInfo {
        name,
        class_tag,
        level,
        money,
    })
}

fn read_i32_at_key(data: &[u8], key: &'static str) -> Result<i32, ParseError> {
    let at = find_key(data, key, 0).ok_or(ParseError::MissingSection(key))?;
    Ok(ByteReader::at(data, at).read_i32()?)
}

fn parse_inventory(data: &[u8]) -> Result<Vec<Sack>, ParseError> {
    let landmark = "itemPositionsSavedAsGridCoords";
    let value_at = find_key(data, landmark, 0).ok_or(ParseError::MissingSection(landmark))?;
    let mut reader = ByteReader::at(data, value_at);
    reader.read_i32()?;
    Ok(parse_sacks_block(&mut reader)?.sacks)
}

/// The inventory item block: `numberOfSacks` header plus the sacks.
/// Shared by `Player.chr` (after the grid-coords landmark) and legacy
/// binary vault files, which are exactly this block and nothing else.
pub(crate) struct SacksBlock {
    pub(crate) sacks: Vec<Sack>,
    pub(crate) focused_sack: i32,
    pub(crate) selected_sack: i32,
}

pub(crate) fn parse_sacks_block(reader: &mut ByteReader<'_>) -> Result<SacksBlock, ParseError> {
    reader.expect_key("numberOfSacks")?;
    let count = reader.read_i32()?;
    let count = usize::try_from(count).map_err(|_| ParseError::InvalidCount {
        what: "sack",
        count,
    })?;
    reader.expect_key("currentlyFocusedSackNumber")?;
    let focused_sack = reader.read_i32()?;
    reader.expect_key("currentlySelectedSackNumber")?;
    let selected_sack = reader.read_i32()?;

    let sacks = (0..count)
        .map(|_| parse_sack(reader))
        .collect::<Result<_, _>>()?;
    Ok(SacksBlock {
        sacks,
        focused_sack,
        selected_sack,
    })
}

fn parse_sack(reader: &mut ByteReader<'_>) -> Result<Sack, ParseError> {
    reader.expect_key("begin_block")?;
    reader.read_i32()?;
    reader.expect_key("tempBool")?;
    let temp_bool = reader.read_i32()?;
    reader.expect_key("size")?;
    let count = reader.read_i32()?;
    let count = usize::try_from(count).map_err(|_| ParseError::InvalidCount {
        what: "item",
        count,
    })?;

    let mut items: Vec<Item> = Vec::with_capacity(count);
    for _ in 0..count {
        let at = Offset(reader.pos());
        let raw = parse_raw_item(reader, ItemContext::PlayerSack)?;
        match items.last_mut() {
            Some(last) if raw.is_stack_continuation() => {
                last.stack_size += 1;
                last.folded_members.push(FoldedMember {
                    seed: raw.seed,
                    var2: raw.atlantis.as_ref().map(|(_, _, var2)| *var2),
                });
            }
            _ => items.push(raw.into_item().ok_or(ParseError::EmptyBaseName { at })?),
        }
    }

    reader.expect_key("end_block")?;
    reader.read_i32()?;
    Ok(Sack { items, temp_bool })
}

fn parse_equipment(data: &[u8]) -> Result<Equipment, ParseError> {
    let value_at =
        find_key(data, "useAlternate", 0).ok_or(ParseError::MissingSection("useAlternate"))?;
    let mut reader = ByteReader::at(data, value_at);
    reader.read_i32()?;
    reader.expect_key("equipmentCtrlIOStreamVersion")?;
    reader.read_i32()?;

    let mut slots: [Option<Item>; EQUIPMENT_SLOTS] = std::array::from_fn(|_| None);
    for (index, slot) in slots.iter_mut().enumerate() {
        // Weapon-set slots (7..=8 primary, 9..=10 alternate) are wrapped
        // in an extra block carrying the "alternate" flag.
        if index == 7 || index == 9 {
            reader.expect_key("begin_block")?;
            reader.read_i32()?;
            reader.expect_key("alternate")?;
            reader.read_i32()?;
        }
        let raw = parse_raw_item(&mut reader, ItemContext::Equipment)?;
        reader.expect_key("itemAttached")?;
        reader.read_i32()?;
        *slot = raw.into_item();
        if index == 8 || index == 10 {
            reader.expect_key("end_block")?;
            reader.read_i32()?;
        }
    }

    reader.expect_key("end_block")?;
    reader.read_i32()?;
    Ok(Equipment { slots })
}

/// Where an item entry is being parsed from; the surrounding keys
/// differ per container. Stash callers read the leading `stackCount`
/// themselves before calling [`parse_raw_item`].
#[derive(Clone, Copy)]
pub(crate) enum ItemContext {
    PlayerSack,
    Equipment,
    Stash,
}

/// Item fields exactly as stored, before empty-`baseName` and stacking
/// semantics are applied.
pub(crate) struct RawItem {
    base: String,
    prefix: String,
    suffix: String,
    relic: String,
    relic_bonus: String,
    seed: i32,
    var1: i32,
    atlantis: Option<(String, String, i32)>,
    position: GridPos,
}

impl RawItem {
    /// Stack members after the first are stored at (-1,-1).
    fn is_stack_continuation(&self) -> bool {
        self.position == GridPos { x: -1, y: -1 }
    }

    /// `None` when `baseName` is empty — an empty equipment slot.
    pub(crate) fn into_item(self) -> Option<Item> {
        let base = RecordId::parse(self.base)?;
        Some(Item {
            base,
            prefix: RecordId::parse(self.prefix),
            suffix: RecordId::parse(self.suffix),
            relic: RecordId::parse(self.relic),
            relic_bonus: RecordId::parse(self.relic_bonus),
            seed: ItemSeed::new(self.seed),
            var1: self.var1,
            atlantis: self.atlantis.map(|(relic, bonus, var2)| AtlantisRelic {
                relic: RecordId::parse(relic),
                bonus: RecordId::parse(bonus),
                var2,
            }),
            position: self.position,
            stack_size: 1,
            folded_members: Vec::new(),
        })
    }
}

pub(crate) fn parse_raw_item(
    reader: &mut ByteReader<'_>,
    context: ItemContext,
) -> Result<RawItem, ParseError> {
    match context {
        ItemContext::PlayerSack => {
            reader.expect_key("begin_block")?;
            reader.read_i32()?;
        }
        ItemContext::Equipment | ItemContext::Stash => {}
    }
    reader.expect_key("begin_block")?;
    reader.read_i32()?;

    reader.expect_key("baseName")?;
    let base = reader.read_cstring()?;
    reader.expect_key("prefixName")?;
    let prefix = reader.read_cstring()?;
    reader.expect_key("suffixName")?;
    let suffix = reader.read_cstring()?;
    reader.expect_key("relicName")?;
    let relic = reader.read_cstring()?;
    reader.expect_key("relicBonus")?;
    let relic_bonus = reader.read_cstring()?;
    reader.expect_key("seed")?;
    let seed = reader.read_i32()?;
    reader.expect_key("var1")?;
    let var1 = reader.read_i32()?;

    let atlantis = if reader.next_key_is("relicName2") {
        reader.expect_key("relicName2")?;
        let relic2 = reader.read_cstring()?;
        reader.expect_key("relicBonus2")?;
        let bonus2 = reader.read_cstring()?;
        reader.expect_key("var2")?;
        let var2 = reader.read_i32()?;
        Some((relic2, bonus2, var2))
    } else {
        None
    };

    reader.expect_key("end_block")?;
    reader.read_i32()?;

    let position = match context {
        ItemContext::PlayerSack => {
            reader.expect_key("pointX")?;
            let x = reader.read_i32()?;
            reader.expect_key("pointY")?;
            let y = reader.read_i32()?;
            reader.expect_key("end_block")?;
            reader.read_i32()?;
            GridPos { x, y }
        }
        ItemContext::Equipment => GridPos { x: 0, y: 0 },
        ItemContext::Stash => {
            reader.expect_key("xOffset")?;
            let x = reader.read_f32()?;
            reader.expect_key("yOffset")?;
            let y = reader.read_f32()?;
            GridPos {
                x: offset_to_cell(x),
                y: offset_to_cell(y),
            }
        }
    };

    Ok(RawItem {
        base,
        prefix,
        suffix,
        relic,
        relic_bonus,
        seed,
        var1,
        atlantis,
        position,
    })
}

// Stash grids store whole-number cells as floats; C# reads them with
// Convert.ToInt32 (rounding), so truncation cannot occur in range.
#[allow(clippy::cast_possible_truncation)]
fn offset_to_cell(offset: f32) -> i32 {
    offset.round() as i32
}

/// Rebuilds the inventory item block from `sacks` and copies every
/// other byte of `original` through untouched — the targeted-splice
/// rule in ARCHITECTURE.md. The focused/selected sack numbers are
/// carried over from `original` verbatim. Never writes to disk; the
/// shell owns file IO and the backup-first step.
///
/// # Errors
/// The parse errors of locating the original block, or
/// [`ParseError::Overflow`] on absurd sack counts.
pub fn replace_inventory(original: &[u8], sacks: &[Sack]) -> Result<Vec<u8>, ParseError> {
    let landmark = "itemPositionsSavedAsGridCoords";
    let value_at = find_key(original, landmark, 0).ok_or(ParseError::MissingSection(landmark))?;
    let mut reader = ByteReader::at(original, value_at);
    reader.read_i32()?;
    let block_start = reader.pos();
    let block = parse_sacks_block(&mut reader)?;
    let block_end = reader.pos();

    let mut encoded = Vec::new();
    write_keyed_i32(
        &mut encoded,
        "numberOfSacks",
        encodable_count(sacks.len(), "sacks")?,
    );
    write_keyed_i32(
        &mut encoded,
        "currentlyFocusedSackNumber",
        block.focused_sack,
    );
    write_keyed_i32(
        &mut encoded,
        "currentlySelectedSackNumber",
        block.selected_sack,
    );
    for sack in sacks {
        encode_sack(&mut encoded, sack)?;
    }

    let mut out = Vec::with_capacity(original.len() - (block_end - block_start) + encoded.len());
    out.extend_from_slice(&original[..block_start]);
    out.extend_from_slice(&encoded);
    out.extend_from_slice(&original[block_end..]);
    Ok(out)
}

fn encodable_count(count: usize, what: &'static str) -> Result<i32, ParseError> {
    i32::try_from(count).map_err(|_| ParseError::Overflow { what, count })
}

/// Patches the character's `money` value in place — a four-byte edit
/// at the same anchored location the parser reads, leaving every
/// other byte untouched.
///
/// # Errors
/// [`ParseError::MissingSection`] when the save has no `money` key.
pub fn replace_money(original: &[u8], money: i32) -> Result<Vec<u8>, ParseError> {
    let anchor = find_key(original, "playerClassTag", 0).unwrap_or(0);
    let value_at =
        find_key(original, "money", anchor).ok_or(ParseError::MissingSection("money"))?;
    let mut out = original.to_vec();
    out.get_mut(value_at..value_at + 4)
        .ok_or(ParseError::Read(ReadError::UnexpectedEof {
            at: Offset(value_at),
            wanted: 4,
        }))?
        .copy_from_slice(&money.to_le_bytes());
    Ok(out)
}

fn encode_sack(buf: &mut Vec<u8>, sack: &Sack) -> Result<(), ParseError> {
    write_keyed_i32(buf, "begin_block", BEGIN_BLOCK_VALUE);
    write_keyed_i32(buf, "tempBool", sack.temp_bool);
    // "size" counts item entries including the folded stack members.
    let entries: usize = sack
        .items
        .iter()
        .map(|item| item.stack_size.max(1) as usize)
        .sum();
    write_keyed_i32(buf, "size", encodable_count(entries, "sack item entries")?);
    for item in &sack.items {
        encode_sack_item(buf, item);
    }
    write_keyed_i32(buf, "end_block", END_BLOCK_VALUE);
    Ok(())
}

/// Encodes one inventory item as its full stack: members after the
/// first repeat the item at position (-1,-1), with their preserved
/// per-member `seed`/`var2` where the item came from a chr parse, or
/// the first member's values otherwise (`TQVaultAE` rolls random
/// seeds here; potions are the only stackables and carry no rolls).
pub(crate) fn encode_sack_item(buf: &mut Vec<u8>, item: &Item) {
    let members = item.stack_size.max(1);
    for index in 0..members {
        let (seed, var2, position) = if index == 0 {
            (item.seed.value(), None, item.position)
        } else {
            let folded = usize::try_from(index - 1)
                .ok()
                .and_then(|member| item.folded_members.get(member).copied());
            (
                folded.map_or_else(|| item.seed.value(), |member| member.seed),
                folded.and_then(|member| member.var2),
                GridPos { x: -1, y: -1 },
            )
        };
        write_keyed_i32(buf, "begin_block", BEGIN_BLOCK_VALUE);
        encode_item_body_with(buf, item, seed, var2);
        write_keyed_i32(buf, "pointX", position.x);
        write_keyed_i32(buf, "pointY", position.y);
        write_keyed_i32(buf, "end_block", END_BLOCK_VALUE);
    }
}

/// The inner item block shared by every container format.
pub(crate) fn encode_item_body(buf: &mut Vec<u8>, item: &Item, seed: i32) {
    encode_item_body_with(buf, item, seed, None);
}

fn encode_item_body_with(buf: &mut Vec<u8>, item: &Item, seed: i32, var2_override: Option<i32>) {
    fn raw(id: Option<&RecordId>) -> &str {
        id.map_or("", RecordId::as_str)
    }
    write_keyed_i32(buf, "begin_block", BEGIN_BLOCK_VALUE);
    write_cstring(buf, "baseName");
    write_cstring(buf, item.base.as_str());
    write_cstring(buf, "prefixName");
    write_cstring(buf, raw(item.prefix.as_ref()));
    write_cstring(buf, "suffixName");
    write_cstring(buf, raw(item.suffix.as_ref()));
    write_cstring(buf, "relicName");
    write_cstring(buf, raw(item.relic.as_ref()));
    write_cstring(buf, "relicBonus");
    write_cstring(buf, raw(item.relic_bonus.as_ref()));
    write_keyed_i32(buf, "seed", seed);
    write_keyed_i32(buf, "var1", item.var1);
    if let Some(second) = &item.atlantis {
        write_cstring(buf, "relicName2");
        write_cstring(buf, second.relic.as_ref().map_or("", RecordId::as_str));
        write_cstring(buf, "relicBonus2");
        write_cstring(buf, second.bonus.as_ref().map_or("", RecordId::as_str));
        write_keyed_i32(buf, "var2", var2_override.unwrap_or(second.var2));
    }
    write_keyed_i32(buf, "end_block", END_BLOCK_VALUE);
}

/// Builds save-format byte images in the exact shape `TQVaultAE`
/// writes them, so parsers meet the same layout as in real files.
/// Shared with the vault module's legacy-import tests.
#[cfg(test)]
pub(crate) mod fixture {
    #[derive(Default)]
    pub(crate) struct Fixture {
        pub(crate) bytes: Vec<u8>,
    }

    pub(crate) const BEGIN_BLOCK: i32 = -1_340_212_530;
    pub(crate) const END_BLOCK: i32 = -559_038_242;

    impl Fixture {
        pub(crate) fn key(mut self, key: &str) -> Self {
            self.bytes
                .extend_from_slice(&i32::try_from(key.len()).unwrap().to_le_bytes());
            self.bytes.extend_from_slice(key.as_bytes());
            self
        }

        pub(crate) fn int(mut self, value: i32) -> Self {
            self.bytes.extend_from_slice(&value.to_le_bytes());
            self
        }

        pub(crate) fn cstr(self, key: &str, value: &str) -> Self {
            self.key(key).key(value)
        }

        pub(crate) fn utf16(mut self, key: &str, value: &str) -> Self {
            self = self.key(key);
            let units: Vec<u16> = value.encode_utf16().collect();
            self.bytes
                .extend_from_slice(&i32::try_from(units.len()).unwrap().to_le_bytes());
            for unit in units {
                self.bytes.extend_from_slice(&unit.to_le_bytes());
            }
            self
        }

        pub(crate) fn keyed_int(self, key: &str, value: i32) -> Self {
            self.key(key).int(value)
        }

        pub(crate) fn keyed_f32(mut self, key: &str, value: f32) -> Self {
            self = self.key(key);
            self.bytes.extend_from_slice(&value.to_le_bytes());
            self
        }

        pub(crate) fn begin_block(self) -> Self {
            self.keyed_int("begin_block", BEGIN_BLOCK)
        }

        pub(crate) fn end_block(self) -> Self {
            self.keyed_int("end_block", END_BLOCK)
        }

        pub(crate) fn item_body(self, base: &str, seed: i32) -> Self {
            self.begin_block()
                .cstr("baseName", base)
                .cstr("prefixName", "")
                .cstr("suffixName", "")
                .cstr("relicName", "")
                .cstr("relicBonus", "")
                .keyed_int("seed", seed)
                .keyed_int("var1", 0)
                .end_block()
        }

        pub(crate) fn sack_item(self, base: &str, seed: i32, x: i32, y: i32) -> Self {
            self.begin_block()
                .item_body(base, seed)
                .keyed_int("pointX", x)
                .keyed_int("pointY", y)
                .end_block()
        }

        pub(crate) fn equipment_slot(self, base: &str) -> Self {
            self.item_body(base, 1)
                .keyed_int("itemAttached", i32::from(!base.is_empty()))
        }

        pub(crate) fn atlantis_sack_item(
            self,
            base: &str,
            seed: i32,
            var2: i32,
            x: i32,
            y: i32,
        ) -> Self {
            self.begin_block()
                .begin_block()
                .cstr("baseName", base)
                .cstr("prefixName", "")
                .cstr("suffixName", "")
                .cstr("relicName", "")
                .cstr("relicBonus", "")
                .keyed_int("seed", seed)
                .keyed_int("var1", 0)
                .cstr("relicName2", "")
                .cstr("relicBonus2", "")
                .keyed_int("var2", var2)
                .end_block()
                .keyed_int("pointX", x)
                .keyed_int("pointY", y)
                .end_block()
        }

        pub(crate) fn stash_item(
            self,
            base: &str,
            seed: i32,
            stack_count: i32,
            x: f32,
            y: f32,
        ) -> Self {
            self.keyed_int("stackCount", stack_count)
                .item_body(base, seed)
                .keyed_f32("xOffset", x)
                .keyed_f32("yOffset", y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Fixture;
    use super::*;

    fn player_file() -> Vec<u8> {
        let mut fixture = Fixture::default()
            .utf16("myPlayerName", "Ajax")
            .cstr("playerClassTag", "tagCClass01")
            .keyed_int("playerLevel", 17)
            .keyed_int("money", 12_345)
            .begin_block()
            .keyed_int("itemPositionsSavedAsGridCoords", 1)
            .keyed_int("numberOfSacks", 2)
            .keyed_int("currentlyFocusedSackNumber", 0)
            .keyed_int("currentlySelectedSackNumber", 0)
            // Sack 0: a sword, then a potion stack of three.
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 4)
            .sack_item("records\\item\\equipmentweapon\\sword_01.dbr", 11, 2, 3)
            .sack_item("records\\item\\potion\\healthpotion.dbr", 21, 5, 0)
            .sack_item("records\\item\\potion\\healthpotion.dbr", 22, -1, -1)
            .sack_item("records\\item\\potion\\healthpotion.dbr", 23, -1, -1)
            .end_block()
            // Sack 1: empty.
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 0)
            .end_block()
            // Equipment: helmet in slot 0, weapon in slot 7, rest empty.
            .begin_block()
            .keyed_int("useAlternate", 0)
            .keyed_int("equipmentCtrlIOStreamVersion", 0);

        for slot in 0..EQUIPMENT_SLOTS {
            if slot == 7 || slot == 9 {
                fixture = fixture
                    .begin_block()
                    .keyed_int("alternate", i32::from(slot == 9));
            }
            fixture = match slot {
                0 => fixture.equipment_slot("records\\item\\equipmenthelm\\helm_01.dbr"),
                7 => fixture.equipment_slot("records\\item\\equipmentweapon\\club_01.dbr"),
                _ => fixture.equipment_slot(""),
            };
            if slot == 8 || slot == 10 {
                fixture = fixture.end_block();
            }
        }
        fixture.end_block().end_block().bytes
    }

    #[test]
    fn parses_player_info() {
        let character = parse_player(&player_file()).unwrap();
        assert_eq!(character.info.name.as_deref(), Some("Ajax"));
        assert_eq!(character.info.class_tag.as_deref(), Some("tagCClass01"));
        assert_eq!(character.info.level, 17);
        assert_eq!(character.info.money, 12_345);
    }

    #[test]
    fn parses_sacks_and_folds_stacks() {
        let character = parse_player(&player_file()).unwrap();
        assert_eq!(character.sacks.len(), 2);

        let sack = &character.sacks[0];
        assert_eq!(sack.items.len(), 2);
        assert_eq!(sack.items[0].base.file_stem(), "sword_01");
        assert_eq!(sack.items[0].position, GridPos { x: 2, y: 3 });
        assert_eq!(sack.items[0].stack_size, 1);
        assert_eq!(sack.items[1].base.file_stem(), "healthpotion");
        assert_eq!(sack.items[1].stack_size, 3);

        assert!(character.sacks[1].items.is_empty());
    }

    #[test]
    fn parses_equipment_slots() {
        let character = parse_player(&player_file()).unwrap();
        let occupied: Vec<usize> = character
            .equipment
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|_| index))
            .collect();
        assert_eq!(occupied, vec![0, 7]);
        assert_eq!(
            character.equipment.slots[7]
                .as_ref()
                .unwrap()
                .base
                .file_stem(),
            "club_01"
        );
    }

    #[test]
    fn atlantis_second_relic_is_detected_by_lookahead() {
        let bytes = Fixture::default()
            .begin_block()
            .begin_block()
            .cstr("baseName", "records\\item\\ring.dbr")
            .cstr("prefixName", "")
            .cstr("suffixName", "")
            .cstr("relicName", "")
            .cstr("relicBonus", "")
            .keyed_int("seed", 9)
            .keyed_int("var1", 0)
            .cstr("relicName2", "records\\item\\relic2.dbr")
            .cstr("relicBonus2", "")
            .keyed_int("var2", 4)
            .end_block()
            .keyed_int("pointX", 0)
            .keyed_int("pointY", 0)
            .end_block()
            .bytes;
        let mut reader = ByteReader::new(&bytes);
        let item = parse_raw_item(&mut reader, ItemContext::PlayerSack)
            .unwrap()
            .into_item()
            .unwrap();
        let atlantis = item.atlantis.unwrap();
        assert_eq!(atlantis.relic.unwrap().file_stem(), "relic2");
        assert_eq!(atlantis.var2, 4);
    }

    #[test]
    fn missing_landmark_reports_not_a_player_save() {
        assert_eq!(
            parse_player(b"not a save file"),
            Err(ParseError::MissingSection("playerLevel")),
        );
    }

    #[test]
    fn unchanged_inventory_resplices_byte_identically() {
        let original = player_file();
        let character = parse_player(&original).unwrap();
        let respliced = replace_inventory(&original, &character.sacks).unwrap();
        assert!(
            respliced == original,
            "splice of unchanged sacks altered bytes (len {} -> {})",
            original.len(),
            respliced.len()
        );
    }

    #[test]
    fn removing_an_item_leaves_the_rest_of_the_file_untouched() {
        let original = player_file();
        let mut character = parse_player(&original).unwrap();
        let removed = character.sacks[0].items.remove(0);
        assert_eq!(removed.base.file_stem(), "sword_01");

        let modified = replace_inventory(&original, &character.sacks).unwrap();
        let reparsed = parse_player(&modified).unwrap();
        assert_eq!(reparsed.sacks[0].items.len(), 1);
        assert_eq!(reparsed.sacks[0].items[0].stack_size, 3);
        assert_eq!(reparsed.equipment, character.equipment);
        assert_eq!(reparsed.info, character.info);

        // Everything after the inventory block (equipment onward) must
        // be the original bytes: both files end with the same tail.
        let equipment_key = crate::reader::find_key(&original, "useAlternate", 0).unwrap();
        let modified_key = crate::reader::find_key(&modified, "useAlternate", 0).unwrap();
        assert_eq!(original[equipment_key..], modified[modified_key..]);
    }

    #[test]
    fn stack_grown_outside_a_parse_encodes_with_repeated_seed() {
        let original = player_file();
        let mut character = parse_player(&original).unwrap();
        character.sacks[1].items.push(Item {
            base: RecordId::parse("records\\item\\potion\\manapotion.dbr".to_string()).unwrap(),
            prefix: None,
            suffix: None,
            relic: None,
            relic_bonus: None,
            seed: ItemSeed::new(99),
            var1: 0,
            atlantis: None,
            position: GridPos { x: 1, y: 1 },
            stack_size: 3,
            folded_members: Vec::new(),
        });
        let modified = replace_inventory(&original, &character.sacks).unwrap();
        let reparsed = parse_player(&modified).unwrap();
        let potion = &reparsed.sacks[1].items[0];
        assert_eq!(potion.stack_size, 3);
        assert_eq!(
            potion.folded_members,
            vec![
                FoldedMember {
                    seed: 99,
                    var2: None
                };
                2
            ]
        );
    }

    #[test]
    fn replace_money_patches_exactly_four_bytes() {
        let original = player_file();
        let modified = replace_money(&original, 999_999).unwrap();
        assert_eq!(parse_player(&modified).unwrap().info.money, 999_999);
        assert_eq!(original.len(), modified.len());
        let differing = original
            .iter()
            .zip(&modified)
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing <= 4);
        // Everything else (name, level, items) is untouched.
        let before = parse_player(&original).unwrap();
        let after = parse_player(&modified).unwrap();
        assert_eq!(before.sacks, after.sacks);
        assert_eq!(before.info.level, after.info.level);
    }

    #[test]
    fn atlantis_stack_members_keep_their_own_var2() {
        let original = Fixture::default()
            .utf16("myPlayerName", "Ajax")
            .cstr("playerClassTag", "tagCClass01")
            .keyed_int("playerLevel", 1)
            .keyed_int("money", 0)
            .begin_block()
            .keyed_int("itemPositionsSavedAsGridCoords", 1)
            .keyed_int("numberOfSacks", 1)
            .keyed_int("currentlyFocusedSackNumber", 0)
            .keyed_int("currentlySelectedSackNumber", 0)
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 2)
            .atlantis_sack_item("records\\item\\potion\\scroll.dbr", 5, 111, 2, 0)
            .atlantis_sack_item("records\\item\\potion\\scroll.dbr", 6, 222, -1, -1)
            .end_block()
            .begin_block()
            .keyed_int("useAlternate", 0)
            .keyed_int("equipmentCtrlIOStreamVersion", 0)
            .bytes;
        // Equipment block is truncated, so splice only the inventory.
        let landmark_at = crate::reader::find_key(&original, "numberOfSacks", 0).unwrap();
        let mut reader = ByteReader::at(&original, landmark_at - 4 - "numberOfSacks".len());
        let block = parse_sacks_block(&mut reader).unwrap();
        assert_eq!(block.sacks[0].items[0].stack_size, 2);

        let respliced = replace_inventory(&original, &block.sacks).unwrap();
        assert!(
            respliced == original,
            "atlantis stack resplice altered bytes"
        );
    }

    #[test]
    fn truncated_file_reports_eof() {
        let full = player_file();
        let truncated = &full[..full.len() - 40];
        assert!(matches!(
            parse_player(truncated),
            Err(ParseError::Read(ReadError::UnexpectedEof { .. }))
        ));
    }
}
