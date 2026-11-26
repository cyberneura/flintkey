//! Turns a [`Command`] or [`Settings`] back into YAML text.
//!
//! New files are written in a fixed format. Existing files are edited in place, rewriting only
//! keys whose meaning changed. Hand-written comments, blank lines and quoting survive, and
//! changing one setting gives a one-line Git diff.

use crate::command::{Action, Command, TYPE_KEYS, parse_command};
use crate::settings::{Settings, parse_settings};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use std::collections::BTreeMap;
use std::str::FromStr;
use yaml_edit::{Document, Mapping, ScalarValue, YamlValue};

/// The value of one key. Only the shapes that appear in command files.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Text(String),
    Flag(bool),
    List(Vec<String>),
    Map(BTreeMap<String, String>),
    Table(Fields),
}

/// Keys in the order they are written to a new file.
type Fields = Vec<(&'static str, Node)>;

/// The keys a command needs. Keys at their default value are left out.
fn fields(command: &Command) -> Fields {
    let mut out: Fields = vec![("title", Node::Text(command.title.clone()))];
    if let Some(keywords) = one_or_many(&command.keywords) {
        out.push(("keyword", keywords));
    }
    if let Some(hotkey) = &command.hotkey {
        out.push(("hotkey", Node::Text(hotkey.clone())));
    }
    out.push(("type", Node::Text(command.action.type_name().to_string())));
    match &command.action {
        Action::Url { url } => out.push(("url", Node::Text(url.clone()))),
        Action::App { app, args } => {
            out.push(("app", Node::Text(app.clone())));
            if !args.is_empty() {
                out.push(("args", Node::List(args.clone())));
            }
        }
        Action::Script { run, cwd, env } => {
            out.push(("run", Node::Text(run.clone())));
            if let Some(cwd) = cwd {
                out.push(("cwd", Node::Text(cwd.clone())));
            }
            if !env.is_empty() {
                out.push(("env", Node::Map(env.clone())));
            }
        }
        Action::Terminal { run, cwd } => {
            out.push(("run", Node::Text(run.clone())));
            if let Some(cwd) = cwd {
                out.push(("cwd", Node::Text(cwd.clone())));
            }
        }
    }
    if command.pin {
        out.push(("pin", Node::Flag(true)));
    }
    if !command.tags.is_empty() {
        out.push(("tags", Node::List(command.tags.clone())));
    }
    if !command.os.is_empty() {
        let names = command.os.iter().map(|os| os.name().to_string()).collect();
        out.push(("when", Node::Table(vec![("os", Node::List(names))])));
    }
    out
}

/// A single keyword is written as a plain string: most commands have one, and that is how people write it.
fn one_or_many(items: &[String]) -> Option<Node> {
    match items {
        [] => None,
        [one] => Some(Node::Text(one.clone())),
        many => Some(Node::List(many.to_vec())),
    }
}

struct Ordered<'a>(&'a Fields);

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, node) in self.0 {
            map.serialize_entry(key, node)?;
        }
        map.end()
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Node::Text(text) => serializer.serialize_str(text),
            Node::Flag(flag) => serializer.serialize_bool(*flag),
            Node::List(items) => items.serialize(serializer),
            Node::Map(map) => map.serialize(serializer),
            Node::Table(fields) => Ordered(fields).serialize(serializer),
        }
    }
}

/// The text of a new command file.
pub(crate) fn render(command: &Command) -> String {
    let mut options = serde_saphyr::SerializerOptions::default();
    // Lists are indented under their key, to match what the in-place editing library writes.
    options.compact_list_indent = false;
    serde_saphyr::to_string_with_options(&Ordered(&fields(command)), options).expect("a command always serializes")
}

/// The result of applying a changed command to an existing file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub text: String,
    /// False if the whole file was rewritten in the fixed format (comments are lost).
    /// Happens when the original text was not a valid command, or when the in-place edit
    /// did not produce the requested command.
    pub kept_formatting: bool,
}

/// The keys of `settings.yaml`. Unlike commands, every key is written, so a new file shows what can be set.
fn settings_fields(settings: &Settings) -> Fields {
    let mut out: Fields = vec![
        ("hotkey", Node::Text(settings.hotkey.clone())),
        (
            "window",
            Node::Table(vec![("display", Node::Text(settings.display.name().to_string()))]),
        ),
    ];
    if let Some(app) = settings.terminal {
        out.push((
            "terminal",
            Node::Table(vec![("app", Node::Text(app.name().to_string()))]),
        ));
    }
    out
}

