//! Vault files — this project's item storage, byte-compatible with
//! `TQVaultAE`'s modern JSON vault format so both tools open the same
//! vaults (see ARCHITECTURE.md "External boundaries").
//!
//! The wire schema is `TQVaultAE`'s `VaultDto`/`SackDto`/`ItemDto`
//! serialized with `IncludeFields`, verbatim member names, and indented
//! output: empty record ids are written as `""` (never null), absent
//! objects as `null`, and non-Atlantis items carry
//! [`VAR2_DEFAULT`] in `var2`. Unknown JSON fields are preserved
//! through a read/write round trip so newer `TQVaultAE` data is never
//! silently dropped. Legacy binary `.vault` files (the pre-JSON format:
//! a bare inventory item block) are import-only.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::chr::{AtlantisRelic, GridPos, Item, ItemSeed, ParseError, RecordId};
use crate::reader::ByteReader;

/// `var2` value `TQVaultAE` writes for items without an Atlantis
/// second relic (`Item.var2Default`).
pub const VAR2_DEFAULT: i32 = 2_035_248;

/// A vault: tabs ("sacks") of stored items.
#[derive(Debug, Clone, PartialEq)]
pub struct Vault {
    /// Tooltip-disabled bag ids, preserved verbatim (`null` and `[]`
    /// are distinct on the wire).
    pub disabled_tooltips: Option<Vec<i32>>,
    pub focused_sack: i32,
    pub selected_sack: i32,
    pub sacks: Vec<VaultSack>,
    extra: Map<String, Value>,
}

/// One vault tab.
#[derive(Debug, Clone, PartialEq)]
pub struct VaultSack {
    pub items: Vec<VaultItem>,
    /// `TQVaultAE`'s bag-button icon settings, preserved verbatim and
    /// not interpreted here.
    pub icon_info: Option<Value>,
    extra: Map<String, Value>,
}

/// A stored item: the item itself plus the grid footprint the vault
/// format records (`TQVaultAE` fills it from game data; legacy imports
/// carry 0×0 until an ARZ lookup exists).
#[derive(Debug, Clone, PartialEq)]
pub struct VaultItem {
    pub item: Item,
    pub width: i32,
    pub height: i32,
    extra: Map<String, Value>,
}

/// Errors from reading or writing a vault file.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("invalid vault JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("item {index} in sack {sack} has an empty baseName")]
    EmptyBaseName { sack: usize, index: usize },
    #[error("item {index} in sack {sack} has negative stackSize {value}")]
    NegativeStackSize {
        sack: usize,
        index: usize,
        value: i32,
    },
    #[error("legacy vault: {0}")]
    Legacy(#[from] ParseError),
}

impl Vault {
    /// Parses a modern (JSON) vault file.
    ///
    /// # Errors
    /// [`VaultError::Json`] on malformed JSON, otherwise the item-level
    /// validation errors.
    pub fn from_json(json: &str) -> Result<Self, VaultError> {
        let dto: VaultDto = serde_json::from_str(json)?;
        let sacks = dto
            .sacks
            .into_iter()
            .enumerate()
            .map(|(sack, dto)| sack_from_dto(dto, sack))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            disabled_tooltips: dto.disabled_tooltips,
            focused_sack: dto.focused_sack,
            selected_sack: dto.selected_sack,
            sacks,
            extra: dto.extra,
        })
    }

    /// Imports a legacy binary `.vault` file — a bare inventory item
    /// block (`numberOfSacks` onward), exactly the structure inside a
    /// `Player.chr`. Import-only per ARCHITECTURE.md; grid footprints
    /// come back 0×0.
    ///
    /// # Errors
    /// The chr item-block parse errors, wrapped in
    /// [`VaultError::Legacy`].
    pub fn from_legacy_binary(data: &[u8]) -> Result<Self, VaultError> {
        let mut reader = ByteReader::new(data);
        let block = crate::chr::parse_sacks_block(&mut reader)?;
        let sacks = block
            .sacks
            .into_iter()
            .map(|sack| VaultSack {
                items: sack
                    .items
                    .into_iter()
                    .map(|item| VaultItem {
                        item,
                        width: 0,
                        height: 0,
                        extra: Map::new(),
                    })
                    .collect(),
                icon_info: None,
                extra: Map::new(),
            })
            .collect();
        Ok(Self {
            disabled_tooltips: None,
            focused_sack: block.focused_sack,
            selected_sack: block.selected_sack,
            sacks,
            extra: Map::new(),
        })
    }

    /// Serializes to the `TQVaultAE`-compatible indented JSON form.
    ///
    /// # Errors
    /// [`VaultError::Json`] only on serializer failure, which the types
    /// here do not normally produce.
    pub fn to_json(&self) -> Result<String, VaultError> {
        let dto = VaultDto {
            disabled_tooltips: self.disabled_tooltips.clone(),
            focused_sack: self.focused_sack,
            selected_sack: self.selected_sack,
            sacks: self.sacks.iter().map(sack_to_dto).collect(),
            extra: self.extra.clone(),
        };
        Ok(serde_json::to_string_pretty(&dto)?)
    }
}

