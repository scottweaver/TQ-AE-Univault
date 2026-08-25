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
//! "Jǫrmungandr"): `UVC1` magic, source-fingerprint list (path, size,
//! mtime seconds), then entries keyed by normalized record path:
//! name, footprint, and an optional zlib-compressed RGBA icon.

use std::collections::HashMap;
use std::io::Read;

use flate2::Compression;
use flate2::read::ZlibDecoder;

use crate::arz::normalize;
use crate::chr::{Item, RecordId};
use crate::gamedata::FALLBACK_FOOTPRINT;
use crate::reader::{ByteReader, ReadError};
use crate::tex::RgbaImage;
use crate::writer::{write_i32, write_i64};

const MAGIC: i32 = 0x3143_5655; // "UVC1"

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
    pub(crate) icon: Option<CachedIcon>,
}

/// The runtime item database, loaded from (or about to be saved as)
/// a cache file.
pub struct GameCache {
    stamps: Vec<SourceStamp>,
    entries: HashMap<String, CacheEntry>,
}

/// Errors from reading a cache file.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("not a tq-univault cache file")]
    BadMagic,
    #[error("icon decompression failed: {0}")]
    Icon(std::io::Error),
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl GameCache {
    pub(crate) fn from_entries(
        stamps: Vec<SourceStamp>,
        entries: HashMap<String, CacheEntry>,
    ) -> Self {
        Self { stamps, entries }
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

    /// Localized name of a record, mirroring
    /// [`crate::gamedata::GameData::record_name`].
    #[must_use]
    pub fn record_name(&self, id: &RecordId) -> Option<String> {
        self.entries.get(&normalize(id.as_str()))?.name.clone()
    }

    /// Grid footprint for an item, with the same conservative fallback
    /// as the live database.
    #[must_use]
    pub fn item_footprint(&self, item: &Item) -> (i32, i32) {
        self.entries
            .get(&normalize(item.base.as_str()))
            .map_or(FALLBACK_FOOTPRINT, |entry| entry.footprint)
    }

    /// Decoded icon for an item, when one was imported.
    #[must_use]
    pub fn item_icon(&self, item: &Item) -> Option<RgbaImage> {
        let icon = self
            .entries
            .get(&normalize(item.base.as_str()))?
            .icon
            .as_ref()?;
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
                    icon,
                },
            );
        }
        Ok(Self { stamps, entries })
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
