//! [`Command`] を YAML のテキストに戻す。
//!
//! 新規ファイルは決まった書式で出力する。既存ファイルはその場で編集し、意味が変わった
//! キーだけを書き換える。手書きのコメント・空行・クォートが残り、設定を 1 つ変えたときの
//! Git の差分が 1 行で済む。

use crate::command::{Action, Command, TYPE_KEYS, parse_command};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use std::collections::BTreeMap;
use std::str::FromStr;
use yaml_edit::{Document, Mapping, ScalarValue, YamlValue};

/// キー 1 つの値。コマンドファイルに現れる形だけを持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Text(String),
    Flag(bool),
    List(Vec<String>),
    Map(BTreeMap<String, String>),
    Table(Fields),
}

/// 新規ファイルに書き出す順に並べたキー。
type Fields = Vec<(&'static str, Node)>;

/// コマンドに必要なキー。既定値のものは含めない。
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

/// キーワードは 1 つなら文字列で書く。ほとんどのコマンドは 1 つで、手書きの形もそうなるため。
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

/// 新規コマンドファイルのテキスト。
pub(crate) fn render(command: &Command) -> String {
    let mut options = serde_saphyr::SerializerOptions::default();
    // リストはキーの下に字下げする。その場編集のライブラリが書く形に揃えるため。
    options.compact_list_indent = false;
    serde_saphyr::to_string_with_options(&Ordered(&fields(command)), options).expect("a command always serializes")
}

/// 変更したコマンドを既存ファイルに反映した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub text: String,
    /// 決まった書式でファイル全体を書き直した場合は false (コメントは失われる)。
    /// 元のテキストが有効なコマンドでなかった場合と、その場編集の結果が
    /// 要求したコマンドにならなかった場合に起きる。
    pub kept_formatting: bool,
}

/// 既存ファイルのテキストに `command` を反映する。
pub(crate) fn update(source: &str, command: &Command) -> Updated {
    let Ok(current) = parse_command(source) else {
        return Updated {
            text: render(command),
            kept_formatting: false,
        };
    };
    // その場編集のライブラリを通す前に判定する。変更していないフォームを保存したときに、
    // ライブラリの挙動に関係なくファイルが変わらないことを保証するため。
    if current == *command {
        return Updated {
            text: source.to_string(),
            kept_formatting: true,
        };
    }
    // その場編集のライブラリは歴史が浅い。出力を読み直して、要求したコマンドと
    // 完全に同じ意味になっていることを確かめてから採用する。
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
    // `old` は既定値のキーを含まないので、`env: {}` のように既定値で書かれたキーは
    // `apply` の削除対象にならない。type を変えた後に残ると無効なファイルになるので、
    // 新しい type が受け付けないキーはここで除く。
    // type が変わらないなら触らない。値の無いキー (`cwd:`) は別の type のものでも有効で、
    // 無関係な編集で消してはいけない。
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

/// 現在 `old` を意味している `mapping` を、`new` を意味するように書き換える。
///
/// キーはテキストではなく意味で比べる。値が変わっていないキーには書き込まない。
/// 書き込むと、クォートの書き方が正規化されてしまうため。
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
        // 既定値のキーがファイルに書かれていることがある (`pin: false`)。
        // その場合は今ある位置で値を変える。
        if have.is_some() || mapping.contains_key(*key) {
            change(mapping, key, have, want)?;
            continue;
        }
        // 新しいキーは末尾ではなく、決まった順序で直前に来るキーの後ろに入れる。
        // 2 台のマシンが別々のキーを足したときに、変更行が重ならないようにするため。
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
            // 添字での編集は、ファイル上の並びが `have` と同じときだけ正しい。`os` は
            // ソートして読み込むので、手書きの並びとは食い違うことがある。
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
    // スカラーと、上の分岐で扱えない形で書かれたコレクション
    // (リストではなく `os: macos`) は、値ごと置き換える。
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
cwd: ~/workspace/ytyng-blog
env:
  # token for gh
  GITHUB_TOKEN: op://Private/GitHub/token
  FOO: \"bar\"   # quoted on purpose
";

    /// 不注意な書き出しをすると、別の型になったり YAML として壊れたりする文字列。
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
                cwd: Some("~/workspace/ytyng-blog".into()),
                env: BTreeMap::from([
                    ("GITHUB_TOKEN".to_string(), "op://Private/GitHub/token".to_string()),
                    ("FOO".to_string(), "bar".to_string()),
                ]),
            },
        }
    }

    /// `before` の行のうち、`after` に残っていないもの。
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
            tags: vec!["ytyng/コマンド".into()],
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
  - ytyng/コマンド
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

        // Assert: 末尾に足したリストは、その場編集のライブラリが字下げせずに書く (YAML としては同じ意味)。
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
        // Arrange: ユーザーが何も変えていないコマンドを、フォームから保存する。
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
        assert_eq!(
            changed_lines(HAND_WRITTEN, &updated.text),
            ["cwd: ~/workspace/ytyng-blog"]
        );
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
        // Arrange: `pin: false` は `pin` が無いのと同じ意味。
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
        // Arrange: 読み込んだ `os` はソート済みだが、ファイル上の並びは手書きのまま。
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
        // Arrange: `env: {}` と `args: []` は既定値なので、読み込んだコマンドには現れない。
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
        // Arrange: 値の無いキーは読み込み時に「無い」扱いなので、このファイルは有効。
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
        // Arrange: 元のテキストはコマンドとして無効なので、キー単位の編集ができない。
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
