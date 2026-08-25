//! The shell's file-writing path, implementing ARCHITECTURE.md's
//! backup-first rule: the backup exists (and is synced) on disk
//! before the original is touched. v1 backups are timestamped
//! siblings, e.g. `Player.chr.univault-bak-1756070000`, rotated to
//! the [`MAX_BACKUPS`] newest per file.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many backups of one file are kept; older ones are pruned
/// after each successful backup.
pub const MAX_BACKUPS: usize = 5;

/// Backs `path` up beside itself, syncs the backup, prunes backups
/// beyond [`MAX_BACKUPS`], then overwrites `path` with `bytes` and
/// syncs. Returns the backup's path. When `path` does not exist yet
/// (a new vault), it is simply created.
pub fn backup_first_write(path: &Path, bytes: &[u8]) -> io::Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let backup = fresh_backup_path(path);
        fs::copy(path, &backup).map_err(|error| step("copying backup", &error))?;
        best_effort_sync(&backup).map_err(|error| step("syncing backup", &error))?;
        prune_backups(path);
        Some(backup)
    } else {
        None
    };
    fs::write(path, bytes).map_err(|error| step("writing file", &error))?;
    best_effort_sync(path).map_err(|error| step("syncing file", &error))?;
    Ok(backup)
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
