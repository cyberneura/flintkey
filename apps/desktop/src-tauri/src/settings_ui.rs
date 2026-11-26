//! Commands for the settings window. Everything it changes is written to the YAML files through `Store`.
//!
//! These are synchronous commands, so they run on the main thread. That matters for saving: the hotkeys are
//! registered again right after a save, and registering must happen on the main thread (see `sync_hotkeys`).

use crate::forms::{CommandForm, SettingsForm};
use crate::hotkeys;
use crate::{AppState, sync_hotkeys};
use flintkey_core::launch::{self, Machine};
use flintkey_core::secrets::OnePassword;
use flintkey_core::store::{StoreError, StoredCommand};
use flintkey_core::{Command, Os, hotkey, parse_command, settings};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_global_shortcut::Shortcut;

/// Sent to the settings window when a command started with "Try" fails after it was planned.
pub const EVENT_TRY_FAILED: &str = "settings-try-failed";

type Problems = Vec<String>;

fn problems(error: StoreError) -> Problems {
    match error {
        StoreError::Invalid(found) => found.iter().map(ToString::to_string).collect(),
        other => vec![other.to_string()],
    }
}

#[derive(Serialize)]
pub struct SettingsPage {
    form: SettingsForm,
    /// The raw text of `settings.yaml` (empty if the file does not exist). Sent back with a save, so a file
    /// changed outside the window is not overwritten.
    source: String,
    /// Whether `settings.yaml` can be read. If not, the form shows the last settings that could be read, which
    /// are not what the file says, so the file has to be fixed as YAML.
    readable: bool,
    /// Problems with `settings.yaml` and with registering hotkeys.
    problems: Problems,
    config_dir: String,
    os: &'static str,
}

fn settings_page_of(state: &AppState) -> SettingsPage {
    let mut session = state.session.lock().unwrap();
    session.reload_settings();
    let mut found = session.problems();
    // Command problems are shown on the commands' own pages.
    found.retain(|problem| !problem.starts_with("commands/"));
    found.extend(
        state
            .hotkeys
            .lock()
            .unwrap()
            .problems
            .iter()
            .filter(|problem| !problem.starts_with("commands/"))
            .cloned(),
    );
    let source = session.store().read_settings_source().unwrap_or_default();
    SettingsPage {
        form: SettingsForm::from_settings(session.settings()),
        readable: settings::parse_settings(&source).is_ok(),
        source,
        problems: found,
        config_dir: session.config_dir().display().to_string(),
        os: session.os().name(),
    }
}

#[tauri::command]
pub fn settings_page(state: State<'_, AppState>) -> SettingsPage {
    settings_page_of(&state)
}

fn check_launcher_hotkey(settings: &settings::Settings) -> Result<(), Problems> {
    hotkeys::check(&settings.hotkey)
        .map(|_| ())
        .map_err(|reason| vec![format!("hotkey `{}`: {reason}", settings.hotkey)])
}

/// Refuses a save if the file is no longer the text the window loaded (it was edited outside the window).
/// Without this, the save would put back the window's old values over the outside edits.
fn refuse_changed(current: Option<&str>, base: Option<&str>, name: &str) -> Result<(), Problems> {
    match (current, base) {
        (None, None) => Ok(()),
        (Some(current), Some(base)) if current == base => Ok(()),
        (Some(_), None) => Err(vec![format!("{name} already exists")]),
        _ => Err(vec![format!(
            "{name} was changed outside this window since it was opened. Revert to load it again (unsaved changes \
             here will be lost), or copy them first."
        )]),
    }
}

fn read_if_exists(path: &Path) -> Result<Option<String>, Problems> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(vec![format!("could not read {}: {error}", path.display())]),
    }
}

fn settings_path(state: &AppState) -> std::path::PathBuf {
    state.session.lock().unwrap().config_dir().join("settings.yaml")
}

#[tauri::command]
pub fn settings_save(
    app: AppHandle,
    state: State<'_, AppState>,
    form: SettingsForm,
    base: String,
) -> Result<SettingsPage, Problems> {
    let settings = form.to_settings().map_err(|problem| vec![problem])?;
    check_launcher_hotkey(&settings)?;
    let current = read_if_exists(&settings_path(&state))?;
    refuse_changed(Some(current.as_deref().unwrap_or("")), Some(&base), "settings.yaml")?;
    if settings::parse_settings(current.as_deref().unwrap_or("")).is_err() {
        return Err(vec![
            "settings.yaml has problems; fix it with Edit as YAML first".into(),
        ]);
    }
    {
        let session = state.session.lock().unwrap();
        session.store().save_settings(&settings).map_err(problems)?;
    }
    state.session.lock().unwrap().reload_settings();
    sync_hotkeys(&app);
    Ok(settings_page_of(&state))
}