fn sack_from_dto(dto: SackDto, sack: usize) -> Result<VaultSack, VaultError> {
    let items = dto
        .items
        .into_iter()
        .enumerate()
        .map(|(index, dto)| item_from_dto(dto, sack, index))
        .collect::<Result<_, _>>()?;
    Ok(VaultSack {
        items,
        icon_info: dto.icon_info,
        extra: dto.extra,
    })
}

fn sack_to_dto(sack: &VaultSack) -> SackDto {
    SackDto {
        icon_info: sack.icon_info.clone(),
        items: sack.items.iter().map(item_to_dto).collect(),
        extra: sack.extra.clone(),
    }
}

fn item_from_dto(dto: ItemDto, sack: usize, index: usize) -> Result<VaultItem, VaultError> {
    let base = record(dto.base_name).ok_or(VaultError::EmptyBaseName { sack, index })?;
    let stack_size = u32::try_from(dto.stack_size)
        .map_err(|_| VaultError::NegativeStackSize {
            sack,
            index,
            value: dto.stack_size,
        })?
        .max(1);
    // TQVaultAE treats a non-empty relicName2 as the Atlantis marker;
    // relicBonus2/var2 are meaningless without it.
    let atlantis = record(dto.relic_name2).map(|relic| AtlantisRelic {
        relic: Some(relic),
        bonus: record(dto.relic_bonus2),
        var2: dto.var2,
    });
    Ok(VaultItem {
        item: Item {
            base,
            prefix: record(dto.prefix_name),
            suffix: record(dto.suffix_name),
            relic: record(dto.relic_name),
            relic_bonus: record(dto.relic_bonus),
            seed: ItemSeed::new(dto.seed),
            var1: dto.var1,
            atlantis,
            position: GridPos {
                x: dto.point_x,
                y: dto.point_y,
            },
            stack_size,
            folded_members: Vec::new(),
        },
        width: dto.width,
        height: dto.height,
        extra: dto.extra,
    })
}

fn item_to_dto(vault_item: &VaultItem) -> ItemDto {
    let item = &vault_item.item;
    let (relic_name2, relic_bonus2, var2) = match &item.atlantis {
        Some(second) => (
            raw(second.relic.as_ref()),
            raw(second.bonus.as_ref()),
            second.var2,
        ),
        None => (String::new(), String::new(), VAR2_DEFAULT),
    };
    ItemDto {
        stack_size: i32::try_from(item.stack_size).unwrap_or(i32::MAX),
        seed: item.seed.value(),
        base_name: Some(item.base.as_str().to_string()),
        prefix_name: Some(raw(item.prefix.as_ref())),
        suffix_name: Some(raw(item.suffix.as_ref())),
        relic_name: Some(raw(item.relic.as_ref())),
        relic_bonus: Some(raw(item.relic_bonus.as_ref())),
        var1: item.var1,
        relic_name2: Some(relic_name2),
        relic_bonus2: Some(relic_bonus2),
        var2,
        point_x: item.position.x,
        point_y: item.position.y,
        width: vault_item.width,
        height: vault_item.height,
        extra: vault_item.extra.clone(),
    }
}