/// Text of a new `settings.yaml`.
pub(crate) fn render_settings(settings: &Settings) -> String {
    let mut options = serde_saphyr::SerializerOptions::default();
    options.compact_list_indent = false;
    serde_saphyr::to_string_with_options(&Ordered(&settings_fields(settings)), options)
        .expect("settings always serialize")
}

/// Applies `settings` to the existing text of `settings.yaml`, like [`update`] does for commands.
pub(crate) fn update_settings(source: &str, settings: &Settings) -> Updated {
    let rewrite = || Updated {
        text: render_settings(settings),
        kept_formatting: false,
    };
    let Ok(current) = parse_settings(source) else {
        return rewrite();
    };
    if current == *settings {
        return Updated {
            text: source.to_string(),
            kept_formatting: true,
        };
    }
    let edited = Document::from_str(source).ok().and_then(|document| {
        let root = document.as_mapping()?;
        apply(&root, &settings_fields(&current), &settings_fields(settings))?;
        Some(document.to_string())
    });
    match edited {
        Some(text) if parse_settings(&text).as_ref() == Ok(settings) => Updated {
            text,
            kept_formatting: true,
        },
        _ => rewrite(),
    }
}

/// Applies `command` to the text of an existing file.
pub(crate) fn update(source: &str, command: &Command) -> Updated {
    let Ok(current) = parse_command(source) else {
        return Updated {
            text: render(command),
            kept_formatting: false,
        };
    };
    // Decide before going through the in-place editing library, so saving an unchanged form
    // is guaranteed to leave the file untouched whatever the library does.
    if current == *command {
        return Updated {
            text: source.to_string(),
            kept_formatting: true,
        };
    }
    // The in-place editing library is young. Re-parse its output and accept it only if it means
    // exactly the requested command.
    match edit_in_place(source, &fields(&current), &fields(command)) {
        Some(text) if parse_command(&text).as_ref() == Ok(command) => Updated {
            text,
            kept_formatting: true,
        },
        _ => Updated {
            text: render(command),
            kept_formatting: false,
        },
    }
}

fn edit_in_place(source: &str, old: &Fields, new: &Fields) -> Option<String> {
    let document = Document::from_str(source).ok()?;
    let root = document.as_mapping()?;
    // `old` has no default-valued keys, so a key written with its default such as `env: {}`
    // is never removed by `apply`. Left behind after a type change it makes the file invalid,
    // so keys the new type does not accept are removed here.
    // Leave them alone if the type is unchanged. An empty key (`cwd:`) is valid even for another type,
    // and an unrelated edit must not remove it.
    let Some(Node::Text(new_type)) = lookup(new, "type") else {
        return None;
    };
    if lookup(old, "type") != lookup(new, "type") {
        for (key, accepted_by) in TYPE_KEYS {
            if !accepted_by.contains(&new_type.as_str()) {
                root.remove(key);
            }
        }
    }
    apply(&root, old, new)?;
    Some(document.to_string())
}

fn lookup<'a>(fields: &'a Fields, key: &str) -> Option<&'a Node> {
    fields.iter().find(|(name, _)| *name == key).map(|(_, node)| node)
}

/// Rewrites `mapping`, which currently means `old`, so that it means `new`.
///
/// Keys are compared by meaning, not text. Keys whose value did not change are not written,
/// because writing them would normalize their quoting.
fn apply(mapping: &Mapping, old: &Fields, new: &Fields) -> Option<()> {
    for (key, _) in old {
        if lookup(new, key).is_none() {
            mapping.remove(*key);
        }
    }
    for (index, (key, want)) in new.iter().enumerate() {
        let have = lookup(old, key);
        if have == Some(want) {
            continue;
        }
        // A default-valued key may be written in the file (`pin: false`).
        // In that case change the value where it is.
        if have.is_some() || mapping.contains_key(*key) {
            change(mapping, key, have, want)?;
            continue;
        }
        // A new key goes after the key that precedes it in the fixed order, not at the end,
        // so two machines adding different keys do not change the same lines.
        let predecessor = new[..index]
            .iter()
            .rev()
            .map(|(name, _)| *name)
            .find(|name| mapping.contains_key(*name));
        match predecessor {
            Some(predecessor) => mapping.insert_after(predecessor, *key, to_yaml(want)).then_some(())?,
            None => mapping.insert_at_index(0, *key, to_yaml(want)),
        }
    }
    Some(())
}

