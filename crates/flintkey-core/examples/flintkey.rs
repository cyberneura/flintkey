//! コアを使うコマンドライン。デスクトップアプリ無しで設定フォルダを試すためのもの。
//!
//! ```text
//! cargo run --example flintkey -- list [query]   順位付けしたコマンドと、壊れているファイル
//! cargo run --example flintkey -- plan <id>      実行すると何が起きるか
//! cargo run --example flintkey -- run <id>       実行する
//! cargo run --example flintkey -- import-alfred [--write] [--from <Alfred.alfredpreferences>]
//!                                                 Alfred のワークフローからコマンドを作る (macOS)
//! ```
//!
//! `import-alfred` は既定では何も書かず、作るファイルと変換できなかったものを一覧にする。
//! `--write` を付けると書く。同じ id のファイルがすでにあれば、中身が違っても上書きしない。
//!
//! `~/.config/flintkey` 以外のフォルダを使うには `FLINTKEY_CONFIG_DIR` を設定する。

use flintkey_core::Os;
use flintkey_core::alfred::{self, Workflow};
use flintkey_core::launch::{self, Machine};
use flintkey_core::parse_command;
use flintkey_core::paths;
use flintkey_core::search::Searcher;
use flintkey_core::secrets::OnePassword;
use flintkey_core::store::Store;
use flintkey_core::usage::{USAGE_FILE, Usage};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[&str]) -> Result<(), String> {
    let home = std::env::home_dir().ok_or("no home directory")?;
    let store = Store::new(paths::config_dir().ok_or("no home directory")?);
    let usage_path = paths::state_dir().ok_or("no home directory")?.join(USAGE_FILE);
    let loaded = store
        .load_commands()
        .map_err(|error| format!("{}: {error}", store.root().display()))?;
    let settings = store
        .load_settings()
        .map_err(|error| error.to_string())?
        .map_err(|problems| format!("settings.yaml: {}", problems[0]))?;

    match args {
        ["list", query @ ..] => {
            let active: Vec<_> = loaded
                .commands
                .into_iter()
                .filter(|c| c.command.applies_to(Os::current()))
                .collect();
            let order = Searcher::default().rank(&active, &Usage::load(&usage_path), &query.join(" "));
            for index in order {
                let stored = &active[index];
                println!(
                    "{:24} {:10} {}",
                    stored.id,
                    stored.command.action.type_name(),
                    stored.command.title
                );
            }
            for broken in loaded.broken {
                for problem in broken.problems {
                    println!("BROKEN {}: {problem}", broken.path.display());
                }
            }
            Ok(())
        }
        [verb @ ("plan" | "run"), id] => {
            let stored = loaded
                .commands
                .iter()
                .find(|c| c.id == *id)
                .ok_or(format!("no command `{id}`"))?;
            let shell = std::env::var("SHELL").ok();
            let path = std::env::var("PATH").ok();
            let machine = Machine {
                os: Os::current(),
                home: &home,
                shell: shell.as_deref(),
                path: path.as_deref(),
                terminal: settings.terminal,
            };
            let plan = launch::plan(&stored.command.action, &machine).map_err(|error| error.to_string())?;
            println!("{plan:#?}");
            if *verb == "run" {
                launch::execute(&plan, &OnePassword::locate()).map_err(|error| error.to_string())?;
                let mut usage = Usage::load(&usage_path);
                usage.record(id);
                // コマンドはもう起動している。ここで失敗を返すと、やり直しで二重に実行される。
                if let Err(error) = usage.save(&usage_path) {
                    eprintln!("warning: could not record the use: {error}");
                }
            }
            Ok(())
        }
        ["import-alfred", options @ ..] => import_alfred(&store, &home, options),
        _ => Err(USAGE.to_string()),
    }
}

const USAGE: &str =
    "usage: flintkey list [query] | plan <id> | run <id> | import-alfred [--write] [--from <Alfred.alfredpreferences>]";

