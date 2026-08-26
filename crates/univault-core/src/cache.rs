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
//! "Jǫrmungandr"): `UVC6` magic, source-fingerprint list (path, size,
//! mtime seconds), a table of runtime display labels, then entries
//! keyed by normalized record path: name, footprint, classification
//! and kind tags, an optional JSON-encoded stat block, an optional
//! zlib-compressed RGBA icon, an optional second icon (the relic/
//! charm shard art shown while incomplete), and the completion-bonus
//! table (record path + weight pairs) for relics, charms, and
//! artifacts (an artifact inherits its formula's table).

use std::collections::HashMap;
use std::io::Read;

use flate2::Compression;
use flate2::read::ZlibDecoder;

use crate::arz::normalize;
use crate::chr::{Item, RecordId};
use crate::gamedata::FALLBACK_FOOTPRINT;
use crate::reader::{ByteReader, ReadError};
use crate::stats::StatBlock;
use crate::style::{Classification, GearSlot, ItemKind};
use crate::tex::RgbaImage;
use crate::writer::{write_i32, write_i64};

// The magic is the cache's only version: bump it for layout changes
// AND for content-generation changes (names, stat rendering), so
// existing caches rebuild automatically.
const MAGIC: i32 = 0x3743_5655; // "UVC7"

fn gear_slot_index(slot: Option<GearSlot>) -> i32 {
    slot.and_then(|slot| GearSlot::ALL.iter().position(|other| *other == slot))
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(-1)
}

