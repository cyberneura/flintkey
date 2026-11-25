//! コマンドの実行がこのマシンで何を意味するかを決め、実行する。
//!
//! [`plan`] はアクションとマシンの情報だけで決まる純粋な関数なので、どの OS 向けの挙動も
//! 手元の OS でテストできる。[`execute`] が計画を実行する。

use crate::command::{Action, Os};
use crate::paths::expand_home;
use crate::secrets::{SecretError, SecretResolver, is_reference};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `terminal` コマンドが開く端末アプリ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalApp {
    Terminal,
    WindowsTerminal,
}

impl TerminalApp {
    pub const ALL: [TerminalApp; 2] = [TerminalApp::Terminal, TerminalApp::WindowsTerminal];

    pub fn name(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "terminal",
            TerminalApp::WindowsTerminal => "windows-terminal",
        }
    }

    pub fn from_name(name: &str) -> Option<TerminalApp> {
        TerminalApp::ALL.into_iter().find(|app| app.name() == name)
    }

    fn default_for(os: Os) -> Option<TerminalApp> {
        match os {
            Os::Macos => Some(TerminalApp::Terminal),
            Os::Windows => Some(TerminalApp::WindowsTerminal),
            Os::Linux => None,
        }
    }

    fn runs_on(self, os: Os) -> bool {
        match self {
            TerminalApp::Terminal => os == Os::Macos,
            TerminalApp::WindowsTerminal => os == Os::Windows,
        }
    }
}

/// 計画が依存するマシンの情報。
#[derive(Debug, Clone)]
pub struct Machine<'a> {
    pub os: Os,
    pub home: &'a Path,
    /// ユーザーのログインシェル (`$SHELL`)。分かる場合だけ。
    pub shell: Option<&'a str>,
    /// このプロセスが起動されたときの `PATH`。
    pub path: Option<&'a str>,
    /// 設定で選んだ端末。`None` なら OS 標準の端末を使う。
    pub terminal: Option<TerminalApp>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// 対象を OS に渡す。OS が関連付けられたアプリで開く。
    Open {
        target: String,
    },
    Spawn(Process),
}

#[derive(PartialEq, Eq)]
pub struct Process {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// 継承した環境に追加する変数。値は設定に書かれたままで、秘密情報の参照は
    /// 解決されていない。
    pub env: Vec<(String, String)>,
}