#[tauri::command]
pub fn settings_save_source(
    app: AppHandle,
    state: State<'_, AppState>,
    source: String,
    base: String,
) -> Result<SettingsPage, Problems> {
    let current = read_if_exists(&settings_path(&state))?;
    refuse_changed(Some(current.as_deref().unwrap_or("")), Some(&base), "settings.yaml")?;
    let parsed = settings::parse_settings(&source)
        .map_err(|found| found.iter().map(ToString::to_string).collect::<Problems>())?;
    check_launcher_hotkey(&parsed)?;
    {
        let session = state.session.lock().unwrap();
        session.store().save_settings_source(&source).map_err(problems)?;
    }
    state.session.lock().unwrap().reload_settings();
    sync_hotkeys(&app);
    Ok(settings_page_of(&state))
}

#[derive(Serialize, Default)]
pub struct HotkeyCheck {
    /// Why the hotkey cannot be used at all.
    error: Option<String>,
    /// Why it may not work as expected.
    warnings: Vec<String>,
}

/// Checks a hotkey while it is being chosen. `launcher` is true for the launcher's hotkey; otherwise `id` is the
/// command being edited (`None` for a command that has no file yet).
///
/// Runs off the main thread: it reads every command file and, on macOS, starts `plutil`, which would otherwise
/// hold up the launcher's hotkeys while the user clicks through combinations.
#[tauri::command(async)]
pub fn hotkey_check(state: State<'_, AppState>, hotkey: String, launcher: bool, id: Option<String>) -> HotkeyCheck {
    let mut out = HotkeyCheck::default();
    if hotkey.trim().is_empty() {
        return out;
    }
    let shortcut = match hotkeys::check(&hotkey) {
        Ok(shortcut) => shortcut,
        Err(reason) => {
            out.error = Some(reason);
            return out;
        }
    };
    let same = |other: &str| hotkeys::check(other).is_ok_and(|other: Shortcut| other == shortcut);
    let (store, os, launcher_hotkey) = {
        let session = state.session.lock().unwrap();
        (
            flintkey_core::store::Store::new(session.config_dir()),
            session.os(),
            session.settings().hotkey.clone(),
        )
    };
    if !launcher && same(&launcher_hotkey) {
        out.warnings
            .push("This is the launcher's hotkey; the launcher keeps it.".into());
    }
    if let Ok(loaded) = store.load_commands() {
        for stored in loaded.commands.iter().filter(|stored| stored.command.applies_to(os)) {
            if !launcher && Some(&stored.id) == id.as_ref() {
                continue;
            }
            if stored.command.hotkey.as_deref().is_some_and(same) {
                out.warnings.push(if launcher {
                    format!(
                        "The command `{}` has this hotkey too; the launcher takes it.",
                        stored.id
                    )
                } else {
                    format!("The command `{}` has this hotkey too.", stored.id)
                });
            }
        }
    }
    if os == Os::Macos
        && let Some(system) = hotkey::system_conflict(read_symbolic_hotkeys().as_ref(), &hotkey)
    {
        out.warnings.push(format!(
            "macOS uses this for \u{201c}{}\u{201d}. Turn that shortcut off in System Settings \u{2192} Keyboard \u{2192} \
             Keyboard Shortcuts, or Flintkey may not receive it.",
            system.name
        ));
    }
    out
}

/// macOS's own keyboard shortcuts, as JSON. `None` if they cannot be read (then macOS's defaults are assumed).
fn read_symbolic_hotkeys() -> Option<serde_json::Value> {
    let home = std::env::home_dir()?;
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(home.join("Library/Preferences/com.apple.symbolichotkeys.plist"))
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    serde_json::from_slice(&output.stdout).ok()
}

#[derive(Serialize)]
pub struct CommandSummary {
    id: String,
    title: String,
    /// The type, or empty for a file that cannot be read.
    kind: String,
    keywords: Vec<String>,
    hotkey: Option<String>,
    tags: Vec<String>,
    os: Vec<String>,
    broken: bool,
}

impl CommandSummary {
    fn of(stored: &StoredCommand) -> Self {
        let command = &stored.command;
        Self {
            id: stored.id.clone(),
            title: command.title.clone(),
            kind: command.action.type_name().to_string(),
            keywords: command.keywords.clone(),
            hotkey: command.hotkey.clone(),
            tags: command.tags.clone(),
            os: command.os.iter().map(|os| os.name().to_string()).collect(),
            broken: false,
        }
    }
}

