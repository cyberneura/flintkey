//! Flintkey のデスクトップアプリ。メニューバー / タスクトレイに常駐し、ホットキーでランチャー窓を出す。

// リリースビルドの Windows でコンソール窓を出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkeys;
#[cfg(target_os = "macos")]
mod panel;
mod placement;
mod session;
mod showing;

use flintkey_core::Os;
use flintkey_core::launch::{self, Plan};
use flintkey_core::paths;
use flintkey_core::secrets::OnePassword;
use hotkeys::{Registered, Target};
use placement::{Monitor, Position, Rect};
use serde::Serialize;
use session::{Row, Session};
use showing::Showing;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const LAUNCHER: &str = "launcher";
/// `tauri.conf.json` の窓の大きさと同じ (論理サイズ)。
const WINDOW_SIZE: (f64, f64) = (720.0, 480.0);

const EVENT_SHOWN: &str = "launcher-shown";
const EVENT_HIDDEN: &str = "launcher-hidden";

struct AppState {
    session: Mutex<Session>,
    hotkeys: Mutex<Registry>,
    /// 窓を隠したあとに起きた失敗。次に窓を出したときに見せる。
    notices: Mutex<Vec<String>>,
    showing: Mutex<Showing>,
    /// 窓がキー入力の宛先になっているか。
    has_key: AtomicBool,
    /// 窓をキー入力を受け取れるパネルに変えられたか。
    #[cfg(target_os = "macos")]
    panel: AtomicBool,
}

/// 登録できているホットキー。
#[derive(Default)]
struct Registry {
    /// ショートカット → 押したときに起きることと、設定に書かれていた文字列。
    registered: Registered,
    /// 最後に登録し直したときの問題 (読めない、重なっている、登録できない)。
    problems: Vec<String>,
}

impl Registry {
    fn has_launcher(&self) -> bool {
        self.registered.values().any(|(target, _)| *target == Target::Launcher)
    }
}

/// 窓を出すたびにページへ渡す内容。
#[derive(Serialize)]
struct View {
    rows: Vec<Row>,
    problems: Vec<String>,
    config_dir: String,
    /// 今の表示の番号。ページは実行要求にこれを付ける。
    showing: u64,
}

#[tauri::command]
fn open_session(state: State<'_, AppState>, query: String) -> View {
    let mut problems = std::mem::take(&mut *state.notices.lock().unwrap());
    problems.extend(state.hotkeys.lock().unwrap().problems.iter().cloned());
    // コマンドは窓を出したときに読み直してある (`sync_hotkeys`)。
    let mut session = state.session.lock().unwrap();
    problems.extend(session.problems());
    View {
        rows: session.search(&query),
        problems,
        config_dir: session.config_dir().display().to_string(),
        showing: state.showing.lock().unwrap().id(),
    }
}

#[tauri::command]
fn search(state: State<'_, AppState>, query: String) -> Vec<Row> {
    state.session.lock().unwrap().search(&query)
}

/// コマンドを実行する。計画を立てられなければ窓を開いたままエラーを返す。
/// 計画が立てば窓を隠し、起動は別スレッドで行う (1Password の認証待ちで窓を固めないため)。
#[tauri::command]
fn run(app: AppHandle, state: State<'_, AppState>, id: String, showing: u64) -> Result<(), String> {
    // Enter の連打や続けてのクリックで要求が複数届く。窓は最初の要求で隠れるので、残りは
    // 利用者が見ていない一覧に対する要求になる。最初の 1 件だけ受け付ける。
    if !state.showing.lock().unwrap().claim(showing) {
        return Ok(());
    }
    let plan = match plan_command(&state, &id) {
        Ok(plan) => plan,
        Err(message) => {
            // 窓は開いたまま。別のコマンドを選び直せるようにする。
            state.showing.lock().unwrap().release(showing);
            return Err(message);
        }
    };
    hide_launcher(&app);

    std::thread::spawn(move || execute_and_record(&app, &id, &plan));
    Ok(())
}

fn plan_command(state: &AppState, id: &str) -> Result<Plan, String> {
    let shell = std::env::var("SHELL").ok();
    let path = std::env::var("PATH").ok();
    state
        .session
        .lock()
        .unwrap()
        .plan(id, shell.as_deref(), path.as_deref())
}

