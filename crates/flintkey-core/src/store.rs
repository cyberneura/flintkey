//! ディスク上の設定フォルダ。
//!
//! ```text
//! ~/.config/flintkey/
//! ├── settings.yaml
//! └── commands/
//!     ├── open-github.yaml     1 コマンド 1 ファイル。ファイル名が id
//!     └── deploy-blog.yaml
//! ```
//!
//! 1 コマンド 1 ファイルなので、2 台のマシンが別々にコマンドを足しても同じファイルに
//! 触れない。フォルダを Git でマージしても、解決するものが無い。

use crate::atomic::write_atomically;
use crate::command::{Command, parse_command};
use crate::diagnostic::Diagnostic;
use crate::document;
use crate::settings::{Settings, parse_settings};
use std::io;
use std::path::{Path, PathBuf};

const COMMANDS_DIR: &str = "commands";
const SETTINGS_FILE: &str = "settings.yaml";
const EXTENSION: &str = "yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCommand {
    /// `.yaml` を除いたファイル名。
    pub id: String,
    pub command: Command,
}

/// 使えなかったコマンドファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenCommand {
    pub id: String,
    pub path: PathBuf,
    pub problems: Vec<Diagnostic>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LoadedCommands {
    /// id 順。
    pub commands: Vec<StoredCommand>,
    /// id 順。壊れたファイルがあっても、有効なファイルは隠れない。
    pub broken: Vec<BrokenCommand>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("`{0}` cannot be used as a command id")]
    InvalidId(String),
    #[error("the command is not valid: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<Diagnostic>),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// コマンドの保存がファイルに対して行ったこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// ファイルがすでに同じ意味だったので、書き込まなかった。
    Unchanged,
    Created,
    /// 変わったキーだけを書き換えた。コメントと書式は元のまま。
    Edited,
    /// 決まった書式でファイル全体を書き直した。コメントは失われる。
    Rewritten,
}

pub struct Store {
    root: PathBuf,
}