/// Every command file, for every OS, in id order (broken files included, so they can be fixed here).
#[tauri::command]
pub fn commands_list(state: State<'_, AppState>) -> Result<Vec<CommandSummary>, Problems> {
    let loaded = state
        .session
        .lock()
        .unwrap()
        .store()
        .load_commands()
        .map_err(|error| vec![error.to_string()])?;
    let mut out: Vec<CommandSummary> = loaded.commands.iter().map(CommandSummary::of).collect();
    out.extend(loaded.broken.iter().map(|broken| CommandSummary {
        id: broken.id.clone(),
        title: broken.id.clone(),
        kind: String::new(),
        keywords: Vec::new(),
        hotkey: None,
        tags: Vec::new(),
        os: Vec::new(),
        broken: true,
    }));
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[derive(Serialize)]
pub struct CommandPage {
    id: String,
    /// `None` when the file cannot be read as a command; the page then only offers the raw YAML.
    form: Option<CommandForm>,
    source: String,
    /// Problems in the file, and problems registering its hotkey.
    problems: Problems,
}

fn command_page_of(state: &AppState, id: &str) -> Result<CommandPage, Problems> {
    let source = state
        .session
        .lock()
        .unwrap()
        .store()
        .read_command_source(id)
        .map_err(problems)?;
    let (form, mut found) = match parse_command(&source) {
        Ok(command) => (Some(CommandForm::from_command(&command)), Vec::new()),
        Err(found) => (None, found.iter().map(ToString::to_string).collect()),
    };
    let prefix = format!("commands/{id}.yaml: ");
    found.extend(
        state
            .hotkeys
            .lock()
            .unwrap()
            .problems
            .iter()
            .filter_map(|problem| problem.strip_prefix(&prefix))
            .map(str::to_string),
    );
    Ok(CommandPage {
        id: id.to_string(),
        form,
        source,
        problems: found,
    })
}

#[tauri::command]
pub fn command_page(state: State<'_, AppState>, id: String) -> Result<CommandPage, Problems> {
    command_page_of(&state, &id)
}

/// The text saving `form` would write, for the YAML view of a form that has not been saved.
#[tauri::command]
pub fn command_preview(state: State<'_, AppState>, id: String, form: CommandForm) -> Result<String, Problems> {
    let command = form.to_command().map_err(|problem| vec![problem])?;
    state
        .session
        .lock()
        .unwrap()
        .store()
        .preview_command(&id, &command)
        .map_err(problems)
}

/// Reads raw YAML into a form, for switching from the YAML view back to the form.
#[tauri::command]
pub fn command_parse(source: String) -> Result<CommandForm, Problems> {
    parse_command(&source)
        .map(|command| CommandForm::from_command(&command))
        .map_err(|found| found.iter().map(ToString::to_string).collect())
}

/// Checks what can only be checked here before a command is written: that the file is still the one the window
/// loaded (`base`; `None` for a new command, whose id must be free) and that its hotkey can be registered.
fn check_before_saving(state: &AppState, id: &str, base: Option<&str>, command: &Command) -> Result<(), Problems> {
    let path = state
        .session
        .lock()
        .unwrap()
        .store()
        .command_path(id)
        .map_err(problems)?;
    let current = read_if_exists(&path)?;
    refuse_changed(current.as_deref(), base, &format!("commands/{id}.yaml"))?;
    if let Some(hotkey) = &command.hotkey {
        hotkeys::check(hotkey).map_err(|reason| vec![format!("hotkey `{hotkey}`: {reason}")])?;
    }
    Ok(())
}

/// Saves a command edited in the form. `base` is the file text the window loaded, or `None` for a new command.
#[tauri::command]
pub fn command_save(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    form: CommandForm,
    base: Option<String>,
) -> Result<CommandPage, Problems> {
    let command = form.to_command().map_err(|problem| vec![problem])?;
    check_before_saving(&state, &id, base.as_deref(), &command)?;
    {
        let session = state.session.lock().unwrap();
        session.store().save_command(&id, &command).map_err(problems)?;
    }
    sync_hotkeys(&app);
    command_page_of(&state, &id)
}

#[tauri::command]
pub fn command_save_source(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    source: String,
    base: Option<String>,
) -> Result<CommandPage, Problems> {
    let command =
        parse_command(&source).map_err(|found| found.iter().map(ToString::to_string).collect::<Problems>())?;
    check_before_saving(&state, &id, base.as_deref(), &command)?;
    {
        let session = state.session.lock().unwrap();
        session.store().save_command_source(&id, &source).map_err(problems)?;
    }
    sync_hotkeys(&app);
    command_page_of(&state, &id)
}

#[tauri::command]
pub fn command_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), Problems> {
    {
        let session = state.session.lock().unwrap();
        session.store().delete_command(&id).map_err(problems)?;
    }
    sync_hotkeys(&app);
    Ok(())
}

