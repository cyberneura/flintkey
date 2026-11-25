//! コマンドのモデルと、コマンドファイルが満たすべきルール。
//!
//! コマンドファイルはフラットな YAML で、共通キー (`title` `keyword` `hotkey` `pin` `tags` `when`) と
//! `type` ごとのキーを持つ。全フィールドを Option にした鏡の構造体で読み込む。
//! serde が最初に見つけた 1 件で止まらず、ルール違反をすべて行番号付きで報告するため。

use crate::diagnostic::Diagnostic;
use serde::Deserialize;
use serde::de::{self, SeqAccess, Visitor};
use serde_saphyr::Spanned;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub title: String,
    /// 完全一致で入力すると、このコマンドが最優先で選ばれる短い語。書かれた順。
    pub keywords: Vec<String>,
    /// このコマンドを直接実行するグローバルショートカット。書式はデスクトップアプリが検証する。
    pub hotkey: Option<String>,
    /// 検索欄が空のとき、ピン留めしたコマンドを先頭に並べる。
    pub pin: bool,
    /// 設定 UI で絞り込み・まとめるための分類。書かれた順。検索の対象ではない。
    pub tags: Vec<String>,
    /// このコマンドを出す OS。空ならすべての OS。
    pub os: BTreeSet<Os>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Url {
        url: String,
    },
    App {
        app: String,
        args: Vec<String>,
    },
    Script {
        run: String,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    },
    Terminal {
        run: String,
        cwd: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Os {
    Macos,
    Windows,
    Linux,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Macos, Os::Windows, Os::Linux];

    pub fn name(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }

    pub fn from_name(name: &str) -> Option<Os> {
        Os::ALL.into_iter().find(|os| os.name() == name)
    }

    /// このバイナリのビルド対象の OS。
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

impl Action {
    pub fn type_name(&self) -> &'static str {
        match self {
            Action::Url { .. } => "url",
            Action::App { .. } => "app",
            Action::Script { .. } => "script",
            Action::Terminal { .. } => "terminal",
        }
    }
}

impl Command {
    pub fn applies_to(&self, os: Os) -> bool {
        self.os.is_empty() || self.os.contains(&os)
    }
}

const TYPE_NAMES: [&str; 4] = ["url", "app", "script", "terminal"];