impl fmt::Debug for Process {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 環境変数の値は、参照でなく直接書かれた機密でありうるので、ログに出さない。
        let names: Vec<&str> = self.env.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("Process")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env (names only)", &names)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("{0}")]
    Unsupported(String),
    #[error("could not start it: {0}")]
    Io(#[from] std::io::Error),
}

/// Dock から起動した macOS アプリの PATH に入っていないフォルダ。これが無いと、
/// スクリプトはユーザーが入れたツールではなく、OS 同梱の古いツールを見つけてしまう。
const MACOS_EXTRA_PATH: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// `-l` を受け付けて、ユーザーのプロファイルを読み込むことが分かっているシェル。
const LOGIN_SHELLS: [&str; 4] = ["zsh", "bash", "fish", "ksh"];

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn path_with_extras(machine: &Machine) -> Option<String> {
    if machine.os != Os::Macos {
        return None;
    }
    let current: Vec<&str> = machine
        .path
        .unwrap_or_default()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .collect();
    let missing: Vec<&str> = MACOS_EXTRA_PATH
        .into_iter()
        .filter(|dir| !current.contains(dir))
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(missing.into_iter().chain(current).collect::<Vec<_>>().join(":"))
}

fn script_process(run: &str, cwd: Option<&str>, env: Vec<(String, String)>, machine: &Machine) -> Process {
    let cwd = cwd.map(|dir| expand_home(dir, machine.home));
    if machine.os == Os::Windows {
        return Process {
            program: "powershell.exe".into(),
            args: vec!["-NoProfile".into(), "-Command".into(), run.into()],
            cwd,
            env,
        };
    }
    let shell = machine.shell.filter(|shell| !shell.is_empty()).unwrap_or("/bin/sh");
    let shell_name = shell.rsplit('/').next().unwrap_or(shell);
    let mut args = Vec::new();
    if LOGIN_SHELLS.contains(&shell_name) {
        args.push("-l".to_string());
    }
    args.extend(["-c".to_string(), run.to_string()]);
    let mut env = env;
    if !env.iter().any(|(name, _)| name == "PATH")
        && let Some(path) = path_with_extras(machine)
    {
        env.push(("PATH".into(), path));
    }
    Process {
        program: shell.into(),
        args,
        cwd,
        env,
    }
}

/// コマンドラインを引数で受け取る AppleScript。コマンドを AppleScript のソースに
/// 埋め込まないので、エスケープが要らない。
fn osascript(lines: &[&str], command_line: String) -> Process {
    let mut args = Vec::new();
    for line in lines {
        args.extend(["-e".to_string(), line.to_string()]);
    }
    args.extend(["--".to_string(), command_line]);
    Process {
        program: "/usr/bin/osascript".into(),
        args,
        cwd: None,
        env: Vec::new(),
    }
}

fn terminal_process(run: &str, cwd: Option<&str>, machine: &Machine) -> Result<Process, LaunchError> {
    let app = machine
        .terminal
        .or(TerminalApp::default_for(machine.os))
        .ok_or_else(|| {
            LaunchError::Unsupported(format!(
                "terminal commands are not supported on {} yet",
                machine.os.name()
            ))
        })?;
    if !app.runs_on(machine.os) {
        return Err(LaunchError::Unsupported(format!(
            "the terminal `{}` is not available on {}",
            app.name(),
            machine.os.name()
        )));
    }
    let cwd = cwd.map(|dir| expand_home(dir, machine.home));
    let unix_command_line = || match &cwd {
        Some(dir) => format!("cd {} && {run}", sh_quote(&dir.to_string_lossy())),
        None => run.to_string(),
    };
    Ok(match app {
        TerminalApp::Terminal => osascript(
            &[
                "on run argv",
                "tell application \"Terminal\"",
                // 起動していなかった Terminal は、起動時に空のウインドウを 1 つ開く。2 つ目を開かず、
                // そのウインドウにコマンドを入れる。起動中の Terminal はウインドウが 0 個のことが
                // あるので、"window 1" を使うのは起動直後だけにする。
                "set wasRunning to running",
                "activate",
                "if (not wasRunning) and (count of windows) > 0 then",
                "do script (item 1 of argv) in window 1",
                "else",
                "do script (item 1 of argv)",
                "end if",
                "end tell",
                "end run",
            ],
            unix_command_line(),
        ),
        TerminalApp::WindowsTerminal => {
            let mut args = Vec::new();
            // Windows Terminal は引数の中でも `;` でコマンドラインを区切る。フォルダ名や
            // ユーザーのコマンドに含まれる `;` はエスケープする。
            let escape = |text: &str| text.replace(';', "\\;");
            if let Some(dir) = &cwd {
                args.extend(["-d".to_string(), escape(&dir.to_string_lossy())]);
            }
            args.extend([
                "powershell.exe".to_string(),
                "-NoExit".to_string(),
                "-Command".to_string(),
                escape(run),
            ]);
            Process {
                program: "wt.exe".into(),
                args,
                cwd: None,
                env: Vec::new(),
            }
        }
    })
}

fn app_plan(app: &str, args: &[String], machine: &Machine) -> Plan {
    // アプリはシェルを通さずに渡すので、`~/` はここで展開しないと名前の一部として扱われる。
    let expanded = app
        .starts_with("~/")
        .then(|| expand_home(app, machine.home).display().to_string());
    let app = expanded.as_deref().unwrap_or(app);
    match machine.os {
        Os::Macos => {
            let mut open_args = vec!["-a".to_string(), app.to_string()];
            if !args.is_empty() {
                open_args.push("--args".to_string());
                open_args.extend(args.iter().cloned());
            }
            Plan::Spawn(Process {
                program: "/usr/bin/open".into(),
                args: open_args,
                cwd: None,
                env: Vec::new(),
            })
        }
        // 引数が無ければシェルに任せる。名前・ショートカット・登録済みアプリの解決は、
        // プロセスの直接起動ではできない。
        Os::Windows if args.is_empty() => Plan::Open {
            target: app.to_string(),
        },
        Os::Windows | Os::Linux => Plan::Spawn(Process {
            program: app.to_string(),
            args: args.to_vec(),
            cwd: None,
            env: Vec::new(),
        }),
    }
}

/// `action` の実行が `machine` 上で何を意味するかを決める。
///
/// 秘密情報には触れない。計画を表示するだけで 1Password のロック解除を求めることが
/// 無いようにするため。
pub fn plan(action: &Action, machine: &Machine) -> Result<Plan, LaunchError> {
    Ok(match action {
        Action::Url { url } => Plan::Open { target: url.clone() },
        Action::App { app, args } => app_plan(app, args, machine),
        Action::Script { run, cwd, env } => {
            let env = env.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
            Plan::Spawn(script_process(run, cwd.as_deref(), env, machine))
        }
        Action::Terminal { run, cwd } => Plan::Spawn(terminal_process(run, cwd.as_deref(), machine)?),
    })
}

/// 計画を実行する。終了は待たない。
///
/// 環境変数に含まれる秘密情報の参照は、起動の直前にまとめて 1 回で解決する。解決した値は
/// この関数の中にしか存在せず、子プロセスに渡した時点で破棄する。解決は 1Password の
/// ロック解除を待って止まることがあるので、UI スレッドから呼ばないこと。
pub fn execute(plan: &Plan, secrets: &dyn SecretResolver) -> Result<(), LaunchError> {
    let process = match plan {
        Plan::Open { target } => return Ok(open::that_detached(target)?),
        Plan::Spawn(process) => process,
    };
    let references: Vec<String> = process
        .env
        .iter()
        .map(|(_, value)| value)
        .filter(|value| is_reference(value))
        .cloned()
        .collect();
    let resolved = secrets.resolve(&references)?;
    // 数が合わないまま進めると、足りない分が空の値でスクリプトに渡ってしまう。
    if resolved.len() != references.len() {
        return Err(SecretError::UnexpectedOutput.into());
    }
    // 空の値は、設定の誤りである可能性が高い。空の認証情報でスクリプトを走らせない。
    if let Some(index) = resolved.iter().position(String::is_empty) {
        return Err(SecretError::Empty(references[index].clone()).into());
    }
    let mut resolved = resolved.into_iter();

    let mut command = Command::new(&process.program);
    command
        .args(&process.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = &process.cwd {
        command.current_dir(cwd);
    }
    for (name, value) in &process.env {
        if is_reference(value) {
            command.env(name, resolved.next().expect("one value per reference"));
        } else {
            command.env(name, value);
        }
    }
    let mut child = command.spawn()?;
    // ランチャーは何日も動き続ける。wait しない子プロセスはゾンビとして残る。
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// `op://a/b` を `value-of(op://a/b)` に解決し、呼ばれ方を記録する。
    #[derive(Default)]
    struct FakeSecrets {
        calls: std::cell::RefCell<Vec<Vec<String>>>,
    }

    impl SecretResolver for FakeSecrets {
        fn resolve(&self, references: &[String]) -> Result<Vec<String>, SecretError> {
            self.calls.borrow_mut().push(references.to_vec());
            Ok(references
                .iter()
                .map(|reference| format!("value-of({reference})"))
                .collect())
        }
    }

    struct FailingSecrets;

    impl SecretResolver for FailingSecrets {
        fn resolve(&self, references: &[String]) -> Result<Vec<String>, SecretError> {
            if references.is_empty() {
                Ok(Vec::new())
            } else {
                Err(SecretError::Rejected("locked".into()))
            }
        }
    }

    /// 参照がいくつあっても値を 1 つも返さない、契約違反の解決器。
    struct ForgetfulSecrets;

    impl SecretResolver for ForgetfulSecrets {
        fn resolve(&self, _: &[String]) -> Result<Vec<String>, SecretError> {
            Ok(Vec::new())
        }
    }

    /// 環境変数 `TOKEN` と `PLAIN` を `out.txt` に書き出すスクリプトの計画。
    #[cfg(unix)]
    fn env_dumping_plan(dir: &Path, env: &[(&str, &str)]) -> Plan {
        Plan::Spawn(Process {
            program: "/bin/sh".into(),
            args: strings(&["-c", "printf '%s|%s|%s' \"$A_TOKEN\" \"$PLAIN\" \"$Z_TOKEN\" > out.txt"]),
            cwd: Some(dir.to_path_buf()),
            env: env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        })
    }

    /// 起動は終了を待たないので、ファイルが期待した内容になるまで短時間ポーリングする。
    #[cfg(unix)]
    fn wait_for_content(path: &Path, expected: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read_to_string(path).unwrap_or_default() != expected {
            assert!(
                std::time::Instant::now() < deadline,
                "{} never became {expected:?}",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn machine(os: Os) -> Machine<'static> {
        Machine {
            os,
            home: Path::new("/home/me"),
            shell: Some("/bin/zsh"),
            path: Some("/usr/bin:/bin"),
            terminal: None,
        }
    }

    fn spawned(plan: Plan) -> Process {
        match plan {
            Plan::Spawn(process) => process,
            other => panic!("expected a process, got {other:?}"),
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn url_is_handed_to_the_os_on_every_system() {
        // Arrange
        let action = Action::Url {
            url: "https://example.com/?a=1&b=2".into(),
        };

        for os in Os::ALL {
            // Act
            let plan = plan(&action, &machine(os)).unwrap();

            // Assert
            assert_eq!(
                plan,
                Plan::Open {
                    target: "https://example.com/?a=1&b=2".into()
                }
            );
        }
    }

    #[test]
    fn app_under_the_home_folder_is_expanded_on_every_system() {
        // Arrange
        let action = Action::App {
            app: "~/workspace/tool/Tool.app".into(),
            args: vec![],
        };

        // Act
        let mac = spawned(plan(&action, &machine(Os::Macos)).unwrap());
        let linux = spawned(plan(&action, &machine(Os::Linux)).unwrap());
        let windows = plan(&action, &machine(Os::Windows)).unwrap();

        // Assert
        assert_eq!(mac.args, strings(&["-a", "/home/me/workspace/tool/Tool.app"]));
        assert_eq!(linux.program, "/home/me/workspace/tool/Tool.app");
        assert_eq!(
            windows,
            Plan::Open {
                target: Path::new("/home/me")
                    .join("workspace/tool/Tool.app")
                    .display()
                    .to_string()
            }
        );
    }

    #[test]
    fn macos_app_is_opened_by_name_with_its_arguments() {
        // Arrange
        let action = Action::App {
            app: "Visual Studio Code".into(),
            args: strings(&["--new-window", "a b"]),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Macos)).unwrap());

        // Assert
        assert_eq!(process.program, "/usr/bin/open");
        assert_eq!(
            process.args,
            strings(&["-a", "Visual Studio Code", "--args", "--new-window", "a b"])
        );
    }

    #[test]
    fn macos_app_without_arguments_passes_no_args_flag() {
        // Arrange
        let action = Action::App {
            app: "Slack".into(),
            args: vec![],
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Macos)).unwrap());

        // Assert
        assert_eq!(process.args, strings(&["-a", "Slack"]));
    }

    #[test]
    fn windows_app_goes_through_the_shell_unless_it_has_arguments() {
        // Arrange
        let bare = Action::App {
            app: "notepad".into(),
            args: vec![],
        };
        let with_args = Action::App {
            app: "C:\\Tools\\app.exe".into(),
            args: strings(&["--x"]),
        };

        // Act
        let bare_plan = plan(&bare, &machine(Os::Windows)).unwrap();
        let args_process = spawned(plan(&with_args, &machine(Os::Windows)).unwrap());

        // Assert
        assert_eq!(
            bare_plan,
            Plan::Open {
                target: "notepad".into()
            }
        );
        assert_eq!(args_process.program, "C:\\Tools\\app.exe");
        assert_eq!(args_process.args, strings(&["--x"]));
    }

    #[test]
    fn script_runs_in_a_login_shell_with_homebrew_on_the_path() {
        // Arrange
        let action = Action::Script {
            run: "fab deploy && echo 'done'".into(),
            cwd: Some("~/work/blog".into()),
            env: BTreeMap::new(),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Macos)).unwrap());

        // Assert
        assert_eq!(process.program, "/bin/zsh");
        assert_eq!(process.args, strings(&["-l", "-c", "fab deploy && echo 'done'"]));
        assert_eq!(process.cwd.as_deref(), Some(Path::new("/home/me/work/blog")));
        assert_eq!(
            process.env,
            [(
                "PATH".to_string(),
                "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_string()
            )]
        );
    }

