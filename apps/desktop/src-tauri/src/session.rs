//! App state that does not depend on the window. Reads the config folder, finds installed apps, searches, and
//! plans runs.

use flintkey_core::apps::{self, InstalledApp};
use flintkey_core::launch::{self, Machine, Plan};
use flintkey_core::search::Searcher;
use flintkey_core::settings::Settings;
use flintkey_core::store::{LoadedCommands, Store, StoredCommand};
use flintkey_core::usage::{USAGE_FILE, Usage};
use flintkey_core::{Action, Os};
use serde::Serialize;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.yaml";
/// The id of an installed app found by [`apps::scan`]. No command file can have it: `:` is not in file names
/// Flintkey writes, and the folder is read on macOS, where Finder shows `:` as `/`.
const APP_ID_PREFIX: &str = "app:";

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub keywords: Vec<String>,
    pub hotkey: Option<String>,
    /// `url` / `app` / `script` / `terminal`。
    pub kind: &'static str,
    /// One line describing what happens (URL, app name, command).
    pub detail: String,
    /// The app bundle whose icon the row shows: the app itself, or the app a command opens.
    pub icon: Option<String>,
}

pub struct Session {
    store: Store,
    usage_path: PathBuf,
    home: PathBuf,
    os: Os,
    settings: Settings,
    settings_problems: Vec<String>,
    commands: Vec<StoredCommand>,
    command_problems: Vec<String>,
    app_folders: Vec<PathBuf>,
    apps: Vec<InstalledApp>,
    /// `commands`, then the installed apps no command already opens. What is searched and run.
    entries: Vec<StoredCommand>,
    usage: Usage,
    searcher: Searcher,
}

/// Reads the commands, usage counts and apps from disk. Built by [`Session::reading`].
pub struct Reading {
    store: Store,
    usage_path: PathBuf,
    app_folders: Vec<PathBuf>,
}

/// What [`Reading::read`] found, for [`Session::apply`].
pub struct Read {
    commands: std::io::Result<LoadedCommands>,
    usage: Usage,
    apps: Vec<InstalledApp>,
}

impl Reading {
    /// Apps are found by their file name; the folders are read, not Spotlight.
    pub fn read(&self) -> Read {
        Read {
            commands: self.store.load_commands(),
            usage: Usage::load(&self.usage_path),
            apps: apps::scan(&self.app_folders),
        }
    }
}

impl Session {
    pub fn new(config_dir: PathBuf, state_dir: &Path, home: PathBuf, os: Os) -> Self {
        let app_folders = if os == Os::Macos {
            apps::folders(&home)
        } else {
            Vec::new()
        };
        Self {
            store: Store::new(config_dir),
            usage_path: state_dir.join(USAGE_FILE),
            home,
            os,
            settings: Settings::default(),
            settings_problems: Vec::new(),
            commands: Vec::new(),
            command_problems: Vec::new(),
            app_folders,
            apps: Vec::new(),
            entries: Vec::new(),
            usage: Usage::default(),
            searcher: Searcher::default(),
        }
    }