fn gear_slot_from_index(index: i32) -> Result<Option<GearSlot>, CacheError> {
    if index == -1 {
        return Ok(None);
    }
    usize::try_from(index)
        .ok()
        .and_then(|index| GearSlot::ALL.get(index).copied())
        .map(Some)
        .ok_or(CacheError::Corrupt)
}

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
    /// The shard art a relic/charm shows while incomplete.
    pub(crate) shard_icon: Option<CachedIcon>,
    /// Completion bonuses a relic/charm can roll: game-style record
    /// path + table weight, in table order.
    pub(crate) bonuses: Vec<(String, i32)>,
    /// The equipment family of a gear record; `None` for everything
    /// that cannot take a socket.
    pub(crate) gear_slot: Option<GearSlot>,
    /// A relic/charm record's allow-flags as a bitmask over
    /// [`GearSlot::ALL`] order; 0 for non-relics.
    pub(crate) socket_targets: u16,
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

    /// Decoded icon for an item, when one was imported. Incomplete
    /// relics and charms get their shard art so partials read at a
    /// glance; everything else (and completed pieces) gets the
    /// record's main icon.
    #[must_use]
    pub fn item_icon(&self, item: &Item) -> Option<RgbaImage> {
        let entry = self.entry(&item.base)?;
        let icon = if self.is_incomplete_relic(item) {
            entry.shard_icon.as_ref().or(entry.icon.as_ref())
        } else {
            entry.icon.as_ref()
        }?;
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

    /// The shard count that completes a relic/charm record; `None`
    /// for other kinds (or when the record is unknown).
    #[must_use]
    pub fn completed_relic_level(&self, id: &RecordId) -> Option<i32> {
        match self.entry(id)?.kind {
            ItemKind::RelicOrCharm {
                completed_level, ..
            } => completed_level,
            ItemKind::Gear
            | ItemKind::Artifact
            | ItemKind::Formula
            | ItemKind::Scroll
            | ItemKind::Potion
            | ItemKind::Quest => None,
        }
    }

    /// Whether an item is a relic/charm still short of its completion
    /// level (`var1` is the shard count).
    #[must_use]
    pub fn is_incomplete_relic(&self, item: &Item) -> bool {
        self.completed_relic_level(&item.base)
            .is_some_and(|needed| item.var1 < needed)
    }

    /// The completion bonuses a relic/charm/artifact can roll, as
    /// game-style record paths with their table weights; empty for
    /// other kinds (an artifact inherits its formula's table).
    #[must_use]
    pub fn relic_bonuses(&self, id: &RecordId) -> &[(String, i32)] {
        self.entry(id).map_or(&[], |entry| entry.bonuses.as_slice())
    }

    /// The equipment family of a gear record; `None` for anything
    /// that cannot take a socket (relics, potions, artifacts, …).
    #[must_use]
    pub fn gear_slot(&self, id: &RecordId) -> Option<GearSlot> {
        self.entry(id)?.gear_slot
    }

    /// Whether the record is an artifact — the one equippable that
    /// carries no [`GearSlot`] family (only the artifact slot takes
    /// it).
    #[must_use]
    pub fn is_artifact(&self, id: &RecordId) -> bool {
        self.entry(id)
            .is_some_and(|entry| matches!(entry.kind, ItemKind::Artifact))
    }

    /// Whether the relic/charm record's own allow-flags permit the
    /// given equipment family. Rarity is deliberately not part of
    /// this — only the game's type rules are.
    #[must_use]
    pub fn relic_allows(&self, relic: &RecordId, slot: GearSlot) -> bool {
        self.entry(relic).is_some_and(|entry| {
            let index = GearSlot::ALL.iter().position(|other| *other == slot);
            index.is_some_and(|index| entry.socket_targets & (1 << index) != 0)
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
            for icon in [&entry.icon, &entry.shard_icon] {
                match icon {
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
            write_i32(&mut out, i32::try_from(entry.bonuses.len()).unwrap_or(0));
            for (record, weight) in &entry.bonuses {
                write_utf8(&mut out, record);
                write_i32(&mut out, *weight);
            }
            write_i32(&mut out, gear_slot_index(entry.gear_slot));
            write_i32(&mut out, i32::from(entry.socket_targets));
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
            let read_icon =
                |reader: &mut ByteReader<'_>| -> Result<Option<CachedIcon>, CacheError> {
                    if reader.read_i32()? != 1 {
                        return Ok(None);
                    }
                    let width = reader.read_i32()?;
                    let height = reader.read_i32()?;
                    let length =
                        usize::try_from(reader.read_i32()?).map_err(|_| CacheError::BadMagic)?;
                    let zlib_rgba = reader.read_bytes(length)?.to_vec();
                    Ok(Some(CachedIcon {
                        width,
                        height,
                        zlib_rgba,
                    }))
                };
            let icon = read_icon(&mut reader)?;
            let shard_icon = read_icon(&mut reader)?;
            let bonus_count =
                usize::try_from(reader.read_i32()?).map_err(|_| CacheError::Corrupt)?;
            let mut bonuses = Vec::with_capacity(bonus_count);
            for _ in 0..bonus_count {
                let record = read_utf8(&mut reader)?;
                let weight = reader.read_i32()?;
                bonuses.push((record, weight));
            }
            let gear_slot = gear_slot_from_index(reader.read_i32()?)?;
            let socket_targets =
                u16::try_from(reader.read_i32()?).map_err(|_| CacheError::Corrupt)?;
            entries.insert(
                path,
                CacheEntry {
                    name,
                    footprint,
                    classification,
                    kind,
                    stats,
                    icon,
                    shard_icon,
                    bonuses,
                    gear_slot,
                    socket_targets,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_header(classification_tag: i32) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_i32(&mut bytes, MAGIC);
        write_i32(&mut bytes, 0); // stamps
        write_i32(&mut bytes, 0); // labels
        write_i32(&mut bytes, 1); // entries
        write_utf8(&mut bytes, "RECORDS\\ITEM\\X.DBR");
        write_i32(&mut bytes, 0); // no name
        write_i32(&mut bytes, 1); // footprint
        write_i32(&mut bytes, 1);
        write_i32(&mut bytes, classification_tag);
        bytes
    }

    #[test]
    fn unknown_classification_tag_is_corrupt() {
        let bytes = entry_header(9);
        assert!(matches!(
            GameCache::from_bytes(&bytes),
            Err(CacheError::Corrupt)
        ));
    }

    #[test]
    fn unknown_kind_tag_is_corrupt() {
        let mut bytes = entry_header(0);
        write_i32(&mut bytes, 9);
        assert!(matches!(
            GameCache::from_bytes(&bytes),
            Err(CacheError::Corrupt)
        ));
    }

    #[test]
    fn undecodable_stats_blob_is_corrupt() {
        let mut bytes = entry_header(0);
        write_i32(&mut bytes, 0); // kind: gear
        write_i32(&mut bytes, 1); // stats present
        write_i32(&mut bytes, 4);
        bytes.extend_from_slice(b"?!?!");
        assert!(matches!(
            GameCache::from_bytes(&bytes),
            Err(CacheError::Corrupt)
        ));
    }

    #[test]
    fn truncated_cache_is_a_read_error() {
        let mut bytes = Vec::new();
        write_i32(&mut bytes, MAGIC);
        write_i32(&mut bytes, 3); // claims stamps that are not there
        assert!(matches!(
            GameCache::from_bytes(&bytes),
            Err(CacheError::Read(_))
        ));
    }

    #[test]
    fn foreign_file_is_bad_magic() {
        assert!(matches!(
            GameCache::from_bytes(b"PK\x03\x04not a cache"),
            Err(CacheError::BadMagic)
        ));
    }
}
