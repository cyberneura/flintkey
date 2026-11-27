//! Finding installed apps, so they can be searched like commands without writing a file for each.
//!
//! Only macOS has app search. An app is an `.app` bundle in one of the [`folders`], or in a folder directly inside
//! one (`/Applications/Utilities`, the `… Apps.localized` folders browsers make for web apps).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// The bundle's file name without `.app`, as Finder shows it.
    pub name: String,
    pub path: PathBuf,
}

/// The folders searched for apps, in order of precedence: an app in `~/Applications` hides one with the same name
/// further down.
pub fn folders(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Applications"),
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        // Finder is not in an Applications folder.
        PathBuf::from("/System/Library/CoreServices/Finder.app"),
    ]
}

/// The apps in `folders`, sorted by name. A folder that is itself an `.app` is that app. Missing or unreadable
/// folders are skipped; a symlink is followed, so an app built elsewhere can be linked into `~/Applications`.
pub fn scan(folders: &[PathBuf]) -> Vec<InstalledApp> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    let mut add = |path: PathBuf| {
        let Some(name) = app_name(&path) else { return };
        if seen.insert(name.to_lowercase()) {
            apps.push(InstalledApp { name, path });
        }
    };
    for folder in folders {
        if app_name(folder).is_some() {
            if folder.is_dir() {
                add(folder.clone());
            }
            continue;
        }
        for entry in entries(folder) {
            if app_name(&entry).is_some() {
                add(entry);
            } else {
                entries(&entry)
                    .into_iter()
                    .filter(|inner| app_name(inner).is_some())
                    .for_each(&mut add);
            }
        }
    }
    apps.sort_by_cached_key(|app| (app.name.to_lowercase(), app.path.clone()));
    apps
}

/// The folders and bundles directly inside `folder`, hidden ones left out, in name order.
fn entries(folder: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        // `is_dir` follows symlinks.
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    entries
}

fn app_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".app")?;
    (!stem.is_empty()).then(|| stem.to_string())
}

/// The app that `app` (an `app:` value of a command: a name, or a path to a bundle) refers to, if it was found.
pub fn find<'a>(apps: &'a [InstalledApp], app: &str, home: &Path) -> Option<&'a InstalledApp> {
    let expanded = crate::paths::expand_home(app, home);
    if expanded.is_absolute() {
        return apps.iter().find(|installed| installed.path == expanded);
    }
    let name = app.strip_suffix(".app").unwrap_or(app);
    apps.iter().find(|installed| installed.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn bundle(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.join("Contents")).unwrap();
        path
    }

    fn names(apps: &[InstalledApp]) -> Vec<&str> {
        apps.iter().map(|app| app.name.as_str()).collect()
    }

    #[test]
    fn finds_bundles_in_the_folders_and_one_level_below() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Applications");
        bundle(&root, "Editor.app");
        bundle(&root, "Utilities/Terminal.app");
        bundle(&root, "Web Apps.localized/Mail Web.app");
        bundle(&root, "Deep/Nested/Hidden Away.app");
        bundle(&root, ".Hidden.app");
        bundle(&root, "Editor.app/Contents/Helpers/Editor Helper.app");
        fs::write(root.join("notes.app"), "not a bundle").unwrap();

        // Act
        let apps = scan(&[root.clone(), dir.path().join("missing")]);

        // Assert
        assert_eq!(names(&apps), ["Editor", "Mail Web", "Terminal"]);
        assert_eq!(apps[2].path, root.join("Utilities/Terminal.app"));
    }

    #[test]
    fn the_first_folder_wins_when_two_have_the_same_app() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let mine = bundle(dir.path(), "home/Applications/Editor.app");
        bundle(dir.path(), "system/Applications/editor.app");
        let finder = bundle(dir.path(), "core/Finder.app");

        // Act
        let apps = scan(&[
            dir.path().join("home/Applications"),
            dir.path().join("system/Applications"),
            finder.clone(),
        ]);

        // Assert
        assert_eq!(
            apps,
            [
                InstalledApp {
                    name: "Editor".into(),
                    path: mine
                },
                InstalledApp {
                    name: "Finder".into(),
                    path: finder
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_bundle_is_found() {
        // Arrange: an app built in a checkout, linked into ~/Applications.
        let dir = tempfile::tempdir().unwrap();
        let built = bundle(dir.path(), "workspace/tool/build/Tool.app");
        let applications = dir.path().join("Applications");
        fs::create_dir_all(&applications).unwrap();
        std::os::unix::fs::symlink(&built, applications.join("Tool.app")).unwrap();

        // Act
        let apps = scan(std::slice::from_ref(&applications));

        // Assert
        assert_eq!(names(&apps), ["Tool"]);
        assert_eq!(apps[0].path, applications.join("Tool.app"));
    }

    #[test]
    fn a_command_finds_its_app_by_name_or_path() {
        // Arrange
        // Absolute on every host, so the path lookup is tested on Windows too.
        let home = std::env::temp_dir();
        let home = home.as_path();
        let apps = [
            InstalledApp {
                name: "Messenger".into(),
                path: "/Applications/Messenger.app".into(),
            },
            InstalledApp {
                name: "Tool".into(),
                path: home.join("Applications/Tool.app"),
            },
        ];

        // Act / Assert
        assert_eq!(
            find(&apps, "messenger", home).map(|a| a.name.as_str()),
            Some("Messenger")
        );
        assert_eq!(
            find(&apps, "Messenger.app", home).map(|a| a.name.as_str()),
            Some("Messenger")
        );
        assert_eq!(
            find(&apps, "~/Applications/Tool.app", home).map(|a| a.name.as_str()),
            Some("Tool")
        );
        assert_eq!(find(&apps, "/Elsewhere/Messenger.app", home), None);
        assert_eq!(find(&apps, "Mail", home), None);
    }
}