    /// Searches `folders` for apps instead of the system's Applications folders.
    #[cfg(test)]
    pub fn with_app_folders(mut self, folders: Vec<PathBuf>) -> Self {
        self.app_folders = folders;
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn os(&self) -> Os {
        self.os
    }

    pub fn config_dir(&self) -> &Path {
        self.store.root()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Reloads `settings.yaml`. If it cannot be read, keeps the last good settings
    /// (if a typo reset the hotkey to the default, the user would lose the way to fix it).
    pub fn reload_settings(&mut self) {
        self.settings_problems.clear();
        match self.store.load_settings() {
            Ok(Ok(settings)) => self.settings = settings,
            Ok(Err(problems)) => {
                self.settings_problems
                    .extend(problems.iter().map(|problem| format!("{SETTINGS_FILE}: {problem}")));
            }
            Err(error) => self.settings_problems.push(format!("{SETTINGS_FILE}: {error}")),
        }
    }

    /// Reads `commands/*.yaml`, the usage counts and the installed apps again, holding the session the whole time.
    /// The app uses [`Session::reading`] instead, so the session is not locked while the disk is read.
    pub fn reload(&mut self) {
        let read = self.reading().read();
        self.apply(read);
    }

    /// What [`Session::reload`] reads, detached from the session so it can be read without holding its lock.
    pub fn reading(&self) -> Reading {
        Reading {
            store: Store::new(self.store.root().to_path_buf()),
            usage_path: self.usage_path.clone(),
            app_folders: self.app_folders.clone(),
        }
    }

    /// Takes in what [`Reading::read`] read.
    pub fn apply(&mut self, read: Read) {
        self.command_problems.clear();
        self.usage = read.usage;
        self.apps = read.apps;
        match read.commands {
            Ok(loaded) => {
                self.commands = loaded
                    .commands
                    .into_iter()
                    .filter(|stored| stored.command.enabled && stored.command.applies_to(self.os))
                    .collect();
                for broken in loaded.broken {
                    let name = broken
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    self.command_problems.extend(
                        broken
                            .problems
                            .iter()
                            .map(|problem| format!("commands/{name}: {problem}")),
                    );
                }
            }
            Err(error) => {
                self.commands.clear();
                self.command_problems.push(format!("commands: {error}"));
            }
        }
        self.rebuild_entries();
    }

    fn rebuild_entries(&mut self) {
        // An app that a command already opens as it is would be listed twice under the same name.
        let opened: Vec<&Path> = self
            .commands
            .iter()
            .filter_map(|stored| match &stored.command.action {
                Action::App { app, args } if args.is_empty() => apps::find(&self.apps, app, &self.home),
                _ => None,
            })
            .map(|installed| installed.path.as_path())
            .collect();
        let found = self
            .apps
            .iter()
            .filter(|installed| !opened.contains(&installed.path.as_path()))
            .map(|installed| StoredCommand {
                id: format!("{APP_ID_PREFIX}{}", installed.path.display()),
                command: flintkey_core::Command {
                    title: installed.name.clone(),
                    enabled: true,
                    keywords: Vec::new(),
                    hotkey: None,
                    pin: false,
                    tags: Vec::new(),
                    os: [].into(),
                    action: Action::App {
                        app: installed.path.display().to_string(),
                        args: Vec::new(),
                    },
                },
            })
            .collect::<Vec<_>>();
        self.entries = self.commands.iter().cloned().chain(found).collect();
    }

    /// Problems in the config files. Settings first, then commands.
    pub fn problems(&self) -> Vec<String> {
        self.settings_problems
            .iter()
            .chain(&self.command_problems)
            .cloned()
            .collect()
    }

    /// Returns the commands and apps matching `query`, best match first. An empty query lists only the commands:
    /// hundreds of apps would bury them.
    pub fn search(&mut self, query: &str) -> Vec<Row> {
        let commands = self.commands.len();
        let empty = query.trim().is_empty();
        self.searcher
            .rank(&self.entries, &self.usage, query)
            .into_iter()
            .filter(|&index| !empty || index < commands)
            .map(|index| row(&self.entries[index], &self.apps, &self.home))
            .collect()
    }

    /// (id, hotkey) of the commands that have a hotkey, in id order.
    pub fn hotkeys(&self) -> Vec<(String, String)> {
        self.commands
            .iter()
            .filter_map(|stored| Some((stored.id.clone(), stored.command.hotkey.clone()?)))
            .collect()
    }

    /// Plans how to run command `id`. Does not touch secrets.
    pub fn plan(&self, id: &str, shell: Option<&str>, path: Option<&str>) -> Result<Plan, String> {
        let stored = self
            .entries
            .iter()
            .find(|stored| stored.id == id)
            .ok_or_else(|| format!("`{id}` no longer exists"))?;
        let machine = Machine {
            os: self.os,
            home: &self.home,
            shell,
            path,
            terminal: self.settings.terminal,
        };
        launch::plan(&stored.command.action, &machine).map_err(|error| error.to_string())
    }

    /// Records one use of `id`. Rereads the file before writing so counts written by other processes are not lost.
    pub fn record_use(&mut self, id: &str) -> std::io::Result<()> {
        self.usage = Usage::load(&self.usage_path);
        self.usage.record(id);
        self.usage.save(&self.usage_path)
    }
}

fn row(stored: &StoredCommand, apps: &[InstalledApp], home: &Path) -> Row {
    let command = &stored.command;
    let icon = match &command.action {
        Action::App { app, .. } => apps::find(apps, app, home).map(|installed| installed.path.display().to_string()),
        _ => None,
    };
    let detail = match &command.action {
        Action::Url { url } => url.clone(),
        Action::App { app, args } => std::iter::once(app).chain(args).cloned().collect::<Vec<_>>().join(" "),
        Action::Script { run, .. } | Action::Terminal { run, .. } => run.lines().next().unwrap_or_default().to_string(),
    };
    Row {
        id: stored.id.clone(),
        title: command.title.clone(),
        keywords: command.keywords.clone(),
        hotkey: command.hotkey.clone(),
        kind: command.action.type_name(),
        detail,
        icon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir_all(dir.path().join("config/commands")).unwrap();
            Self { dir }
        }

        fn command(&self, id: &str, source: &str) {
            fs::write(self.dir.path().join(format!("config/commands/{id}.yaml")), source).unwrap();
        }

        fn settings(&self, source: &str) {
            fs::write(self.dir.path().join("config/settings.yaml"), source).unwrap();
        }

        fn session(&self, os: Os) -> Session {
            Session::new(
                self.dir.path().join("config"),
                &self.dir.path().join("state"),
                PathBuf::from("/Users/me"),
                os,
            )
            .with_app_folders(Vec::new())
        }
    }

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.id.as_str()).collect()
    }

    #[test]
    fn search_returns_rows_describing_each_action() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("site", "title: Site\nkeyword: s\ntype: url\nurl: https://example.com\n");
        fixture.command(
            "editor",
            "title: Editor\ntype: app\napp: Editor\nargs: [--new, ~/work]\n",
        );
        fixture.command(
            "deploy",
            "title: Deploy\ntype: script\nrun: |\n  fab deploy\n  echo done\n",
        );
        fixture.command("logs", "title: Logs\ntype: terminal\nrun: tail -f log\n");
        let mut session = fixture.session(Os::Macos);
        session.reload();

        // Act
        let rows = session.search("");

        // Assert
        let by_id = |id: &str| rows.iter().find(|row| row.id == id).unwrap().clone();
        assert_eq!(
            by_id("site"),
            Row {
                id: "site".into(),
                title: "Site".into(),
                keywords: vec!["s".into()],
                hotkey: None,
                kind: "url",
                detail: "https://example.com".into(),
                icon: None,
            }
        );
        assert_eq!(
            (by_id("editor").kind, by_id("editor").detail),
            ("app", "Editor --new ~/work".into())
        );
        assert_eq!(
            (by_id("deploy").kind, by_id("deploy").detail),
            ("script", "fab deploy".into())
        );
        assert_eq!(
            (by_id("logs").kind, by_id("logs").detail),
            ("terminal", "tail -f log".into())
        );
    }

    /// A session whose app search looks in `Applications` inside the fixture, holding these bundles.
    fn with_apps(fixture: &Fixture, bundles: &[&str]) -> Session {
        let folder = fixture.dir.path().join("Applications");
        for bundle in bundles {
            fs::create_dir_all(folder.join(bundle).join("Contents")).unwrap();
        }
        let mut session = fixture.session(Os::Macos).with_app_folders(vec![folder]);
        session.reload();
        session
    }

    #[test]
    fn installed_apps_are_found_by_a_query_but_not_listed_without_one() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("site", "title: Site\ntype: url\nurl: https://example.com\n");
        let mut session = with_apps(&fixture, &["Calculator.app", "Calendar.app"]);
        let calculator = fixture
            .dir
            .path()
            .join("Applications/Calculator.app")
            .display()
            .to_string();

        // Act
        let empty = session.search("");
        let found = session.search("calcu");

        // Assert
        assert_eq!(ids(&empty), ["site"]);
        assert_eq!(
            found,
            [Row {
                id: format!("app:{calculator}"),
                title: "Calculator".into(),
                keywords: vec![],
                hotkey: None,
                kind: "app",
                detail: calculator.clone(),
                icon: Some(calculator),
            }]
        );
    }

    #[test]
    fn an_installed_app_can_be_planned_and_its_use_counted() {
        // Arrange
        let fixture = Fixture::new();
        let mut session = with_apps(&fixture, &["Calculator.app"]);
        let id = session.search("calc")[0].id.clone();

        // Act
        let plan = session.plan(&id, None, None);
        session.record_use(&id).unwrap();

        // Assert
        let Ok(Plan::Spawn(process)) = plan else {
            panic!("{plan:?}")
        };
        assert_eq!(process.program, "/usr/bin/open");
        assert_eq!(
            process.args[1],
            fixture
                .dir
                .path()
                .join("Applications/Calculator.app")
                .display()
                .to_string()
        );
        assert_eq!(
            Usage::load(&fixture.dir.path().join("state").join(USAGE_FILE)).count(&id),
            1
        );
    }

    #[test]
    fn an_app_a_command_opens_as_it_is_is_listed_once_with_its_icon() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("chat", "title: Team chat\nkeyword: chat\ntype: app\napp: Messenger\n");
        fixture.command(
            "editor-new",
            "title: New editor window\ntype: app\napp: Editor\nargs: [--new]\n",
        );
        let mut session = with_apps(&fixture, &["Messenger.app", "Editor.app"]);
        let messenger = fixture
            .dir
            .path()
            .join("Applications/Messenger.app")
            .display()
            .to_string();

        // Act
        let messenger_rows = session.search("messenger");
        let chat_rows = session.search("chat");
        let editor_rows = session.search("editor");

        // Assert
        assert!(
            messenger_rows.iter().all(|row| row.title != "Messenger"),
            "{messenger_rows:?}"
        );
        assert_eq!(chat_rows[0].id, "chat");
        assert_eq!(chat_rows[0].icon.as_deref(), Some(messenger.as_str()));
        // A command that passes arguments does not hide the plain app.
        assert!(editor_rows.iter().any(|row| row.title == "Editor"), "{editor_rows:?}");
    }

    #[test]
    fn apps_are_only_searched_on_macos() {
        // Arrange
        let fixture = Fixture::new();
        let mut session = Session::new(
            fixture.dir.path().join("config"),
            &fixture.dir.path().join("state"),
            PathBuf::from("/home/me"),
            Os::Linux,
        );

        // Act
        session.reload();

        // Assert
        assert!(session.app_folders.is_empty());
    }

    #[test]
    fn disabled_commands_are_neither_listed_nor_given_their_hotkey() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command(
            "on",
            "title: On\nhotkey: ctrl+alt+cmd+o\ntype: url\nurl: https://example.com\n",
        );
        fixture.command(
            "off",
            "title: Off\nenabled: 0\nkeyword: off\nhotkey: ctrl+alt+cmd+f\ntype: url\nurl: https://example.com\n",
        );
        let mut session = fixture.session(Os::Macos);

        // Act
        session.reload();

        // Assert
        assert_eq!(ids(&session.search("")), ["on"]);
        assert_eq!(ids(&session.search("off")), Vec::<&str>::new());
        assert_eq!(session.hotkeys(), [("on".to_string(), "ctrl+alt+cmd+o".to_string())]);
        assert_eq!(session.problems(), Vec::<String>::new());
    }

    #[test]
    fn commands_for_another_os_are_left_out() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command(
            "mac",
            "title: Mac only\ntype: url\nurl: https://example.com\nwhen:\n  os: [macos]\n",
        );
        fixture.command(
            "win",
            "title: Windows only\ntype: url\nurl: https://example.com\nwhen:\n  os: [windows]\n",
        );
        fixture.command("any", "title: Anywhere\ntype: url\nurl: https://example.com\n");
        let mut session = fixture.session(Os::Windows);
        session.reload();

        // Act
        let rows = session.search("");

        // Assert
        assert_eq!(ids(&rows), ["any", "win"]);
    }

    #[test]
    fn broken_files_are_reported_and_do_not_hide_valid_ones() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("good", "title: Good\ntype: url\nurl: https://example.com\n");
        fixture.command("bad", "title: Bad\ntype: url\n");
        let mut session = fixture.session(Os::Macos);

        // Act
        session.reload();

        // Assert
        assert_eq!(ids(&session.search("")), ["good"]);
        let problems = session.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("commands/bad.yaml: "), "{problems:?}");
        assert!(problems[0].contains("url"), "{problems:?}");
    }

    #[test]
    fn fixing_a_broken_file_clears_its_problem_on_reload() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("bad", "title: Bad\ntype: url\n");
        let mut session = fixture.session(Os::Macos);
        session.reload();
        assert_eq!(session.problems().len(), 1);

        // Act
        fixture.command("bad", "title: Bad\ntype: url\nurl: https://example.com\n");
        session.reload();

        // Assert
        assert_eq!(session.problems(), Vec::<String>::new());
        assert_eq!(ids(&session.search("")), ["bad"]);
    }

    #[test]
    fn missing_config_folder_is_an_empty_session_without_problems() {
        // Arrange
        let dir = TempDir::new().unwrap();
        let mut session = Session::new(
            dir.path().join("absent"),
            &dir.path().join("state"),
            PathBuf::from("/Users/me"),
            Os::Macos,
        )
        .with_app_folders(Vec::new());

        // Act
        session.reload_settings();
        session.reload();

        // Assert
        assert_eq!(session.search(""), Vec::<Row>::new());
        assert_eq!(session.problems(), Vec::<String>::new());
        assert_eq!(*session.settings(), Settings::default());
    }

    #[test]
    fn invalid_settings_keep_the_last_good_ones_and_report_the_problem() {
        // Arrange
        let fixture = Fixture::new();
        fixture.settings("hotkey: ctrl+shift+k\n");
        let mut session = fixture.session(Os::Macos);
        session.reload_settings();
        assert_eq!(session.settings().hotkey, "ctrl+shift+k");

        // Act
        fixture.settings("hotkey: ctrl+shift+k\nwindow:\n  display: nowhere\n");
        session.reload_settings();

        // Assert
        assert_eq!(session.settings().hotkey, "ctrl+shift+k");
        let problems = session.problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("settings.yaml: line 3"), "{problems:?}");
    }

    #[test]
    fn settings_problem_disappears_once_the_file_is_fixed() {
        // Arrange
        let fixture = Fixture::new();
        fixture.settings("window:\n  display: nowhere\n");
        let mut session = fixture.session(Os::Macos);
        session.reload_settings();
        assert_eq!(session.problems().len(), 1);

        // Act
        fixture.settings("window:\n  display: primary\n");
        session.reload_settings();

        // Assert
        assert_eq!(session.problems(), Vec::<String>::new());
        assert_eq!(session.settings().display, flintkey_core::settings::Display::Primary);
    }

    #[test]
    fn hotkeys_come_from_the_commands_of_this_os_in_id_order() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command(
            "b",
            "title: B\nhotkey: ctrl+alt+cmd+b\ntype: url\nurl: https://example.com\n",
        );
        fixture.command(
            "a",
            "title: A\nhotkey: ctrl+alt+cmd+a\ntype: url\nurl: https://example.com\n",
        );
        fixture.command("none", "title: N\ntype: url\nurl: https://example.com\n");
        fixture.command(
            "win",
            "title: W\nhotkey: ctrl+alt+w\ntype: url\nurl: https://example.com\nwhen:\n  os: windows\n",
        );
        let mut session = fixture.session(Os::Macos);

        // Act
        session.reload();

        // Assert
        assert_eq!(
            session.hotkeys(),
            [
                ("a".to_string(), "ctrl+alt+cmd+a".to_string()),
                ("b".to_string(), "ctrl+alt+cmd+b".to_string())
            ]
        );
    }

    #[test]
    fn plan_uses_the_machine_and_the_terminal_from_settings() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("logs", "title: Logs\ntype: terminal\nrun: tail -f log\n");
        fixture.settings("terminal:\n  app: windows-terminal\n");
        let mut session = fixture.session(Os::Windows);
        session.reload_settings();
        session.reload();

        // Act
        let plan = session.plan("logs", None, None).unwrap();

        // Assert
        let Plan::Spawn(process) = plan else {
            panic!("expected a process, got {plan:?}");
        };
        assert_eq!(process.program, "wt.exe");
    }

    #[test]
    fn plan_reports_a_terminal_setting_that_this_os_does_not_have() {
        // Arrange: settings choosing Windows Terminal, used on macOS. Ignoring them would quietly use Terminal.app.
        let fixture = Fixture::new();
        fixture.command("logs", "title: Logs\ntype: terminal\nrun: tail -f log\n");
        fixture.settings("terminal:\n  app: windows-terminal\n");
        let mut session = fixture.session(Os::Macos);
        session.reload_settings();
        session.reload();

        // Act
        let result = session.plan("logs", None, None);

        // Assert
        let message = result.unwrap_err();
        assert!(message.contains("windows-terminal"), "{message}");
    }

    #[test]
    fn problems_list_settings_before_commands() {
        // Arrange
        let fixture = Fixture::new();
        fixture.settings("window:\n  display: nowhere\n");
        fixture.command("bad", "title: Bad\ntype: url\n");
        let mut session = fixture.session(Os::Macos);

        // Act
        session.reload();
        session.reload_settings();

        // Assert
        let problems = session.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("settings.yaml: "), "{problems:?}");
        assert!(problems[1].starts_with("commands/bad.yaml: "), "{problems:?}");
    }

    #[test]
    fn plan_for_a_command_deleted_after_the_list_was_shown_is_an_error() {
        // Arrange
        let fixture = Fixture::new();
        let mut session = fixture.session(Os::Macos);
        session.reload();

        // Act
        let result = session.plan("gone", None, None);

        // Assert
        assert_eq!(result.unwrap_err(), "`gone` no longer exists");
    }

    #[test]
    fn recorded_use_moves_the_command_up_and_survives_a_new_session() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("alpha", "title: Alpha\ntype: url\nurl: https://example.com/a\n");
        fixture.command("beta", "title: Beta\ntype: url\nurl: https://example.com/b\n");
        let mut session = fixture.session(Os::Macos);
        session.reload();
        assert_eq!(ids(&session.search("")), ["alpha", "beta"]);

        // Act
        session.record_use("beta").unwrap();

        // Assert
        assert_eq!(ids(&session.search("")), ["beta", "alpha"]);
        let mut fresh = fixture.session(Os::Macos);
        fresh.reload();
        assert_eq!(ids(&fresh.search("")), ["beta", "alpha"]);
    }

    #[test]
    fn recording_a_use_keeps_counts_written_by_another_process() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("alpha", "title: Alpha\ntype: url\nurl: https://example.com/a\n");
        fixture.command("beta", "title: Beta\ntype: url\nurl: https://example.com/b\n");
        let mut session = fixture.session(Os::Macos);
        session.reload();
        let mut other = fixture.session(Os::Macos);
        other.record_use("beta").unwrap();
        other.record_use("beta").unwrap();

        // Act
        session.record_use("alpha").unwrap();

        // Assert
        assert_eq!(ids(&session.search("")), ["beta", "alpha"]);
    }
}
