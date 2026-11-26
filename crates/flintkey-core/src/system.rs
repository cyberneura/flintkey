//! Built-in commands for the operating system itself (System Settings, the screen saver, locking the screen,
//! restarting, logging out, Launchpad), so they work without a command file. Only macOS has them.
//!
//! Like apps, they are listed only once something is typed.

use crate::command::{Action, Command};
use crate::store::StoredCommand;
use std::collections::BTreeMap;
use std::path::Path;

/// The prefix of a built-in command's id. No command file can have it (`:` is not in file names Flintkey writes).
pub const ID_PREFIX: &str = "system:";

const LAUNCHPAD: &str = "/System/Applications/Launchpad.app";
/// macOS 26 replaced Launchpad with the Apps app.
const APPS: &str = "/System/Applications/Apps.app";

/// Restart and log out go through the login window, which asks for confirmation (with its usual countdown) and
/// lets apps save their documents. These are the events that ask first: `rrst` (kAEShowRestartDialog) and `logo`
/// (kAELogOut). Not `rlgo` (kAEReallyLogOut) or `rest` (kAERestart), which act at once.
const RESTART: &str = "/usr/bin/osascript -e 'tell application \"loginwindow\" to «event aevtrrst»'";
const LOG_OUT: &str = "/usr/bin/osascript -e 'tell application \"loginwindow\" to «event aevtlogo»'";
/// Turns the display off, which locks the screen when "Require password after the screen saver begins or display
/// is turned off" is set to Immediately (the default). It needs no permission, unlike sending ⌃⌘Q.
const LOCK: &str = "/usr/bin/pmset displaysleepnow";

/// The built-in commands for macOS. `exists` tells whether a path exists, so Launchpad can open the Apps app on
/// systems that no longer have Launchpad.
pub fn macos_commands(exists: impl Fn(&Path) -> bool) -> Vec<StoredCommand> {
    let launchpad = if exists(Path::new(LAUNCHPAD)) { LAUNCHPAD } else { APPS };
    vec![
        entry(
            "settings",
            "System Settings",
            &["settings", "preferences"],
            app("/System/Applications/System Settings.app"),
        ),
        entry(
            "screen-saver",
            "Start Screen Saver",
            &["screensaver", "screen saver"],
            app("/System/Library/CoreServices/ScreenSaverEngine.app"),
        ),
        entry("lock", "Lock Screen", &["lock"], script(LOCK)),
        entry("restart", "Restart…", &["restart", "reboot"], script(RESTART)),
        entry("log-out", "Log Out…", &["logout", "log out"], script(LOG_OUT)),
        entry("launchpad", "Launchpad", &["launchpad", "apps"], app(launchpad)),
    ]
}

fn app(path: &str) -> Action {
    Action::App {
        app: path.to_string(),
        args: Vec::new(),
    }
}

fn script(run: &str) -> Action {
    Action::Script {
        run: run.to_string(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

fn entry(id: &str, title: &str, keywords: &[&str], action: Action) -> StoredCommand {
    StoredCommand {
        id: format!("{ID_PREFIX}{id}"),
        command: Command {
            title: title.to_string(),
            enabled: true,
            keywords: keywords.iter().map(|keyword| keyword.to_string()).collect(),
            hotkey: None,
            pin: false,
            icon: None,
            tags: Vec::new(),
            os: [].into(),
            action,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launchpad_app(commands: &[StoredCommand]) -> String {
        let launchpad = commands.iter().find(|stored| stored.id == "system:launchpad").unwrap();
        match &launchpad.command.action {
            Action::App { app, .. } => app.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn restart_and_log_out_use_the_events_that_ask_for_confirmation() {
        // Act / Assert
        assert!(RESTART.contains("«event aevtrrst»"));
        assert!(LOG_OUT.contains("«event aevtlogo»"));
    }

    #[test]
    fn launchpad_opens_the_apps_app_where_launchpad_is_gone() {
        // Act
        let old = macos_commands(|_| true);
        let new = macos_commands(|path| path != Path::new(LAUNCHPAD));

        // Assert
        assert_eq!(launchpad_app(&old), LAUNCHPAD);
        assert_eq!(launchpad_app(&new), APPS);
    }

    #[test]
    fn every_built_in_command_has_a_system_id_and_a_keyword() {
        // Act
        let commands = macos_commands(|_| true);

        // Assert
        let ids: Vec<&str> = commands.iter().map(|stored| stored.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "system:settings",
                "system:screen-saver",
                "system:lock",
                "system:restart",
                "system:log-out",
                "system:launchpad"
            ]
        );
        assert!(commands.iter().all(|stored| !stored.command.keywords.is_empty()));
    }
}
