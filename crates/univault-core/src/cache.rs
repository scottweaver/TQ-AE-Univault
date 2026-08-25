//! The local game-data cache: everything the app queries about items
//! (localized names, grid footprints, icon bitmaps) distilled from the
//! game archives by one import pass into a single compact file, so
//! launches never re-read the install. The archives remain the source
//! of truth (ARCHITECTURE.md); this file is derived, fingerprint-keyed
//! to its sources, and regenerable at any time. It contains extracted
//! game assets for local personal use and is never distributed.
//!
//! Format (all little-endian, strings length-prefixed **UTF-8** —
//! localized names exceed Windows-1252, e.g. Ragnarök's
//! "Jǫrmungandr"): `UVC3` magic, source-fingerprint list (path, size,
//! mtime seconds), a table of runtime display labels, then entries
//! keyed by normalized record path: name, footprint, classification
//! and kind tags, an optional JSON-encoded stat block, and an
//! optional zlib-compressed RGBA icon.

use std::collections::HashMap;
use std::io::Read;

use flate2::Compression;
use flate2::read::ZlibDecoder;

use crate::arz::normalize;
use crate::chr::{Item, RecordId};
use crate::gamedata::FALLBACK_FOOTPRINT;
use crate::reader::{ByteReader, ReadError};
use crate::stats::StatBlock;
use crate::style::{Classification, ItemKind};
use crate::tex::RgbaImage;
use crate::writer::{write_i32, write_i64};

const MAGIC: i32 = 0x3343_5655; // "UVC3"

/// Identity of one source file at import time; the shell compares
/// these against the live files to detect a game update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStamp {
    pub path: String,
    pub size: i64,
    pub mtime_seconds: i64,
}

pub(crate) struct CachedIcon {
    width: i32,
    height: i32,
    zlib_rgba: Vec<u8>,
}

pub(crate) struct CacheEntry {
    pub(crate) name: Option<String>,
    pub(crate) footprint: (i32, i32),
    pub(crate) classification: Classification,
    pub(crate) kind: ItemKind,
    pub(crate) stats: Option<StatBlock>,
    pub(crate) icon: Option<CachedIcon>,
}

/// The runtime item database, loaded from (or about to be saved as)
/// a cache file.
pub struct GameCache {
    stamps: Vec<SourceStamp>,
    labels: HashMap<String, String>,
    entries: HashMap<String, CacheEntry>,
}