/// 計画を実行し、使ったことを記録する。失敗は知らせで見せる。1Password の認証を待つことがあるので、
/// メインスレッドから呼ばない。
fn execute_and_record(app: &AppHandle, id: &str, plan: &Plan) {
    let state = app.state::<AppState>();
    match launch::execute(plan, &OnePassword::locate()) {
        Ok(()) => {
            // コマンドはもう起動している。記録の失敗で利用者を止めない。
            if let Err(error) = state.session.lock().unwrap().record_use(id) {
                eprintln!("could not record the use of `{id}`: {error}");
            }
        }
        Err(error) => notify(app, format!("`{id}` did not start: {error}")),
    }
}

/// 知らせを積み、キー入力を奪わずに窓で見せる。
fn notify(app: &AppHandle, notice: String) {
    app.state::<AppState>().notices.lock().unwrap().push(notice);
    show_launcher(app, Focus::Leave);
}

/// ホットキーが押された。メインスレッド以外から呼ぶ (コマンドの実行は 1Password の認証を待つことがある)。
fn fire(app: &AppHandle, shortcut: Shortcut) {
    let state = app.state::<AppState>();
    let target = state
        .hotkeys
        .lock()
        .unwrap()
        .registered
        .get(&shortcut)
        .map(|(target, _)| target.clone());
    match target {
        Some(Target::Launcher) => toggle_launcher(app),
        Some(Target::Command(id)) => match plan_command(&state, &id) {
            Ok(plan) => execute_and_record(app, &id, &plan),
            Err(message) => notify(app, format!("hotkey for `{id}`: {message}")),
        },
        None => {}
    }
}

#[tauri::command]
fn hide(app: AppHandle) {
    hide_launcher(&app);
}

/// コマンドを読み直し、ランチャーとコマンドのホットキーを登録し直す。メインスレッドから呼ぶ
/// (登録はメインスレッドの応答を待つので、別スレッドからだとロックを持ったまま互いに止まりうる)。
fn sync_hotkeys(app: &AppHandle) {
    let state = app.state::<AppState>();
    let (launcher, commands) = {
        let mut session = state.session.lock().unwrap();
        session.reload_commands();
        (session.settings().hotkey.clone(), session.hotkeys())
    };
    let wanted = hotkeys::wanted(&launcher, &commands);
    let shortcuts = app.global_shortcut();
    let mut registry = state.hotkeys.lock().unwrap();
    registry.problems = hotkeys::apply(
        &mut registry.registered,
        wanted,
        |shortcut| shortcuts.register(shortcut).is_ok(),
        |shortcut| {
            let _ = shortcuts.unregister(shortcut);
        },
    );
}

fn rect(position: PhysicalPosition<i32>, size: tauri::PhysicalSize<u32>) -> Rect {
    Rect {
        x: position.x.into(),
        y: position.y.into(),
        width: size.width.into(),
        height: size.height.into(),
    }
}

/// 設定に従って、窓を置く位置を決める。モニターの情報が取れなければ `None`。
fn launcher_position(app: &AppHandle) -> Option<Position> {
    let primary = app.primary_monitor().ok().flatten();
    let monitors: Vec<Monitor> = app
        .available_monitors()
        .ok()?
        .iter()
        .map(|monitor| Monitor {
            bounds: rect(*monitor.position(), *monitor.size()),
            work_area: rect(monitor.work_area().position, monitor.work_area().size),
            scale_factor: monitor.scale_factor(),
            primary: primary
                .as_ref()
                .is_some_and(|primary| primary.position() == monitor.position()),
        })
        .collect();
    let cursor = app.cursor_position().ok().map(|cursor| (cursor.x, cursor.y));
    let display = app.state::<AppState>().session.lock().unwrap().settings().display;
    placement::place(Os::current(), &monitors, cursor, display, WINDOW_SIZE)
}

/// 窓を出すときに、キー入力の宛先にするかどうか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// 利用者が窓を出す操作をした。キー入力を受け取る。
    Take,
    /// アプリの都合で知らせを出す。利用者は別のアプリで打鍵しているかもしれないので、キー入力を奪わない
    /// (奪うと、別のアプリに打ったつもりの Enter でコマンドが実行される)。
    Leave,
}

