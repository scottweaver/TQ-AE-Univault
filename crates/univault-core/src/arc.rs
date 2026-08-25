//! Reader for the game's `.arc` resource archives — localization text
//! (`Text_EN.arc`), textures, and other assets. Read-only reference
//! data per ARCHITECTURE.md. Ported from `TQVaultAE`'s
//! `ArcFileProvider.cs` (MIT).
//!
//! Layout: `"ARC"` magic; entry count at 0x08, part count at 0x0C,
//! table offset at 0x18. At the table offset: 12-byte part entries
//! (offset, compressed size, real size), then the null-terminated
//! ASCII names of active entries, in entry order. The last
//! `44 × entries` bytes of the file are the directory records
//! (storage type, offset, sizes, part range, name info). Storage type
//! 1 is stored raw; everything else concatenates zlib-compressed
//! parts. A 0x03 byte where a name should start marks an inactive
//! ("null file") entry.

use std::collections::HashMap;
use std::io::Read;

use flate2::read::ZlibDecoder;

use crate::arz::normalize;
use crate::reader::{ByteReader, ReadError};

/// A parsed `.arc` archive: the directory plus the raw bytes, from
/// which contained files extract on demand.
pub struct ArcFile {
    data: Vec<u8>,
    entries: HashMap<String, DirEntry>,
}

#[derive(Clone, Copy)]
struct Part {
    offset: usize,
    compressed_size: usize,
}

enum Storage {
    Stored { offset: usize },
    Parts(Vec<Part>),
}

struct DirEntry {
    name: String,
    real_size: usize,
    storage: Storage,
}

/// Errors from parsing an ARC archive or extracting a file from it.
#[derive(Debug, thiserror::Error)]
pub enum ArcError {
    #[error("not an ARC archive (bad magic or too short)")]
    NotArc,
    #[error("invalid {what} count {count}")]
    InvalidCount { what: &'static str, count: i32 },
    #[error("invalid table offset {offset}")]
    InvalidOffset { offset: i32 },
    #[error("directory record table larger than the file")]
    TruncatedDirectory,
    #[error("entry {name}: data range outside the file")]
    DataOutOfRange { name: String },
    #[error("entry {name}: part {part}: zlib decompression failed: {source}")]
    Decompress {
        name: String,
        part: usize,
        source: std::io::Error,
    },
    #[error("entry {name}: extracted {actual} bytes, directory says {expected}")]
    SizeMismatch {
        name: String,
        actual: usize,
        expected: usize,
    },
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl ArcFile {
    /// Parses the archive directory. File contents stay in place until
    /// [`ArcFile::file`] extracts them.
    ///
    /// # Errors
    /// Any structural error in the header or directory tables.
    pub fn parse(data: Vec<u8>) -> Result<Self, ArcError> {
        if data.len() < 0x21 || &data[..3] != b"ARC" {
            return Err(ArcError::NotArc);
        }
        let mut header = ByteReader::at(&data, 0x08);
        let entry_count = count_field(header.read_i32()?, "entry")?;
        let part_count = count_field(header.read_i32()?, "part")?;
        let toc_offset = {
            let offset = ByteReader::at(&data, 0x18).read_i32()?;
            usize::try_from(offset).map_err(|_| ArcError::InvalidOffset { offset })?
        };

        let mut reader = ByteReader::at(&data, toc_offset);
        let parts = (0..part_count)
            .map(|_| {
                let offset = count_field(reader.read_i32()?, "part offset")?;
                let compressed_size = count_field(reader.read_i32()?, "part size")?;
                reader.read_i32()?;
                Ok(Part {
                    offset,
                    compressed_size,
                })
            })
            .collect::<Result<Vec<_>, ArcError>>()?;
        let names_offset = reader.pos();

        let directory_bytes = entry_count
            .checked_mul(44)
            .ok_or(ArcError::TruncatedDirectory)?;
        let directory_start = data
            .len()
            .checked_sub(directory_bytes)
            .ok_or(ArcError::TruncatedDirectory)?;

        let raw_records = read_directory(&data, directory_start, entry_count)?;
        let mut names = ByteReader::at(&data, names_offset);
        let mut entries = HashMap::new();
        for raw in raw_records {
            let Some(storage) = raw.storage(&parts) else {
                continue;
            };
            let Some(name) = read_entry_name(&mut names, directory_start)? else {
                continue;
            };
            entries.insert(
                normalize(&name),
                DirEntry {
                    name,
                    real_size: raw.real_size,
                    storage,
                },
            );
        }
        Ok(Self { data, entries })
    }

    /// Extracts one contained file by its internal path (matched
    /// case-insensitively with `/` and `\` interchangeable). `None`
    /// when the archive has no such entry.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<Result<Vec<u8>, ArcError>> {
        let entry = self.entries.get(&normalize(name))?;
        Some(self.extract(entry))
    }

    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.entries.values().map(|entry| entry.name.as_str())
    }

