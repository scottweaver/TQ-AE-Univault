//! Reader for the game's `database.arz` — the compressed record
//! database holding item stats, classifications, and the localization
//! tags that name things — plus [`compose`], which serializes records
//! into **new** database images for this app's own mod bundles. The
//! game's databases remain read-only reference data per
//! ARCHITECTURE.md: nothing here ever rewrites them. Reader ported
//! from `TQVaultAE`'s `ArzFileProvider.cs` / `RecordInfoProvider.cs`
//! (MIT); writer layout from the MIT `TQArchive-Wrapper` reference.
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

use crate::writer::{write_i32, write_i64};

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
    /// Normalized ids in record-table order, so re-serialization and
    /// iteration are deterministic.
    order: Vec<String>,
}

struct RecordEntry {
    id: RecordId,
    record_type: String,
    payload_offset: usize,
    payload_size: usize,
    timestamp: i64,
}

/// One decompressed database record: a set of named, typed variables
/// in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct DbRecord {
    pub id: RecordId,
    /// The record's class string, e.g. `ArmorProtective_Head`.
    pub record_type: String,
    variables: Vec<DbVariable>,
}

impl DbRecord {
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&DbVariable> {
        self.variables.iter().find(|variable| variable.name == name)
    }

    pub fn variables(&self) -> impl Iterator<Item = &DbVariable> {
        self.variables.iter()
    }