/// 窓を出す。どのスレッドからでも呼べる。
///
/// 設定の反映、ホットキーの登録、窓の操作はメインスレッドに集める。ホットキーの登録は
/// メインスレッドの応答を待つので、別スレッドがロックを持ったまま登録すると、同じロックを待つ
/// メインスレッドと互いに止まる。
fn show_launcher(app: &AppHandle, focus: Focus) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || show_launcher_on_main(&handle, focus));
}

fn show_launcher_on_main(app: &AppHandle, focus: Focus) {
    let Some(window) = app.get_webview_window(LAUNCHER) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if focus == Focus::Leave && !visible && !shows_without_key(app) {
        // 通常の窓は、出すだけでキー入力を奪う。知らせは、次に利用者が窓を出したときに見せる。
        return;
    }
    app.state::<AppState>().session.lock().unwrap().reload_settings();
    match launcher_position(app) {
        Some(Position::Logical { x, y }) => {
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
        Some(Position::Physical { x, y }) => {
            let _ = window.set_position(PhysicalPosition::new(x, y));
            // 拡大率の違うモニターへ移ると物理サイズが変わるので、論理サイズで指定し直す。
            let _ = window.set_size(LogicalSize::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
        }
        None => {}
    }
    present(app, &window, focus);
    // すでに出ている窓の内容を更新するだけなら、表示は続いている。番号を変えない。
    let state = app.state::<AppState>();
    let id = {
        let mut showing = state.showing.lock().unwrap();
        if visible { showing.id() } else { showing.begin() }
    };
    let _ = window.emit(EVENT_SHOWN, id);
    // コマンドの読み直しは窓を出したあとに行う (数百件あると、出るまでの時間に響く)。
    // ページが一覧を求めるのはこの関数を抜けたあとなので、一覧には読み直した結果が出る。
    sync_hotkeys(app);
}

/// キー入力を奪わずに窓を出せるか。パネルにできた macOS だけ。
fn shows_without_key(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    return app.state::<AppState>().panel.load(Ordering::Relaxed);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

/// 窓を出す。`Focus::Take` ならキー入力も受け取れるようにする。メインスレッドから呼ぶ。
fn present(app: &AppHandle, window: &WebviewWindow, focus: Focus) {
    #[cfg(target_os = "macos")]
    if app.state::<AppState>().panel.load(Ordering::Relaxed) {
        if let Ok(ns_window) = window.ns_window() {
            // SAFETY: この窓は起動時にパネルへ変えてあり、アプリが終わるまで閉じない。ここはメインスレッド。
            unsafe { panel::show(ns_window, focus == Focus::Take) };
        }
        return;
    }
    let _ = app;
    // 通常の窓は `show()` だけでキー入力を受け取る。`Focus::Leave` でここに来るのは、
    // すでに出ている窓の内容を更新する場合だけなので、何もしない。
    if focus == Focus::Take {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LAUNCHER) else {
        return;
    };
    let state = app.state::<AppState>();
    state.showing.lock().unwrap().end();
    state.has_key.store(false, Ordering::SeqCst);
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let _ = window.hide();
    // パネルにできなかった場合は Flintkey が前面のアプリになっている。窓を隠すだけでは
    // 元のアプリにキー入力が戻らない。
    #[cfg(target_os = "macos")]
    if !app.state::<AppState>().panel.load(Ordering::Relaxed) {
        let _ = app.hide();
    }
    let _ = window.emit(EVENT_HIDDEN, ());
}

/// 窓が出ていれば隠し、隠れていれば出す。どのスレッドからでも呼べる。
fn toggle_launcher(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let visible = handle
            .get_webview_window(LAUNCHER)
            .is_some_and(|window| window.is_visible().unwrap_or(false));
        if visible {
            hide_launcher(&handle);
        } else {
            show_launcher_on_main(&handle, Focus::Take);
        }
    });
}

/// フォーカスの通知が届いた時点で、窓がキー入力を持っているか。`reported` は通知の値。
///
/// macOS では通知が遅れて届く。その間に窓を出し直していると、古い「失った」通知で出したばかりの
/// 窓を隠してしまうので、今の状態を見る。他の OS では通知の値をそのまま使う (Windows では
/// 通知の対象は WebView、`is_focused()` の対象は親の窓で、同じものを指していない)。
fn holds_key_now(window: &tauri::Window, reported: bool) -> bool {
    if cfg!(target_os = "macos") {
        window.is_focused().unwrap_or(reported)
    } else {
        reported
    }
}

fn open_config_folder(app: &AppHandle) {
    let folder = app
        .state::<AppState>()
        .session
        .lock()
        .unwrap()
        .config_dir()
        .to_path_buf();
    // 初回は `commands` フォルダが無い。置き場所が分かるように作ってから開く。
    let result = std::fs::create_dir_all(folder.join("commands")).and_then(|()| open::that_detached(&folder));
    if let Err(error) = result {
        let notice = format!("could not open {}: {error}", folder.display());
        app.state::<AppState>().notices.lock().unwrap().push(notice);
        show_launcher(app, Focus::Leave);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Flintkey", true, None::<&str>)?;
    let config = MenuItem::with_id(app, "config", "Open Config Folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Flintkey", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &config, &PredefinedMenuItem::separator(app)?, &quit])?;

    let tray = TrayIconBuilder::with_id("main")
        .tooltip("Flintkey")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_launcher(app, Focus::Take),
            "config" => open_config_folder(app),
            "quit" => app.exit(0),
            _ => {}
        });
    // macOS のメニューバーは、明暗に合わせて色が変わるテンプレート画像にする。
    #[cfg(target_os = "macos")]
    let tray = tray
        .icon(tauri::include_image!("icons/tray-template.png"))
        .icon_as_template(true);
    // タスクトレイは左クリックで窓、右クリックでメニューが慣習。
    #[cfg(not(target_os = "macos"))]
    let tray = tray
        .icon(app.default_window_icon().cloned().expect("the bundle has an icon"))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_launcher(tray.app_handle(), Focus::Take);
            }
        });
    tray.build(app)?;
    Ok(())
}