    fn extract(&self, entry: &DirEntry) -> Result<Vec<u8>, ArcError> {
        let out_of_range = || ArcError::DataOutOfRange {
            name: entry.name.clone(),
        };
        match &entry.storage {
            Storage::Stored { offset } => offset
                .checked_add(entry.real_size)
                .and_then(|end| self.data.get(*offset..end))
                .map(<[u8]>::to_vec)
                .ok_or_else(out_of_range),
            Storage::Parts(parts) => {
                let mut out = Vec::with_capacity(entry.real_size);
                for (index, part) in parts.iter().enumerate() {
                    let compressed = part
                        .offset
                        .checked_add(part.compressed_size)
                        .and_then(|end| self.data.get(part.offset..end))
                        .ok_or_else(out_of_range)?;
                    ZlibDecoder::new(compressed)
                        .read_to_end(&mut out)
                        .map_err(|source| ArcError::Decompress {
                            name: entry.name.clone(),
                            part: index,
                            source,
                        })?;
                }
                if out.len() == entry.real_size {
                    Ok(out)
                } else {
                    Err(ArcError::SizeMismatch {
                        name: entry.name.clone(),
                        actual: out.len(),
                        expected: entry.real_size,
                    })
                }
            }
        }
    }
}

struct RawRecord {
    storage_type: i32,
    offset: usize,
    real_size: usize,
    part_count: i32,
    first_part: i32,
}

impl RawRecord {
    /// `None` marks an inactive entry (no parts and not stored raw, or
    /// an out-of-range part window) — skipped like `TQVaultAE` does.
    fn storage(&self, parts: &[Part]) -> Option<Storage> {
        if self.storage_type == 1 {
            return Some(Storage::Stored {
                offset: self.offset,
            });
        }
        let first = usize::try_from(self.first_part).ok()?;
        let count = usize::try_from(self.part_count).ok().filter(|&c| c >= 1)?;
        let window = parts.get(first..first.checked_add(count)?)?;
        Some(Storage::Parts(window.to_vec()))
    }
}

fn read_directory(data: &[u8], start: usize, count: usize) -> Result<Vec<RawRecord>, ArcError> {
    let mut reader = ByteReader::at(data, start);
    (0..count)
        .map(|_| {
            let storage_type = reader.read_i32()?;
            let offset = count_field(reader.read_i32()?, "entry offset")?;
            reader.read_i32()?;
            let real_size = count_field(reader.read_i32()?, "entry size")?;
            reader.read_i32()?;
            reader.read_i32()?;
            reader.read_i32()?;
            let part_count = reader.read_i32()?;
            let first_part = reader.read_i32()?;
            reader.read_i32()?;
            reader.read_i32()?;
            Ok(RawRecord {
                storage_type,
                offset,
                real_size,
                part_count,
                first_part,
            })
        })
        .collect()
}

/// Reads the next null-terminated ASCII name, stopping at the
/// directory records. A 0x03 byte in first position is `TQVaultAE`'s
/// "null file" marker: the entry is skipped and the byte is left for
/// the record data that follows.
fn read_entry_name(
    names: &mut ByteReader<'_>,
    directory_start: usize,
) -> Result<Option<String>, ArcError> {
    let mut bytes = Vec::new();
    loop {
        if names.pos() >= directory_start {
            return Ok(none_if_empty(&bytes));
        }
        let byte = names.read_u8()?;
        match byte {
            0x00 => return Ok(none_if_empty(&bytes)),
            0x03 if bytes.is_empty() => return Ok(None),
            _ => bytes.push(byte),
        }
    }
}