/// 特定の `type` だけが使うキーと、それを受け付ける type。
pub(crate) const TYPE_KEYS: [(&str, &[&str]); 6] = [
    ("url", &["url"]),
    ("app", &["app"]),
    ("args", &["app"]),
    ("run", &["script", "terminal"]),
    ("cwd", &["script", "terminal"]),
    ("env", &["script"]),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    title: Option<Spanned<String>>,
    keyword: Option<Spanned<OneOrMany>>,
    hotkey: Option<Spanned<String>>,
    tags: Option<Spanned<OneOrMany>>,
    #[serde(rename = "type")]
    kind: Option<Spanned<String>>,
    url: Option<Spanned<String>>,
    app: Option<Spanned<String>>,
    args: Option<Spanned<Vec<String>>>,
    run: Option<Spanned<String>>,
    cwd: Option<Spanned<String>>,
    env: Option<Spanned<BTreeMap<String, String>>>,
    pin: Option<Spanned<Flag>>,
    when: Option<RawWhen>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWhen {
    os: Option<Spanned<OneOrMany>>,
}

/// `os: macos` と `os: [macos, windows]` の両方を受け付ける。
///
/// 1 つか複数かは値の形を見て決める。値の形で決める読み方では、YAML 1.1 の流儀だと `on` や `y` が
/// 真偽値になってしまうので、[`parse_command`] は YAML 1.2 の真偽値 (`true` / `false`) だけを
/// 真偽値として扱う設定で読む。それでも `1.0` `+1` `true` は数や真偽値として届き、書かれた文字が
/// 失われる (`1.0` は `1` になる)。その場合は `typed` を立て、呼び出し側がファイルの文字を取り直す。
struct OneOrMany {
    names: Vec<String>,
    typed: bool,
}

impl OneOrMany {
    fn typed(value: impl ToString) -> Self {
        Self {
            names: vec![value.to_string()],
            typed: true,
        }
    }
}

/// 値の形を見て読んだために失われた文字を、ファイルのテキストから取り直す。
fn as_written(value: Spanned<OneOrMany>, source: &str) -> Spanned<OneOrMany> {
    if !value.value.typed {
        return value;
    }
    let span = value.referenced.span();
    let written = span.byte_offset().zip(span.byte_len()).and_then(|(start, len)| {
        let start = usize::try_from(start).ok()?;
        let end = start.checked_add(usize::try_from(len).ok()?)?;
        source.get(start..end)
    });
    match written.map(str::trim) {
        Some(text) if !text.is_empty() => Spanned {
            value: OneOrMany {
                names: vec![text.to_string()],
                typed: false,
            },
            ..value
        },
        _ => value,
    }
}

impl<'de> Deserialize<'de> for OneOrMany {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OneOrManyVisitor;

        impl<'de> Visitor<'de> for OneOrManyVisitor {
            type Value = OneOrMany;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a name or a list of names")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<OneOrMany, E> {
                Ok(OneOrMany {
                    names: vec![value.to_string()],
                    typed: false,
                })
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::typed(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::typed(value))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::typed(value))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<OneOrMany, E> {
                Ok(OneOrMany::typed(value))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<OneOrMany, A::Error> {
                let mut names = Vec::new();
                while let Some(name) = seq.next_element::<String>()? {
                    names.push(name);
                }
                Ok(OneOrMany { names, typed: false })
            }
        }

        deserializer.deserialize_any(OneOrManyVisitor)
    }
}

/// 真偽値。手書きする人のために `yes` / `no` / `on` / `off` も受け付ける。
///
/// 読み込みを YAML 1.2 の真偽値だけにしているので、それ以外の綴りは文字列として届く。
/// 引用符で囲んだ `"true"` も文字列として届く。
struct Flag(bool);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FlagVisitor;

        impl Visitor<'_> for FlagVisitor {
            type Value = Flag;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("true or false")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Flag, E> {
                Ok(Flag(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Flag, E> {
                match value.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "y" | "on" => Ok(Flag(true)),
                    "false" | "no" | "n" | "off" => Ok(Flag(false)),
                    _ => Err(E::invalid_value(de::Unexpected::Str(value), &self)),
                }
            }
        }

        deserializer.deserialize_any(FlagVisitor)
    }
}

fn position<T>(spanned: &Spanned<T>) -> (u64, u64) {
    (spanned.referenced.line(), spanned.referenced.column())
}

fn problem_at<T>(spanned: &Spanned<T>, message: impl Into<String>) -> Diagnostic {
    let (line, column) = position(spanned);
    Diagnostic::at(line, column, message)
}

/// 文字列の値を取り出す。空白だけの場合は問題として報告する。
fn non_blank(key: &str, value: Option<Spanned<String>>, problems: &mut Vec<Diagnostic>) -> Option<String> {
    let value = value?;
    if value.value.trim().is_empty() {
        problems.push(problem_at(&value, format!("`{key}` must not be empty")));
        return None;
    }
    Some(value.value)
}

/// 文字列のリストを取り出す。空白だけの値と重複は問題として報告する。
fn non_blank_list(key: &str, value: Option<Spanned<OneOrMany>>, problems: &mut Vec<Diagnostic>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for item in &value.value.names {
        if item.trim().is_empty() {
            problems.push(problem_at(&value, format!("`{key}` must not be empty")));
        } else if out.contains(item) {
            problems.push(problem_at(&value, format!("`{key}` lists `{item}` twice")));
        } else {
            out.push(item.clone());
        }
    }
    out
}

