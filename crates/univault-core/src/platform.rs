//! Per-OS path conventions — the one place `cfg(target_os)` is
//! allowed (ARCHITECTURE.md's platform-confinement rule). Functions
//! here compute paths from environment variables; reading or writing
//! the filesystem stays in the shell.

use std::path::PathBuf;

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
}
