//! 設定に書かれたホットキーから、登録するショートカットの組を決める (純粋な関数)。

use std::collections::HashMap;
use tauri_plugin_global_shortcut::Shortcut;

/// ホットキーを押したときに起きること。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Launcher,
    /// コマンドの id。
    Command(String),
}

/// 登録済みのショートカット → 押したときに起きることと、設定に書かれていた文字列。
pub type Registered = HashMap<Shortcut, (Target, String)>;

#[derive(Debug, Default, PartialEq)]
pub struct Wanted {
    /// 登録したいショートカットと、押したときに起きること。書かれていた文字列も持つ (知らせに使う)。
    pub shortcuts: HashMap<Shortcut, (Target, String)>,
    /// ランチャーのホットキーが読めたか。読めなければ、今登録しているものを使い続ける。
    pub launcher_readable: bool,
    pub problems: Vec<String>,
}

/// `launcher` はランチャーのホットキー、`commands` は (id, ホットキー) を id の順に並べたもの。
///
/// 重なったら、ランチャーが優先、コマンド同士は先に来たもの。大文字小文字や書き方の違い
/// (`Ctrl+Alt+C` と `ctrl+alt+KeyC`) は同じショートカットとして比べる。
pub fn wanted(launcher: &str, commands: &[(String, String)]) -> Wanted {
    let mut out = Wanted::default();
    match launcher.parse::<Shortcut>() {
        Ok(shortcut) => {
            out.launcher_readable = true;
            out.shortcuts.insert(shortcut, (Target::Launcher, launcher.to_string()));
        }
        Err(error) => out.problems.push(format!("hotkey `{launcher}`: {}", reason(&error))),
    }
    for (id, hotkey) in commands {
        let place = format!("commands/{id}.yaml: hotkey `{hotkey}`");
        let shortcut = match hotkey.parse::<Shortcut>() {
            Ok(shortcut) => shortcut,
            Err(error) => {
                out.problems.push(format!("{place}: {}", reason(&error)));
                continue;
            }
        };
        if !has_modifier_or_is_a_function_key(&shortcut) {
            out.problems.push(format!(
                "{place} needs a modifier key; on its own it would fire while typing in other applications"
            ));
            continue;
        }
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

/// 登録済みの組を `wanted` に合わせる。`register` は登録できたら `true` を返す。問題の一覧を返す
/// (`wanted` が見つけたものと、登録できなかったもの)。
///
/// 登録できなかったものは `registered` に入れないので、次に呼ばれたときにもう一度試す。
/// ランチャーのホットキーは、新しいものが読めない・登録できない場合は古いものを残す
/// (ランチャーを出す手段が無くなると、設定を直すために窓を出せない)。
pub fn apply(
    registered: &mut Registered,
    wanted: Wanted,
    mut register: impl FnMut(Shortcut) -> bool,
    mut unregister: impl FnMut(Shortcut),
) -> Vec<String> {
    let desired = wanted.shortcuts;
    let mut problems = wanted.problems;
    // 足すものを先に登録する。ランチャーのホットキーを変えて登録に失敗しても、古い方を残せるように。
    let mut launcher_failed = !wanted.launcher_readable;
    let mut additions: Vec<(Shortcut, (Target, String))> = desired
        .iter()
        .filter(|(shortcut, _)| !registered.contains_key(shortcut))
        .map(|(shortcut, entry)| (*shortcut, entry.clone()))
        .collect();
    // ランチャーを先に試す。順序で結果が変わることは無いが、知らせの並びを決まったものにする。
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
    // 登録済みのショートカットの行き先を付け替える (あるコマンドのホットキーを別のコマンドへ移した場合など)。
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

/// 修飾キーがあるか、ふだんの入力に使わない F キーか。修飾キー無しの `a` を登録できる OS があり、
/// 登録できると、別のアプリで `a` を打つたびにコマンドが動く。
fn has_modifier_or_is_a_function_key(shortcut: &Shortcut) -> bool {
    if !shortcut.mods.is_empty() {
        return true;
    }
    let key = format!("{:?}", shortcut.key);
    key.strip_prefix('F')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// ライブラリのエラー文から、ライブラリへの報告の依頼 (利用者には関係ない) を除く。
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

    /// `apply` を、登録の成否を決められる偽の登録先で動かす。
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

        // Act: a のホットキーを c に移し、b のホットキーを消す。
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
        // Arrange: ランチャーを alt+space から移そうとして失敗し、alt+space をコマンドに渡そうとした。
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
                "commands/a.yaml: hotkey `a` needs a modifier key; on its own it would fire while typing in other applications",
                "commands/b.yaml: hotkey `space` needs a modifier key; on its own it would fire while typing in other applications",
            ]
        );
        assert_eq!(target(&wanted, "f13"), Some(Target::Command("f".into())));
        assert_eq!(wanted.shortcuts.len(), 2);
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
        // Arrange: 移行が書き出す修飾キーの名前と、すべてのキーの名前。
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
