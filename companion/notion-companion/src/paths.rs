//! XDG resolution of the *shared* vault the main app and companion both use.
//!
//! The main app is a Tauri app with identifier `co.merai.notion`, so on Linux
//! its `app_data_dir()` resolves to `$XDG_DATA_HOME/co.merai.notion` (falling
//! back to `~/.local/share/co.merai.notion`). The companion must point at the
//! exact same directory — that shared file is the whole memory-saving strategy.

use std::path::PathBuf;

/// Tauri application identifier — the app-data subdirectory name.
pub const APP_ID: &str = "co.merai.notion";
/// Encrypted database filename inside the app-data directory.
pub const DB_FILE: &str = "notion.db";
/// Non-secret vault metadata filename.
pub const VAULT_FILE: &str = "vault.json";

/// The shared app-data directory (`$XDG_DATA_HOME/co.merai.notion` or
/// `~/.local/share/co.merai.notion`). Returns `None` only if neither
/// `XDG_DATA_HOME` nor `HOME` is set.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join(APP_ID));
        }
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join(APP_ID)
    })
}

/// Full path to the shared encrypted database.
pub fn db_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join(DB_FILE))
}

/// Full path to the vault metadata file.
pub fn vault_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join(VAULT_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env_lock;

    #[test]
    fn prefers_xdg_data_home() {
        let _g = env_lock();
        let prev = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", "/custom/data");
        assert_eq!(
            db_path().unwrap(),
            PathBuf::from("/custom/data/co.merai.notion/notion.db")
        );
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn falls_back_to_home_local_share() {
        let _g = env_lock();
        let prev_xdg = std::env::var_os("XDG_DATA_HOME");
        let prev_home = std::env::var_os("HOME");
        std::env::remove_var("XDG_DATA_HOME");
        std::env::set_var("HOME", "/home/tester");
        assert_eq!(
            data_dir().unwrap(),
            PathBuf::from("/home/tester/.local/share/co.merai.notion")
        );
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