fn change(mapping: &Mapping, key: &str, have: Option<&Node>, want: &Node) -> Option<()> {
    match (have, want) {
        (Some(Node::List(have)), Node::List(want)) => {
            // Editing by index is only correct when the file order matches `have`. `os` is
            // sorted on load, so it can differ from the hand-written order.
            let sequence = mapping.get_sequence(key).filter(|sequence| {
                let on_file: Option<Vec<String>> = sequence
                    .values()
                    .map(|item| item.as_scalar().map(|scalar| scalar.as_string()))
                    .collect();
                on_file.as_ref() == Some(have)
            });
            if let Some(sequence) = sequence {
                let shared = have.len().min(want.len());
                for index in 0..shared {
                    if have[index] != want[index] {
                        sequence.set(index, ScalarValue::string(&want[index])).then_some(())?;
                    }
                }
                for item in &want[shared..] {
                    sequence.push(ScalarValue::string(item));
                }
                for _ in want.len()..have.len() {
                    sequence.pop()?;
                }
                return Some(());
            }
        }
        (Some(Node::Map(have)), Node::Map(want)) => {
            if let Some(inner) = mapping.get_mapping(key) {
                for name in have.keys().filter(|name| !want.contains_key(*name)) {
                    inner.remove(name.as_str());
                }
                for (name, value) in want {
                    if have.get(name) != Some(value) {
                        inner.set(name.as_str(), ScalarValue::string(value));
                    }
                }
                return Some(());
            }
        }
        (Some(Node::Table(have)), Node::Table(want)) => {
            if let Some(inner) = mapping.get_mapping(key) {
                return apply(&inner, have, want);
            }
        }
        _ => {}
    }
    // Scalars, and collections written in a shape the branches above cannot handle
    // (`os: macos` instead of a list), are replaced wholesale.
    mapping.set(key, to_yaml(want));
    Some(())
}