/// Errors from reading a cache file.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("not a tq-univault cache file")]
    BadMagic,
    #[error("unrecognized cache entry data")]
    Corrupt,
    #[error("icon decompression failed: {0}")]
    Icon(std::io::Error),
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl GameCache {
    pub(crate) fn from_entries(
        stamps: Vec<SourceStamp>,
        labels: HashMap<String, String>,
        entries: HashMap<String, CacheEntry>,
    ) -> Self {
        Self {
            stamps,
            labels,
            entries,
        }
    }

    /// A translated display label captured at import (see
    /// [`crate::stats::RUNTIME_LABEL_TAGS`]).
    pub(crate) fn runtime_label(&self, tag: &str) -> Option<&str> {
        self.labels.get(tag).map(String::as_str)
    }

    /// The source files this cache was built from.
    #[must_use]
    pub fn stamps(&self) -> &[SourceStamp] {
        &self.stamps
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entry(&self, id: &RecordId) -> Option<&CacheEntry> {
        self.entries.get(&normalize(id.as_str()))
    }

    /// Localized name of a record, mirroring
    /// [`crate::gamedata::GameData::record_name`].
    #[must_use]
    pub fn record_name(&self, id: &RecordId) -> Option<String> {
        self.entry(id)?.name.clone()
    }

    /// Grid footprint for an item, with the same conservative fallback
    /// as the live database.
    #[must_use]
    pub fn item_footprint(&self, item: &Item) -> (i32, i32) {
        self.entry(&item.base)
            .map_or(FALLBACK_FOOTPRINT, |entry| entry.footprint)
    }

    /// Decoded icon for an item, when one was imported.
    #[must_use]
    pub fn item_icon(&self, item: &Item) -> Option<RgbaImage> {
        let icon = self.entry(&item.base)?.icon.as_ref()?;
        let mut pixels = Vec::new();
        ZlibDecoder::new(icon.zlib_rgba.as_slice())
            .read_to_end(&mut pixels)
            .ok()?;
        Some(RgbaImage {
            width: usize::try_from(icon.width).ok()?,
            height: usize::try_from(icon.height).ok()?,
            pixels,
        })
    }

    /// Serializes the cache to its file format.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_i32(&mut out, MAGIC);
        write_i32(&mut out, i32::try_from(self.stamps.len()).unwrap_or(0));
        for stamp in &self.stamps {
            write_utf8(&mut out, &stamp.path);
            write_i64(&mut out, stamp.size);
            write_i64(&mut out, stamp.mtime_seconds);
        }
        write_i32(&mut out, i32::try_from(self.labels.len()).unwrap_or(0));
        let mut label_keys: Vec<&String> = self.labels.keys().collect();
        label_keys.sort_unstable();
        for key in label_keys {
            write_utf8(&mut out, key);
            write_utf8(&mut out, &self.labels[key]);
        }
        write_i32(&mut out, i32::try_from(self.entries.len()).unwrap_or(0));
        let mut paths: Vec<&String> = self.entries.keys().collect();
        paths.sort_unstable();
        for path in paths {
            let entry = &self.entries[path];
            write_utf8(&mut out, path);
            match &entry.name {
                Some(name) => {
                    write_i32(&mut out, 1);
                    write_utf8(&mut out, name);
                }
                None => write_i32(&mut out, 0),
            }
            write_i32(&mut out, entry.footprint.0);
            write_i32(&mut out, entry.footprint.1);
            write_classification(&mut out, entry.classification);
            write_kind(&mut out, entry.kind);
            match entry
                .stats
                .as_ref()
                .and_then(|stats| serde_json::to_vec(stats).ok())
            {
                Some(json) => {
                    write_i32(&mut out, 1);
                    write_i32(&mut out, i32::try_from(json.len()).unwrap_or(0));
                    out.extend_from_slice(&json);
                }
                None => write_i32(&mut out, 0),
            }
            match &entry.icon {
                Some(icon) => {
                    write_i32(&mut out, 1);
                    write_i32(&mut out, icon.width);
                    write_i32(&mut out, icon.height);
                    write_i32(&mut out, i32::try_from(icon.zlib_rgba.len()).unwrap_or(0));
                    out.extend_from_slice(&icon.zlib_rgba);
                }
                None => write_i32(&mut out, 0),
            }
        }
        out
    }

    /// Parses a cache file.
    ///
    /// # Errors
    /// [`CacheError`] on a foreign or truncated file.
    pub fn from_bytes(data: &[u8]) -> Result<Self, CacheError> {
        let mut reader = ByteReader::new(data);
        if reader.read_i32()? != MAGIC {
            return Err(CacheError::BadMagic);
        }
        let stamp_count = usize::try_from(reader.read_i32()?).map_err(|_| CacheError::BadMagic)?;
        let mut stamps = Vec::with_capacity(stamp_count);
        for _ in 0..stamp_count {
            stamps.push(SourceStamp {
                path: read_utf8(&mut reader)?,
                size: reader.read_i64()?,
                mtime_seconds: reader.read_i64()?,
            });
        }
        let label_count = usize::try_from(reader.read_i32()?).map_err(|_| CacheError::BadMagic)?;
        let mut labels = HashMap::with_capacity(label_count);
        for _ in 0..label_count {
            let key = read_utf8(&mut reader)?;
            let value = read_utf8(&mut reader)?;
            labels.insert(key, value);
        }
        let entry_count = usize::try_from(reader.read_i32()?).map_err(|_| CacheError::BadMagic)?;
        let mut entries = HashMap::with_capacity(entry_count);
        for _ in 0..entry_count {
            let path = read_utf8(&mut reader)?;
            let name = if reader.read_i32()? == 1 {
                Some(read_utf8(&mut reader)?)
            } else {
                None
            };
            let footprint = (reader.read_i32()?, reader.read_i32()?);
            let classification = read_classification(&mut reader)?;
            let kind = read_kind(&mut reader)?;
            let stats = if reader.read_i32()? == 1 {
                let length =
                    usize::try_from(reader.read_i32()?).map_err(|_| CacheError::Corrupt)?;
                let json = reader.read_bytes(length)?;
                Some(serde_json::from_slice(json).map_err(|_| CacheError::Corrupt)?)
            } else {
                None
            };
            let icon = if reader.read_i32()? == 1 {
                let width = reader.read_i32()?;
                let height = reader.read_i32()?;
                let length =
                    usize::try_from(reader.read_i32()?).map_err(|_| CacheError::BadMagic)?;
                let zlib_rgba = reader.read_bytes(length)?.to_vec();
                Some(CachedIcon {
                    width,
                    height,
                    zlib_rgba,
                })
            } else {
                None
            };
            entries.insert(
                path,
                CacheEntry {
                    name,
                    footprint,
                    classification,
                    kind,
                    stats,
                    icon,
                },
            );
        }
        Ok(Self {
            stamps,
            labels,
            entries,
        })
    }
}

