//! Decides which shortcuts to register from the hotkeys in the settings (pure functions).

use std::collections::HashMap;
use tauri_plugin_global_shortcut::Shortcut;

/// What happens when a hotkey is pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Launcher,
    /// The command's id.
    Command(String),
}

/// Registered shortcut -> what happens when it is pressed, and the text written in the settings.
pub type Registered = HashMap<Shortcut, (Target, String)>;

#[derive(Debug, Default, PartialEq)]
pub struct Wanted {
    /// Shortcuts to register and what they do. Also keeps the text as written (used in notices).
    pub shortcuts: HashMap<Shortcut, (Target, String)>,
    /// Whether the launcher hotkey could be read. If not, the currently registered one stays in use.
    pub launcher_readable: bool,
    pub problems: Vec<String>,
}

/// `launcher` is the launcher hotkey; `commands` is (id, hotkey) pairs in id order.
///
/// On conflicts the launcher wins, and among commands the first one wins. Differences in case or spelling
/// (`Ctrl+Alt+C` vs `ctrl+alt+KeyC`) compare as the same shortcut.
pub fn wanted(launcher: &str, commands: &[(String, String)]) -> Wanted {
    let mut out = Wanted::default();
    match check(launcher) {
        Ok(shortcut) => {
            out.launcher_readable = true;
            out.shortcuts.insert(shortcut, (Target::Launcher, launcher.to_string()));
        }
        Err(reason) => out.problems.push(format!("hotkey `{launcher}`: {reason}")),
    }
    for (id, hotkey) in commands {
        let place = format!("commands/{id}.yaml: hotkey `{hotkey}`");
        let shortcut = match check(hotkey) {
            Ok(shortcut) => shortcut,
            Err(reason) => {
                out.problems.push(format!("{place}: {reason}"));
                continue;
            }
        };
        match out.shortcuts.get(&shortcut) {
            Some((Target::Launcher, _)) => out.problems.push(format!("{place} is already the launcher's hotkey")),
            Some((Target::Command(other), _)) => out.problems.push(format!("{place} is already used by `{other}`")),
            None => {
                out.shortcuts
                    .insert(shortcut, (Target::Command(id.clone()), hotkey.clone()));
            }
        }
    }
    out
}

/// Reads a hotkey and refuses one that would fire during ordinary typing.
pub fn check(hotkey: &str) -> Result<Shortcut, String> {
    let shortcut = hotkey.parse::<Shortcut>().map_err(|error| reason(&error))?;
    if !has_modifier_or_is_a_function_key(&shortcut) {
        return Err("needs a modifier key; on its own it would fire while typing in other applications".into());
    }
    Ok(shortcut)
}

/// Brings the registered set in line with `wanted`. `register` returns `true` on success. Returns the problems
/// (those found by `wanted` and those that could not be registered).
///
/// Failed registrations are not added to `registered`, so they are retried on the next call.
/// If the new launcher hotkey cannot be read or registered, the old one is kept
/// (without a way to open the launcher, the user cannot open the window to fix the settings).
pub fn apply(
    registered: &mut Registered,
    wanted: Wanted,
    mut register: impl FnMut(Shortcut) -> bool,
    mut unregister: impl FnMut(Shortcut),
) -> Vec<String> {
    let desired = wanted.shortcuts;
    let mut problems = wanted.problems;
    // Register additions first, so the old launcher hotkey can stay if registering a new one fails.
    let mut launcher_failed = !wanted.launcher_readable;
    let mut additions: Vec<(Shortcut, (Target, String))> = desired
        .iter()
        .filter(|(shortcut, _)| !registered.contains_key(shortcut))
        .map(|(shortcut, entry)| (*shortcut, entry.clone()))
        .collect();
    // Try the launcher first. Order does not change the result, but it keeps the notices in a stable order.
    additions.sort_by_key(|(_, (target, text))| (*target != Target::Launcher, text.clone()));
    for (shortcut, (target, text)) in additions {
        if register(shortcut) {
            registered.insert(shortcut, (target, text));
            continue;
        }
        let reason = "could not be registered (another application may be using it)";
        problems.push(match target {
            Target::Launcher => {
                launcher_failed = true;
                format!("hotkey `{text}`: {reason}")
            }
            Target::Command(id) => format!("commands/{id}.yaml: hotkey `{text}` {reason}"),
        });
    }
    let keep_old_launcher = |target: &Target| launcher_failed && *target == Target::Launcher;
    let stale: Vec<Shortcut> = registered
        .iter()
        .filter(|(shortcut, (target, _))| !desired.contains_key(shortcut) && !keep_old_launcher(target))
        .map(|(shortcut, _)| *shortcut)
        .collect();
    for shortcut in stale {
        unregister(shortcut);
        registered.remove(&shortcut);
    }
    // Retarget registered shortcuts (e.g. when a hotkey moved from one command to another).
    for (shortcut, entry) in desired {
        let Some(current) = registered.get_mut(&shortcut) else {
            continue;
        };
        if keep_old_launcher(&current.0)
            && let Target::Command(id) = &entry.0
        {
            problems.push(format!(
                "commands/{id}.yaml: hotkey `{}` is still the launcher's hotkey",
                entry.1
            ));
            continue;
        }
        *current = entry;
    }
    problems
}

