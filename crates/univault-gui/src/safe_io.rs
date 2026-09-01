//! The shell's file IO path for game-owned files: backup-first
//! writes (ARCHITECTURE.md — the backup exists and is synced on disk
//! before the original is touched; v1 backups are timestamped
//! siblings, e.g. `Player.chr.univault-bak-1756070000`, rotated to
//! the [`MAX_BACKUPS`] newest per file) and cache-bypassing verified
//! reads.
//!
//! Reads and writes here stay out of the local page cache. The save
//! tree lives on an SMB mount that other machines write — the game
//! runs elsewhere — and macOS has served *stale cached pages under a
//! fresh stamp* for such a file, minutes after another client rewrote
//! it. Bytes this app itself cached while writing were exactly what a
//! later launch was fed back, so both directions bypass the cache,
//! and every read is length-checked against the file's own metadata
//! to turn the surviving stale-read shapes into retryable errors
//! instead of clean parses of the wrong bytes.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many backups of one file are kept; older ones are pruned
/// after each successful backup.
pub const MAX_BACKUPS: usize = 5;

/// Reads the whole file, bypassing the local page cache and verifying
/// the byte count against the file's own metadata. A count that
/// disagrees is returned as an error — the cache or the mount served
/// bytes that are not the file — so callers' existing retry machinery
/// treats it as the failed read it is, rather than parsing stale
/// bytes that look right.
pub fn read_verified(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    set_nocache(&file);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let on_disk = file.metadata()?.len();
    match length_mismatch(bytes.len(), on_disk) {
        None => Ok(bytes),
        Some(mismatch) => Err(io::Error::new(io::ErrorKind::InvalidData, mismatch)),
    }
}

/// Backs `path` up beside itself, syncs the backup, prunes backups
/// beyond [`MAX_BACKUPS`], then overwrites `path` with `bytes` and
/// syncs. Returns the backup's path. When `path` does not exist yet
/// (a new vault), it is simply created. The backup is taken through
/// [`read_verified`] — a save whose baseline cannot be faithfully
/// backed up must fail before the original is touched.
pub fn backup_first_write(path: &Path, bytes: &[u8]) -> io::Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let current = read_verified(path).map_err(|error| step("reading for backup", &error))?;
        let backup = fresh_backup_path(path);
        write_uncached(&backup, &current).map_err(|error| step("copying backup", &error))?;
        copy_permissions(path, &backup);
        best_effort_sync(&backup).map_err(|error| step("syncing backup", &error))?;
        prune_backups(path);
        Some(backup)
    } else {
        None
    };
    write_uncached(path, bytes).map_err(|error| step("writing file", &error))?;
    best_effort_sync(path).map_err(|error| step("syncing file", &error))?;
    Ok(backup)
}

/// Overwrites `path` without taking a fresh backup, still syncing —
/// the autosave path for files already backed up since they were
/// last loaded.
pub fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_uncached(path, bytes).map_err(|error| step("writing file", &error))?;
    best_effort_sync(path)
}

/// Creates or truncates `path` and writes `bytes` without leaving the
/// pages in the local cache — pages cached by our own writes are what
/// a stale-cache read serves back later.
pub fn write_uncached(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    set_nocache(&file);
    file.write_all(bytes)
}

/// `None` when the bytes read match the file's metadata, otherwise
/// the error text naming both counts.
fn length_mismatch(bytes_read: usize, on_disk: u64) -> Option<String> {
    (u64::try_from(bytes_read) != Ok(on_disk)).then(|| {
        format!(
            "read {bytes_read} bytes of a file whose metadata says {on_disk} — \
             a stale or mid-write network read; retrying usually sees the real file"
        )
    })
}

/// Asks the OS to keep this file's bytes out of the page cache.
/// Best-effort: a file that refuses the flag still reads and writes
/// correctly, merely through the cache again.
#[cfg(target_os = "macos")]
fn set_nocache(file: &File) {
    use std::os::fd::AsRawFd;
    // SAFETY: fcntl on a File's own fd, which is open for its
    // lifetime; F_NOCACHE takes a plain integer argument.
    let _ = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
}

#[cfg(not(target_os = "macos"))]
fn set_nocache(_file: &File) {}

/// Best-effort permission mirroring onto a fresh backup, matching
/// what `fs::copy` used to preserve; a failure never fails the save
/// the backup protects.
fn copy_permissions(from: &Path, to: &Path) {
    if let Ok(metadata) = fs::metadata(from) {
        let _ = fs::set_permissions(to, metadata.permissions());
    }
}

/// Deletes the oldest backups of `path` beyond [`MAX_BACKUPS`].
/// Best-effort: a failed prune never fails the save that a fresh,
/// synced backup already protects.
fn prune_backups(path: &Path) {
    let mut backups = existing_backups(path);
    backups.sort_unstable();
    let excess = backups.len().saturating_sub(MAX_BACKUPS);
    for (_, _, old_backup) in backups.into_iter().take(excess) {
        let _ = fs::remove_file(old_backup);
    }
}

