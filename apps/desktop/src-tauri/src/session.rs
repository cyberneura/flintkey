//! App state that does not depend on the window. Reads the config folder, searches, and plans runs.

use flintkey_core::launch::{self, Machine, Plan};
use flintkey_core::search::Searcher;
use flintkey_core::settings::Settings;
use flintkey_core::store::{Store, StoredCommand};
use flintkey_core::usage::{USAGE_FILE, Usage};
use flintkey_core::{Action, Os};
use serde::Serialize;
use std::path::{Path, PathBuf};

const SETTINGS_FILE: &str = "settings.yaml";

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
    usage: Usage,
    searcher: Searcher,
}

impl Session {
    pub fn new(config_dir: PathBuf, state_dir: &Path, home: PathBuf, os: Os) -> Self {
        Self {
            store: Store::new(config_dir),
            usage_path: state_dir.join(USAGE_FILE),
            home,
            os,
            settings: Settings::default(),
            settings_problems: Vec::new(),
            commands: Vec::new(),
            command_problems: Vec::new(),
            usage: Usage::default(),
            searcher: Searcher::default(),
        }
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

    /// Reloads `commands/*.yaml` and the usage counts.
    pub fn reload_commands(&mut self) {
        self.command_problems.clear();
        self.usage = Usage::load(&self.usage_path);
        match self.store.load_commands() {
            Ok(loaded) => {
                self.commands = loaded
                    .commands
                    .into_iter()
                    .filter(|stored| stored.command.applies_to(self.os))
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
    }

    /// Problems in the config files. Settings first, then commands.
    pub fn problems(&self) -> Vec<String> {
        self.settings_problems
            .iter()
            .chain(&self.command_problems)
            .cloned()
            .collect()
    }

    /// Returns the commands matching `query`, best match first.
    pub fn search(&mut self, query: &str) -> Vec<Row> {
        self.searcher
            .rank(&self.commands, &self.usage, query)
            .into_iter()
            .map(|index| row(&self.commands[index]))
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
            .commands
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

fn row(stored: &StoredCommand) -> Row {
    let command = &stored.command;
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
        fixture.command("editor", "title: Editor\ntype: app\napp: Zed\nargs: [--new, ~/work]\n");
        fixture.command(
            "deploy",
            "title: Deploy\ntype: script\nrun: |\n  fab deploy\n  echo done\n",
        );
        fixture.command("logs", "title: Logs\ntype: terminal\nrun: tail -f log\n");
        let mut session = fixture.session(Os::Macos);
        session.reload_commands();

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
            }
        );
        assert_eq!(
            (by_id("editor").kind, by_id("editor").detail),
            ("app", "Zed --new ~/work".into())
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
        session.reload_commands();

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
        session.reload_commands();

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
        session.reload_commands();
        assert_eq!(session.problems().len(), 1);

        // Act
        fixture.command("bad", "title: Bad\ntype: url\nurl: https://example.com\n");
        session.reload_commands();

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
        );

        // Act
        session.reload_settings();
        session.reload_commands();

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
        session.reload_commands();

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
        session.reload_commands();

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
        session.reload_commands();

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
        session.reload_commands();
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
        session.reload_commands();

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
        session.reload_commands();
        assert_eq!(ids(&session.search("")), ["alpha", "beta"]);

        // Act
        session.record_use("beta").unwrap();

        // Assert
        assert_eq!(ids(&session.search("")), ["beta", "alpha"]);
        let mut fresh = fixture.session(Os::Macos);
        fresh.reload_commands();
        assert_eq!(ids(&fresh.search("")), ["beta", "alpha"]);
    }

    #[test]
    fn recording_a_use_keeps_counts_written_by_another_process() {
        // Arrange
        let fixture = Fixture::new();
        fixture.command("alpha", "title: Alpha\ntype: url\nurl: https://example.com/a\n");
        fixture.command("beta", "title: Beta\ntype: url\nurl: https://example.com/b\n");
        let mut session = fixture.session(Os::Macos);
        session.reload_commands();
        let mut other = fixture.session(Os::Macos);
        other.record_use("beta").unwrap();
        other.record_use("beta").unwrap();

        // Act
        session.record_use("alpha").unwrap();

        // Assert
        assert_eq!(ids(&session.search("")), ["beta", "alpha"]);
    }
}
