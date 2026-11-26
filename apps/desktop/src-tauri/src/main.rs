//! The Flintkey desktop app. Lives in the menu bar / system tray and shows the launcher window on a hotkey.

// Do not open a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod forms;
mod hotkeys;
#[cfg(target_os = "macos")]
mod panel;
mod placement;
mod session;
mod settings_ui;
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
const SETTINGS: &str = "settings";
/// Same as the window size in `tauri.conf.json` (logical size).
const WINDOW_SIZE: (f64, f64) = (720.0, 480.0);

const EVENT_SHOWN: &str = "launcher-shown";
const EVENT_HIDDEN: &str = "launcher-hidden";
const EVENT_SETTINGS_SHOWN: &str = "settings-shown";

struct AppState {
    session: Mutex<Session>,
    hotkeys: Mutex<Registry>,
    /// Failures that happened after the window was hidden. Shown the next time the window appears.
    notices: Mutex<Vec<String>>,
    showing: Mutex<Showing>,
    /// Whether the window is the target of keyboard input.
    has_key: AtomicBool,
    /// Whether the window could be turned into a panel that receives keyboard input.
    #[cfg(target_os = "macos")]
    panel: AtomicBool,
}

/// The hotkeys that are registered.
#[derive(Default)]
struct Registry {
    /// Shortcut -> what happens when it is pressed, and the text written in the settings.
    registered: Registered,
    /// Problems from the last re-registration (unreadable, conflicting, could not be registered).
    problems: Vec<String>,
}

impl Registry {
    fn has_launcher(&self) -> bool {
        self.registered.values().any(|(target, _)| *target == Target::Launcher)
    }
}

/// What is passed to the page each time the window is shown.
#[derive(Serialize)]
struct View {
    rows: Vec<Row>,
    problems: Vec<String>,
    config_dir: String,
    /// The number of the current showing. The page attaches it to run requests.
    showing: u64,
}

