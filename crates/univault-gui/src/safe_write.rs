//! The shell's file-writing path, implementing ARCHITECTURE.md's
//! backup-first rule: the backup exists (and is synced) on disk
//! before the original is touched. v1 backups are timestamped
//! siblings, e.g. `Player.chr.univault-bak-1756070000`.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Backs `path` up beside itself, syncs the backup, then overwrites
/// `path` with `bytes` and syncs. Returns the backup's path. When
/// `path` does not exist yet (a new vault), it is simply created.
pub fn backup_first_write(path: &Path, bytes: &[u8]) -> io::Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let backup = fresh_backup_path(path);
        fs::copy(path, &backup)?;
        File::open(&backup)?.sync_all()?;
        Some(backup)
    } else {
        None
    };
    fs::write(path, bytes)?;
    File::open(path)?.sync_all()?;
    Ok(backup)
}

fn fresh_backup_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut counter = 0_u32;
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