    /// Replaces (or appends) a variable — the mod-patching edit.
    pub fn set_variable(&mut self, variable: DbVariable) {
        match self
            .variables
            .iter_mut()
            .find(|existing| existing.name == variable.name)
        {
            Some(existing) => *existing = variable,
            None => self.variables.push(variable),
        }
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

impl DbVariable {
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.values {
            DbValues::Integers(values) => values.len(),
            DbValues::Floats(values) => values.len(),
            DbValues::Strings(values) => values.len(),
            DbValues::Booleans(values) => values.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
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
        let (entries, order) =
            read_record_table(&data, record_table_start, record_count, &strings)?;
        Ok(Self {
            data,
            strings,
            entries,
            order,
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

    /// Record ids in record-table order.
    pub fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        self.order.iter().map(|key| &self.entries[key].id)
    }

    /// The record's stored build timestamp, preserved when composing
    /// mod databases.
    #[must_use]
    pub fn record_timestamp(&self, id: &RecordId) -> Option<i64> {
        Some(self.entries.get(&normalize(id.as_str()))?.timestamp)
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
) -> Result<(HashMap<String, RecordEntry>, Vec<String>), ArzError> {
    let mut reader = ByteReader::at(data, start);
    let mut entries = HashMap::with_capacity(count);
    let mut order = Vec::with_capacity(count);
    for index in 0..count {
        let id_index = reader.read_i32()?;
        let record_type = reader.read_cstring()?;
        let payload_offset = offset_field(reader.read_i32()?, "record payload offset")?;
        let payload_size = offset_field(reader.read_i32()?, "record payload size")?;
        let timestamp = reader.read_i64()?;

        let raw_id = usize::try_from(id_index)
            .ok()
            .and_then(|id_index| strings.get(id_index))
            .ok_or(ArzError::StringIndex {
                index: id_index,
                len: strings.len(),
            })?;
        let id = RecordId::parse(raw_id.clone()).ok_or(ArzError::EmptyRecordPath { index })?;
        let key = normalize(id.as_str());
        order.push(key.clone());
        entries.insert(
            key,
            RecordEntry {
                id,
                record_type,
                payload_offset: HEADER_SIZE + payload_offset,
                payload_size,
                timestamp,
            },
        );
    }
    Ok((entries, order))
}

fn parse_variables(
    payload: &[u8],
    record_id: &RecordId,
    strings: &[String],
) -> Result<Vec<DbVariable>, ArzError> {
    let id = || record_id.as_str().to_string();
    if !payload.len().is_multiple_of(4) {
        return Err(ArzError::UnalignedPayload {
            id: id(),
            len: payload.len(),
        });
    }
    let mut reader = ByteReader::new(payload);
    let mut variables = Vec::new();
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
        variables.push(DbVariable { name, values });
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
#[must_use]
pub fn normalize(path: &str) -> String {
    path.to_uppercase().replace('/', "\\")
}

/// Serializes records into a new database image — the write half of
/// this format, used to build the app's **own** mod archives. The
/// game's databases stay read-only (ARCHITECTURE.md); a composed
/// image is always a new file. Layout mirrors `ArtManager`'s output
/// (via the MIT `TQArchive-Wrapper` reference): 24-byte header,
/// zlib-compressed record payloads, record table, string table.
#[must_use]
pub fn compose(records: &[(DbRecord, i64)]) -> Vec<u8> {
    use std::io::Write as _;

    use flate2::Compression;

    let mut interner = Interner::default();
    let mut payloads = Vec::new();
    let mut table = Vec::new();
    for (record, timestamp) in records {
        let name_index = interner.intern(record.id.as_str());
        let mut raw = Vec::new();
        for variable in record.variables() {
            let (type_code, count) = match &variable.values {
                DbValues::Integers(values) => (0_i16, values.len()),
                DbValues::Floats(values) => (1_i16, values.len()),
                DbValues::Strings(values) => (2_i16, values.len()),
                DbValues::Booleans(values) => (3_i16, values.len()),
            };
            raw.extend_from_slice(&type_code.to_le_bytes());
            raw.extend_from_slice(&i16::try_from(count).unwrap_or(i16::MAX).to_le_bytes());
            write_i32(&mut raw, interner.intern(&variable.name));
            match &variable.values {
                DbValues::Integers(values) => {
                    for value in values {
                        write_i32(&mut raw, *value);
                    }
                }
                DbValues::Floats(values) => {
                    for value in values {
                        crate::writer::write_f32(&mut raw, *value);
                    }
                }
                DbValues::Strings(values) => {
                    for value in values {
                        write_i32(&mut raw, interner.intern(value));
                    }
                }
                DbValues::Booleans(values) => {
                    for value in values {
                        write_i32(&mut raw, i32::from(*value));
                    }
                }
            }
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
        let _ = encoder.write_all(&raw);
        let compressed = encoder.finish().unwrap_or_default();
        table.push((
            name_index,
            record.record_type.clone(),
            payloads.len(),
            compressed.len(),
            *timestamp,
        ));
        payloads.extend_from_slice(&compressed);
    }

    let mut record_table = Vec::new();
    for (name_index, record_type, offset, length, timestamp) in &table {
        write_i32(&mut record_table, *name_index);
        crate::writer::write_cstring(&mut record_table, record_type);
        write_i32(&mut record_table, i32::try_from(*offset).unwrap_or(0));
        write_i32(&mut record_table, i32::try_from(*length).unwrap_or(0));
        write_i64(&mut record_table, *timestamp);
    }
    let mut string_table = Vec::new();
    write_i32(
        &mut string_table,
        i32::try_from(interner.strings.len()).unwrap_or(0),
    );
    for value in &interner.strings {
        crate::writer::write_cstring(&mut string_table, value);
    }

    let record_table_start = HEADER_SIZE + payloads.len();
    let string_table_start = record_table_start + record_table.len();
    let mut out = Vec::with_capacity(string_table_start + string_table.len());
    write_i32(&mut out, 0x0003_0004);
    write_i32(&mut out, i32::try_from(record_table_start).unwrap_or(0));
    write_i32(&mut out, i32::try_from(record_table.len()).unwrap_or(0));
    write_i32(&mut out, i32::try_from(table.len()).unwrap_or(0));
    write_i32(&mut out, i32::try_from(string_table_start).unwrap_or(0));
    write_i32(&mut out, i32::try_from(string_table.len()).unwrap_or(0));
    out.extend_from_slice(&payloads);
    out.extend_from_slice(&record_table);
    out.extend_from_slice(&string_table);
    out
}

#[derive(Default)]
struct Interner {
    strings: Vec<String>,
    indexes: HashMap<String, i32>,
}

impl Interner {
    fn intern(&mut self, value: &str) -> i32 {
        if let Some(index) = self.indexes.get(value) {
            return *index;
        }
        let index = i32::try_from(self.strings.len()).unwrap_or(0);
        self.strings.push(value.to_string());
        self.indexes.insert(value.to_string(), index);
        index
    }
}

/// Builds synthetic ARZ byte images so tests (here and in `gamedata`)
/// can assemble records with chosen types and variables.
#[cfg(test)]
pub(crate) mod fixture {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::HEADER_SIZE;

    #[derive(Default)]
    pub(crate) struct ArzBuilder {
        strings: Vec<String>,
        records: Vec<(i32, String, Vec<u8>)>,
    }

    pub(crate) enum Values<'a> {
        Ints(&'a [i32]),
        Floats(&'a [f32]),
        Strings(&'a [&'a str]),
        Bools(&'a [bool]),
    }

    impl ArzBuilder {
        pub(crate) fn intern(&mut self, value: &str) -> i32 {
            let index = self
                .strings
                .iter()
                .position(|existing| existing == value)
                .unwrap_or_else(|| {
                    self.strings.push(value.to_string());
                    self.strings.len() - 1
                });
            i32::try_from(index).unwrap()
        }

        pub(crate) fn record(
            &mut self,
            id: &str,
            record_type: &str,
            variables: &[(&str, Values<'_>)],
        ) {
            let mut payload = Vec::new();
            for (name, values) in variables {
                let name_index = self.intern(name);
                let (type_code, count): (i16, usize) = match values {
                    Values::Ints(v) => (0, v.len()),
                    Values::Floats(v) => (1, v.len()),
                    Values::Strings(v) => (2, v.len()),
                    Values::Bools(v) => (3, v.len()),
                };
                payload.extend_from_slice(&type_code.to_le_bytes());
                payload.extend_from_slice(&i16::try_from(count).unwrap().to_le_bytes());
                payload.extend_from_slice(&name_index.to_le_bytes());
                match values {
                    Values::Ints(v) => {
                        for value in *v {
                            payload.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Values::Floats(v) => {
                        for value in *v {
                            payload.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Values::Strings(v) => {
                        for value in *v {
                            let index = self.intern(value);
                            payload.extend_from_slice(&index.to_le_bytes());
                        }
                    }
                    Values::Bools(v) => {
                        for value in *v {
                            payload.extend_from_slice(&i32::from(*value).to_le_bytes());
                        }
                    }
                }
            }
            self.record_raw(id, record_type, payload);
        }

        /// Adds a record with an arbitrary (possibly corrupt) payload.
        pub(crate) fn record_raw(&mut self, id: &str, record_type: &str, payload: Vec<u8>) {
            let id_index = self.intern(id);
            self.records
                .push((id_index, record_type.to_string(), payload));
        }

        pub(crate) fn build(self) -> Vec<u8> {
            self.build_with_layout().0
        }

        /// Also returns the record-table start offset, for tests that
        /// corrupt table bytes.
        pub(crate) fn build_with_layout(self) -> (Vec<u8>, usize) {
            fn push_i32(buf: &mut Vec<u8>, value: i32) {
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

            let mut payloads = Vec::new();
            let mut record_table = Vec::new();
            let mut payload_offset = 0_usize;
            for (id_index, record_type, payload) in &self.records {
                let compressed = zlib(payload);
                push_i32(&mut record_table, *id_index);
                push_cstr(&mut record_table, record_type);
                push_i32(&mut record_table, i32::try_from(payload_offset).unwrap());
                push_i32(&mut record_table, i32::try_from(compressed.len()).unwrap());
                push_i32(&mut record_table, 0);
                push_i32(&mut record_table, 0);
                payload_offset += compressed.len();
                payloads.extend_from_slice(&compressed);
            }

            let mut string_table = Vec::new();
            push_i32(
                &mut string_table,
                i32::try_from(self.strings.len()).unwrap(),
            );
            for string in &self.strings {
                push_cstr(&mut string_table, string);
            }

            let record_table_start = HEADER_SIZE + payloads.len();
            let string_table_start = record_table_start + record_table.len();
            let mut file = Vec::new();
            push_i32(&mut file, 2);
            push_i32(&mut file, i32::try_from(record_table_start).unwrap());
            push_i32(&mut file, i32::try_from(record_table.len()).unwrap());
            push_i32(&mut file, i32::try_from(self.records.len()).unwrap());
            push_i32(&mut file, i32::try_from(string_table_start).unwrap());
            push_i32(&mut file, i32::try_from(string_table.len()).unwrap());
            file.extend_from_slice(&payloads);
            file.extend_from_slice(&record_table);
            file.extend_from_slice(&string_table);
            (file, record_table_start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{ArzBuilder, Values};
    use super::*;

    const SWORD_ID: &str = "records\\item\\equipmentweapon\\sword_01.dbr";

    fn sword_arz() -> Vec<u8> {
        let mut builder = ArzBuilder::default();
        builder.record(
            SWORD_ID,
            "WeaponMelee_Sword",
            &[
                ("itemLevel", Values::Ints(&[12])),
                ("attackSpeed", Values::Floats(&[1.5, 2.0])),
                ("description", Values::Strings(&["  tagSwordName01 "])),
                ("active", Values::Bools(&[true])),
            ],
        );
        builder.build()
    }

    fn record_id(raw: &str) -> RecordId {
        RecordId::parse(raw.to_string()).unwrap()
    }

    #[test]
    fn lookup_ignores_case_and_slash_direction() {
        let arz = ArzFile::parse(sword_arz()).unwrap();
        let record = arz
            .record(&record_id("RECORDS/ITEM/EQUIPMENTWEAPON/SWORD_01.DBR"))
            .unwrap()
            .unwrap();
        assert_eq!(record.id.as_str(), SWORD_ID);
        assert_eq!(record.record_type, "WeaponMelee_Sword");
    }

    #[test]
    fn decodes_all_four_value_types() {
        let arz = ArzFile::parse(sword_arz()).unwrap();
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
        let arz = ArzFile::parse(sword_arz()).unwrap();
        let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
        assert_eq!(record.string("itemLevel"), None);
        assert_eq!(record.integer("description"), None);
    }

    #[test]
    fn unknown_record_is_none() {
        let arz = ArzFile::parse(sword_arz()).unwrap();
        assert!(arz.record(&record_id("records\\nothing.dbr")).is_none());
    }

    #[test]
    fn compose_round_trips_records_order_and_timestamps() {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\item\\zeta.dbr",
            "WeaponMelee_Sword",
            &[
                ("itemNameTag", Values::Strings(&["tagZeta"])),
                ("offensivePhysicalMin", Values::Floats(&[12.5, 14.0])),
                ("levelRequirement", Values::Ints(&[4])),
                ("cannotPickUpMultiple", Values::Bools(&[true])),
            ],
        );
        builder.record(
            "records\\item\\alpha.dbr",
            "LootRandomizer",
            &[("lootRandomizerName", Values::Strings(&["tagAlpha"]))],
        );
        let original = ArzFile::parse(builder.build()).unwrap();

        let records: Vec<(DbRecord, i64)> = original
            .record_ids()
            .map(|id| {
                (
                    original.record(id).unwrap().unwrap(),
                    original.record_timestamp(id).unwrap(),
                )
            })
            .collect();
        let composed = ArzFile::parse(compose(&records)).unwrap();

        let original_ids: Vec<&str> = original.record_ids().map(RecordId::as_str).collect();
        let composed_ids: Vec<&str> = composed.record_ids().map(RecordId::as_str).collect();
        assert_eq!(original_ids, composed_ids);
        for id in original.record_ids() {
            assert_eq!(
                original.record(id).unwrap().unwrap(),
                composed.record(id).unwrap().unwrap(),
                "{id:?}"
            );
            assert_eq!(original.record_timestamp(id), composed.record_timestamp(id));
        }
    }

    #[test]
    fn set_variable_replaces_in_place_and_appends() {
        let mut builder = ArzBuilder::default();
        builder.record(
            "records\\skills\\thing.dbr",
            "Skill_Attack",
            &[
                ("skillTargetNumber", Values::Ints(&[4])),
                ("skillManaCost", Values::Floats(&[10.0])),
            ],
        );
        let arz = ArzFile::parse(builder.build()).unwrap();
        let mut record = arz
            .record(&RecordId::parse("records\\skills\\thing.dbr".into()).unwrap())
            .unwrap()
            .unwrap();
        record.set_variable(DbVariable {
            name: "skillTargetNumber".to_string(),
            values: DbValues::Integers(vec![12]),
        });
        record.set_variable(DbVariable {
            name: "skillTargetRadius".to_string(),
            values: DbValues::Floats(vec![18.0]),
        });
        assert_eq!(record.integer("skillTargetNumber"), Some(12));
        assert_eq!(record.float("skillTargetRadius"), Some(18.0));
        // Replacement keeps position; the append lands last.
        let names: Vec<&str> = record.variables().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["skillTargetNumber", "skillManaCost", "skillTargetRadius"]
        );
    }

    #[test]
    fn record_ids_yields_raw_ids() {
        let arz = ArzFile::parse(sword_arz()).unwrap();
        let ids: Vec<&str> = arz.record_ids().map(RecordId::as_str).collect();
        assert_eq!(ids, vec![SWORD_ID]);
    }

    #[test]
    fn bad_data_type_is_reported_with_variable_name() {
        let mut builder = ArzBuilder::default();
        let name_index = builder.intern("brokenVar");
        let mut payload = Vec::new();
        payload.extend_from_slice(&7_i16.to_le_bytes());
        payload.extend_from_slice(&1_i16.to_le_bytes());
        payload.extend_from_slice(&name_index.to_le_bytes());
        payload.extend_from_slice(&0_i32.to_le_bytes());
        builder.record_raw(SWORD_ID, "WeaponMelee_Sword", payload);
        let arz = ArzFile::parse(builder.build()).unwrap();
        assert!(matches!(
            arz.record(&record_id(SWORD_ID)).unwrap(),
            Err(ArzError::InvalidDataType { data_type: 7, .. })
        ));
    }

    #[test]
    fn unaligned_payload_is_rejected() {
        let mut builder = ArzBuilder::default();
        builder.record_raw(SWORD_ID, "WeaponMelee_Sword", vec![0, 1, 2]);
        let arz = ArzFile::parse(builder.build()).unwrap();
        assert!(matches!(
            arz.record(&record_id(SWORD_ID)).unwrap(),
            Err(ArzError::UnalignedPayload { len: 3, .. })
        ));
    }

    #[test]
    fn string_index_out_of_range_fails_at_parse() {
        let mut builder = ArzBuilder::default();
        builder.record(
            SWORD_ID,
            "WeaponMelee_Sword",
            &[("itemLevel", Values::Ints(&[1]))],
        );
        let (mut file, record_table_start) = builder.build_with_layout();
        file[record_table_start..record_table_start + 4].copy_from_slice(&99_i32.to_le_bytes());
        assert!(matches!(
            ArzFile::parse(file),
            Err(ArzError::StringIndex { index: 99, .. })
        ));
    }
}
