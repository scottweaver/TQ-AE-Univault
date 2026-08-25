//! Reader for the game's `database.arz` — the compressed record
//! database holding item stats, classifications, and the localization
//! tags that name things. Read-only reference data per ARCHITECTURE.md;
//! this module never writes. Ported from `TQVaultAE`'s
//! `ArzFileProvider.cs` and `RecordInfoProvider.cs` (MIT).
//!
//! Layout: a 24-byte header (six `i32`s: unknown, record-table start /
//! size / count, string-table start / size), a string table (`i32`
//! count, then length-prefixed strings), and a record table whose
//! variable-length entries point at zlib-compressed record payloads.
//! Stored payload offsets are relative to the header end, so
//! [`HEADER_SIZE`] is added on read. Payloads decompress lazily, one
//! record at a time.

use std::collections::HashMap;
use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::chr::RecordId;
use crate::reader::{ByteReader, ReadError};

/// Size of the ARZ header; stored record offsets are relative to it.
pub const HEADER_SIZE: usize = 24;

/// A parsed `database.arz`: the record index plus the raw bytes, from
/// which individual records decompress on demand.
pub struct ArzFile {
    data: Vec<u8>,
    strings: Vec<String>,
    entries: HashMap<String, RecordEntry>,
}

struct RecordEntry {
    id: RecordId,
    record_type: String,
    payload_offset: usize,
    payload_size: usize,
}

/// One decompressed database record: a set of named, typed variables.
#[derive(Debug, Clone, PartialEq)]
pub struct DbRecord {
    pub id: RecordId,
    /// The record's class string, e.g. `ArmorProtective_Head`.
    pub record_type: String,
    variables: HashMap<String, DbVariable>,
}

impl DbRecord {
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&DbVariable> {
        self.variables.get(name)
    }

    pub fn variables(&self) -> impl Iterator<Item = &DbVariable> {
        self.variables.values()
    }

    /// First value of a string variable — the common case for tag
    /// lookups like `description` or `itemNameTag`.
    #[must_use]
    pub fn string(&self, name: &str) -> Option<&str> {
        match &self.variable(name)?.values {
            DbValues::Strings(values) => values.first().map(String::as_str),
            DbValues::Integers(_) | DbValues::Floats(_) | DbValues::Booleans(_) => None,
        }
    }

    /// First value of an integer variable.
    #[must_use]
    pub fn integer(&self, name: &str) -> Option<i32> {
        match &self.variable(name)?.values {
            DbValues::Integers(values) => values.first().copied(),
            DbValues::Strings(_) | DbValues::Floats(_) | DbValues::Booleans(_) => None,
        }
    }

    /// First value of a float variable.
    #[must_use]
    pub fn float(&self, name: &str) -> Option<f32> {
        match &self.variable(name)?.values {
            DbValues::Floats(values) => values.first().copied(),
            DbValues::Strings(_) | DbValues::Integers(_) | DbValues::Booleans(_) => None,
        }
    }
}

/// A record variable: its name and homogeneous typed values (arrays
/// are common; single values are one-element arrays).
#[derive(Debug, Clone, PartialEq)]
pub struct DbVariable {
    pub name: String,
    pub values: DbValues,
}

/// The four value types the format defines (0=int, 1=float,
/// 2=string-table index, 3=bool-as-i32).
#[derive(Debug, Clone, PartialEq)]
pub enum DbValues {
    Integers(Vec<i32>),
    Floats(Vec<f32>),
    Strings(Vec<String>),
    Booleans(Vec<bool>),
}

