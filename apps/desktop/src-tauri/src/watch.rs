//! Watches the configuration folder and reports changes to `settings.yaml`, `icons.yaml`, `commands/*.yaml` and the
//! images in `icons/`.
//!
//! Editors and `git pull` touch a file several times in a row (temporary file, rename, attribute change), and a pull
//! can change hundreds of files at once. Changes are therefore reported once things have been quiet for
//! [`QUIET`], not once per event.

use flintkey_core::icon;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const QUIET: Duration = Duration::from_millis(300);

/// The paths a change is reported for.
#[derive(Debug, Clone)]
pub struct Watched {
    settings: PathBuf,
    icons_file: PathBuf,
    icons_dir: PathBuf,
    commands: PathBuf,
}

impl Watched {
    /// `dir` must be the canonical path: macOS reports events with symlinks resolved, so a configuration folder kept
    /// in a Git checkout and linked into `~/.config` would otherwise never match.
    pub fn new(dir: &Path) -> Self {
        let commands = dir.join("commands");
        let icons_dir = dir.join(icon::ICONS_DIR);
        Self {
            settings: dir.join("settings.yaml"),
            icons_file: dir.join(icon::ICONS_FILE),
            icons_dir: icons_dir.canonicalize().unwrap_or(icons_dir),
            commands: commands.canonicalize().unwrap_or(commands),
        }
    }

    /// Whether a change at `path` can change what the app loads. Hidden files (the app's own temporary files, editor
    /// swap files) and files in subfolders are not loaded, so they are not changes.
    pub fn concerns(&self, path: &Path) -> bool {
        if path == self.settings || path == self.icons_file || path == self.commands {
            return true;
        }
        let hidden = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        // Any image in the icons folder, at any depth: commands refer to them by relative path.
        if path.starts_with(&self.icons_dir) {
            return !hidden;
        }
        path.parent() == Some(self.commands.as_path())
            && path.extension().is_some_and(|extension| extension == "yaml")
            && !hidden
    }
}

impl Watched {
    /// Whether an event can change what the app loads. Windows also reports a change to the `commands` folder itself
    /// whenever a file in it is written, hidden ones included; the files are reported on their own, so a modification
    /// of the folder is not a change (its creation, removal or renaming still is).
    pub fn concerns_event(&self, kind: &notify::EventKind, paths: &[PathBuf]) -> bool {
        paths
            .iter()
            .any(|path| self.concerns(path) && !(is_content_modification(kind) && path == &self.commands))
    }
}

/// A modification other than a rename, which notify also reports as a modification.
fn is_content_modification(kind: &notify::EventKind) -> bool {
    matches!(kind, notify::EventKind::Modify(modify) if !matches!(modify, notify::event::ModifyKind::Name(_)))
}

/// Starts watching `dir`, creating it and its `commands` folder if they are missing. `on_change` runs on a
/// background thread after each burst of relevant changes. The watch stops when the returned watcher is dropped.
pub fn start(dir: &Path, on_change: impl Fn() + Send + 'static) -> notify::Result<RecommendedWatcher> {
    std::fs::create_dir_all(dir.join("commands"))?;
    let dir = dir.canonicalize()?;
    let watched = Watched::new(&dir);
    let (sender, receiver) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        // An error (e.g. the event queue overflowed) may hide changes, so treat it as one.
        let relevant = result.map_or(true, |event| watched.concerns_event(&event.kind, &event.paths));
        if relevant {
            let _ = sender.send(());
        }
    })?;
    watcher.watch(&dir, RecursiveMode::Recursive)?;
    std::thread::spawn(move || {
        // Ends when the watcher, and with it the sender, is dropped.
        while receiver.recv().is_ok() {
            while receiver.recv_timeout(QUIET).is_ok() {}
            on_change();
        }
    });
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn a_modification_of_the_commands_folder_itself_is_not_a_change() {
        // Arrange
        use notify::EventKind;
        use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
        let watched = Watched::new(Path::new("/config"));
        let folder = [PathBuf::from("/config/commands")];
        let file = [PathBuf::from("/config/commands/site.yaml")];

        // Act / Assert
        assert!(!watched.concerns_event(&EventKind::Modify(ModifyKind::Any), &folder));
        assert!(watched.concerns_event(&EventKind::Remove(RemoveKind::Folder), &folder));
        assert!(watched.concerns_event(&EventKind::Create(CreateKind::Folder), &folder));
        assert!(watched.concerns_event(&EventKind::Modify(ModifyKind::Name(RenameMode::Any)), &folder));
        assert!(watched.concerns_event(&EventKind::Modify(ModifyKind::Any), &file));
    }

    #[test]
    fn only_files_the_app_loads_are_changes() {
        // Arrange
        let watched = Watched::new(Path::new("/config"));

        // Act / Assert
        for path in [
            "/config/settings.yaml",
            "/config/icons.yaml",
            "/config/icons/work.png",
            "/config/icons/sites/docs.svg",
            "/config/commands/site.yaml",
            "/config/commands",
        ] {
            assert!(watched.concerns(Path::new(path)), "{path}");
        }
        for path in [
            "/config/commands/.site.yaml.123.0.tmp",
            "/config/commands/.site.yaml.swp",
            "/config/commands/notes.md",
            "/config/commands/old.yml",
            "/config/commands/archive/site.yaml",
            "/config/icons/.work.png.swp",
            "/config/README.md",
            "/config/.git/index",
            "/elsewhere/settings.yaml",
        ] {
            assert!(!watched.concerns(Path::new(path)), "{path}");
        }
    }

    fn wait_for(count: &AtomicUsize, at_least: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_burst_of_writes_is_reported_once_and_hidden_files_are_not_reported() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&count);
        let _watcher = start(dir.path(), move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        // Let the watch settle; FSEvents can report changes made just before it started.
        std::thread::sleep(Duration::from_millis(500));
        count.store(0, Ordering::SeqCst);

        // Act: what `git pull` does to several files, then a change the app does not load.
        for index in 0..20 {
            std::fs::write(dir.path().join(format!("commands/c{index}.yaml")), "title: x\n").unwrap();
        }
        let reported = wait_for(&count, 1);
        std::thread::sleep(QUIET * 3);
        let after_burst = count.load(Ordering::SeqCst);
        std::fs::write(dir.path().join("commands/.hidden.yaml"), "x").unwrap();
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        std::thread::sleep(QUIET * 4);

        // Assert
        assert!(reported, "the burst was never reported");
        assert_eq!(after_burst, 1, "a burst should be reported once");
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "hidden and unrelated files were reported"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_symlink_is_watched() {
        // Arrange: the configuration kept in a Git checkout and linked into place.
        let parent = tempfile::tempdir().unwrap();
        let real = parent.path().join("dotfiles/flintkey");
        std::fs::create_dir_all(real.join("commands")).unwrap();
        let link = parent.path().join("config-flintkey");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&count);
        let _watcher = start(&link, move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        count.store(0, Ordering::SeqCst);

        // Act
        std::fs::write(link.join("commands/site.yaml"), "title: x\n").unwrap();

        // Assert
        assert!(wait_for(&count, 1), "a change through the symlink was never reported");
    }

    #[test]
    fn settings_changes_are_reported_and_missing_folders_are_created() {
        // Arrange
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("flintkey");
        let count = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&count);

        // Act
        let _watcher = start(&dir, move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        count.store(0, Ordering::SeqCst);
        std::fs::write(dir.join("settings.yaml"), "hotkey: ctrl+alt+k\n").unwrap();

        // Assert
        assert!(dir.join("commands").is_dir());
        assert!(wait_for(&count, 1), "the settings change was never reported");
    }
}