#[tauri::command]
fn open_session(state: State<'_, AppState>, query: String) -> View {
    let mut problems = std::mem::take(&mut *state.notices.lock().unwrap());
    problems.extend(state.hotkeys.lock().unwrap().problems.iter().cloned());
    // Commands were already reloaded when the window was shown (`sync_hotkeys`).
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

/// Runs a command. If no plan can be made, returns the error and keeps the window open.
/// Otherwise hides the window and launches on another thread (so waiting for 1Password does not freeze the window).
#[tauri::command]
fn run(app: AppHandle, state: State<'_, AppState>, id: String, showing: u64) -> Result<(), String> {
    // Repeated Enter presses or clicks send several requests. The window hides on the first one, so the
    // rest are for a list the user no longer sees. Accept only the first.
    if !state.showing.lock().unwrap().claim(showing) {
        return Ok(());
    }
    let plan = match plan_command(&state, &id) {
        Ok(plan) => plan,
        Err(message) => {
            // Keep the window open so the user can pick another command.
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

/// Executes the plan and records the use. Failures are shown as notices. May wait for 1Password
/// authentication, so do not call it from the main thread.
fn execute_and_record(app: &AppHandle, id: &str, plan: &Plan) {
    let state = app.state::<AppState>();
    match launch::execute(plan, &OnePassword::locate()) {
        Ok(()) => {
            // The command has already started. Do not bother the user over a failure to record it.
            if let Err(error) = state.session.lock().unwrap().record_use(id) {
                eprintln!("could not record the use of `{id}`: {error}");
            }
        }
        Err(error) => notify(app, format!("`{id}` did not start: {error}")),
    }
}

/// Queues a notice and shows it in the window without taking keyboard input.
fn notify(app: &AppHandle, notice: String) {
    app.state::<AppState>().notices.lock().unwrap().push(notice);
    show_launcher(app, Focus::Leave);
}

/// A hotkey was pressed. Call off the main thread (running a command may wait for 1Password authentication).
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

/// Reloads commands and re-registers the launcher and command hotkeys. Call from the main thread
/// (registration waits on the main thread, so calling it from another thread while holding a lock can deadlock).
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

/// Decides where to put the window according to the settings. `None` if monitor information is unavailable.
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

/// Whether showing the window also makes it the target of keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The user asked for the window. It receives keyboard input.
    Take,
    /// The app shows a notice on its own. The user may be typing in another app, so do not take keyboard input
    /// (otherwise an Enter meant for the other app would run a command).
    Leave,
}

/// Shows the window. Can be called from any thread.
///
/// Applying settings, registering hotkeys and window operations all run on the main thread. Hotkey
/// registration waits on the main thread, so registering from another thread while holding a lock
/// deadlocks with the main thread waiting for the same lock.
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
        // A normal window takes keyboard input just by appearing. The notice waits for the next showing.
        return;
    }
    app.state::<AppState>().session.lock().unwrap().reload_settings();
    match launcher_position(app) {
        Some(Position::Logical { x, y }) => {
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
        Some(Position::Physical { x, y }) => {
            let _ = window.set_position(PhysicalPosition::new(x, y));
            // Moving to a monitor with a different scale changes the physical size, so set the logical size again.
            let _ = window.set_size(LogicalSize::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
        }
        None => {}
    }
    present(app, &window, focus);
    // Refreshing an already visible window continues the same showing. Keep its number.
    let state = app.state::<AppState>();
    let id = {
        let mut showing = state.showing.lock().unwrap();
        if visible { showing.id() } else { showing.begin() }
    };
    let _ = window.emit(EVENT_SHOWN, id);
    // Reload commands after showing the window (with hundreds of them, it delays the window).
    // The page asks for the list only after this function returns, so the list reflects the reload.
    sync_hotkeys(app);
}

/// Whether the window can be shown without taking keyboard input. Only on macOS, when the panel conversion worked.
fn shows_without_key(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    return app.state::<AppState>().panel.load(Ordering::Relaxed);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

/// Shows the window. With `Focus::Take` it also receives keyboard input. Call from the main thread.
fn present(app: &AppHandle, window: &WebviewWindow, focus: Focus) {
    #[cfg(target_os = "macos")]
    if app.state::<AppState>().panel.load(Ordering::Relaxed) {
        if let Ok(ns_window) = window.ns_window() {
            // SAFETY: this window became a panel at startup and stays open until the app exits.
            // This is the main thread.
            unsafe { panel::show(ns_window, focus == Focus::Take) };
        }
        return;
    }
    let _ = app;
    // A normal window receives keyboard input from `show()` alone. `Focus::Leave` only gets here when
    // refreshing an already visible window, so do nothing.
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
    // Without the panel, Flintkey is the frontmost app. Hiding the window alone does not return
    // keyboard input to the previous app.
    #[cfg(target_os = "macos")]
    if !app.state::<AppState>().panel.load(Ordering::Relaxed) {
        let _ = app.hide();
    }
    let _ = window.emit(EVENT_HIDDEN, ());
}

/// Hides the window if visible, shows it if hidden. Can be called from any thread.
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

/// Whether the window holds keyboard input when a focus event arrives. `reported` is the event's value.
///
/// On macOS the event arrives late. If the window was shown again in the meantime, a stale "lost" event
/// would hide the freshly shown window, so check the current state. Other OSes use the event's value as is
/// (on Windows the event is about the WebView and `is_focused()` about the parent window; they differ).
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
    // On first run there is no `commands` folder. Create it before opening so users see where commands go.
    let result = std::fs::create_dir_all(folder.join("commands")).and_then(|()| open::that_detached(&folder));
    if let Err(error) = result {
        let notice = format!("could not open {}: {error}", folder.display());
        app.state::<AppState>().notices.lock().unwrap().push(notice);
        show_launcher(app, Focus::Leave);
    }
}

/// Shows the settings window. Can be called from any thread.
///
/// On macOS the app normally stays out of the Dock and the app switcher. While the settings window is open it
/// behaves like a regular app, so the window can be found again with ⌘Tab and gets keyboard input.
fn show_settings(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        hide_launcher(&handle);
        let Some(window) = handle.get_webview_window(SETTINGS) else {
            return;
        };
        #[cfg(target_os = "macos")]
        let _ = handle.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(EVENT_SETTINGS_SHOWN, ());
    });
}

fn hide_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Flintkey", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings\u{2026}", true, None::<&str>)?;
    let config = MenuItem::with_id(app, "config", "Open Config Folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Flintkey", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show, &settings, &config, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .tooltip("Flintkey")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_launcher(app, Focus::Take),
            "settings" => show_settings(app),
            "config" => open_config_folder(app),
            "quit" => app.exit(0),
            _ => {}
        });
    // Use a template image in the macOS menu bar so its color follows light/dark mode.
    #[cfg(target_os = "macos")]
    let tray = tray
        .icon(tauri::include_image!("icons/tray-template.png"))
        .icon_as_template(true);
    // In system trays, left click conventionally opens the window and right click the menu.
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
        // Launching a second instance shows the window of the running one.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_launcher(app, Focus::Take)
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        // For macOS media keys, the library calls this while holding its own lock.
                        // Re-registering hotkeys here would wait for that same lock and hang, so go through
                        // another thread (which hands window operations to the main thread).
                        let app = app.clone();
                        let shortcut = *shortcut;
                        std::thread::spawn(move || fire(&app, shortcut));
                    }
                })
                .build(),
        )
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            open_session,
            search,
            run,
            hide,
            settings_ui::settings_page,
            settings_ui::settings_save,
            settings_ui::settings_save_source,
            settings_ui::hotkey_check,
            settings_ui::commands_list,
            settings_ui::command_page,
            settings_ui::command_preview,
            settings_ui::command_parse,
            settings_ui::command_save,
            settings_ui::command_save_source,
            settings_ui::command_delete,
            settings_ui::command_try,
            settings_ui::reveal,
            settings_ui::open_keyboard_settings,
            settings_ui::open_settings,
        ])
        .setup(|app| {
            // Keep the app out of the Dock and the app switcher.
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                let window = app
                    .get_webview_window(LAUNCHER)
                    .expect("the launcher window is configured");
                // SAFETY: setup runs on the main thread, and the window was just created and is alive.
                let converted = unsafe { panel::convert(window.ns_window()?) };
                app.state::<AppState>().panel.store(converted, Ordering::Relaxed);
            }
            build_tray(app.handle())?;
            app.state::<AppState>().session.lock().unwrap().reload_settings();
            sync_hotkeys(app.handle());
            // Without a working launcher hotkey the user has no obvious way to open the window, so tell them up front.
            if !app.state::<AppState>().hotkeys.lock().unwrap().has_launcher() {
                show_launcher(app.handle(), Focus::Leave);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == SETTINGS => {
                api.prevent_close();
                hide_settings(window.app_handle());
            }
            // Everything below is about the launcher.
            _ if window.label() != LAUNCHER => {}
            WindowEvent::Focused(true) => {
                let has_key = holds_key_now(window, true);
                window.state::<AppState>().has_key.store(has_key, Ordering::SeqCst);
            }
            WindowEvent::Focused(false) => {
                // Hide only when a window that had keyboard input no longer has it. A window shown for a notice
                // never had keyboard input, so it does not hide here.
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