/// Errors from parsing an ARZ file or one of its records.
#[derive(Debug, thiserror::Error)]
pub enum ArzError {
    #[error("invalid header field {field}: {value}")]
    InvalidHeader { field: &'static str, value: i32 },
    #[error("invalid {what} count {count}")]
    InvalidCount { what: &'static str, count: i32 },
    #[error("string index {index} out of range ({len} strings)")]
    StringIndex { index: i32, len: usize },
    #[error("record table entry {index} has an empty record path")]
    EmptyRecordPath { index: usize },
    #[error("record {id}: payload range outside the file")]
    PayloadOutOfRange { id: String },
    #[error("record {id}: zlib decompression failed: {source}")]
    Decompress { id: String, source: std::io::Error },
    #[error("record {id}: payload length {len} is not a multiple of 4")]
    UnalignedPayload { id: String, len: usize },
    #[error("record {id}: variable {name}: invalid data type {data_type}")]
    InvalidDataType {
        id: String,
        name: String,
        data_type: i16,
    },
    #[error("record {id}: variable {name}: invalid value count {count}")]
    InvalidValueCount {
        id: String,
        name: String,
        count: i16,
    },
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl ArzFile {
    /// Parses the header, string table, and record index. Record
    /// payloads stay compressed until [`ArzFile::record`] asks for
    /// them.
    ///
    /// # Errors
    /// Any structural error in the header or tables.
    pub fn parse(data: Vec<u8>) -> Result<Self, ArzError> {
        let mut header = ByteReader::new(&data);
        header.read_i32()?;
        let record_table_start = offset_field(header.read_i32()?, "record table start")?;
        header.read_i32()?;
        let record_count = header.read_i32()?;
        let record_count = usize::try_from(record_count).map_err(|_| ArzError::InvalidCount {
            what: "record",
            count: record_count,
        })?;
        let string_table_start = offset_field(header.read_i32()?, "string table start")?;

        let strings = read_string_table(&data, string_table_start)?;
        let entries = read_record_table(&data, record_table_start, record_count, &strings)?;
        Ok(Self {
            data,
            strings,
            entries,
        })
    }

    /// Looks up and decompresses a record. `None` when the id is not
    /// in the database (ids match case-insensitively with `/` and `\`
    /// interchangeable, like `TQVaultAE`'s `NormalizeRecordPath`).
    #[must_use]
    pub fn record(&self, id: &RecordId) -> Option<Result<DbRecord, ArzError>> {
        let entry = self.entries.get(&normalize(id.as_str()))?;
        Some(self.decompress(entry))
    }

    pub fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        self.entries.values().map(|entry| &entry.id)
    }

    fn decompress(&self, entry: &RecordEntry) -> Result<DbRecord, ArzError> {
        let compressed = entry
            .payload_offset
            .checked_add(entry.payload_size)
            .and_then(|end| self.data.get(entry.payload_offset..end))
            .ok_or_else(|| ArzError::PayloadOutOfRange {
                id: entry.id.as_str().to_string(),
            })?;
        let mut payload = Vec::new();
        ZlibDecoder::new(compressed)
            .read_to_end(&mut payload)
            .map_err(|source| ArzError::Decompress {
                id: entry.id.as_str().to_string(),
                source,
            })?;
        let variables = parse_variables(&payload, &entry.id, &self.strings)?;
        Ok(DbRecord {
            id: entry.id.clone(),
            record_type: entry.record_type.clone(),
            variables,
        })
    }
}

fn offset_field(value: i32, field: &'static str) -> Result<usize, ArzError> {
    usize::try_from(value).map_err(|_| ArzError::InvalidHeader { field, value })
}

fn read_string_table(data: &[u8], start: usize) -> Result<Vec<String>, ArzError> {
    let mut reader = ByteReader::at(data, start);
    let count = reader.read_i32()?;
    let count = usize::try_from(count).map_err(|_| ArzError::InvalidCount {
        what: "string",
        count,
    })?;
    (0..count).map(|_| Ok(reader.read_cstring()?)).collect()
}

fn read_record_table(
    data: &[u8],
    start: usize,
    count: usize,
    strings: &[String],
) -> Result<HashMap<String, RecordEntry>, ArzError> {
    let mut reader = ByteReader::at(data, start);
    let mut entries = HashMap::with_capacity(count);
    for index in 0..count {
        let id_index = reader.read_i32()?;
        let record_type = reader.read_cstring()?;
        let payload_offset = offset_field(reader.read_i32()?, "record payload offset")?;
        let payload_size = offset_field(reader.read_i32()?, "record payload size")?;
        reader.read_i32()?;
        reader.read_i32()?;

        let raw_id = usize::try_from(id_index)
            .ok()
            .and_then(|id_index| strings.get(id_index))
            .ok_or(ArzError::StringIndex {
                index: id_index,
                len: strings.len(),
            })?;
        let id = RecordId::parse(raw_id.clone()).ok_or(ArzError::EmptyRecordPath { index })?;
        entries.insert(
            normalize(id.as_str()),
            RecordEntry {
                id,
                record_type,
                payload_offset: HEADER_SIZE + payload_offset,
                payload_size,
            },
        );
    }
    Ok(entries)
}

fn parse_variables(
    payload: &[u8],
    record_id: &RecordId,
    strings: &[String],
) -> Result<HashMap<String, DbVariable>, ArzError> {
    let id = || record_id.as_str().to_string();
    if !payload.len().is_multiple_of(4) {
        return Err(ArzError::UnalignedPayload {
            id: id(),
            len: payload.len(),
        });
    }
    let mut reader = ByteReader::new(payload);
    let mut variables = HashMap::new();
    while reader.pos() < payload.len() {
        let data_type = reader.read_i16()?;
        let count = reader.read_i16()?;
        let name_index = reader.read_i32()?;
        let name = usize::try_from(name_index)
            .ok()
            .and_then(|name_index| strings.get(name_index))
            .ok_or(ArzError::StringIndex {
                index: name_index,
                len: strings.len(),
            })?
            .clone();
        let value_count = usize::try_from(count)
            .ok()
            .filter(|&value_count| value_count >= 1)
            .ok_or_else(|| ArzError::InvalidValueCount {
                id: id(),
                name: name.clone(),
                count,
            })?;

        let values = match data_type {
            0 => DbValues::Integers(read_values(&mut reader, value_count, ByteReader::read_i32)?),
            1 => DbValues::Floats(read_values(&mut reader, value_count, ByteReader::read_f32)?),
            2 => DbValues::Strings(
                read_values(&mut reader, value_count, ByteReader::read_i32)?
                    .into_iter()
                    .map(|index| {
                        usize::try_from(index)
                            .ok()
                            .and_then(|index| strings.get(index))
                            .map(|value| value.trim().to_string())
                            .ok_or(ArzError::StringIndex {
                                index,
                                len: strings.len(),
                            })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            3 => DbValues::Booleans(
                read_values(&mut reader, value_count, ByteReader::read_i32)?
                    .into_iter()
                    .map(|value| value != 0)
                    .collect(),
            ),
            _ => {
                return Err(ArzError::InvalidDataType {
                    id: id(),
                    name,
                    data_type,
                });
            }
        };
        variables.insert(name.clone(), DbVariable { name, values });
    }
    Ok(variables)
}

fn read_values<'data, T>(
    reader: &mut ByteReader<'data>,
    count: usize,
    mut read: impl FnMut(&mut ByteReader<'data>) -> Result<T, ReadError>,
) -> Result<Vec<T>, ArzError> {
    (0..count).map(|_| Ok(read(reader)?)).collect()
}

/// `TQVaultAE`'s `NormalizeRecordPath`: uppercase, `/` → `\`. Shared
/// by the ARZ and ARC lookup keys.
pub(crate) fn normalize(path: &str) -> String {
    path.to_uppercase().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;

    const SWORD_ID: &str = "records\\item\\equipmentweapon\\sword_01.dbr";

    fn push_i32(buf: &mut Vec<u8>, value: i32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn push_i16(buf: &mut Vec<u8>, value: i16) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn push_cstr(buf: &mut Vec<u8>, value: &str) {
        push_i32(buf, i32::try_from(value.len()).unwrap());
        buf.extend_from_slice(value.as_bytes());
    }

    fn zlib(payload: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    fn sword_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        // itemLevel = 12
        push_i16(&mut payload, 0);
        push_i16(&mut payload, 1);
        push_i32(&mut payload, 1);
        push_i32(&mut payload, 12);
        // attackSpeed = [1.5, 2.0]
        push_i16(&mut payload, 1);
        push_i16(&mut payload, 2);
        push_i32(&mut payload, 2);
        push_i32(&mut payload, 1.5_f32.to_bits().cast_signed());
        push_i32(&mut payload, 2.0_f32.to_bits().cast_signed());
        // description = string 4 (stored with padding, trimmed on read)
        push_i16(&mut payload, 2);
        push_i16(&mut payload, 1);
        push_i32(&mut payload, 3);
        push_i32(&mut payload, 4);
        // active = true
        push_i16(&mut payload, 3);
        push_i16(&mut payload, 1);
        push_i32(&mut payload, 5);
        push_i32(&mut payload, 1);
        payload
    }

    fn arz_file(payload: &[u8]) -> Vec<u8> {
        let strings = [
            SWORD_ID,
            "itemLevel",
            "attackSpeed",
            "description",
            "  tagSwordName01 ",
            "active",
        ];
        let compressed = zlib(payload);

        let mut record_table = Vec::new();
        push_i32(&mut record_table, 0);
        push_cstr(&mut record_table, "WeaponMelee_Sword");
        push_i32(&mut record_table, 0);
        push_i32(&mut record_table, i32::try_from(compressed.len()).unwrap());
        push_i32(&mut record_table, 0);
        push_i32(&mut record_table, 0);

        let mut string_table = Vec::new();
        push_i32(&mut string_table, i32::try_from(strings.len()).unwrap());
        for string in strings {
            push_cstr(&mut string_table, string);
        }

        let record_table_start = HEADER_SIZE + compressed.len();
        let string_table_start = record_table_start + record_table.len();
        let mut file = Vec::new();
        push_i32(&mut file, 2);
        push_i32(&mut file, i32::try_from(record_table_start).unwrap());
        push_i32(&mut file, i32::try_from(record_table.len()).unwrap());
        push_i32(&mut file, 1);
        push_i32(&mut file, i32::try_from(string_table_start).unwrap());
        push_i32(&mut file, i32::try_from(string_table.len()).unwrap());
        file.extend_from_slice(&compressed);
        file.extend_from_slice(&record_table);
        file.extend_from_slice(&string_table);
        file
    }

    fn record_id(raw: &str) -> RecordId {
        RecordId::parse(raw.to_string()).unwrap()
    }

    #[test]
    fn lookup_ignores_case_and_slash_direction() {
        let arz = ArzFile::parse(arz_file(&sword_payload())).unwrap();
        let record = arz
            .record(&record_id("RECORDS/ITEM/EQUIPMENTWEAPON/SWORD_01.DBR"))
            .unwrap()
            .unwrap();
        assert_eq!(record.id.as_str(), SWORD_ID);
        assert_eq!(record.record_type, "WeaponMelee_Sword");
    }

    #[test]
    fn decodes_all_four_value_types() {
        let arz = ArzFile::parse(arz_file(&sword_payload())).unwrap();
        let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
        assert_eq!(record.integer("itemLevel"), Some(12));
        assert_eq!(
            record.variable("attackSpeed").unwrap().values,
            DbValues::Floats(vec![1.5, 2.0])
        );
        assert_eq!(record.string("description"), Some("tagSwordName01"));
        assert_eq!(
            record.variable("active").unwrap().values,
            DbValues::Booleans(vec![true])
        );
    }

    #[test]
    fn typed_accessors_refuse_other_types() {
        let arz = ArzFile::parse(arz_file(&sword_payload())).unwrap();
        let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
        assert_eq!(record.string("itemLevel"), None);
        assert_eq!(record.integer("description"), None);
    }

    #[test]
    fn unknown_record_is_none() {
        let arz = ArzFile::parse(arz_file(&sword_payload())).unwrap();
        assert!(arz.record(&record_id("records\\nothing.dbr")).is_none());
    }

    #[test]
    fn record_ids_yields_raw_ids() {
        let arz = ArzFile::parse(arz_file(&sword_payload())).unwrap();
        let ids: Vec<&str> = arz.record_ids().map(RecordId::as_str).collect();
        assert_eq!(ids, vec![SWORD_ID]);
    }

    #[test]
    fn bad_data_type_is_reported_with_variable_name() {
        let mut payload = Vec::new();
        push_i16(&mut payload, 7);
        push_i16(&mut payload, 1);
        push_i32(&mut payload, 1);
        push_i32(&mut payload, 0);
        let arz = ArzFile::parse(arz_file(&payload)).unwrap();
        assert!(matches!(
            arz.record(&record_id(SWORD_ID)).unwrap(),
            Err(ArzError::InvalidDataType { data_type: 7, .. })
        ));
    }

    #[test]
    fn unaligned_payload_is_rejected() {
        let arz = ArzFile::parse(arz_file(&[0, 1, 2])).unwrap();
        assert!(matches!(
            arz.record(&record_id(SWORD_ID)).unwrap(),
            Err(ArzError::UnalignedPayload { len: 3, .. })
        ));
    }

    #[test]
    fn string_index_out_of_range_fails_at_parse() {
        let mut file = arz_file(&sword_payload());
        // Corrupt the record table's id string index (first i32 after
        // the header-sized prefix + compressed payload).
        let record_table_start = HEADER_SIZE + zlib(&sword_payload()).len();
        file[record_table_start..record_table_start + 4].copy_from_slice(&99_i32.to_le_bytes());
        assert!(matches!(
            ArzFile::parse(file),
            Err(ArzError::StringIndex { index: 99, .. })
        ));
    }
}