/// Whether the shortcut has a modifier or is an F key not used in normal typing. Some OSes allow registering
/// a bare `a`, and then every `a` typed in another app would run the command.
fn has_modifier_or_is_a_function_key(shortcut: &Shortcut) -> bool {
    if !shortcut.mods.is_empty() {
        return true;
    }
    let key = format!("{:?}", shortcut.key);
    key.strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// Strips the library's request to report the issue to it (irrelevant to users) from its error message.
fn reason(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();
    match text.split_once(", if you feel like it should be") {
        Some((head, _)) => head.to_string(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(id, hotkey)| (id.to_string(), hotkey.to_string()))
            .collect()
    }

    fn target(wanted: &Wanted, hotkey: &str) -> Option<Target> {
        wanted
            .shortcuts
            .get(&hotkey.parse::<Shortcut>().unwrap())
            .map(|(target, _)| target.clone())
    }

    /// Runs `apply` against a fake registrar whose success or failure the test controls.
    struct Fake {
        registered: Registered,
        refuse: Vec<Shortcut>,
        calls: Vec<String>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                registered: Registered::new(),
                refuse: Vec::new(),
                calls: Vec::new(),
            }
        }

        fn sync(&mut self, launcher: &str, pairs: &[(&str, &str)]) -> Vec<String> {
            self.calls.clear();
            let refuse = self.refuse.clone();
            let calls = std::cell::RefCell::new(Vec::new());
            let problems = apply(
                &mut self.registered,
                wanted(launcher, &commands(pairs)),
                |shortcut| {
                    calls.borrow_mut().push(format!("register {shortcut}"));
                    !refuse.contains(&shortcut)
                },
                |shortcut| calls.borrow_mut().push(format!("unregister {shortcut}")),
            );
            self.calls = calls.into_inner();
            self.calls.sort();
            problems
        }

        fn target(&self, hotkey: &str) -> Option<Target> {
            self.registered.get(&shortcut(hotkey)).map(|(target, _)| target.clone())
        }
    }

    fn shortcut(hotkey: &str) -> Shortcut {
        hotkey.parse().unwrap()
    }

    fn call(verb: &str, hotkey: &str) -> String {
        format!("{verb} {}", shortcut(hotkey))
    }

    #[test]
    fn first_sync_registers_everything_and_a_repeat_touches_nothing() {
        // Arrange
        let mut fake = Fake::new();
        let pairs = [("a", "ctrl+alt+cmd+a"), ("b", "ctrl+alt+cmd+b")];

        // Act
        let first = fake.sync("alt+space", &pairs);
        let first_calls = fake.calls.clone();
        let second = fake.sync("alt+space", &pairs);

        // Assert
        assert_eq!(first, Vec::<String>::new());
        assert_eq!(first_calls.len(), 3, "{first_calls:?}");
        assert_eq!(second, Vec::<String>::new());
        assert_eq!(fake.calls, Vec::<String>::new());
        assert_eq!(fake.target("alt+space"), Some(Target::Launcher));
        assert_eq!(fake.target("ctrl+alt+cmd+b"), Some(Target::Command("b".into())));
    }

    #[test]
    fn removed_hotkeys_are_unregistered_and_moved_ones_are_retargeted_without_calls() {
        // Arrange
        let mut fake = Fake::new();
        fake.sync("alt+space", &[("a", "ctrl+alt+cmd+a"), ("b", "ctrl+alt+cmd+b")]);

        // Act: move a's hotkey to c and remove b's hotkey.
        let problems = fake.sync("alt+space", &[("c", "ctrl+alt+cmd+a")]);

        // Assert
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(fake.calls, [call("unregister", "ctrl+alt+cmd+b")]);
        assert_eq!(fake.target("ctrl+alt+cmd+a"), Some(Target::Command("c".into())));
        assert_eq!(fake.registered.len(), 2);
    }

    #[test]
    fn changing_the_launcher_registers_the_new_one_before_dropping_the_old_one() {
        // Arrange
        let mut fake = Fake::new();
        fake.sync("alt+space", &[]);

        // Act
        fake.sync("ctrl+alt+space", &[]);

        // Assert
        assert_eq!(
            fake.calls,
            [call("register", "ctrl+alt+space"), call("unregister", "alt+space")]
        );
        assert_eq!(fake.target("ctrl+alt+space"), Some(Target::Launcher));
        assert_eq!(fake.target("alt+space"), None);
    }

    #[test]
    fn the_old_launcher_stays_when_the_new_one_cannot_be_registered_or_read() {
        // Arrange
        let mut fake = Fake::new();
        fake.sync("alt+space", &[]);
        fake.refuse.push(shortcut("ctrl+alt+space"));

        // Act
        let refused = fake.sync("ctrl+alt+space", &[]);
        let refused_calls = fake.calls.clone();
        let unreadable = fake.sync("ctrl+alt+nope", &[]);

        // Assert
        assert_eq!(
            refused,
            ["hotkey `ctrl+alt+space`: could not be registered (another application may be using it)"]
        );
        assert_eq!(refused_calls, [call("register", "ctrl+alt+space")]);
        assert_eq!(unreadable.len(), 1, "{unreadable:?}");
        assert_eq!(fake.calls, Vec::<String>::new());
        assert_eq!(fake.target("alt+space"), Some(Target::Launcher));
    }

    #[test]
    fn the_old_launcher_is_not_handed_to_a_command_while_it_is_kept() {
        // Arrange: moving the launcher off alt+space failed, and alt+space was given to a command.
        let mut fake = Fake::new();
        fake.sync("alt+space", &[]);
        fake.refuse.push(shortcut("ctrl+alt+space"));

        // Act
        let problems = fake.sync("ctrl+alt+space", &[("a", "alt+space")]);

        // Assert
        assert_eq!(fake.target("alt+space"), Some(Target::Launcher));
        assert!(
            problems.contains(&"commands/a.yaml: hotkey `alt+space` is still the launcher's hotkey".to_string()),
            "{problems:?}"
        );
    }

    #[test]
    fn a_command_hotkey_that_cannot_be_registered_is_reported_and_tried_again_next_time() {
        // Arrange
        let mut fake = Fake::new();
        fake.refuse.push(shortcut("ctrl+alt+cmd+c"));

        // Act
        let first = fake.sync("alt+space", &[("chrome", "ctrl+alt+cmd+c")]);
        fake.refuse.clear();
        let second = fake.sync("alt+space", &[("chrome", "ctrl+alt+cmd+c")]);

        // Assert
        assert_eq!(
            first,
            [
                "commands/chrome.yaml: hotkey `ctrl+alt+cmd+c` could not be registered (another application may be using it)"
            ]
        );
        assert_eq!(second, Vec::<String>::new());
        assert_eq!(fake.calls, [call("register", "ctrl+alt+cmd+c")]);
        assert_eq!(fake.target("ctrl+alt+cmd+c"), Some(Target::Command("chrome".into())));
    }

    #[test]
    fn launcher_and_command_hotkeys_are_all_wanted() {
        // Arrange / Act
        let wanted = wanted(
            "alt+space",
            &commands(&[("chrome", "ctrl+alt+cmd+c"), ("claude", "ctrl+cmd+space")]),
        );

        // Assert
        assert_eq!(wanted.problems, Vec::<String>::new());
        assert!(wanted.launcher_readable);
        assert_eq!(wanted.shortcuts.len(), 3);
        assert_eq!(target(&wanted, "alt+space"), Some(Target::Launcher));
        assert_eq!(
            target(&wanted, "ctrl+alt+cmd+c"),
            Some(Target::Command("chrome".into()))
        );
        assert_eq!(
            target(&wanted, "ctrl+cmd+space"),
            Some(Target::Command("claude".into()))
        );
    }

    #[test]
    fn the_launcher_wins_and_the_first_command_wins_even_when_spelled_differently() {
        // Arrange
        let wanted = wanted(
            "alt+space",
            &commands(&[
                ("a", "Alt+Space"),
                ("b", "ctrl+alt+cmd+c"),
                ("c", "Control+Option+Command+KeyC"),
            ]),
        );

        // Assert
        assert_eq!(
            wanted.problems,
            [
                "commands/a.yaml: hotkey `Alt+Space` is already the launcher's hotkey",
                "commands/c.yaml: hotkey `Control+Option+Command+KeyC` is already used by `b`",
            ]
        );
        assert_eq!(target(&wanted, "alt+space"), Some(Target::Launcher));
        assert_eq!(target(&wanted, "ctrl+alt+cmd+c"), Some(Target::Command("b".into())));
    }

    #[test]
    fn command_hotkeys_without_a_modifier_are_refused_except_function_keys() {
        // Arrange / Act
        let wanted = wanted("alt+space", &commands(&[("a", "a"), ("b", "space"), ("f", "f13")]));

        // Assert
        assert_eq!(
            wanted.problems,
            [
                "commands/a.yaml: hotkey `a`: needs a modifier key; on its own it would fire while typing in other applications",
                "commands/b.yaml: hotkey `space`: needs a modifier key; on its own it would fire while typing in other applications",
            ]
        );
        assert_eq!(target(&wanted, "f13"), Some(Target::Command("f".into())));
        assert_eq!(wanted.shortcuts.len(), 2);
    }

    #[test]
    fn a_launcher_hotkey_without_a_modifier_is_refused_like_a_command_hotkey() {
        // Arrange / Act
        let wanted = wanted("space", &[]);

        // Assert
        assert!(!wanted.launcher_readable);
        assert_eq!(
            wanted.problems,
            ["hotkey `space`: needs a modifier key; on its own it would fire while typing in other applications"]
        );
    }

    #[test]
    fn unreadable_hotkeys_are_reported_and_left_out() {
        // Arrange / Act
        let wanted = wanted("alt+spcae", &commands(&[("a", "ctrl+alt+nope"), ("b", "ctrl+b")]));

        // Assert
        assert!(!wanted.launcher_readable);
        assert_eq!(wanted.problems.len(), 2, "{:?}", wanted.problems);
        assert!(
            wanted.problems[0].starts_with("hotkey `alt+spcae`: "),
            "{:?}",
            wanted.problems
        );
        assert!(
            wanted.problems[1].starts_with("commands/a.yaml: hotkey `ctrl+alt+nope`: "),
            "{:?}",
            wanted.problems
        );
        assert_eq!(wanted.shortcuts.len(), 1);
        assert_eq!(target(&wanted, "ctrl+b"), Some(Target::Command("b".into())));
    }

    #[test]
    fn every_hotkey_the_alfred_import_writes_can_be_read() {
        // Arrange: the modifier names the importer writes, and every key name.
        let keys: Vec<&str> = flintkey_core::alfred::key_names().collect();
        assert!(keys.len() > 60, "{keys:?}");

        for key in keys {
            let hotkey = format!("ctrl+alt+shift+cmd+{key}");

            // Act
            let parsed = hotkey.parse::<Shortcut>();

            // Assert
            assert!(parsed.is_ok(), "{hotkey}: {parsed:?}");
        }
    }
}
