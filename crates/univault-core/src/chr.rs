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
}

/// One inventory bag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sack {
    pub items: Vec<Item>,
}

/// Fixed equipment slot count for Anniversary Edition (Immortal Throne
/// layout: 11 gear slots plus the artifact).
pub const EQUIPMENT_SLOTS: usize = 12;

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
    reader.read_i32()?;
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
            Some(last) if raw.is_stack_continuation() => last.stack_size += 1,
            _ => items.push(raw.into_item().ok_or(ParseError::EmptyBaseName { at })?),
        }
    }

    reader.expect_key("end_block")?;
    reader.read_i32()?;
    Ok(Sack { items })
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
/// differ per container.
#[derive(Clone, Copy)]
enum ItemContext {
    PlayerSack,
    Equipment,
}

/// Item fields exactly as stored, before empty-`baseName` and stacking
/// semantics are applied.
struct RawItem {
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
    fn into_item(self) -> Option<Item> {
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
        })
    }
}

fn parse_raw_item(
    reader: &mut ByteReader<'_>,
    context: ItemContext,
) -> Result<RawItem, ParseError> {
    if let ItemContext::PlayerSack = context {
        reader.expect_key("begin_block")?;
        reader.read_i32()?;
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
    fn truncated_file_reports_eof() {
        let full = player_file();
        let truncated = &full[..full.len() - 40];
        assert!(matches!(
            parse_player(truncated),
            Err(ParseError::Read(ReadError::UnexpectedEof { .. }))
        ));
    }
}