/// Every backup of `path` beside it, as `(stamp, slot, path)` — the
/// tuple orders oldest-first.
fn existing_backups(path: &Path) -> Vec<(u64, u32, PathBuf)> {
    let Some(directory) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().map(|name| name.to_string_lossy()) else {
        return Vec::new();
    };
    let prefix = format!("{file_name}.univault-bak-");
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let age = backup_age(name.strip_prefix(prefix.as_str())?)?;
            Some((age.0, age.1, entry.path()))
        })
        .collect()
}

/// Parses a backup suffix, `<stamp>` or `<stamp>-<counter>`, into an
/// ordering key; anything else is not one of our backups.
fn backup_age(suffix: &str) -> Option<(u64, u32)> {
    match suffix.split_once('-') {
        Some((stamp, counter)) => Some((
            stamp.parse().ok()?,
            counter.parse::<u32>().ok()?.checked_add(1)?,
        )),
        None => Some((suffix.parse().ok()?, 0)),
    }
}

fn step(what: &str, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

/// Flush-to-disk, degrading gracefully: macOS `sync_all` issues
/// `F_FULLFSYNC`, which network filesystems (SMB — "os error 45")
/// reject. There the close-flush is the strongest guarantee
/// available, so "unsupported" is not an error. The handle must be
/// writable: Windows' `FlushFileBuffers` denies read-only handles.
fn best_effort_sync(path: &Path) -> io::Result<()> {
    match File::options().write(true).open(path)?.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == io::ErrorKind::Unsupported || error.raw_os_error() == Some(45) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn fresh_backup_path(path: &Path) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    // Rotation ages backups by name, so a new backup must never
    // reuse a pruned slot below the newest existing one — it would
    // be mistaken for the oldest and pruned on the next save.
    let newest = existing_backups(path)
        .into_iter()
        .map(|(stamp, slot, _)| (stamp, slot))
        .max();
    let (stamp, mut counter) = match newest {
        Some((stamp, slot)) if stamp >= now => (stamp, slot),
        _ => (now, 0),
    };
    loop {
        let suffix = if counter == 0 {
            format!("univault-bak-{stamp}")
        } else {
            format!("univault-bak-{stamp}-{counter}")
        };
        let candidate = append_extension(path, &suffix);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

fn append_extension(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from("file"),
        std::ffi::OsStr::to_os_string,
    );
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_exists_with_original_content_after_write() {
        let dir = std::env::temp_dir().join(format!("univault-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Player.chr");
        fs::write(&target, b"original bytes").unwrap();

        let backup = backup_first_write(&target, b"new bytes").unwrap().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new bytes");
        assert_eq!(fs::read(&backup).unwrap(), b"original bytes");

        let second = backup_first_write(&target, b"third").unwrap().unwrap();
        assert_ne!(backup, second);
        assert_eq!(fs::read(&second).unwrap(), b"new bytes");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backups_rotate_keeping_the_newest_five() {
        let dir = std::env::temp_dir().join(format!("univault-rotate-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("Player.chr");
        let decoy = dir.join("Other.chr");
        fs::write(&target, b"v0").unwrap();
        fs::write(&decoy, b"other").unwrap();
        backup_first_write(&decoy, b"other2").unwrap();

        for round in 1..=8_u8 {
            backup_first_write(&target, &[round]).unwrap();
        }
        let mut backups: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("Player.chr.univault-bak-"))
            .collect();
        backups.sort();
        assert_eq!(backups.len(), MAX_BACKUPS, "{backups:?}");
        // The newest backup (of the pre-write content) always survives.
        let newest_content = fs::read(dir.join(backups.last().unwrap())).unwrap();
        assert_eq!(newest_content, vec![7]);
        // The decoy's backup is untouched by Player.chr's rotation.
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Other.chr.univault-bak-"))
                .count(),
            1
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_suffixes_order_by_stamp_then_counter() {
        assert_eq!(backup_age("100"), Some((100, 0)));
        assert_eq!(backup_age("100-1"), Some((100, 2)));
        assert!(backup_age("100-1") > backup_age("100"));
        assert!(backup_age("101") > backup_age("100-9"));
        assert_eq!(backup_age("not-a-stamp"), None);
        assert_eq!(backup_age(""), None);
    }

    #[test]
    fn read_verified_returns_the_file_and_errors_on_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("univault-readv-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("winsys.dxb");
        fs::write(&target, b"stash bytes").unwrap();
        assert_eq!(read_verified(&target).unwrap(), b"stash bytes");
        assert!(read_verified(&dir.join("absent.dxb")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn length_mismatch_names_both_counts_and_passes_agreement() {
        assert_eq!(length_mismatch(19468, 19468), None);
        assert_eq!(length_mismatch(0, 0), None);
        let mismatch = length_mismatch(15234, 19468).unwrap();
        assert!(mismatch.contains("15234"), "{mismatch}");
        assert!(mismatch.contains("19468"), "{mismatch}");
    }

    #[test]
    fn creating_a_new_file_needs_no_backup() {
        let dir = std::env::temp_dir().join(format!("univault-new-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("fresh.json");
        let backup = backup_first_write(&target, b"{}").unwrap();
        assert!(backup.is_none());
        assert_eq!(fs::read(&target).unwrap(), b"{}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