fn to_yaml(node: &Node) -> YamlValue {
    match node {
        Node::Text(text) => YamlValue::Scalar(ScalarValue::string(text)),
        Node::Flag(flag) => YamlValue::Scalar(ScalarValue::from(*flag)),
        Node::List(items) => YamlValue::Sequence(
            items
                .iter()
                .map(|item| YamlValue::Scalar(ScalarValue::string(item)))
                .collect(),
        ),
        Node::Map(map) => YamlValue::Mapping(
            map.iter()
                .map(|(name, value)| (name.clone(), YamlValue::Scalar(ScalarValue::string(value))))
                .collect(),
        ),
        Node::Table(fields) => YamlValue::Mapping(
            fields
                .iter()
                .map(|(name, node)| (name.to_string(), to_yaml(node)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Os;

    const HAND_WRITTEN: &str = "\
# Deploy the blog. Written by hand.
title: Deploy blog   # shown in the list
keyword: deploy

type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  # token for gh
  GITHUB_TOKEN: op://Private/GitHub/token
  FOO: \"bar\"   # quoted on purpose
";

    /// Strings that careless output would turn into another type or into broken YAML.
    const TRICKY: [&str; 20] = [
        "no",
        "on",
        "true",
        "0123",
        "1.0",
        "null",
        "~",
        "a: b",
        "x # not a comment",
        "# heading",
        "ブログをデプロイ",
        "say \"hi\" and 'bye'",
        "{not: flow}",
        "[not, a, list]",
        "*alias",
        "&anchor",
        " padded ",
        "C:\\Users\\me\\app.exe",
        "echo one\necho two\n",
        "a\tb",
    ];

    fn script(run: &str) -> Command {
        Command {
            title: "Deploy blog".into(),
            keywords: vec!["deploy".into()],
            hotkey: None,
            pin: false,
            tags: vec![],
            os: [].into(),
            action: Action::Script {
                run: run.into(),
                cwd: Some("~/workspace/blog".into()),
                env: BTreeMap::from([
                    ("GITHUB_TOKEN".to_string(), "op://Private/GitHub/token".to_string()),
                    ("FOO".to_string(), "bar".to_string()),
                ]),
            },
        }
    }

    /// Lines of `before` that are not in `after`.
    fn changed_lines(before: &str, after: &str) -> Vec<String> {
        before
            .lines()
            .filter(|line| !after.lines().any(|other| other == *line))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn new_file_has_the_fixed_layout() {
        // Arrange
        let command = Command {
            title: "Deploy blog".into(),
            keywords: vec!["deploy".into()],
            hotkey: None,
            pin: true,
            tags: vec![],
            os: [Os::Macos, Os::Windows].into(),
            action: Action::Script {
                run: "fab deploy".into(),
                cwd: Some("~/workspace/blog".into()),
                env: BTreeMap::from([("STAGE".to_string(), "prod".to_string())]),
            },
        };

        // Act
        let text = render(&command);

        // Assert
        assert_eq!(
            text,
            "\
title: Deploy blog
keyword: deploy
type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  STAGE: prod
pin: true
when:
  os:
    - macos
    - windows
"
        );
    }

    #[test]
    fn new_file_writes_aliases_hotkey_and_tags_in_the_fixed_order() {
        // Arrange
        let command = Command {
            title: "Today".into(),
            keywords: vec!["今日".into(), "today".into()],
            hotkey: Some("ctrl+alt+cmd+t".into()),
            pin: false,
            tags: vec!["個人/コマンド".into()],
            os: [Os::Macos].into(),
            action: Action::App {
                app: "/Applications/Clock.app".into(),
                args: vec![],
            },
        };

        // Act
        let text = render(&command);

        // Assert
        assert_eq!(
            text,
            "\
title: Today
keyword:
  - 今日
  - today
hotkey: ctrl+alt+cmd+t
type: app
app: /Applications/Clock.app
tags:
  - 個人/コマンド
when:
  os:
    - macos
"
        );
        assert_eq!(parse_command(&text), Ok(command));
    }

    #[test]
    fn tricky_keywords_and_tags_read_back() {
        // Arrange
        let words: Vec<String> = TRICKY.iter().map(ToString::to_string).collect();
        let command = Command {
            title: "T".into(),
            keywords: words.clone(),
            hotkey: None,
            pin: false,
            tags: words,
            os: [].into(),
            action: Action::Url {
                url: "https://example.com".into(),
            },
        };

        // Act
        let rendered = render(&command);

        // Assert
        assert_eq!(parse_command(&rendered), Ok(command), "rendered as:\n{rendered}");
    }

    #[test]
    fn adding_an_alias_hotkey_and_tag_keeps_the_rest_of_the_file() {
        // Arrange
        let source = "# mine\ntitle: A   # shown\nkeyword: a\ntype: url\nurl: https://example.com\npin: true\n";
        let mut command = parse_command(source).unwrap();
        command.keywords.push("b".into());
        command.hotkey = Some("ctrl+alt+cmd+a".into());
        command.tags = vec!["work".into()];

        // Act
        let updated = update(source, &command);

        // Assert: the in-place editing library writes an appended list without indentation (same meaning in YAML).
        assert!(updated.kept_formatting, "{}", updated.text);
        assert_eq!(
            updated.text,
            "# mine\ntitle: A   # shown\nkeyword:\n  - a\n  - b\nhotkey: ctrl+alt+cmd+a\ntype: url\nurl: https://example.com\npin: true\ntags:\n- work\n"
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn new_file_reads_back_as_the_same_command_for_tricky_text() {
        for text in TRICKY {
            // Arrange
            let command = Command {
                title: text.into(),
                keywords: vec![text.into()],
                hotkey: None,
                pin: false,
                tags: vec![],
                os: [].into(),
                action: Action::Script {
                    run: text.into(),
                    cwd: Some(text.into()),
                    env: BTreeMap::from([("VALUE".to_string(), text.to_string())]),
                },
            };

            // Act
            let rendered = render(&command);

            // Assert
            assert_eq!(
                parse_command(&rendered),
                Ok(command),
                "text {text:?} rendered as:\n{rendered}"
            );
        }
    }

    #[test]
    fn new_file_reads_back_for_every_action_type() {
        // Arrange
        let actions = [
            Action::Url {
                url: "https://example.com/?a=1&b=2#frag".into(),
            },
            Action::App {
                app: "Visual Studio Code".into(),
                args: vec!["--new-window".into(), "~/x y".into()],
            },
            Action::App {
                app: "Slack".into(),
                args: vec![],
            },
            Action::Terminal {
                run: "ssh host".into(),
                cwd: None,
            },
        ];

        for action in actions {
            let command = Command {
                title: "T".into(),
                keywords: vec![],
                hotkey: None,
                pin: false,
                tags: vec![],
                os: [Os::Linux].into(),
                action,
            };

            // Act
            let rendered = render(&command);

            // Assert
            assert_eq!(parse_command(&rendered), Ok(command), "rendered as:\n{rendered}");
        }
    }

    #[test]
    fn unchanged_command_leaves_the_file_byte_identical() {
        // Arrange: save, from the form, a command the user did not change.
        let command = parse_command(HAND_WRITTEN).unwrap();

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert_eq!(
            updated,
            Updated {
                text: HAND_WRITTEN.to_string(),
                kept_formatting: true
            }
        );
    }

    #[test]
    fn changing_one_value_changes_one_line() {
        // Arrange
        let mut command = parse_command(HAND_WRITTEN).unwrap();
        command.title = "Deploy the blog".into();

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            changed_lines(HAND_WRITTEN, &updated.text),
            ["title: Deploy blog   # shown in the list"]
        );
        assert!(
            updated.text.contains("title: Deploy the blog   # shown in the list"),
            "{}",
            updated.text
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn changing_one_environment_variable_keeps_the_quoting_of_the_others() {
        // Arrange
        let mut command = script("fab deploy");
        let Action::Script { env, .. } = &mut command.action else {
            unreachable!()
        };
        env.insert("GITHUB_TOKEN".into(), "op://Work/GitHub/token".into());

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            changed_lines(HAND_WRITTEN, &updated.text),
            ["  GITHUB_TOKEN: op://Private/GitHub/token"]
        );
        assert!(
            updated.text.contains("  FOO: \"bar\"   # quoted on purpose"),
            "{}",
            updated.text
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn adding_and_removing_environment_variables_touches_only_those_lines() {
        // Arrange
        let mut command = script("fab deploy");
        let Action::Script { env, .. } = &mut command.action else {
            unreachable!()
        };
        env.remove("FOO");
        env.insert("STAGE".into(), "prod".into());

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            changed_lines(HAND_WRITTEN, &updated.text),
            ["  FOO: \"bar\"   # quoted on purpose"]
        );
        assert!(updated.text.contains("  STAGE: prod\n"), "{}", updated.text);
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn removing_an_optional_key_removes_its_line() {
        // Arrange
        let mut command = script("fab deploy");
        let Action::Script { cwd, .. } = &mut command.action else {
            unreachable!()
        };
        *cwd = None;

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(changed_lines(HAND_WRITTEN, &updated.text), ["cwd: ~/workspace/blog"]);
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn new_keys_are_inserted_in_the_fixed_order_not_at_the_end() {
        // Arrange
        let source = "title: A\ntype: script\nrun: x\npin: true\n";
        let mut command = parse_command(source).unwrap();
        command.keywords = vec!["a".into()];
        let Action::Script { cwd, env, .. } = &mut command.action else {
            unreachable!()
        };
        *cwd = Some("~/x".into());
        env.insert("K".into(), "v".into());
        command.os = [Os::Macos].into();

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            updated.text,
            "title: A\nkeyword: a\ntype: script\nrun: x\ncwd: ~/x\nenv:\n  K: v\npin: true\nwhen:\n  os:\n    - macos\n"
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn key_written_at_its_default_is_changed_where_it_stands() {
        // Arrange: `pin: false` means the same as no `pin`.
        let source = "title: A\npin: false   # maybe later\ntype: url\nurl: https://example.com\n";
        let mut command = parse_command(source).unwrap();
        command.pin = true;

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            updated.text,
            "title: A\npin: true   # maybe later\ntype: url\nurl: https://example.com\n"
        );
    }

    #[test]
    fn list_edits_keep_untouched_items_and_their_comments() {
        // Arrange
        let source = "title: A\ntype: app\napp: Code\nargs:\n  - --new-window   # always\n  - one\n  - two\n";
        let mut command = parse_command(source).unwrap();
        let Action::App { args, .. } = &mut command.action else {
            unreachable!()
        };
        *args = vec!["--new-window".into(), "uno".into()];

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert!(
            updated.text.contains("  - --new-window   # always\n"),
            "{}",
            updated.text
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn single_os_name_is_replaced_when_more_are_added() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\nwhen:\n  os: macos\n";
        let mut command = parse_command(source).unwrap();
        command.os = [Os::Macos, Os::Windows].into();

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn os_list_written_out_of_order_does_not_cost_the_rest_of_the_file_its_comments() {
        // Arrange: the loaded `os` is sorted, but the file keeps the hand-written order.
        let source = "\
# why this exists
title: A   # shown
type: url
url: https://example.com
when:
  os: [windows, macos]   # not linux
";
        for wanted in [
            vec![Os::Macos],
            vec![Os::Macos, Os::Linux],
            vec![Os::Windows, Os::Linux],
        ] {
            let mut command = parse_command(source).unwrap();
            command.os = wanted.iter().copied().collect();

            // Act
            let updated = update(source, &command);

            // Assert
            assert!(
                updated.kept_formatting,
                "rewrote the whole file for {wanted:?}:\n{}",
                updated.text
            );
            assert!(
                updated.text.starts_with("# why this exists\ntitle: A   # shown\n"),
                "{}",
                updated.text
            );
            assert_eq!(parse_command(&updated.text), Ok(command));
        }
    }

    #[test]
    fn changing_the_type_removes_keys_of_the_old_type_that_were_written_at_their_default() {
        // Arrange: `env: {}` and `args: []` are defaults, so they do not appear in the loaded command.
        let cases = [
            "# keep me\ntitle: A   # shown\ntype: script\nrun: x\nenv: {}\n",
            "# keep me\ntitle: A   # shown\ntype: app\napp: Code\nargs: []\n",
        ];
        for source in cases {
            let mut command = parse_command(source).unwrap();
            command.action = Action::Url {
                url: "https://example.com".into(),
            };

            // Act
            let updated = update(source, &command);

            // Assert
            assert!(updated.kept_formatting, "rewrote the whole file:\n{}", updated.text);
            assert!(
                updated.text.starts_with("# keep me\ntitle: A   # shown\n"),
                "{}",
                updated.text
            );
            assert_eq!(parse_command(&updated.text), Ok(command));
        }
    }

    #[test]
    fn key_of_the_current_type_written_at_its_default_is_left_alone() {
        // Arrange
        let source = "title: A\ntype: app\napp: Code\nargs: []   # none yet\n";
        let mut command = parse_command(source).unwrap();
        command.title = "B".into();

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(updated.text, "title: B\ntype: app\napp: Code\nargs: []   # none yet\n");
    }

    #[test]
    fn empty_key_of_another_type_is_left_alone_while_the_type_stays() {
        // Arrange: an empty key counts as absent when loaded, so this file is valid.
        let source = "title: A\ntype: url\nurl: https://example.com\ncwd:   # for later\n";
        let mut command = parse_command(source).unwrap();
        command.title = "B".into();

        // Act
        let updated = update(source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert_eq!(
            updated.text,
            "title: B\ntype: url\nurl: https://example.com\ncwd:   # for later\n"
        );
    }

    #[test]
    fn changing_the_type_swaps_the_type_specific_keys() {
        // Arrange
        let mut command = parse_command(HAND_WRITTEN).unwrap();
        command.action = Action::Url {
            url: "https://example.com".into(),
        };

        // Act
        let updated = update(HAND_WRITTEN, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert!(
            updated.text.contains("# Deploy the blog. Written by hand."),
            "{}",
            updated.text
        );
        assert!(
            updated.text.contains("title: Deploy blog   # shown in the list"),
            "{}",
            updated.text
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn tricky_text_survives_an_in_place_edit() {
        for text in TRICKY {
            // Arrange
            let command = script(text);

            // Act
            let updated = update(HAND_WRITTEN, &command);

            // Assert
            assert!(updated.kept_formatting, "fell back to a rewrite for {text:?}");
            assert_eq!(
                parse_command(&updated.text),
                Ok(command),
                "text {text:?} written as:\n{}",
                updated.text
            );
            assert!(updated.text.contains("# token for gh"), "{}", updated.text);
        }
    }

    #[test]
    fn crlf_file_stays_crlf() {
        // Arrange
        let source = HAND_WRITTEN.replace('\n', "\r\n");
        let mut command = parse_command(&source).unwrap();
        command.title = "X".into();
        command.pin = true;

        // Act
        let updated = update(&source, &command);

        // Assert
        assert!(updated.kept_formatting);
        assert!(
            !updated.text.replace("\r\n", "").contains('\n'),
            "mixed line endings:\n{:?}",
            updated.text
        );
        assert_eq!(parse_command(&updated.text), Ok(command));
    }

    #[test]
    fn invalid_file_is_rewritten_and_says_so() {
        // Arrange: the original text is not a valid command, so per-key editing is impossible.
        let source = "# notes\ntitle: A\ntype: nope\n";
        let command = Command {
            title: "A".into(),
            keywords: vec![],
            hotkey: None,
            pin: false,
            tags: vec![],
            os: [].into(),
            action: Action::Url {
                url: "https://example.com".into(),
            },
        };

        // Act
        let updated = update(source, &command);

        // Assert
        assert_eq!(
            updated,
            Updated {
                text: "title: A\ntype: url\nurl: https://example.com\n".to_string(),
                kept_formatting: false
            }
        );
    }
}