fn main() {
    let home = std::env::home_dir().expect("the user has a home directory");
    let config_dir = paths::config_dir().expect("the user has a home directory");
    let state_dir = paths::state_dir().expect("the user has a home directory");
    let state = AppState {
        session: Mutex::new(Session::new(config_dir, &state_dir, home, Os::current())),
        hotkeys: Mutex::new(Registry::default()),
        notices: Mutex::new(Vec::new()),
        showing: Mutex::new(Showing::default()),
        has_key: AtomicBool::new(false),
        #[cfg(target_os = "macos")]
        panel: AtomicBool::new(false),
    };

    tauri::Builder::default()
        // 2 つ目を起動したら、常駐している方の窓を出す。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_launcher(app, Focus::Take)
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        // macOS のメディアキーでは、ライブラリが自分のロックを持ったままここを呼ぶ。
                        // この場でホットキーを登録し直すと同じロックを取りに行って止まるので、
                        // 別スレッドを経由する (窓の操作はそこからメインスレッドへ回す)。
                        let app = app.clone();
                        let shortcut = *shortcut;
                        std::thread::spawn(move || fire(&app, shortcut));
                    }
                })
                .build(),
        )
        .manage(state)
        .invoke_handler(tauri::generate_handler![open_session, search, run, hide])
        .setup(|app| {
            // Dock とアプリ切り替えに出さない。
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                let window = app
                    .get_webview_window(LAUNCHER)
                    .expect("the launcher window is configured");
                // SAFETY: setup はメインスレッドで動き、窓は作られたばかりで生きている。
                let converted = unsafe { panel::convert(window.ns_window()?) };
                app.state::<AppState>().panel.store(converted, Ordering::Relaxed);
            }
            build_tray(app.handle())?;
            app.state::<AppState>().session.lock().unwrap().reload_settings();
            sync_hotkeys(app.handle());
            // ランチャーのホットキーが使えないと窓を出す手段に気づけないので、最初に窓で知らせる。
            if !app.state::<AppState>().hotkeys.lock().unwrap().has_launcher() {
                show_launcher(app.handle(), Focus::Leave);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(true) => {
                let has_key = holds_key_now(window, true);
                window.state::<AppState>().has_key.store(has_key, Ordering::SeqCst);
            }
            WindowEvent::Focused(false) => {
                // キー入力を持っていた窓が、いま持っていないときだけ隠す。知らせで出した窓は
                // キー入力を持たないので、ここでは隠れない。
                let had_key = window.state::<AppState>().has_key.load(Ordering::SeqCst);
                if had_key && !holds_key_now(window, false) {
                    hide_launcher(window.app_handle());
                }
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_launcher(window.app_handle());
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("the application starts");
}