    #[test]
    fn path_is_left_alone_when_it_already_has_homebrew_or_the_command_sets_it() {
        // Arrange
        let plain = Action::Script {
            run: "x".into(),
            cwd: None,
            env: BTreeMap::new(),
        };
        let own_path = Action::Script {
            run: "x".into(),
            cwd: None,
            env: BTreeMap::from([("PATH".to_string(), "/mine".to_string())]),
        };
        let mut complete = machine(Os::Macos);
        complete.path = Some("/opt/homebrew/bin:/usr/local/bin:/usr/bin");

        // Act
        let already_complete = spawned(plan(&plain, &complete).unwrap());
        let overridden = spawned(plan(&own_path, &machine(Os::Macos)).unwrap());
        let on_linux = spawned(plan(&plain, &machine(Os::Linux)).unwrap());

        // Assert
        assert!(already_complete.env.is_empty());
        assert_eq!(overridden.env, [("PATH".to_string(), "/mine".to_string())]);
        assert!(on_linux.env.is_empty());
    }

    #[test]
    fn only_the_missing_homebrew_folder_is_added() {
        // Arrange
        let action = Action::Script {
            run: "x".into(),
            cwd: None,
            env: BTreeMap::new(),
        };
        let mut partial = machine(Os::Macos);
        partial.path = Some("/usr/local/bin:/usr/bin");

        // Act
        let process = spawned(plan(&action, &partial).unwrap());

        // Assert
        assert_eq!(
            process.env,
            [(
                "PATH".to_string(),
                "/opt/homebrew/bin:/usr/local/bin:/usr/bin".to_string()
            )]
        );
    }