/// id はファイル名になるので、無害なパス要素ちょうど 1 つでなければならない。
fn validate_id(id: &str) -> Result<(), StoreError> {
    let harmless = !id.is_empty()
        && !id.starts_with('.')
        && !id.contains(['/', '\\', ':', '\0'])
        && Path::new(id).components().count() == 1;
    if harmless {
        Ok(())
    } else {
        Err(StoreError::InvalidId(id.to_string()))
    }
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn command_path(&self, id: &str) -> Result<PathBuf, StoreError> {
        validate_id(id)?;
        Ok(self.root.join(COMMANDS_DIR).join(format!("{id}.{EXTENSION}")))
    }

    /// `commands/*.yaml` をすべて読む。フォルダが無ければ空の設定として扱う。
    pub fn load_commands(&self) -> io::Result<LoadedCommands> {
        let entries = match std::fs::read_dir(self.root.join(COMMANDS_DIR)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(LoadedCommands::default()),
            Err(error) => return Err(error),
        };
        let mut loaded = LoadedCommands::default();
        for entry in entries {
            let path = entry?.path();
            let is_command_file = path.extension().is_some_and(|extension| extension == EXTENSION) && path.is_file();
            let Some(id) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
                continue;
            };
            if !is_command_file || id.starts_with('.') {
                continue;
            }
            let parsed = match std::fs::read_to_string(&path) {
                Ok(source) => parse_command(&source),
                Err(error) => Err(vec![Diagnostic::file(format!("could not read the file: {error}"))]),
            };
            match parsed {
                Ok(command) => loaded.commands.push(StoredCommand { id, command }),
                Err(problems) => loaded.broken.push(BrokenCommand { id, path, problems }),
            }
        }
        loaded.commands.sort_by(|a, b| a.id.cmp(&b.id));
        loaded.broken.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(loaded)
    }

    /// コマンドファイルの生のテキスト。設定 UI の YAML 表示用。
    pub fn read_command_source(&self, id: &str) -> Result<String, StoreError> {
        Ok(std::fs::read_to_string(self.command_path(id)?)?)
    }

    /// フォームで編集したコマンドを保存する。既存ファイルは、変更に関係しない部分を
    /// すべてそのまま残す。
    pub fn save_command(&self, id: &str, command: &Command) -> Result<Saved, StoreError> {
        let path = self.command_path(id)?;
        let (text, saved) = match std::fs::read_to_string(&path) {
            Ok(source) => {
                let updated = document::update(&source, command);
                let saved = if updated.text == source {
                    Saved::Unchanged
                } else if updated.kept_formatting {
                    Saved::Edited
                } else {
                    Saved::Rewritten
                };
                (updated.text, saved)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (document::render(command), Saved::Created),
            Err(error) => return Err(error.into()),
        };
        // `Command` は空のタイトルのような無効な値でも組み立てられる。そのまま書くと、
        // 次の読み込みでコマンドが一覧から消える。書く前に、読み戻せることを確かめる。
        // テキストが変わらない場合も確かめる。無効なコマンドの書き直しが、すでに壊れている
        // ファイルと同じテキストになることがあるため。
        match parse_command(&text) {
            Ok(written) if written == *command => {}
            Ok(_) => {
                return Err(StoreError::Invalid(vec![Diagnostic::file(
                    "the command would not read back unchanged from its file",
                )]));
            }
            Err(problems) => return Err(StoreError::Invalid(problems)),
        }
        if saved != Saved::Unchanged {
            write_atomically(&path, &text)?;
        }
        Ok(saved)
    }

    /// 生の YAML として編集したコマンドを保存する。有効なコマンドでないテキストは拒否する。
    /// 入力ミスで、動いていたコマンドがランチャーから消えるのを防ぐため。
    pub fn save_command_source(&self, id: &str, source: &str) -> Result<Command, StoreError> {
        let path = self.command_path(id)?;
        let command = parse_command(source).map_err(StoreError::Invalid)?;
        write_atomically(&path, source)?;
        Ok(command)
    }

    /// コマンドファイルを削除する。存在しないファイルの削除はエラーにしない。
    pub fn delete_command(&self, id: &str) -> Result<(), StoreError> {
        match std::fs::remove_file(self.command_path(id)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// `settings.yaml` を読む。ファイルが無ければ既定値。
    pub fn load_settings(&self) -> io::Result<Result<Settings, Vec<Diagnostic>>> {
        match std::fs::read_to_string(self.root.join(SETTINGS_FILE)) {
            Ok(source) => Ok(parse_settings(&source)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Ok(Settings::default())),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Action;
    use crate::settings::Display;

    fn url_command(title: &str, url: &str) -> Command {
        Command {
            title: title.into(),
            keyword: None,
            pin: false,
            os: [].into(),
            action: Action::Url { url: url.into() },
        }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        (dir, store)
    }

    fn write(store: &Store, relative: &str, text: &str) {
        let path = store.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn files_in_commands(store: &Store) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.root().join("commands"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn missing_folder_is_an_empty_configuration() {
        // Arrange
        let (_dir, store) = store();

        // Act
        let loaded = store.load_commands().unwrap();
        let settings = store.load_settings().unwrap();

        // Assert
        assert_eq!(loaded, LoadedCommands::default());
        assert_eq!(settings, Ok(Settings::default()));
    }

    #[test]
    fn commands_are_loaded_sorted_and_a_broken_file_does_not_hide_the_others() {
        // Arrange
        let (_dir, store) = store();
        write(
            &store,
            "commands/zeta.yaml",
            "title: Z\ntype: url\nurl: https://z.example\n",
        );
        write(
            &store,
            "commands/alpha.yaml",
            "title: A\ntype: url\nurl: https://a.example\n",
        );
        write(&store, "commands/broken.yaml", "title: B\ntype: nope\n");

        // Act
        let loaded = store.load_commands().unwrap();

        // Assert
        assert_eq!(
            loaded.commands,
            [
                StoredCommand {
                    id: "alpha".into(),
                    command: url_command("A", "https://a.example")
                },
                StoredCommand {
                    id: "zeta".into(),
                    command: url_command("Z", "https://z.example")
                },
            ]
        );
        assert_eq!(loaded.broken.len(), 1);
        assert_eq!(loaded.broken[0].id, "broken");
        assert_eq!(loaded.broken[0].path, store.root().join("commands/broken.yaml"));
        assert_eq!(loaded.broken[0].problems[0].line, Some(2));
    }

    #[test]
    fn files_that_are_not_commands_are_ignored() {
        // Arrange
        let (_dir, store) = store();
        write(
            &store,
            "commands/real.yaml",
            "title: R\ntype: url\nurl: https://r.example\n",
        );
        write(&store, "commands/README.md", "notes");
        write(&store, "commands/old.yml", "title: O\ntype: nope\n");
        write(&store, "commands/.hidden.yaml", "title: H\ntype: nope\n");
        write(&store, "commands/folder.yaml/inner.yaml", "title: I\ntype: nope\n");

        // Act
        let loaded = store.load_commands().unwrap();

        // Assert
        assert_eq!(
            loaded.commands.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["real"]
        );
        assert!(loaded.broken.is_empty(), "{:?}", loaded.broken);
    }

    #[test]
    fn file_that_is_not_text_is_reported_as_broken() {
        // Arrange
        let (_dir, store) = store();
        std::fs::create_dir_all(store.root().join("commands")).unwrap();
        std::fs::write(store.root().join("commands/binary.yaml"), [0xff, 0xfe, 0x00]).unwrap();

        // Act
        let loaded = store.load_commands().unwrap();

        // Assert
        assert_eq!(loaded.broken.len(), 1);
        assert!(
            loaded.broken[0].problems[0]
                .message
                .starts_with("could not read the file")
        );
    }

    #[test]
    fn new_command_is_created_and_loads_back() {
        // Arrange
        let (_dir, store) = store();
        let command = url_command("GitHub", "https://github.com");

        // Act
        let saved = store.save_command("open-github", &command).unwrap();

        // Assert
        assert_eq!(saved, Saved::Created);
        assert_eq!(
            store.read_command_source("open-github").unwrap(),
            "title: GitHub\ntype: url\nurl: https://github.com\n"
        );
        assert_eq!(
            store.load_commands().unwrap().commands,
            [StoredCommand {
                id: "open-github".into(),
                command
            }]
        );
        assert_eq!(files_in_commands(&store), ["open-github.yaml"]);
    }

    #[test]
    fn saving_an_unchanged_command_does_not_write_the_file() {
        // Arrange
        let (_dir, store) = store();
        let source = "# mine\ntitle: \"GitHub\"\ntype: url\nurl: https://github.com\n";
        write(&store, "commands/gh.yaml", source);
        let path = store.command_path("gh").unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        // Act
        let saved = store
            .save_command("gh", &url_command("GitHub", "https://github.com"))
            .unwrap();

        // Assert
        assert_eq!(saved, Saved::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn saving_a_changed_command_edits_the_file_in_place() {
        // Arrange
        let (_dir, store) = store();
        write(
            &store,
            "commands/gh.yaml",
            "# mine\ntitle: \"GitHub\"\ntype: url\nurl: https://github.com   # home\n",
        );

        // Act
        let saved = store
            .save_command("gh", &url_command("GitHub", "https://github.com/pulls"))
            .unwrap();

        // Assert
        assert_eq!(saved, Saved::Edited);
        assert_eq!(
            store.read_command_source("gh").unwrap(),
            "# mine\ntitle: \"GitHub\"\ntype: url\nurl: https://github.com/pulls   # home\n"
        );
        assert_eq!(files_in_commands(&store), ["gh.yaml"]);
    }

    #[test]
    fn saving_over_a_broken_file_rewrites_it() {
        // Arrange
        let (_dir, store) = store();
        write(&store, "commands/gh.yaml", "title: GitHub\ntype: nope\n");

        // Act
        let saved = store
            .save_command("gh", &url_command("GitHub", "https://github.com"))
            .unwrap();

        // Assert
        assert_eq!(saved, Saved::Rewritten);
        assert!(store.load_commands().unwrap().broken.is_empty());
    }

    #[test]
    fn command_that_would_not_load_again_is_refused_and_nothing_is_written() {
        // Arrange: フィールドは公開なので、検証を通らない値でも組み立てられる。
        let (_dir, store) = store();
        let original = "# mine\ntitle: GitHub\ntype: url\nurl: https://github.com\n";
        write(&store, "commands/existing.yaml", original);
        write(&store, "commands/broken.yaml", "title: GitHub\ntype: nope\n");
        let invalid = [
            url_command("", "https://github.com"),
            url_command("GitHub", "github.com"),
        ];

        for command in &invalid {
            for id in ["new", "existing", "broken"] {
                // Act
                let saved = store.save_command(id, command);

                // Assert
                assert!(
                    matches!(saved, Err(StoreError::Invalid(_))),
                    "saved {command:?} as {id}: {saved:?}"
                );
            }
        }
        assert_eq!(store.read_command_source("existing").unwrap(), original);
        assert_eq!(
            store.read_command_source("broken").unwrap(),
            "title: GitHub\ntype: nope\n"
        );
        assert_eq!(files_in_commands(&store), ["broken.yaml", "existing.yaml"]);
    }

    #[test]
    fn saving_an_invalid_command_over_identical_invalid_text_is_still_refused() {
        // Arrange: 書き直した結果が元のテキストと同じになるので、「変更なし」に見える。
        let (_dir, store) = store();
        let source = "title: GitHub\ntype: url\nurl: github.com\n";
        write(&store, "commands/gh.yaml", source);

        // Act
        let saved = store.save_command("gh", &url_command("GitHub", "github.com"));

        // Assert
        assert!(matches!(saved, Err(StoreError::Invalid(_))), "{saved:?}");
    }

    #[test]
    fn raw_yaml_is_saved_verbatim_when_valid_and_refused_when_not() {
        // Arrange
        let (_dir, store) = store();
        let valid = "# hand written\ntitle:   Spaced\ntype: url\nurl: https://example.com\n";
        let invalid = "title: Spaced\ntype: url\n";

        // Act
        let accepted = store.save_command_source("raw", valid).unwrap();
        let refused = store.save_command_source("raw", invalid);

        // Assert
        assert_eq!(accepted, url_command("Spaced", "https://example.com"));
        let Err(StoreError::Invalid(problems)) = refused else {
            panic!("expected a refusal")
        };
        assert_eq!(problems[0].to_string(), "line 2, column 7: type `url` needs `url`");
        assert_eq!(store.read_command_source("raw").unwrap(), valid);
    }

    #[test]
    fn deleting_removes_the_file_and_is_harmless_when_repeated() {
        // Arrange
        let (_dir, store) = store();
        store
            .save_command("gone", &url_command("X", "https://example.com"))
            .unwrap();

        // Act
        store.delete_command("gone").unwrap();
        store.delete_command("gone").unwrap();

        // Assert
        assert!(files_in_commands(&store).is_empty());
    }

    #[test]
    fn ids_that_would_escape_the_commands_folder_are_refused() {
        // Arrange
        let (_dir, store) = store();
        let command = url_command("X", "https://example.com");

        for id in [
            "",
            ".",
            "..",
            "../settings",
            "a/b",
            "a\\b",
            ".hidden",
            "C:evil",
            "nul\0byte",
        ] {
            // Act
            let saved = store.save_command(id, &command);
            let deleted = store.delete_command(id);

            // Assert
            assert!(matches!(saved, Err(StoreError::InvalidId(_))), "save accepted {id:?}");
            assert!(
                matches!(deleted, Err(StoreError::InvalidId(_))),
                "delete accepted {id:?}"
            );
        }
        assert!(!store.root().join("commands").exists());
    }

    #[test]
    fn ids_with_spaces_dots_and_japanese_are_accepted() {
        // Arrange
        let (_dir, store) = store();
        let command = url_command("X", "https://example.com");

        for id in ["open github", "v1.2", "ブログ", "a-b_c"] {
            // Act
            let saved = store.save_command(id, &command).unwrap();

            // Assert
            assert_eq!(saved, Saved::Created, "id {id:?}");
        }
        assert_eq!(store.load_commands().unwrap().commands.len(), 4);
    }

    #[test]
    fn settings_file_is_read_and_its_problems_are_returned() {
        // Arrange
        let (_dir, store) = store();
        write(&store, "settings.yaml", "window:\n  display: primary\n");

        // Act
        let good = store.load_settings().unwrap();
        write(&store, "settings.yaml", "window:\n  display: sideways\n");
        let bad = store.load_settings().unwrap();

        // Assert
        assert_eq!(good.unwrap().display, Display::Primary);
        assert_eq!(bad.unwrap_err()[0].line, Some(2));
    }
}