fn none_if_empty(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn count_field(value: i32, what: &'static str) -> Result<usize, ArcError> {
    usize::try_from(value).map_err(|_| ArcError::InvalidCount { what, count: value })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;

    fn push_i32(buf: &mut Vec<u8>, value: i32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn zlib(payload: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    /// Two files: "text\\stored.txt" stored raw, "text\\split.txt"
    /// zlib-compressed in two parts.
    fn sample_arc() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let stored = b"stored contents".to_vec();
        let split_a = b"first half / ".to_vec();
        let split_b = b"second half".to_vec();
        let part_a = zlib(&split_a);
        let part_b = zlib(&split_b);

        let header_size = 0x21_usize;
        let stored_offset = header_size;
        let part_a_offset = stored_offset + stored.len();
        let part_b_offset = part_a_offset + part_a.len();
        let toc_offset = part_b_offset + part_b.len();

        let mut toc = Vec::new();
        push_i32(&mut toc, i32::try_from(part_a_offset).unwrap());
        push_i32(&mut toc, i32::try_from(part_a.len()).unwrap());
        push_i32(&mut toc, i32::try_from(split_a.len()).unwrap());
        push_i32(&mut toc, i32::try_from(part_b_offset).unwrap());
        push_i32(&mut toc, i32::try_from(part_b.len()).unwrap());
        push_i32(&mut toc, i32::try_from(split_b.len()).unwrap());
        toc.extend_from_slice(b"text\\stored.txt\0");
        toc.extend_from_slice(b"text\\split.txt\0");

        let mut records = Vec::new();
        // stored.txt: storage type 1
        push_i32(&mut records, 1);
        push_i32(&mut records, i32::try_from(stored_offset).unwrap());
        push_i32(&mut records, i32::try_from(stored.len()).unwrap());
        push_i32(&mut records, i32::try_from(stored.len()).unwrap());
        records.extend_from_slice(&[0; 12]);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);
        // split.txt: storage type 3, parts 0..2
        let split_len = split_a.len() + split_b.len();
        push_i32(&mut records, 3);
        push_i32(&mut records, i32::try_from(part_a_offset).unwrap());
        push_i32(
            &mut records,
            i32::try_from(part_a.len() + part_b.len()).unwrap(),
        );
        push_i32(&mut records, i32::try_from(split_len).unwrap());
        records.extend_from_slice(&[0; 12]);
        push_i32(&mut records, 2);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);
        push_i32(&mut records, 0);

        let mut file = Vec::new();
        file.extend_from_slice(b"ARC");
        file.extend_from_slice(&[0; 5]);
        push_i32(&mut file, 2);
        push_i32(&mut file, 2);
        file.extend_from_slice(&[0; 8]);
        push_i32(&mut file, i32::try_from(toc_offset).unwrap());
        file.extend_from_slice(&[0; 5]);
        assert_eq!(file.len(), header_size);
        file.extend_from_slice(&stored);
        file.extend_from_slice(&part_a);
        file.extend_from_slice(&part_b);
        file.extend_from_slice(&toc);
        file.extend_from_slice(&records);

        let split_full = [split_a, split_b].concat();
        (file, stored, split_full)
    }

    #[test]
    fn extracts_stored_entries() {
        let (file, stored, _) = sample_arc();
        let arc = ArcFile::parse(file).unwrap();
        assert_eq!(arc.file("text\\stored.txt").unwrap().unwrap(), stored);
    }

    #[test]
    fn extracts_and_concatenates_compressed_parts() {
        let (file, _, split) = sample_arc();
        let arc = ArcFile::parse(file).unwrap();
        assert_eq!(arc.file("text\\split.txt").unwrap().unwrap(), split);
    }

    #[test]
    fn lookup_ignores_case_and_slash_direction() {
        let (file, stored, _) = sample_arc();
        let arc = ArcFile::parse(file).unwrap();
        assert_eq!(arc.file("TEXT/STORED.TXT").unwrap().unwrap(), stored);
    }

    #[test]
    fn unknown_entry_is_none() {
        let (file, _, _) = sample_arc();
        let arc = ArcFile::parse(file).unwrap();
        assert!(arc.file("text\\missing.txt").is_none());
    }

    #[test]
    fn lists_entry_names() {
        let (file, _, _) = sample_arc();
        let arc = ArcFile::parse(file).unwrap();
        let mut names: Vec<&str> = arc.file_names().collect();
        names.sort_unstable();
        assert_eq!(names, vec!["text\\split.txt", "text\\stored.txt"]);
    }

    #[test]
    fn rejects_non_arc_data() {
        assert!(matches!(
            ArcFile::parse(b"definitely not an archive".to_vec()),
            Err(ArcError::NotArc)
        ));
    }

    #[test]
    fn size_mismatch_is_reported() {
        let (mut file, _, _) = sample_arc();
        // Shrink split.txt's recorded real size (second record, 4th i32).
        let records_start = file.len() - 88;
        let real_size_at = records_start + 44 + 12;
        file[real_size_at..real_size_at + 4].copy_from_slice(&5_i32.to_le_bytes());
        let arc = ArcFile::parse(file).unwrap();
        assert!(matches!(
            arc.file("text\\split.txt").unwrap(),
            Err(ArcError::SizeMismatch { expected: 5, .. })
        ));
    }
}
