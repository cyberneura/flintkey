//! The shapes the settings window edits, and their conversion to and from the core types (pure functions).
//!
//! The form keeps every field of every command type, so switching the type in the form does not lose
//! what was typed. Only the fields of the chosen type end up in the command.

use flintkey_core::launch::TerminalApp;
use flintkey_core::settings::{Display, Settings};
use flintkey_core::{Action, Command, Os};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsForm {
    pub hotkey: String,
    /// `cursor` / `primary`.
    pub display: String,
    /// A terminal name, or `None` for the OS default.
    pub terminal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CommandForm {
    pub title: String,
    pub keywords: Vec<String>,
    pub hotkey: String,
    /// `url` / `app` / `script` / `terminal`.
    pub kind: String,
    pub url: String,
    pub app: String,
    pub args: Vec<String>,
    pub run: String,
    pub cwd: String,
    /// Environment variables in the order shown. A list rather than a map so that a row being typed
    /// (empty or repeated name) survives until it is saved.
    pub env: Vec<(String, String)>,
    pub pin: bool,
    pub tags: Vec<String>,
    /// OS names. Empty means every OS.
    pub os: Vec<String>,
}

impl SettingsForm {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            hotkey: settings.hotkey.clone(),
            display: settings.display.name().to_string(),
            terminal: settings.terminal.map(|app| app.name().to_string()),
        }
    }

    pub fn to_settings(&self) -> Result<Settings, String> {
        let display = Display::from_name(&self.display).ok_or_else(|| format!("unknown display `{}`", self.display))?;
        let terminal = match self.terminal.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(name) => Some(TerminalApp::from_name(name).ok_or_else(|| format!("unknown terminal `{name}`"))?),
        };
        Ok(Settings {
            hotkey: self.hotkey.trim().to_string(),
            display,
            terminal,
        })
    }
}

impl CommandForm {
    pub fn from_command(command: &Command) -> Self {
        let mut form = CommandForm {
            title: command.title.clone(),
            keywords: command.keywords.clone(),
            hotkey: command.hotkey.clone().unwrap_or_default(),
            kind: command.action.type_name().to_string(),
            pin: command.pin,
            tags: command.tags.clone(),
            os: command.os.iter().map(|os| os.name().to_string()).collect(),
            ..CommandForm::default()
        };
        match &command.action {
            Action::Url { url } => form.url = url.clone(),
            Action::App { app, args } => {
                form.app = app.clone();
                form.args = args.clone();
            }
            Action::Script { run, cwd, env } => {
                form.run = run.clone();
                form.cwd = cwd.clone().unwrap_or_default();
                form.env = env.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
            }
            Action::Terminal { run, cwd } => {
                form.run = run.clone();
                form.cwd = cwd.clone().unwrap_or_default();
            }
        }
        form
    }

