//! コアを使うコマンドライン。デスクトップアプリ無しで設定フォルダを試すためのもの。
//!
//! ```text
//! cargo run --example flintkey -- list [query]   順位付けしたコマンドと、壊れているファイル
//! cargo run --example flintkey -- plan <id>      実行すると何が起きるか
//! cargo run --example flintkey -- run <id>       実行する
//! ```
//!
//! `~/.config/flintkey` 以外のフォルダを使うには `FLINTKEY_CONFIG_DIR` を設定する。

use flintkey_core::Os;
use flintkey_core::launch::{self, Machine};
use flintkey_core::paths;
use flintkey_core::search::Searcher;
use flintkey_core::secrets::OnePassword;
use flintkey_core::store::Store;
use flintkey_core::usage::{USAGE_FILE, Usage};
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
        _ => Err("usage: flintkey list [query] | plan <id> | run <id>".to_string()),
    }
}
