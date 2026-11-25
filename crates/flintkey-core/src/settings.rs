//! `settings.yaml`: the few settings that apply to the whole app.

use crate::diagnostic::Diagnostic;
use crate::launch::TerminalApp;
use serde::Deserialize;
use serde_saphyr::Spanned;

pub const DEFAULT_HOTKEY: &str = "alt+space";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Global shortcut that shows the launcher, e.g. `alt+space`.
    pub hotkey: String,
    pub display: Display,
    /// `None` uses the OS default terminal.
    pub terminal: Option<TerminalApp>,
}

/// The screen the launcher window appears on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// The screen with the mouse pointer when the hotkey is pressed.
    #[default]
    Cursor,
    /// The main screen.
    Primary,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            display: Display::default(),
            terminal: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    hotkey: Option<Spanned<String>>,
    window: Option<RawWindow>,
    terminal: Option<RawTerminal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWindow {
    display: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTerminal {
    app: Option<Spanned<String>>,
}

fn problem_at<T>(spanned: &Spanned<T>, message: String) -> Diagnostic {
    Diagnostic::at(spanned.referenced.line(), spanned.referenced.column(), message)
}

/// Reads `settings.yaml`. Missing keys take their defaults. An empty file is valid.
pub fn parse_settings(source: &str) -> Result<Settings, Vec<Diagnostic>> {
    let raw = match serde_saphyr::from_str::<Option<RawSettings>>(source) {
        Ok(Some(raw)) => raw,
        Ok(None) => return Ok(Settings::default()),
        Err(error) => return Err(vec![Diagnostic::from_yaml_error(&error)]),
    };
    let mut settings = Settings::default();
    let mut problems = Vec::new();

    if let Some(hotkey) = raw.hotkey {
        if hotkey.value.trim().is_empty() {
            problems.push(problem_at(&hotkey, "`hotkey` must not be empty".to_string()));
        } else {
            settings.hotkey = hotkey.value;
        }
    }
    if let Some(display) = raw.window.and_then(|window| window.display) {
        match display.value.as_str() {
            "cursor" => settings.display = Display::Cursor,
            "primary" => settings.display = Display::Primary,
            other => problems.push(problem_at(
                &display,
                format!("unknown display `{other}` (expected one of: cursor, primary)"),
            )),
        }
    }
    if let Some(app) = raw.terminal.and_then(|terminal| terminal.app) {
        match TerminalApp::from_name(&app.value) {
            Some(found) => settings.terminal = Some(found),
            None => {
                let names: Vec<&str> = TerminalApp::ALL.iter().map(|app| app.name()).collect();
                problems.push(problem_at(
                    &app,
                    format!(
                        "unknown terminal `{}` (expected one of: {})",
                        app.value,
                        names.join(", ")
                    ),
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(settings)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_the_defaults() {
        for source in ["", "# nothing yet\n"] {
            // Act
            let settings = parse_settings(source).unwrap();

            // Assert
            assert_eq!(
                settings,
                Settings {
                    hotkey: "alt+space".into(),
                    display: Display::Cursor,
                    terminal: None
                }
            );
        }
    }

    #[test]
    fn every_key_is_read() {
        // Arrange
        let source = "hotkey: ctrl+shift+k\nwindow:\n  display: primary\nterminal:\n  app: windows-terminal\n";

        // Act
        let settings = parse_settings(source).unwrap();

        // Assert
        assert_eq!(
            settings,
            Settings {
                hotkey: "ctrl+shift+k".into(),
                display: Display::Primary,
                terminal: Some(TerminalApp::WindowsTerminal)
            }
        );
    }

    #[test]
    fn partial_file_keeps_the_other_defaults() {
        // Arrange
        let source = "window:\n  display: primary\n";

        // Act
        let settings = parse_settings(source).unwrap();

        // Assert
        assert_eq!(settings.display, Display::Primary);
        assert_eq!(settings.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn unknown_values_are_reported_with_their_lines() {
        // Arrange
        let source = "hotkey: ''\nwindow:\n  display: left\nterminal:\n  app: konsole\n";

        // Act
        let problems: Vec<String> = parse_settings(source)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();

        // Assert
        assert_eq!(
            problems,
            [
                "line 1, column 9: `hotkey` must not be empty",
                "line 3, column 12: unknown display `left` (expected one of: cursor, primary)",
                "line 5, column 8: unknown terminal `konsole` (expected one of: terminal, windows-terminal)",
            ]
        );
    }

    #[test]
    fn unknown_key_is_rejected() {
        // Arrange
        let source = "hotkeys: alt+space\n";

        // Act
        let problems = parse_settings(source).unwrap_err();

        // Assert
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, Some(1));
    }
}