fn record(raw: Option<String>) -> Option<RecordId> {
    RecordId::parse(raw.unwrap_or_default())
}

/// Empty record ids serialize as `""`, never null, matching
/// `TQVaultAE`'s `RecordId.Raw`.
fn raw(id: Option<&RecordId>) -> String {
    id.map(|id| id.as_str().to_string()).unwrap_or_default()
}

fn default_sack_number() -> i32 {
    -1
}

#[derive(Serialize, Deserialize)]
struct VaultDto {
    #[serde(rename = "disabledtooltip", default)]
    disabled_tooltips: Option<Vec<i32>>,
    #[serde(rename = "currentlyFocusedSackNumber", default = "default_sack_number")]
    focused_sack: i32,
    #[serde(
        rename = "currentlySelectedSackNumber",
        default = "default_sack_number"
    )]
    selected_sack: i32,
    #[serde(default)]
    sacks: Vec<SackDto>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct SackDto {
    #[serde(rename = "iconinfo", default)]
    icon_info: Option<Value>,
    #[serde(default)]
    items: Vec<ItemDto>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
struct ItemDto {
    #[serde(rename = "stackSize", default)]
    stack_size: i32,
    #[serde(default)]
    seed: i32,
    #[serde(rename = "baseName", default)]
    base_name: Option<String>,
    #[serde(rename = "prefixName", default)]
    prefix_name: Option<String>,
    #[serde(rename = "suffixName", default)]
    suffix_name: Option<String>,
    #[serde(rename = "relicName", default)]
    relic_name: Option<String>,
    #[serde(rename = "relicBonus", default)]
    relic_bonus: Option<String>,
    #[serde(default)]
    var1: i32,
    #[serde(rename = "relicName2", default)]
    relic_name2: Option<String>,
    #[serde(rename = "relicBonus2", default)]
    relic_bonus2: Option<String>,
    #[serde(default)]
    var2: i32,
    #[serde(rename = "pointX", default)]
    point_x: i32,
    #[serde(rename = "pointY", default)]
    point_y: i32,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chr::fixture::Fixture;

    const TQVAULTAE_SAMPLE: &str = r#"{
  "disabledtooltip": null,
  "currentlyFocusedSackNumber": 1,
  "currentlySelectedSackNumber": -1,
  "futureField": "kept",
  "sacks": [
    {
      "iconinfo": { "Icon": "custom" },
      "items": [
        {
          "stackSize": 3,
          "seed": 1234,
          "baseName": "records\\item\\potion\\healthpotion.dbr",
          "prefixName": "",
          "suffixName": "",
          "relicName": "",
          "relicBonus": "",
          "var1": 0,
          "relicName2": "",
          "relicBonus2": "",
          "var2": 2035248,
          "pointX": 2,
          "pointY": 0,
          "width": 1,
          "height": 1
        },
        {
          "stackSize": 1,
          "seed": 777,
          "baseName": "records\\item\\equipmentweapon\\sword_01.dbr",
          "prefixName": "records\\item\\affix\\sharp.dbr",
          "suffixName": "",
          "relicName": "",
          "relicBonus": "",
          "var1": 0,
          "relicName2": "records\\item\\relic\\anubis.dbr",
          "relicBonus2": "records\\item\\relicbonus\\str.dbr",
          "var2": 7,
          "pointX": 0,
          "pointY": 1,
          "width": 2,
          "height": 5
        }
      ]
    }
  ]
}"#;

    #[test]
    fn parses_tqvaultae_sample() {
        let vault = Vault::from_json(TQVAULTAE_SAMPLE).unwrap();
        assert_eq!(vault.disabled_tooltips, None);
        assert_eq!(vault.focused_sack, 1);
        assert_eq!(vault.sacks.len(), 1);

        let potion = &vault.sacks[0].items[0];
        assert_eq!(potion.item.base.file_stem(), "healthpotion");
        assert_eq!(potion.item.stack_size, 3);
        assert_eq!(potion.item.prefix, None);
        assert_eq!(potion.item.atlantis, None);

        let sword = &vault.sacks[0].items[1];
        assert_eq!(sword.item.prefix.as_ref().unwrap().file_stem(), "sharp");
        assert_eq!((sword.width, sword.height), (2, 5));
        let atlantis = sword.item.atlantis.as_ref().unwrap();
        assert_eq!(atlantis.relic.as_ref().unwrap().file_stem(), "anubis");
        assert_eq!(atlantis.var2, 7);
    }

    #[test]
    fn round_trip_preserves_unknown_fields_and_iconinfo() {
        let vault = Vault::from_json(TQVAULTAE_SAMPLE).unwrap();
        let rewritten = vault.to_json().unwrap();
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["futureField"], "kept");
        assert_eq!(value["sacks"][0]["iconinfo"]["Icon"], "custom");
        assert_eq!(Vault::from_json(&rewritten).unwrap(), vault);
    }

    #[test]
    fn writes_tqvaultae_wire_shape() {
        let vault = Vault::from_json(TQVAULTAE_SAMPLE).unwrap();
        let value: Value = serde_json::from_str(&vault.to_json().unwrap()).unwrap();
        let potion = &value["sacks"][0]["items"][0];
        assert_eq!(potion["prefixName"], "");
        assert_eq!(potion["relicName2"], "");
        assert_eq!(potion["var2"], VAR2_DEFAULT);
        assert_eq!(value["disabledtooltip"], Value::Null);
        for key in [
            "stackSize",
            "seed",
            "baseName",
            "prefixName",
            "suffixName",
            "relicName",
            "relicBonus",
            "var1",
            "relicName2",
            "relicBonus2",
            "var2",
            "pointX",
            "pointY",
            "width",
            "height",
        ] {
            assert!(potion.get(key).is_some(), "missing wire key {key}");
        }
    }

    #[test]
    fn null_and_missing_strings_read_as_absent() {
        let json = r#"{"sacks":[{"items":[{"baseName":"records\\a.dbr","prefixName":null,"seed":1,"stackSize":1}]}]}"#;
        let vault = Vault::from_json(json).unwrap();
        let item = &vault.sacks[0].items[0].item;
        assert_eq!(item.prefix, None);
        assert_eq!(item.suffix, None);
        assert_eq!(item.atlantis, None);
    }

    #[test]
    fn empty_base_name_is_rejected_with_location() {
        let json = r#"{"sacks":[{"items":[]},{"items":[{"baseName":" ","stackSize":1}]}]}"#;
        assert!(matches!(
            Vault::from_json(json),
            Err(VaultError::EmptyBaseName { sack: 1, index: 0 })
        ));
    }

    #[test]
    fn negative_stack_size_is_rejected() {
        let json = r#"{"sacks":[{"items":[{"baseName":"records\\a.dbr","stackSize":-2}]}]}"#;
        assert!(matches!(
            Vault::from_json(json),
            Err(VaultError::NegativeStackSize { value: -2, .. })
        ));
    }

    #[test]
    fn imports_legacy_binary_vault() {
        let bytes = Fixture::default()
            .keyed_int("numberOfSacks", 1)
            .keyed_int("currentlyFocusedSackNumber", 0)
            .keyed_int("currentlySelectedSackNumber", 0)
            .begin_block()
            .keyed_int("tempBool", 0)
            .keyed_int("size", 2)
            .sack_item("records\\item\\equipmentweapon\\sword_01.dbr", 5, 1, 1)
            .sack_item("records\\item\\potion\\healthpotion.dbr", 6, -1, -1)
            .end_block()
            .bytes;
        let vault = Vault::from_legacy_binary(&bytes).unwrap();
        assert_eq!(vault.sacks.len(), 1);
        let items = &vault.sacks[0].items;
        assert_eq!(items[0].item.base.file_stem(), "sword_01");
        assert_eq!((items[0].width, items[0].height), (0, 0));
    }
}
