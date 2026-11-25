//! ファイルの置き換え。

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// ファイルを 1 回の操作で置き換える。クラッシュしても、同時に読まれても、
/// 書きかけの内容が見えない。
pub(crate) fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().expect("a file path has a parent");
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().expect("a file path has a name").to_string_lossy();
    // 先頭のドットは、一時ファイルが残ってしまった場合にコマンド一覧に出さないため。
    // 連番は、同じファイルへの同時保存が一時ファイルを奪い合わないようにするため。
    let temporary = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    // 既にあるファイルやシンボリックリンクの先を書き潰さないよう、新規作成に限る。
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let replaced = file
        .write_all(text.as_bytes())
        .and_then(|()| std::fs::rename(&temporary, path));
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn file_is_created_with_its_missing_folders_and_replaced_without_leftovers() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.yaml");

        // Act
        write_atomically(&path, "first\n").unwrap();
        write_atomically(&path, "second\n").unwrap();

        // Assert
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
        assert_eq!(names_in(path.parent().unwrap()), ["file.yaml"]);
    }

    #[test]
    fn concurrent_writers_never_leave_a_mixed_or_partial_file() {
        // Arrange: 長さの違う内容を書き合う。混ざったり切れたりすれば、どちらとも一致しない。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.yaml");
        let contents: Vec<String> = (0..8)
            .map(|writer| format!("{writer}").repeat(1000 * (writer + 1)))
            .collect();

        // Act
        std::thread::scope(|scope| {
            for content in &contents {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..50 {
                        write_atomically(path, content).unwrap();
                    }
                });
            }
        });

        // Assert
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains(&written),
            "file holds {} bytes of mixed content",
            written.len()
        );
        assert_eq!(names_in(dir.path()), ["file.yaml"]);
    }
}
