//! How many times each command has run on this machine.
//!
//! This is state, not configuration. It changes on every launch and differs per machine,
//! so it lives outside the Git-managed config folder.

use crate::atomic::write_atomically;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub const USAGE_FILE: &str = "usage.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    counts: BTreeMap<String, u64>,
}

impl Usage {
    /// Reads the usage file. Starts from zero if the file is missing or broken.
    /// Losing the counts only changes the list order; it is no reason to stop the launcher.
    pub fn load(path: &Path) -> Usage {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("usage counts serialize");
        write_atomically(path, &(text + "\n"))
    }

    pub fn record(&mut self, id: &str) {
        *self.counts.entry(id.to_string()).or_default() += 1;
    }

    pub fn count(&self, id: &str) -> u64 {
        self.counts.get(id).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_survive_a_save_and_load() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(USAGE_FILE);
        let mut usage = Usage::default();
        usage.record("deploy");
        usage.record("deploy");
        usage.record("ブログ");

        // Act
        usage.save(&path).unwrap();
        let loaded = Usage::load(&path);

        // Assert
        assert_eq!(loaded, usage);
        assert_eq!(
            (loaded.count("deploy"), loaded.count("ブログ"), loaded.count("never")),
            (2, 1, 0)
        );
    }

    #[test]
    fn missing_or_damaged_file_starts_from_zero() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let damaged = dir.path().join(USAGE_FILE);
        std::fs::write(&damaged, "{ not json").unwrap();

        // Act
        let from_missing = Usage::load(&dir.path().join("absent.json"));
        let from_damaged = Usage::load(&damaged);

        // Assert
        assert_eq!(from_missing, Usage::default());
        assert_eq!(from_damaged, Usage::default());
    }
}