/// Alfred が使っている設定フォルダ。同期フォルダを設定していれば、そちらを指している。
fn alfred_preferences(home: &Path) -> Result<PathBuf, String> {
    let prefs = home.join("Library/Application Support/Alfred/prefs.json");
    let standard = home.join("Library/Application Support/Alfred/Alfred.alfredpreferences");
    // 同期を解除すると prefs.json は消え、標準のフォルダに戻る。
    let text = match std::fs::read_to_string(&prefs) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(standard),
        Err(error) => return Err(format!("{}: {error}", prefs.display())),
    };
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{}: {error}", prefs.display()))?;
    match json.get("current").and_then(serde_json::Value::as_str) {
        Some(current) => Ok(paths::expand_home(current, home)),
        None => Ok(standard),
    }
}

/// plist は XML と バイナリの 2 つの形があるので、macOS 付属の `plutil` で JSON にして読む。
fn read_plist(path: &Path) -> Result<serde_json::Value, String> {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .map_err(|error| format!("plutil: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

fn import_alfred(store: &Store, home: &Path, options: &[&str]) -> Result<(), String> {
    let mut write = false;
    let mut from = None;
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        match *option {
            "--write" => write = true,
            "--from" => from = Some(paths::expand_home(rest.next().ok_or(USAGE)?, home)),
            _ => return Err(USAGE.to_string()),
        }
    }
    let preferences = match from {
        Some(from) => from,
        None => alfred_preferences(home)?,
    };
    let folder = preferences.join("workflows");
    let mut workflows = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map_err(|error| format!("{}: {error}", folder.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("info.plist")))
        .filter(|path| path.is_file())
        .collect();
    entries.sort();
    for path in entries {
        match read_plist(&path).and_then(|info| Workflow::from_json(&info)) {
            Ok(workflow) => workflows.push(workflow),
            Err(error) => println!("UNREADABLE {}: {error}", path.display()),
        }
    }
    let conversion = alfred::convert(&workflows);

    let (mut created, mut unchanged, mut kept, mut failed) = (0, 0, 0, 0);
    for imported in &conversion.commands {
        let path = store.command_path(&imported.id).map_err(|error| error.to_string())?;
        let mut problem = None;
        let mut status = match std::fs::read_to_string(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "new",
            Err(error) => {
                problem = Some(format!("could not read {}: {error}; left alone", path.display()));
                "exists"
            }
            Ok(text) if parse_command(&text).as_ref() == Ok(&imported.command) => "same",
            Ok(_) => {
                problem = Some(format!(
                    "{} already exists with other contents; left alone",
                    path.display()
                ));
                "exists"
            }
        };
        // 1 件の失敗で残りを止めない。
        if status == "new"
            && write
            && let Err(error) = store.save_command(&imported.id, &imported.command)
        {
            problem = Some(format!("could not write {}: {error}", path.display()));
            status = "failed";
        }
        match status {
            "new" => created += 1,
            "same" => unchanged += 1,
            "failed" => failed += 1,
            _ => kept += 1,
        }
        let command = &imported.command;
        println!(
            "{status:6} {:28} {:8} {:24} {}   ({})",
            imported.id,
            command.action.type_name(),
            command.hotkey.as_deref().unwrap_or("-"),
            command.title,
            imported.origin
        );
        if let Some(problem) = problem {
            println!("         ! {problem}");
        }
        for warning in &imported.warnings {
            println!("         ! {warning}");
        }
    }
    for skipped in &conversion.skipped {
        println!("SKIP   {} — {}", skipped.origin, skipped.reason);
    }
    if conversion
        .commands
        .iter()
        .any(|imported| imported.command.hotkey.is_some())
    {
        println!(
            "\nnote: Alfred keeps its hotkeys. macOS lets both apps register the same hotkey without an error, \
             so disable the Alfred workflows whose hotkeys Flintkey now has."
        );
    }
    let verb = if write { "created" } else { "to create" };
    println!(
        "\n{created} {verb}, {unchanged} already identical, {kept} left alone (id taken), {failed} failed, {} skipped",
        conversion.skipped.len()
    );
    if !write {
        println!(
            "nothing was written; run again with --write to create the files in {}",
            store.root().display()
        );
    }
    Ok(())
}