/// Runs the saved command once, without recording a use. A failure to plan is returned; a failure to start
/// arrives later as [`EVENT_TRY_FAILED`].
#[tauri::command]
pub fn command_try(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), Problems> {
    let plan = {
        let session = state.session.lock().unwrap();
        let source = session.store().read_command_source(&id).map_err(problems)?;
        let command =
            parse_command(&source).map_err(|found| found.iter().map(ToString::to_string).collect::<Problems>())?;
        let home = std::env::home_dir().ok_or_else(|| vec!["no home directory".to_string()])?;
        let shell = std::env::var("SHELL").ok();
        let path = std::env::var("PATH").ok();
        let machine = Machine {
            os: session.os(),
            home: &home,
            shell: shell.as_deref(),
            path: path.as_deref(),
            terminal: session.settings().terminal,
        };
        launch::plan(&command.action, &machine).map_err(|error| vec![error.to_string()])?
    };
    // 1Password may ask to unlock; do not hold up the main thread.
    std::thread::spawn(move || {
        if let Err(error) = launch::execute(&plan, &OnePassword::locate()) {
            let _ = app.emit_to("settings", EVENT_TRY_FAILED, format!("`{id}` did not start: {error}"));
        }
    });
    Ok(())
}

/// Shows a command file (or the configuration folder, without an id) in Finder / Explorer.
#[tauri::command]
pub fn reveal(state: State<'_, AppState>, id: Option<String>) -> Result<(), Problems> {
    let session = state.session.lock().unwrap();
    let target = match &id {
        Some(id) => session.store().command_path(id).map_err(problems)?,
        None => {
            let folder = session.config_dir().to_path_buf();
            std::fs::create_dir_all(folder.join("commands")).map_err(|error| vec![error.to_string()])?;
            folder
        }
    };
    reveal_path(&target, id.is_some()).map_err(|error| vec![error.to_string()])
}

fn reveal_path(path: &Path, select: bool) -> std::io::Result<()> {
    if !select {
        return open::that_detached(path);
    }
    if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(path)
            .spawn()?;
        Ok(())
    } else if cfg!(target_os = "windows") {
        let mut select_arg = std::ffi::OsString::from("/select,");
        select_arg.push(path);
        std::process::Command::new("explorer").arg(select_arg).spawn()?;
        Ok(())
    } else {
        open::that_detached(path.parent().unwrap_or(path))
    }
}

/// Opens System Settings at Keyboard, where macOS's own shortcuts (Spotlight, input sources) are changed.
#[tauri::command]
pub fn open_keyboard_settings() -> Result<(), Problems> {
    open::that_detached("x-apple.systempreferences:com.apple.Keyboard-Settings.extension")
        .map_err(|error| vec![error.to_string()])
}

/// Opens the settings window. Used by the launcher (⌘, / Ctrl+,).
#[tauri::command]
pub fn open_settings(app: AppHandle) {
    crate::show_settings(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_refused_when_the_file_is_not_the_one_the_window_loaded() {
        // Arrange / Act / Assert
        assert_eq!(refuse_changed(Some("a"), Some("a"), "x"), Ok(()));
        assert_eq!(refuse_changed(None, None, "x"), Ok(()));
        assert_eq!(
            refuse_changed(Some("a"), None, "x"),
            Err(vec!["x already exists".to_string()])
        );
        for (current, base) in [(Some("b"), Some("a")), (None, Some("a"))] {
            let refused = refuse_changed(current, base, "x");
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|found| found[0].starts_with("x was changed outside this window")),
                "{current:?} {base:?}: {refused:?}"
            );
        }
    }

    #[test]
    fn store_validation_problems_are_listed_one_by_one() {
        // Arrange
        let found = parse_command("type: x\n").unwrap_err();
        let error = StoreError::Invalid(found);

        // Act
        let listed = problems(error);

        // Assert
        assert_eq!(
            listed,
            [
                "`title` is required",
                "line 1, column 7: unknown type `x` (expected one of: url, app, script, terminal)"
            ]
        );
    }
}