    #[test]
    fn unknown_shell_is_not_given_the_login_flag_and_missing_shell_falls_back_to_sh() {
        // Arrange
        let action = Action::Script {
            run: "x".into(),
            cwd: None,
            env: BTreeMap::new(),
        };
        let mut exotic = machine(Os::Linux);
        exotic.shell = Some("/usr/bin/nu");
        let mut unknown = machine(Os::Linux);
        unknown.shell = None;

        // Act
        let exotic_process = spawned(plan(&action, &exotic).unwrap());
        let fallback_process = spawned(plan(&action, &unknown).unwrap());

        // Assert
        assert_eq!(
            (exotic_process.program.as_str(), exotic_process.args),
            ("/usr/bin/nu", strings(&["-c", "x"]))
        );
        assert_eq!(
            (fallback_process.program.as_str(), fallback_process.args),
            ("/bin/sh", strings(&["-c", "x"]))
        );
    }

    #[test]
    fn windows_script_runs_in_powershell() {
        // Arrange
        let action = Action::Script {
            run: "Get-Date".into(),
            cwd: Some("~\\work".into()),
            env: BTreeMap::new(),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Windows)).unwrap());

        // Assert
        assert_eq!(process.program, "powershell.exe");
        assert_eq!(process.args, strings(&["-NoProfile", "-Command", "Get-Date"]));
        assert_eq!(process.cwd.as_deref(), Some(Path::new("/home/me/work")));
        assert!(process.env.is_empty());
    }

    #[test]
    fn plan_keeps_secret_references_unresolved() {
        // Arrange
        let action = Action::Script {
            run: "deploy".into(),
            cwd: None,
            env: BTreeMap::from([
                ("PLAIN".to_string(), "value".to_string()),
                ("TOKEN".to_string(), "op://Private/A/token".to_string()),
            ]),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Linux)).unwrap());

        // Assert
        assert_eq!(
            process.env,
            [
                ("PLAIN".to_string(), "value".to_string()),
                ("TOKEN".to_string(), "op://Private/A/token".to_string())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn executing_resolves_references_in_one_batch_and_passes_plain_values_through() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let plan = env_dumping_plan(
            dir.path(),
            &[
                ("A_TOKEN", "op://Private/A/token"),
                ("PLAIN", "value"),
                ("Z_TOKEN", "op://Private/Z/token"),
            ],
        );
        let secrets = FakeSecrets::default();

        // Act
        execute(&plan, &secrets).unwrap();

        // Assert
        wait_for_content(
            &dir.path().join("out.txt"),
            "value-of(op://Private/A/token)|value|value-of(op://Private/Z/token)",
        );
        assert_eq!(
            *secrets.calls.borrow(),
            [strings(&["op://Private/A/token", "op://Private/Z/token"])]
        );
    }

    #[cfg(unix)]
    #[test]
    fn failing_secret_lookup_stops_the_launch_before_anything_runs() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let plan = env_dumping_plan(dir.path(), &[("A_TOKEN", "op://Private/A/token")]);

        // Act
        let result = execute(&plan, &FailingSecrets);

        // Assert
        assert!(matches!(&result, Err(LaunchError::Secret(SecretError::Rejected(message))) if message == "locked"));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!dir.path().join("out.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolver_that_returns_too_few_values_stops_the_launch() {
        // Arrange: 黙って進めると、スクリプトが空の秘密情報で走る。
        let dir = tempfile::tempdir().unwrap();
        let plan = env_dumping_plan(dir.path(), &[("A_TOKEN", "op://Private/A/token")]);

        // Act
        let result = execute(&plan, &ForgetfulSecrets);

        // Assert
        assert!(
            matches!(result, Err(LaunchError::Secret(SecretError::UnexpectedOutput))),
            "{result:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!dir.path().join("out.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn empty_secret_stops_the_launch() {
        // Arrange: 空の認証情報でスクリプトを走らせるより、止めて知らせる。
        struct EmptySecrets;
        impl SecretResolver for EmptySecrets {
            fn resolve(&self, references: &[String]) -> Result<Vec<String>, SecretError> {
                Ok(references.iter().map(|_| String::new()).collect())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let plan = env_dumping_plan(dir.path(), &[("PLAIN", ""), ("A_TOKEN", "op://Private/A/token")]);

        // Act
        let result = execute(&plan, &EmptySecrets);

        // Assert
        assert!(
            matches!(&result, Err(LaunchError::Secret(SecretError::Empty(reference))) if reference == "op://Private/A/token"),
            "{result:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!dir.path().join("out.txt").exists());
    }

    #[test]
    fn debug_output_of_a_process_hides_environment_values() {
        // Arrange
        let action = Action::Script {
            run: "deploy".into(),
            cwd: None,
            env: BTreeMap::from([("PASSWORD".to_string(), "hunter2".to_string())]),
        };
        let plan = plan(&action, &machine(Os::Linux)).unwrap();

        // Act
        let printed = format!("{plan:?}");

        // Assert
        assert!(printed.contains("PASSWORD"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }

    #[test]
    fn macos_terminal_receives_the_command_as_an_argument_not_as_script_source() {
        // Arrange: 引用符とバックスラッシュは、AppleScript のソースに埋め込むならエスケープが要る。
        let run = r#"echo "it's" \ done"#;
        let action = Action::Terminal {
            run: run.into(),
            cwd: Some("~/my work".into()),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Macos)).unwrap());

        // Assert
        assert_eq!(process.program, "/usr/bin/osascript");
        let (separator, command_line) = (&process.args[process.args.len() - 2], process.args.last().unwrap());
        assert_eq!(separator, "--");
        assert_eq!(command_line, &format!("cd '/home/me/my work' && {run}"));
        let script_source = process.args[..process.args.len() - 2].join("\n");
        assert!(
            script_source.contains("tell application \"Terminal\""),
            "{script_source}"
        );
        assert!(!script_source.contains("it's"), "{script_source}");
    }

    #[test]
    fn working_directory_with_a_single_quote_is_quoted_for_the_shell() {
        // Arrange
        let action = Action::Terminal {
            run: "ls".into(),
            cwd: Some("/tmp/it's here".into()),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Macos)).unwrap());

        // Assert
        assert_eq!(process.args.last().unwrap(), r"cd '/tmp/it'\''s here' && ls");
    }

    #[test]
    fn windows_terminal_gets_the_directory_and_an_escaped_semicolon() {
        // Arrange
        let action = Action::Terminal {
            run: "cd src; ls".into(),
            cwd: Some("C:\\work;dir".into()),
        };

        // Act
        let process = spawned(plan(&action, &machine(Os::Windows)).unwrap());

        // Assert
        assert_eq!(process.program, "wt.exe");
        assert_eq!(
            process.args,
            strings(&[
                "-d",
                "C:\\work\\;dir",
                "powershell.exe",
                "-NoExit",
                "-Command",
                "cd src\\; ls"
            ])
        );
    }

    #[test]
    fn terminal_that_does_not_exist_on_the_os_is_refused() {
        // Arrange
        let action = Action::Terminal {
            run: "ls".into(),
            cwd: None,
        };
        let mut wrong = machine(Os::Windows);
        wrong.terminal = Some(TerminalApp::Terminal);

        // Act
        let on_linux = plan(&action, &machine(Os::Linux));
        let mismatched = plan(&action, &wrong);

        // Assert
        assert_eq!(
            on_linux.unwrap_err().to_string(),
            "terminal commands are not supported on linux yet"
        );
        assert_eq!(
            mismatched.unwrap_err().to_string(),
            "the terminal `terminal` is not available on windows"
        );
    }

    #[cfg(unix)]
    #[test]
    fn executing_a_script_plan_runs_it_with_its_directory_and_environment() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let plan = Plan::Spawn(Process {
            program: "/bin/sh".into(),
            args: strings(&["-c", "printf '%s|%s' \"$GREETING\" \"$(basename \"$PWD\")\" > out.txt"]),
            cwd: Some(dir.path().to_path_buf()),
            env: vec![("GREETING".into(), "hello world".into())],
        });

        // Act
        execute(&plan, &FakeSecrets::default()).unwrap();

        // Assert
        let expected = format!("hello world|{}", dir.path().file_name().unwrap().to_string_lossy());
        wait_for_content(&dir.path().join("out.txt"), &expected);
    }

    #[test]
    fn executing_a_plan_for_a_missing_program_reports_the_error() {
        // Arrange
        let plan = Plan::Spawn(Process {
            program: "/nonexistent/program".into(),
            args: vec![],
            cwd: None,
            env: vec![],
        });

        // Act
        let result = execute(&plan, &FakeSecrets::default());

        // Assert
        assert!(
            matches!(&result, Err(LaunchError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound),
            "{result:?}"
        );
    }
}