pub(crate) fn has_url_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// コマンドファイルを 1 つ読む。最初の 1 件だけでなく、見つかったルール違反をすべて返す。
pub fn parse_command(source: &str) -> Result<Command, Vec<Diagnostic>> {
    let mut options = serde_saphyr::Options::default();
    options.strict_booleans = true;
    let raw: RawCommand = match serde_saphyr::from_str_with_options::<Option<RawCommand>>(source, options) {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            return Err(vec![Diagnostic::file(
                "the file is empty; a command needs at least `title` and `type`",
            )]);
        }
        Err(error) => return Err(vec![Diagnostic::from_yaml_error(&error)]),
    };
    let mut problems = Vec::new();

    if raw.title.is_none() {
        problems.push(Diagnostic::file("`title` is required"));
    }
    let title = non_blank("title", raw.title, &mut problems);
    let keywords = non_blank_list(
        "keyword",
        raw.keyword.map(|value| as_written(value, source)),
        &mut problems,
    );
    let hotkey = non_blank("hotkey", raw.hotkey, &mut problems);
    let tags = non_blank_list("tags", raw.tags.map(|value| as_written(value, source)), &mut problems);

    let kind = match &raw.kind {
        None => {
            problems.push(Diagnostic::file(format!(
                "`type` is required (one of: {})",
                TYPE_NAMES.join(", ")
            )));
            None
        }
        Some(kind) if !TYPE_NAMES.contains(&kind.value.as_str()) => {
            problems.push(problem_at(
                kind,
                format!(
                    "unknown type `{}` (expected one of: {})",
                    kind.value,
                    TYPE_NAMES.join(", ")
                ),
            ));
            None
        }
        Some(kind) => Some(kind.value.as_str()),
    };

    if let Some(kind) = kind {
        let present: [(&str, Option<(u64, u64)>); 6] = [
            ("url", raw.url.as_ref().map(position)),
            ("app", raw.app.as_ref().map(position)),
            ("args", raw.args.as_ref().map(position)),
            ("run", raw.run.as_ref().map(position)),
            ("cwd", raw.cwd.as_ref().map(position)),
            ("env", raw.env.as_ref().map(position)),
        ];
        for (key, at) in present {
            let Some((line, column)) = at else { continue };
            let accepted_by = TYPE_KEYS
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, types)| *types)
                .unwrap_or(&[]);
            if !accepted_by.contains(&kind) {
                problems.push(Diagnostic::at(
                    line,
                    column,
                    format!("`{key}` is not used by type `{kind}`"),
                ));
            }
        }
    }

    let kind_position = raw.kind.as_ref().map(position);
    let required = |key: &str, value: Option<Spanned<String>>, problems: &mut Vec<Diagnostic>| {
        if value.is_none() {
            let message = format!("type `{}` needs `{key}`", kind.unwrap_or_default());
            problems.push(match kind_position {
                Some((line, column)) => Diagnostic::at(line, column, message),
                None => Diagnostic::file(message),
            });
        }
        non_blank(key, value, problems)
    };

    let action = match kind {
        Some("url") => {
            let url_position = raw.url.as_ref().map(position);
            required("url", raw.url, &mut problems).and_then(|url| {
                if has_url_scheme(&url) {
                    return Some(Action::Url { url });
                }
                let (line, column) = url_position.unwrap_or((1, 1));
                problems.push(Diagnostic::at(
                    line,
                    column,
                    format!("`url` needs a scheme such as https:// (got `{url}`)"),
                ));
                None
            })
        }
        Some("app") => required("app", raw.app, &mut problems).map(|app| Action::App {
            app,
            args: raw.args.map(|args| args.value).unwrap_or_default(),
        }),
        Some("script") => {
            let env = raw.env.map(|env| {
                for name in env.value.keys() {
                    if name.is_empty() || name.contains('=') {
                        problems.push(problem_at(
                            &env,
                            format!("`{name}` is not a valid environment variable name"),
                        ));
                    }
                }
                env.value
            });
            let run = required("run", raw.run, &mut problems);
            let cwd = non_blank("cwd", raw.cwd, &mut problems);
            run.map(|run| Action::Script {
                run,
                cwd,
                env: env.unwrap_or_default(),
            })
        }
        Some("terminal") => {
            let run = required("run", raw.run, &mut problems);
            let cwd = non_blank("cwd", raw.cwd, &mut problems);
            run.map(|run| Action::Terminal { run, cwd })
        }
        _ => None,
    };

    let mut os = BTreeSet::new();
    if let Some(names) = raw.when.and_then(|when| when.os) {
        for name in &names.value.names {
            match Os::from_name(name) {
                Some(found) => {
                    os.insert(found);
                }
                None => problems.push(problem_at(
                    &names,
                    format!("unknown os `{name}` (expected one of: macos, windows, linux)"),
                )),
            }
        }
    }

    match (title, action) {
        (Some(title), Some(action)) if problems.is_empty() => Ok(Command {
            title,
            keywords,
            hotkey,
            pin: raw.pin.is_some_and(|pin| pin.value.0),
            tags,
            os,
            action,
        }),
        _ => {
            problems.sort_by_key(|problem| (problem.line.unwrap_or(0), problem.column.unwrap_or(0)));
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(source: &str) -> Vec<String> {
        parse_command(source)
            .expect_err("expected problems")
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn script_command_with_every_field_is_read() {
        // Arrange
        let source = "\
# Deploy the blog
title: Deploy blog   # shown in the list
keyword: [deploy, blog]
hotkey: ctrl+alt+cmd+d
type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  GITHUB_TOKEN: op://Private/GitHub/token
  STAGE: prod
pin: true
tags: [ytyng, 公開]
when:
  os: [windows, macos]
";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert_eq!(
            command,
            Command {
                title: "Deploy blog".into(),
                keywords: vec!["deploy".into(), "blog".into()],
                hotkey: Some("ctrl+alt+cmd+d".into()),
                pin: true,
                tags: vec!["ytyng".into(), "公開".into()],
                os: [Os::Macos, Os::Windows].into(),
                action: Action::Script {
                    run: "fab deploy".into(),
                    cwd: Some("~/workspace/blog".into()),
                    env: BTreeMap::from([
                        ("GITHUB_TOKEN".to_string(), "op://Private/GitHub/token".to_string()),
                        ("STAGE".to_string(), "prod".to_string()),
                    ]),
                },
            }
        );
    }

    #[test]
    fn minimal_commands_of_each_type_are_read() {
        // Arrange
        let cases = [
            (
                "title: A\ntype: url\nurl: https://example.com/?a=1&b=2\n",
                Action::Url {
                    url: "https://example.com/?a=1&b=2".into(),
                },
            ),
            (
                "title: A\ntype: url\nurl: claude://code/new\n",
                Action::Url {
                    url: "claude://code/new".into(),
                },
            ),
            (
                "title: A\ntype: app\napp: Slack\n",
                Action::App {
                    app: "Slack".into(),
                    args: vec![],
                },
            ),
            (
                "title: A\ntype: app\napp: /Applications/Foo.app\nargs: [--flag, value]\n",
                Action::App {
                    app: "/Applications/Foo.app".into(),
                    args: vec!["--flag".into(), "value".into()],
                },
            ),
            (
                "title: A\ntype: terminal\nrun: ssh host\n",
                Action::Terminal {
                    run: "ssh host".into(),
                    cwd: None,
                },
            ),
        ];

        for (source, expected) in cases {
            // Act
            let command = parse_command(source).unwrap();

            // Assert
            assert_eq!(command.action, expected, "source: {source}");
            assert!(!command.pin);
            assert!(command.os.is_empty());
            assert!(command.keywords.is_empty());
            assert_eq!(command.hotkey, None);
            assert!(command.tags.is_empty());
        }
    }

    #[test]
    fn single_os_name_is_accepted_without_a_list() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\nwhen:\n  os: macos\n";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert_eq!(command.os, [Os::Macos].into());
        assert!(command.applies_to(Os::Macos));
        assert!(!command.applies_to(Os::Windows));
    }

    #[test]
    fn command_without_when_applies_everywhere() {
        // Arrange
        let command = parse_command("title: A\ntype: url\nurl: https://example.com\n").unwrap();

        // Act + Assert
        assert!(Os::ALL.into_iter().all(|os| command.applies_to(os)));
    }

    #[test]
    fn words_that_yaml_1_1_treats_as_booleans_stay_text() {
        // Arrange: 手書きする人が文字列のつもりで書く語。
        let source = "title: no\nkeyword: on\ntype: script\nrun: yes\n";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert_eq!(command.title, "no");
        assert_eq!(command.keywords, ["on"]);
        assert_eq!(
            command.action,
            Action::Script {
                run: "yes".into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
    }

    #[test]
    fn keyword_aliases_and_tags_keep_their_order_and_stay_text() {
        // Arrange: 値の形で読むキーでも、YAML 1.1 で真偽値や数になる語は文字列のまま。
        let source = "title: A\nkeyword: [today, on, y, 1, 'no']\ntype: url\nurl: https://example.com\ntags: off\n";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert_eq!(command.keywords, ["today", "on", "y", "1", "no"]);
        assert_eq!(command.tags, ["off"]);
    }

    #[test]
    fn single_words_that_yaml_1_1_treats_as_booleans_or_numbers_stay_text() {
        for word in [
            "on", "off", "y", "n", "yes", "true", "False", "1", "1.5", "1.0", "+1", "007", "0x1F", "1e3",
        ] {
            // Arrange
            let source = format!("title: A\nkeyword: {word}\ntype: url\nurl: https://example.com\ntags: {word}\n");

            // Act
            let command = parse_command(&source).unwrap();

            // Assert
            assert_eq!(command.keywords, [word], "keyword {word}");
            assert_eq!(command.tags, [word], "tag {word}");
        }
    }

    #[test]
    fn pin_accepts_the_yes_and_no_spellings() {
        for (word, expected) in [
            ("true", true),
            ("\"true\"", true),
            ("Yes", true),
            ("on", true),
            ("y", true),
            ("false", false),
            ("'false'", false),
            ("no", false),
            ("OFF", false),
        ] {
            // Arrange
            let source = format!("title: A\ntype: url\nurl: https://example.com\npin: {word}\n");

            // Act
            let command = parse_command(&source).unwrap();

            // Assert
            assert_eq!(command.pin, expected, "pin: {word}");
        }
    }

    #[test]
    fn blank_and_repeated_keywords_and_tags_are_rejected() {
        // Arrange
        let source = "title: A\nkeyword: [a, '', a]\nhotkey: ' '\ntype: url\nurl: https://example.com\ntags: [x, x]\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            [
                "line 2, column 10: `keyword` must not be empty",
                "line 2, column 10: `keyword` lists `a` twice",
                "line 3, column 9: `hotkey` must not be empty",
                "line 6, column 7: `tags` lists `x` twice",
            ]
        );
    }

    #[test]
    fn hotkey_is_kept_as_written() {
        // Arrange
        let source = "title: A\nhotkey: Ctrl+Alt+Cmd+C\ntype: app\napp: Slack\n";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert_eq!(command.hotkey.as_deref(), Some("Ctrl+Alt+Cmd+C"));
    }

    #[test]
    fn missing_title_and_type_are_both_reported() {
        // Arrange
        let source = "keyword: x\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            [
                "`title` is required",
                "`type` is required (one of: url, app, script, terminal)"
            ]
        );
    }

    #[test]
    fn unknown_type_points_at_its_line() {
        // Arrange
        let source = "title: A\ntype: scirpt\nrun: x\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            ["line 2, column 7: unknown type `scirpt` (expected one of: url, app, script, terminal)"]
        );
    }

    #[test]
    fn missing_key_of_the_type_points_at_the_type_line() {
        // Arrange
        let source = "title: A\n\ntype: script\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(found, ["line 3, column 7: type `script` needs `run`"]);
    }

    #[test]
    fn key_of_another_type_points_at_that_key() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\ncwd: ~/x\nenv:\n  A: b\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            [
                "line 4, column 6: `cwd` is not used by type `url`",
                "line 6, column 3: `env` is not used by type `url`"
            ]
        );
    }

    #[test]
    fn url_without_scheme_is_rejected() {
        // Arrange: スキームが無いと、OS はファイルパスとして扱う。
        let source = "title: A\ntype: url\nurl: example.com/page\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            ["line 3, column 6: `url` needs a scheme such as https:// (got `example.com/page`)"]
        );
    }

    #[test]
    fn blank_values_are_rejected() {
        // Arrange
        let source = "title: '  '\nkeyword: ''\ntype: script\nrun: ''\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            [
                "line 1, column 8: `title` must not be empty",
                "line 2, column 10: `keyword` must not be empty",
                "line 4, column 6: `run` must not be empty",
            ]
        );
    }

    #[test]
    fn blank_working_directory_is_reported_even_when_run_is_missing() {
        for kind in ["script", "terminal"] {
            // Arrange
            let source = format!("title: A\ntype: {kind}\ncwd: ''\n");

            // Act
            let found = messages(&source);

            // Assert
            assert_eq!(
                found,
                [
                    format!("line 2, column 7: type `{kind}` needs `run`"),
                    "line 3, column 6: `cwd` must not be empty".to_string()
                ]
            );
        }
    }

    #[test]
    fn unknown_key_is_rejected_with_its_line() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\nkeywrod: x\n";

        // Act
        let problems = parse_command(source).unwrap_err();

        // Assert
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, Some(4));
        assert!(problems[0].message.contains("keywrod"), "{}", problems[0].message);
    }

    #[test]
    fn wrong_value_type_is_rejected_with_its_line() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\npin: maybe\n";

        // Act
        let problems = parse_command(source).unwrap_err();

        // Assert
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].line, Some(4));
    }

    #[test]
    fn yes_and_no_are_booleans_only_where_a_boolean_is_expected() {
        // Arrange: 読み込み側は受け取る型で解釈を決めるので、`yes` は `pin` では真偽値、
        // `title` では文字列になる。
        let source = "title: yes\ntype: url\nurl: https://example.com\npin: yes\n";

        // Act
        let command = parse_command(source).unwrap();

        // Assert
        assert!(command.pin);
        assert_eq!(command.title, "yes");
    }

    #[test]
    fn unknown_os_is_rejected() {
        // Arrange
        let source = "title: A\ntype: url\nurl: https://example.com\nwhen:\n  os: [macos, beos]\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            ["line 5, column 7: unknown os `beos` (expected one of: macos, windows, linux)"]
        );
    }

    #[test]
    fn invalid_environment_variable_name_is_rejected() {
        // Arrange
        let source = "title: A\ntype: script\nrun: x\nenv:\n  A=B: c\n";

        // Act
        let found = messages(source);

        // Assert
        assert_eq!(
            found,
            ["line 5, column 3: `A=B` is not a valid environment variable name"]
        );
    }

    #[test]
    fn empty_and_comment_only_files_are_reported_as_empty() {
        for source in ["", "\n", "# only a comment\n"] {
            // Act
            let found = messages(source);

            // Assert
            assert_eq!(
                found,
                ["the file is empty; a command needs at least `title` and `type`"],
                "source: {source:?}"
            );
        }
    }

    #[test]
    fn broken_yaml_is_reported_with_a_line() {
        // Arrange
        let source = "title: A\ntype: [url\n";

        // Act
        let problems = parse_command(source).unwrap_err();

        // Assert
        assert_eq!(problems.len(), 1);
        assert!(problems[0].line.is_some());
    }
}