fn write_classification(buf: &mut Vec<u8>, classification: Classification) {
    write_i32(
        buf,
        match classification {
            Classification::Other => 0,
            Classification::Broken => 1,
            Classification::Rare => 2,
            Classification::Epic => 3,
            Classification::Legendary => 4,
        },
    );
}

fn read_classification(reader: &mut ByteReader<'_>) -> Result<Classification, CacheError> {
    match reader.read_i32()? {
        0 => Ok(Classification::Other),
        1 => Ok(Classification::Broken),
        2 => Ok(Classification::Rare),
        3 => Ok(Classification::Epic),
        4 => Ok(Classification::Legendary),
        _ => Err(CacheError::Corrupt),
    }
}

fn write_kind(buf: &mut Vec<u8>, kind: ItemKind) {
    match kind {
        ItemKind::Gear => write_i32(buf, 0),
        ItemKind::Artifact => write_i32(buf, 1),
        ItemKind::Formula => write_i32(buf, 2),
        ItemKind::Scroll => write_i32(buf, 3),
        ItemKind::Potion => write_i32(buf, 4),
        ItemKind::RelicOrCharm {
            completed_level,
            is_charm,
        } => {
            write_i32(buf, 5);
            match completed_level {
                Some(level) => {
                    write_i32(buf, 1);
                    write_i32(buf, level);
                }
                None => write_i32(buf, 0),
            }
            write_i32(buf, i32::from(is_charm));
        }
        ItemKind::Quest => write_i32(buf, 6),
    }
}

fn read_kind(reader: &mut ByteReader<'_>) -> Result<ItemKind, CacheError> {
    match reader.read_i32()? {
        0 => Ok(ItemKind::Gear),
        1 => Ok(ItemKind::Artifact),
        2 => Ok(ItemKind::Formula),
        3 => Ok(ItemKind::Scroll),
        4 => Ok(ItemKind::Potion),
        5 => {
            let completed_level = if reader.read_i32()? == 1 {
                Some(reader.read_i32()?)
            } else {
                None
            };
            let is_charm = reader.read_i32()? == 1;
            Ok(ItemKind::RelicOrCharm {
                completed_level,
                is_charm,
            })
        }
        6 => Ok(ItemKind::Quest),
        _ => Err(CacheError::Corrupt),
    }
}

fn write_utf8(buf: &mut Vec<u8>, value: &str) {
    write_i32(buf, i32::try_from(value.len()).unwrap_or(0));
    buf.extend_from_slice(value.as_bytes());
}

fn read_utf8(reader: &mut ByteReader<'_>) -> Result<String, ReadError> {
    let length = usize::try_from(reader.read_i32()?).unwrap_or(0);
    Ok(String::from_utf8_lossy(reader.read_bytes(length)?).into_owned())
}

pub(crate) fn compress_icon(image: &RgbaImage) -> Option<CachedIcon> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&image.pixels).ok()?;
    Some(CachedIcon {
        width: i32::try_from(image.width).ok()?,
        height: i32::try_from(image.height).ok()?,
        zlib_rgba: encoder.finish().ok()?,
    })
}
