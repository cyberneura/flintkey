//! Where Flintkey keeps its files.
//!
//! Configuration lives in `~/.config/flintkey` on every OS, so one Git repository can be shared
//! across all machines. Machine-specific state such as usage counts lives elsewhere;
//! in the config folder it would produce a diff on every launch.

use crate::command::Os;
use std::path::{Path, PathBuf};

/// Overrides the config folder. For tests, and for trying another config without touching the real one.
pub const CONFIG_DIR_ENV: &str = "FLINTKEY_CONFIG_DIR";
/// Overrides the state folder.
pub const STATE_DIR_ENV: &str = "FLINTKEY_STATE_DIR";

pub fn config_dir_in(home: &Path, override_dir: Option<&str>) -> PathBuf {
    match override_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join(".config").join("flintkey"),
    }
}

pub fn state_dir_in(os: Os, home: &Path, local_app_data: Option<&str>, override_dir: Option<&str>) -> PathBuf {
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    match (os, local_app_data.filter(|dir| !dir.is_empty())) {
        (Os::Windows, Some(local)) => PathBuf::from(local).join("flintkey"),
        _ => home.join(".local").join("state").join("flintkey"),
    }
}

/// The current user's config folder. `None` if there is no home directory.
pub fn config_dir() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    Some(config_dir_in(&home, std::env::var(CONFIG_DIR_ENV).ok().as_deref()))
}

/// The current user's state folder. `None` if there is no home directory.
pub fn state_dir() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    Some(state_dir_in(
        Os::current(),
        &home,
        std::env::var("LOCALAPPDATA").ok().as_deref(),
        std::env::var(STATE_DIR_ENV).ok().as_deref(),
    ))
}

/// Replaces a leading `~` with the home directory. `~user` is left as is.
pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_is_dot_config_on_every_os() {
        // Arrange
        let home = Path::new("/home/me");

        // Act
        let dir = config_dir_in(home, None);

        // Assert
        assert_eq!(dir, Path::new("/home/me/.config/flintkey"));
    }

    #[test]
    fn override_replaces_the_config_dir_and_empty_override_is_ignored() {
        // Arrange
        let home = Path::new("/home/me");

        // Act + Assert
        assert_eq!(config_dir_in(home, Some("/tmp/cfg")), Path::new("/tmp/cfg"));
        assert_eq!(config_dir_in(home, Some("")), Path::new("/home/me/.config/flintkey"));
    }

    #[test]
    fn state_dir_follows_each_os_convention() {
        // Arrange
        let home = Path::new("/home/me");

        // Act + Assert
        assert_eq!(
            state_dir_in(Os::Macos, home, None, None),
            Path::new("/home/me/.local/state/flintkey")
        );
        assert_eq!(
            state_dir_in(Os::Linux, home, Some("ignored"), None),
            Path::new("/home/me/.local/state/flintkey")
        );
        assert_eq!(
            state_dir_in(Os::Windows, home, Some("C:/Local"), None),
            Path::new("C:/Local/flintkey")
        );
        assert_eq!(
            state_dir_in(Os::Windows, home, None, None),
            Path::new("/home/me/.local/state/flintkey")
        );
        assert_eq!(
            state_dir_in(Os::Windows, home, Some("C:/Local"), Some("/tmp/state")),
            Path::new("/tmp/state")
        );
    }

    #[test]
    fn tilde_is_expanded_only_at_the_start() {
        // Arrange
        let home = Path::new("/home/me");

        // Act + Assert
        assert_eq!(expand_home("~", home), Path::new("/home/me"));
        assert_eq!(expand_home("~/work/x", home), Path::new("/home/me/work/x"));
        assert_eq!(expand_home("/abs/~/x", home), Path::new("/abs/~/x"));
        assert_eq!(expand_home("~other/x", home), Path::new("~other/x"));
        assert_eq!(expand_home("relative", home), Path::new("relative"));
    }
}