    /// The command the form describes. Rules the core checks (empty title, URL without a scheme, …) are left
    /// to the save, which reports them with the rest; only what cannot be expressed as a command is refused here.
    pub fn to_command(&self) -> Result<Command, String> {
        let text = |value: &str| value.trim().to_string();
        let optional = |value: &str| Some(value.trim().to_string()).filter(|value| !value.is_empty());
        let list = |items: &[String]| -> Vec<String> {
            items
                .iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        };
        let action = match self.kind.as_str() {
            "url" => Action::Url { url: text(&self.url) },
            "app" => Action::App {
                app: text(&self.app),
                // Arguments are kept as typed: an empty or repeated argument can be intended. Only empty lines at the
                // end are dropped (the form edits one argument per line, and a new line is typed before its text).
                args: {
                    let mut args = self.args.clone();
                    while args.last().is_some_and(String::is_empty) {
                        args.pop();
                    }
                    args
                },
            },
            "script" => {
                let mut env = BTreeMap::new();
                for (name, value) in &self.env {
                    let name = name.trim();
                    if name.is_empty() && value.is_empty() {
                        continue;
                    }
                    if name.is_empty() {
                        return Err("an environment variable has no name".into());
                    }
                    if env.insert(name.to_string(), value.clone()).is_some() {
                        return Err(format!("the environment variable `{name}` is listed twice"));
                    }
                }
                Action::Script {
                    run: self.run.clone(),
                    cwd: optional(&self.cwd),
                    env,
                }
            }
            "terminal" => Action::Terminal {
                run: self.run.clone(),
                cwd: optional(&self.cwd),
            },
            other => return Err(format!("unknown type `{other}`")),
        };
        let mut os = std::collections::BTreeSet::new();
        for name in &self.os {
            os.insert(Os::from_name(name).ok_or_else(|| format!("unknown os `{name}`"))?);
        }
        Ok(Command {
            title: text(&self.title),
            keywords: list(&self.keywords),
            hotkey: optional(&self.hotkey),
            pin: self.pin,
            tags: list(&self.tags),
            os,
            action,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flintkey_core::parse_command;

    #[test]
    fn every_command_type_survives_a_trip_through_the_form() {
        // Arrange
        let sources = [
            "title: Site\nkeyword: [s, site]\nhotkey: ctrl+alt+cmd+s\ntype: url\nurl: https://example.com\npin: true\ntags: [web]\n",
            "title: Editor\ntype: app\napp: Zed\nargs: [--new, ~/work]\nwhen:\n  os: [macos, linux]\n",
            "title: Deploy\ntype: script\nrun: |\n  fab deploy\ncwd: ~/blog\nenv:\n  A: op://Private/A/token\n  B: ''\n",
            "title: Logs\ntype: terminal\nrun: tail -f log\ncwd: /var/log\n",
        ];

        for source in sources {
            let command = parse_command(source).unwrap();

            // Act
            let back = CommandForm::from_command(&command).to_command();

            // Assert
            assert_eq!(back, Ok(command), "{source}");
        }
    }

    #[test]
    fn blank_entries_typed_in_the_form_are_dropped_and_text_is_trimmed() {
        // Arrange
        let form = CommandForm {
            title: "  Site ".into(),
            keywords: vec!["s".into(), " ".into(), " site ".into()],
            hotkey: "  ".into(),
            kind: "url".into(),
            url: " https://example.com ".into(),
            tags: vec!["".into(), "web".into()],
            run: "kept for when the type changes".into(),
            ..CommandForm::default()
        };

        // Act
        let command = form.to_command().unwrap();

        // Assert
        assert_eq!(command.title, "Site");
        assert_eq!(command.keywords, ["s", "site"]);
        assert_eq!(command.hotkey, None);
        assert_eq!(command.tags, ["web"]);
        assert_eq!(
            command.action,
            Action::Url {
                url: "https://example.com".into()
            }
        );
    }

    #[test]
    fn arguments_keep_empty_and_repeated_entries_except_trailing_empty_lines() {
        // Arrange
        let form = CommandForm {
            title: "T".into(),
            kind: "app".into(),
            app: "Tool".into(),
            args: vec!["".into(), "-v".into(), "-v".into(), "".into(), "".into()],
            ..CommandForm::default()
        };

        // Act
        let command = form.to_command().unwrap();

        // Assert
        assert_eq!(
            command.action,
            Action::App {
                app: "Tool".into(),
                args: vec!["".into(), "-v".into(), "-v".into()]
            }
        );
    }

    #[test]
    fn environment_rows_need_distinct_names_but_an_empty_row_is_ignored() {
        // Arrange
        let form = |env: &[(&str, &str)]| CommandForm {
            title: "T".into(),
            kind: "script".into(),
            run: "x".into(),
            env: env.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(),
            ..CommandForm::default()
        };

        // Act
        let empty_row = form(&[("A", "1"), ("", "")]).to_command();
        let nameless = form(&[("", "1")]).to_command();
        let twice = form(&[("A", "1"), (" A", "2")]).to_command();

        // Assert
        assert!(empty_row.is_ok(), "{empty_row:?}");
        assert_eq!(nameless, Err("an environment variable has no name".into()));
        assert_eq!(twice, Err("the environment variable `A` is listed twice".into()));
    }

    #[test]
    fn settings_survive_a_trip_through_the_form_and_bad_names_are_refused() {
        // Arrange
        let settings = Settings {
            hotkey: "shift+ctrl+space".into(),
            display: Display::Primary,
            terminal: Some(TerminalApp::Terminal),
        };
        let mut bad = SettingsForm::from_settings(&settings);
        bad.display = "left".into();

        // Act
        let back = SettingsForm::from_settings(&settings).to_settings();

        // Assert
        assert_eq!(back, Ok(settings));
        assert_eq!(bad.to_settings(), Err("unknown display `left`".into()));
        let default_terminal = SettingsForm {
            hotkey: "alt+space".into(),
            display: "cursor".into(),
            terminal: Some(String::new()),
        };
        assert_eq!(default_terminal.to_settings().unwrap().terminal, None);
    }
}
