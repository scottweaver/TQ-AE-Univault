//! Per-OS path conventions — the one place `cfg(target_os)` is
//! allowed (ARCHITECTURE.md's platform-confinement rule) — plus the
//! game save tree's own layout conventions. Functions here compute
//! paths from environment variables and given paths; reading or
//! writing the filesystem stays in the shell.

use std::path::{Path, PathBuf};

/// The app's configuration directory for this platform (not created
/// here): `~/Library/Application Support/tq-univault` on macOS,
/// `%APPDATA%\tq-univault` on Windows, XDG config on Linux.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/tq-univault"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("tq-univault"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|base| base.join("tq-univault"))
    }
}

/// The character's own bank beside its `Player.chr`: the game keeps
/// each character's private stash as `winsys.dxb` in the character
/// folder. Purely a path computation — the file exists only once the
/// character has used the caravan in game.
#[must_use]
pub fn personal_stash_path(chr_path: &Path) -> Option<PathBuf> {
    chr_path.parent().map(|dir| dir.join("winsys.dxb"))
}

/// Candidate paths for the account-wide transfer stash (the shared
/// bank), nearest first: `<ancestor>/Sys/winsys.dxb` for each
/// ancestor of the character folder. The save tree keeps it at
/// `SaveData/Sys/winsys.dxb` next to `SaveData/Main/_Name/`, but
/// walking every ancestor also covers relocated or custom-map save
/// roots. The shell takes the first candidate that exists on disk.
pub fn transfer_stash_candidates(chr_path: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    sys_file_candidates(chr_path, "winsys.dxb")
}

/// Candidate paths for the account-wide relic bank (Atlantis and
/// later; `TQVaultAE`'s "relic vault stash"), nearest first. Same
/// `Sys` folder as the transfer stash, stored as `miscsys.dxb` in
/// the identical stash format.
pub fn relic_bank_candidates(chr_path: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    sys_file_candidates(chr_path, "miscsys.dxb")
}

fn sys_file_candidates<'p>(
    chr_path: &'p Path,
    file_name: &'static str,
) -> impl Iterator<Item = PathBuf> + 'p {
    chr_path
        .ancestors()
        .skip(1)
        .map(move |dir| dir.join("Sys").join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_the_app_name() {
        // Every supported platform derives from env vars that exist
        // in any real session (HOME / APPDATA / XDG_CONFIG_HOME).
        let dir = config_dir().expect("a config dir on a supported platform");
        assert!(dir.ends_with("tq-univault"), "{dir:?}");
    }

    #[test]
    fn personal_stash_sits_beside_the_character_file() {
        assert_eq!(
            personal_stash_path(Path::new("/saves/SaveData/Main/_Pally Don/Player.chr")),
            Some(PathBuf::from("/saves/SaveData/Main/_Pally Don/winsys.dxb"))
        );
        assert_eq!(personal_stash_path(Path::new("/")), None);
    }

    #[test]
    fn transfer_stash_candidates_walk_ancestors_nearest_first() {
        let candidates: Vec<PathBuf> =
            transfer_stash_candidates(Path::new("/saves/SaveData/Main/_Pally Don/Player.chr"))
                .collect();
        assert_eq!(
            candidates[0],
            PathBuf::from("/saves/SaveData/Main/_Pally Don/Sys/winsys.dxb")
        );
        let expected = PathBuf::from("/saves/SaveData/Sys/winsys.dxb");
        assert!(candidates.contains(&expected), "{candidates:?}");
    }

    #[test]
    fn relic_bank_candidates_use_the_miscsys_file() {
        let candidates: Vec<PathBuf> =
            relic_bank_candidates(Path::new("/saves/SaveData/Main/_Pally Don/Player.chr"))
                .collect();
        let expected = PathBuf::from("/saves/SaveData/Sys/miscsys.dxb");
        assert!(candidates.contains(&expected), "{candidates:?}");
    }
}
