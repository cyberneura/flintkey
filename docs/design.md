# Flintkey design

A keyboard launcher for the same purpose as Alfred / Raycast. What sets it apart is **ease of configuration**:
the configuration is kept as files, can be shared across machines with Git, and the same content can also be edited from a UI.

## Status

| Part | Status |
|---|---|
| Core (`crates/flintkey-core`): reading and writing configuration, validation, actions, secrets, search | Implemented |
| Desktop app (`apps/desktop`): background process, tray, hotkey, launcher window | Implemented |
| Per-command hotkeys | Implemented |
| Migration from Alfred (`import-alfred` in the CLI) | Implemented |
| Widgets (above the launcher window) | Not implemented |
| Settings window (general settings, launcher hotkey, commands) | Implemented |
| Plugin system, development-starter plugin | Not implemented |

Not yet confirmed on real hardware:

- All Windows behavior (running scripts with PowerShell, Windows Terminal, launching apps).
  The assembled command lines are fixed by tests, but they have never been run on a real Windows machine.
- Resolving real 1Password secrets. The input and output format of `op inject` has been confirmed,
  and resolution itself is tested with a stand-in `op`.
- The Japanese IME has been checked on macOS (the display during composition, and that the Enter that commits does not run a command). Windows is not checked.
- The following points of the desktop app (macOS):
  - Key input in a window shown on an external display (the placement coordinates have been checked with a 5-display setup).
  - Appearing on top of a full-screen app.
  - Behavior in a signed (Hardened Runtime) `.app`. In particular, the Apple Events used by `type: terminal`.
  - The tray menu, Esc, and clicks outside the window.
  - Which app responds when both Alfred and Flintkey have the same hotkey.

## Layout

```
flintkey/
├── crates/flintkey-core/   Core that does not depend on a rendering engine. Everything can be tested without a window
├── apps/desktop/           Desktop app (Tauri 2 + Svelte 5 + Tailwind 4). A thin layer that calls the core
│   ├── src/                The launcher window's page
│   └── src-tauri/          Background process, tray, hotkey, window placement
├── docs/design.md
└── spikes/                 Experiments that design decisions were based on. Not part of the product
```

## UI engine: Tauri 2

Tauri 2 was chosen after comparing it with GPUI (gpui-kit). `spikes/` has a minimal launcher and measurement scripts for both.

Measurements on a MacBook (Apple M2, 60Hz) (median):

| Item | GPUI (gpui-kit 0.10.0) | Tauri 2.12.6 |
|---|---|---|
| Re-show a hidden window → drawn | 67–91 ms | 41–42 ms |
| Close and recreate the window → drawn | 34–67 ms | Not measured |
| Key input → result drawn (1000 candidates) | 16–33 ms | 15–16 ms |
| Idle memory (total RSS of all processes) | 84–98 MB | 166–168 MB |

The GPUI times are ranges because the completion of drawing cannot be observed directly. The lower bound was measured at "the start of the frame that draws the change",
and the upper bound at "the start of the next frame" (by then drawing and display are done). Tauri is measured at the point where the main thread's
drawing work is done (only for re-showing, at the point where that notification reaches the Rust side).
Neither is the point where the pixels appear on screen.

Reasons for the choice:

1. Within what was measured, GPUI did not come out faster. Re-showing is faster with Tauri,
   and the key input response is within one 60Hz frame with Tauri, 1–2 frames with GPUI.
2. GPUI has no per-window hide / show, and on Windows the per-app hide / activate is an empty implementation.
   Supporting Windows would need our own Win32 code.
3. Using a component set with GPUI means depending on `gpui-kit`, which is a re-publication of a Zed snapshot
   by a single individual, and its minor version goes up every week.
4. Tauri has standard APIs for global hotkeys, the tray, the cursor position and getting the displays.
5. The settings UI (form and YAML edited side by side) is easier to build with web components.

Weaknesses that come from choosing Tauri:

- About 70 MB more memory (the WebView helper processes stay resident).
- There are reports of a white flash and hangs on `show()` on Windows. Needs checking on a real Windows machine.
- Two languages: Rust and TypeScript.

## Background process and launcher window

### Background process

- Stays resident with an icon in the menu bar on macOS and in the task tray on Windows. On macOS it does not appear in the Dock or the app switcher.
- Tray menu: Show Flintkey / Settings… / Open Config Folder / Quit Flintkey.
  On Windows, left-clicking the icon shows the window and right-clicking shows the menu.
- Pressing the `hotkey` in `settings.yaml` shows the window. Pressing it while the window is shown hides it.
- Pressing a command file's `hotkey` runs that command without showing the window.
  If it fails, the window reports it (handled the same way as in "Error display").
- Starting a second instance shows the window of the resident one (no multiple instances).
- If the hotkey cannot be registered (wrong format, used by another app), the window is shown at startup with the reason.
  The window can still be shown from the tray menu in the meantime.

### Window

- Frameless. No title bar (no macOS traffic-light buttons). The window itself is transparent; only the rounded panel is visible.
- The display to show it on can be chosen in the settings.
  - `cursor`: the display that has the mouse cursor when the hotkey is pressed (default)
  - `primary`: the main display
- The display is determined each time the window is shown. Horizontally, it is centered in the work area. Vertically, 22% of the
  free space (work area height − window height) is placed above it.
- The window is hidden, not closed. It hides on Esc, on a click outside the window, and when it loses focus.
  When hidden, key input goes back to the previous app.

On macOS, the window is made a "panel that receives key input without bringing the app to the front" (NSPanel, nonactivating)
(`apps/desktop/src-tauri/src/panel.rs`). macOS does not allow an app to bring itself to the front in response to a hotkey, so
a normal window cannot receive key input even when shown. Tauri cannot create an NSPanel, so the class of the created NSWindow
is swapped at runtime. On an OS where NSPanel and NSWindow differ in size, the class is not swapped and the window works as a normal window
(in that case the app is brought to the front when showing, and the whole app is hidden when hiding).

The position calculation (`apps/desktop/src-tauri/src/placement.rs`) has different coordinate systems on each OS:

| OS | Coordinates consistent across the whole screen | Values returned by the window API |
|---|---|---|
| macOS | Points (logical coordinates) | Monitor position and size are "points × that monitor's scale factor"; the cursor is "points × the main monitor's scale factor" |
| Windows | Physical coordinates | As is |

On macOS, the values are converted back to points for the calculation, and the window is placed in logical coordinates. When monitors with different scale factors are side by side,
using the returned values as they are gets wrong which monitor the cursor is on.

### Colors

Dark theme only. The OS appearance setting is not followed (`theme: "Dark"` in `tauri.conf.json`, `color-scheme: dark` on the page).

A white background is never shown, not even for a moment.

- The window and page backgrounds are transparent; only the dark panel has a color. Before the page is drawn, nothing is visible.
- Transparency is specified directly in `index.html`. It does not wait for the stylesheet to load.
- The window is created hidden. The page usually finishes loading during that time. Even if the window is shown before it finishes,
  it is transparent, so nothing is visible.

### Controls

| Key | Action |
|---|---|
| Typing | Filter |
| ↓ / Tab / Ctrl+N | Next candidate (after the last comes the first) |
| ↑ / Shift+Tab / Ctrl+P | Previous candidate |
| Enter | Run and hide the window |
| Esc | Hide the window |

- Keys during IME composition (including the Enter that commits) are not treated as launcher controls.
- A candidate row can also be run by clicking it.
- Enter runs only after the search results for the typed text are reflected in the list.
- Only one item is run per showing. A second or later request from repeated Enter presses or successive clicks is discarded.
- Each run request carries the showing's number. A request from an earlier showing that arrives after the window is closed and reopened is discarded.

### When things are reloaded

The configuration folder is watched (`apps/desktop/src-tauri/src/watch.rs`, with the `notify` crate). When
`settings.yaml` or a file in `commands/` changes, whether from an editor, `git pull` or the settings window, the app
loads the files again, registers the hotkeys again, and refreshes the open windows. Nothing has to be reopened.

- Changes are applied once the folder has been quiet for 300 ms, so a save that touches a file several times, or a
  pull that changes hundreds of files, is loaded once.
- Hidden files (the app's own temporary files, editor swap files), files in subfolders and other files are ignored,
  as they are when loading.
- A configuration folder reached through a symlink (for example kept in a Git checkout) is watched at its real path.
  A `settings.yaml` or command file that is itself a symlink to a file outside the folder is not watched.
- The folder and its `commands` folder are created at startup if they are missing, so they can be watched.
- If watching cannot start, a notice says so and the files are still loaded each time the launcher is shown.

The files are also loaded again each time the launcher is shown (`settings.yaml` just before, `commands/*.yaml` and
usage counts just after, since with several hundred commands loading would delay the window).

When hotkeys overlap, the launcher's takes priority, and between commands the one that comes first by id is used.
Different spellings of the same key combination count as an overlap (`Ctrl+Alt+C` and `ctrl+alt+KeyC`).
Unreadable, overlapping and unregistrable hotkeys are shown as errors.
However, on macOS, registering a hotkey that another app uses can succeed (confirmed with Alfred).
Hotkeys that could not be registered are tried again the next time hotkeys are registered.

### Error display

Errors are shown in a red band at the bottom of the window.

- Broken command files (`commands/x.yaml: line 3, column 1: ...`). Valid files are not hidden.
- Problems in `settings.yaml`. While it cannot be read, the last settings that could be read stay in use.
- The reason a hotkey is unreadable, overlapping or unregistrable.
- Run failures. If a plan cannot be made (for example, a type that is not available on that OS), it is shown with the window kept open.
  If launching fails (the process cannot be started, 1Password does not return the secret), the hidden window is shown again to report it.
  In this case key input is not taken (the user may be typing in another app). The notification window
  hides when its transparent area is clicked, or when the panel is clicked and then Esc is pressed. If the hotkey is usable, the hotkey also hides it.
  The window that reports hotkey problems at startup is handled the same way.
  Only the macOS panel can show a window without taking key input, so on Windows the notification window is not shown;
  the error is shown the next time the user shows the window.

Launching is done on a separate thread. While waiting for 1Password authentication, the window stays hidden and does not block anything.

## Widgets (planned)

Allow parts that show information to be placed above the input field of the launcher window.

- Kinds: clock, CPU usage, CPU temperature, memory usage, file contents, script output.
- Which ones to place, and in what order, is set in a configuration file.

Undecided:

- Where the settings live. Some things should differ per machine (such as whether a temperature sensor exists), so
  whether to put them in `settings.yaml`, which is shared with Git, or in a separate file.
- How often to update, and whether to stop updating while the window is hidden.
- How to get the CPU temperature. macOS (Apple Silicon) has no public API for it.

## Configuration folder

```
~/.config/flintkey/            The same on every OS (on Windows, %USERPROFILE%\.config\flintkey)
├── settings.yaml
└── commands/
    ├── open-github.yaml       One command per file. The file name (without the extension) is the id
    └── deploy-blog.yaml
```

- Only `*.yaml` directly under `commands/` is read. Files starting with `.`, `.yml` files and subfolders are ignored.
- The folder can be replaced with the environment variable `FLINTKEY_CONFIG_DIR`.

Rules that make Git conflicts less likely:

| Rule | Reason |
|---|---|
| One command per file | Two machines adding different commands do not touch the same file |
| No ordering in the configuration | An ordered list is the first thing to conflict. Ranking is computed at search time |
| State such as usage counts lives outside the configuration folder | Avoids a diff on every launch |
| On save, only keys whose meaning changed are rewritten | Keeps hand-written comments, blank lines and quotes, and keeps the diff minimal |
| New keys are inserted at a position given by a fixed order | When two machines add different keys, the changed lines are less likely to overlap |
| New files use a fixed format (fixed key order, indent 2, LF, trailing newline) | The text is the same no matter who creates it |

State is kept in `~/.local/state/flintkey/` (on Windows, `%LOCALAPPDATA%\flintkey\`).
It can be replaced with the environment variable `FLINTKEY_STATE_DIR`.

## Command files

```yaml
title: Deploy blog
keyword: deploy
hotkey: ctrl+alt+cmd+d
type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  GITHUB_TOKEN: op://Private/GitHub/token
pin: true
tags: [blog]
when:
  os: [macos, linux]
```

Common keys:

| Key | Required | Description |
|---|---|---|
| `title` | Required | The name shown in the list |
| `type` | Required | `url` / `app` / `script` / `terminal` |
| `keyword` | | A short word that is chosen first when typed as an exact match. For aliases, use a list (`[today, 今日]`) |
| `hotkey` | | A global shortcut that runs this command without showing the window (format below) |
| `pin` | | If `true`, the command is listed first when the search field is empty |
| `tags` | | Categories. A string for one, a list for several. Used for filtering and grouping in the settings UI. Not searched |
| `when.os` | | The OSes it is shown on. `macos` / `windows` / `linux`. A string for one, a list for several. All OSes if omitted |

Keys per `type`:

| type | Keys | Description |
|---|---|---|
| `url` | `url` (required) | A URL with a scheme. Besides `https://`, custom schemes such as `claude://` are allowed |
| `app` | `app` (required), `args` | An app name or path. `args` is a list of strings |
| `script` | `run` (required), `cwd`, `env` | A command run in a shell. `env` values can be `op://` references |
| `terminal` | `run` (required), `cwd` | A command run in a newly opened terminal app |

`hotkey` format: modifier keys and a key joined with `+`. Case-insensitive.

- Modifier keys: `ctrl` (`control`), `alt` (`option`), `shift`, `cmd` (`command`, `super`; the Windows key on Windows)
- Keys: letters, digits, `space`, `enter`, `tab`, `escape`, `f1`–`f24`, arrows (`up` etc.), symbols (`quote`, `comma`, `slash` etc.)

The `hotkey` in `settings.yaml` uses the same format. The format is validated by the desktop app (the core only keeps it as a string).
Command hotkeys need a modifier key (only F keys can be used alone). Some OSes allow registering `a` without a modifier,
and if that registration succeeds, the command would run every time `a` is typed in another app.

`/` in `tags` has no special meaning. Commands migrated from Alfred get the workflow name (such as `Work/URLs`) as a single tag, unchanged.

Validation:

- Empty values and duplicates in `keyword` and `tags` are errors.
- Unknown keys, values of the wrong type, and keys that do not fit the `type` (such as `cwd` with `type: url`) are errors.
  A key without a value (`cwd:`) is treated the same as a missing key. `env` values may be empty.
- Rule violations (a missing required key, a key that does not fit the `type`, an empty value in `title` `keyword` `url` `app` `run` `cwd`,
  a URL without a scheme, an unknown OS name, an invalid environment variable name) are all returned. Those with a known position get a line and column (some, such as a missing required key, have no position).
- If the file cannot be read as YAML, has an unknown key, or has a value of the wrong type, only the first problem is
  returned, with its line and column.
- One broken file does not stop the other commands from loading.
- Files are read as YAML 1.2. `title: no` and `keyword: on` become strings. Values that look like numbers are kept
  exactly as written (`keyword: 1.0` stays `1.0`, `keyword: +1` stays `+1`).
  For keys that expect a boolean, such as `pin: yes`, `yes` / `no` / `on` / `off` / `y` / `n` and quoted `"true"` / `"false"`
  are also accepted as booleans.

## settings.yaml

```yaml
hotkey: alt+space
window:
  display: cursor
terminal:
  app: terminal
```

| Key | Default | Description |
|---|---|---|
| `hotkey` | `alt+space` | The global shortcut that shows the launcher |
| `window.display` | `cursor` | `cursor` / `primary` |
| `terminal.app` | OS default | `terminal` (Terminal.app on macOS) / `windows-terminal` |

If the file is missing or empty, the defaults are used.

## Saving

Saving from the form (`Store::save_command`) does one of the following to the file.

| Result | Condition |
|---|---|
| `Unchanged` | The file already has the same meaning. Nothing is written |
| `Created` | There was no file. It is created in the fixed format |
| `Edited` | Only the keys whose meaning changed were rewritten. Comments and formatting are kept |
| `Rewritten` | The whole file was rewritten in the fixed format. Comments are lost |

- Keys are compared by meaning, not by text. Keys with the same value are not written
  (writing them would normalize the quoting and produce unrelated diffs).
- The result of the in-place edit is read back and validated; if it does not have the same meaning as the requested command, the save switches to `Rewritten`.
  It is also `Rewritten` if the original file was not a valid command.
- Saving from raw YAML (`Store::save_command_source`) rejects text that is not a valid command.
- Saving from the form reads the text back before writing, and rejects the save if it cannot be read as the same command
  (so that a command with an invalid value, such as an empty title, does not break the file).
- Writes go to a temporary file, which is then renamed.
- The id becomes the file name, so ids containing a path separator, `:`, or a leading `.` are rejected.

Reading uses `serde-saphyr`, and in-place editing uses `yaml-edit`.

## Secrets

- Only references are written in the configuration: `op://<vault>/<item>/<field>`.
- They are resolved with the 1Password CLI (`op inject`) just before running, and passed to the child process as environment variables.
- All references needed for one run are resolved with a single `op inject` (so the unlock prompt appears once).
- The run plan (`launch::plan`) does not resolve references. Just showing a plan never asks
  to unlock 1Password. Resolution happens in `launch::execute`, and the resolved values are discarded once they are passed to the child process.
- Resolved values are not written to disk. Debug output shows only the environment variable names.
- The command is not launched if a resolved value is empty, the number of values does not match the number of references, or a reference remains unreplaced.
- Resolution has no timeout. The calling thread blocks while waiting for 1Password to be unlocked, so
  do not call it from the UI thread.
- `op` is looked up in `/opt/homebrew/bin` and `/usr/local/bin` (GUI apps do not inherit the shell's PATH).

## Running actions

| type | macOS | Windows | Linux |
|---|---|---|---|
| `url` | Passed to the OS (`open`) | Passed to the OS | Passed to the OS |
| `app` | `open -a <app> [--args ...]` | Without arguments, passed to the OS. With arguments, launched directly | Launched directly |
| `script` | `$SHELL -l -c <run>` | `powershell.exe -NoProfile -Command <run>` | `$SHELL -l -c <run>` |
| `terminal` | Terminal.app (AppleScript) | `wt.exe [-d <cwd>] powershell.exe -NoExit -Command <run>` | Not supported |

- A leading `~` in `cwd` and `app` is expanded to the home directory (`app` is passed without going through a shell).
- On macOS, scripts get `/opt/homebrew/bin` and `/usr/local/bin` prepended to `PATH` if they are missing.
  The `PATH` of an app launched from the Dock does not contain them. If the command sets `PATH` in `env`,
  they are not added.
- `-l` is added only for zsh / bash / fish / ksh.
- The command line is passed to Terminal.app as an AppleScript argument (not embedded in the source).
  If Terminal was not running, the empty window it opens at launch is used.
- Running does not wait for the command to finish.

Limitations:

- Once the helper process used for launching (`osascript`, `open` etc.) has started, the run is treated as a success. Later failures
  (such as macOS not allowing control of Terminal.app) are not detected. Script output is also discarded.
- With `terminal`, specifying `cwd` types `cd <cwd> && <run>` into the terminal. If `run` is split into
  several statements by `;` or newlines, the second and later statements run even if `cd` fails.
- Quoting of `cwd` follows POSIX shell rules. In fish, a path containing a backslash cannot be passed correctly.

- iTerm2 is not supported. With the approach of sending text right after creating a window, the input gets mixed with the replies to
  queries that the shell's startup sends to the terminal, and the command can be broken.
- "Type into the terminal without running" is not supported.

## Search ranking

When the input is empty: pinned → most used → by title.

When there is input:

1. A `keyword` (any of its aliases) exactly matches the input (case-insensitive)
2. A `keyword` (any of its aliases) starts with the input
3. A fuzzy match on `title` or `keyword` (characters may be skipped; word order does not matter)

Within the same tier: for 3, match quality, then usage count, then title. For 1 and 2, usage count, then title.
Commands that do not match are not shown. Symbols in the input such as `!` and `^` are matched as literal characters.

Usage counts are kept in `usage.json` in the state folder.

## Migrating from Alfred

```shell
flintkey import-alfred            # List the files to create and what was not converted (writes nothing)
flintkey import-alfred --write    # Write to commands/ in the configuration folder
flintkey import-alfred --from <Alfred.alfredpreferences>
```

Without `--from`, it reads `current` (the sync folder) from `~/Library/Application Support/Alfred/prefs.json`.
Workflow `info.plist` files are converted to JSON with `plutil` and then read, so this only works on macOS.

What is converted is a "one keyword or hotkey → one action" connection.

| Alfred | Flintkey |
|---|---|
| Keyword (including `a||b` aliases) | `keyword` (aliases as a list). The title is the Alfred title |
| Hotkey | `hotkey`. If one keyword and one hotkey connect to the same action, they are merged into one command |
| Open URL | `type: url`. If a browser is specified, `type: script` with `open -b <bundle ID> <URL>` |
| Launch Apps / Files (one item) | `type: app` for an app (`.app`). For a folder or document, `type: script` with `open <path>` |
| Open File (folder) | `type: script` with `open <path>` |
| Run Script (zsh / bash / AppleScript / JavaScript), Run NSAppleScript | `type: script` that passes the script to the original interpreter (`/bin/zsh` etc.) as a here-document |
| Terminal Command | `type: terminal` |
| Browse in Terminal | `type: terminal` with `cd <path>` |

- Commands other than URLs get `when.os: macos`. The workflow name is added as a tag.
- The id is made from the keyword (or the app name if there is none). It is lowercased, and `-2` is added on a collision.
  Workflows are processed in order of name, so the same Alfred configuration always gives the same ids.
- If a file with the same id already exists, it is not overwritten even if the contents differ (the list shows `exists`).
- Alfred's hotkeys also stay in Alfred. On macOS, registering a hotkey that another app uses succeeds
  (with Alfred still holding `ctrl+alt+cmd+c`, Flintkey's registration did not fail). The overlap cannot be detected, so
  the Alfred workflow needs to be disabled. Which app responds when both have the hotkey has not been checked.
- If a script touches `token`, `password`, `.env` and so on, the list shows a warning to check it before sharing it with Git.
- If a workflow has variables, a warning says that those variables are not passed to the script (variables are not migrated, since they may contain secrets).
- Converted scripts assume a shell that can read here-documents (zsh, bash).
  Flintkey passes `run` to the login shell (`$SHELL`), so with fish the here-document part does not work.
- Run NSAppleScript code is an `on alfred_script(q)` handler that Alfred calls, so a call to it is appended at the end.
- The "hide if frontmost" (toggle) option of Launch Apps / Files cannot be migrated. A warning is shown, and the command only brings the app to the front.

Not converted (listed with the reason):

- Anything that uses arguments (placeholders such as `{query}`, `$1` / `$@` in scripts). Flintkey does not have arguments yet.
- One trigger that connects to several actions, or an action whose result is passed on to another.
- Output to the clipboard, and triggers such as Script Filter / List Filter / External Trigger.
- Alternative behavior when run with a modifier key held (a warning is shown, and only the behavior without a modifier is converted).
- Hotkeys that work only in specific apps (Related Apps). Flintkey's hotkeys work in every app.
- Disabled workflows.

## Plugins (planned)

Plugins will be external processes (any language, a crash does not stop the main app, works on Windows too).

- The manifest describes "a command that produces candidates" and "actions per candidate (chosen with modifier keys)".
- The candidate command returns a JSON array. The main app calls it once when the window is shown and keeps the result;
  filtering is done by the main app's search (starting a process on every key press would be slow).
- The development-starter feature (pick a project in `~/workspace` and open it in an editor or on GitHub) is
  shipped as one of these plugins, not as a built-in feature.

## Settings window

Opened from the tray menu (Settings…) or with ⌘, (Ctrl+, elsewhere) in the launcher. Everything it changes is written
to the YAML files through `Store`, so hand-written comments and formatting survive (see "Saving"). A file can also be
edited as raw YAML in the same window; text that is not valid is refused and nothing is written.

On macOS the app normally stays out of the Dock and the app switcher. While the settings window is open it behaves
like a regular app (Dock icon, ⌘Tab), so the window can be found again and receives keyboard input. Closing it (the
close button or ⌘W) hides it and returns to the background mode.

The window is dark from the first paint: its background color is set in `tauri.conf.json`, and `settings.html` sets
it inline before any stylesheet loads.

### General

- Launcher hotkey, display (`cursor` / `primary`) and terminal app.
- Saving registers the hotkeys again at once; a hotkey that cannot be registered is shown as a problem.

### Hotkey picker

Used for the launcher hotkey and for command hotkeys.

- Modifier buttons (Control, Option/Alt, Shift, Command/Win) and a key menu, so every hotkey can be set with the mouse.
- Record: press the combination instead. It reads the physical key (`KeyboardEvent.code`), so Option or a non-US
  layout does not change the key. A hotkey that is already registered (by Flintkey or another app) never reaches the
  window, so it cannot be recorded; pick it with the buttons.
- While choosing, the hotkey is checked:
  - Error: it cannot be read, or it has no modifier (only F keys may be used alone).
  - Warning: another command or the launcher has it, or macOS uses it for one of its own shortcuts. The macOS
    shortcuts checked are Spotlight (⌘Space), Finder search (⌥⌘Space) and input source switching (⌃Space, ⌃⌥Space).
    They are read from `~/Library/Preferences/com.apple.symbolichotkeys.plist`; a shortcut missing from that file
    has its default and is enabled. The warning links to System Settings → Keyboard.
  - Whether Flintkey or macOS receives a hotkey that both have has not been checked.

### Commands

- A list of every command file, for every OS, including broken files (which open as raw YAML so they can be fixed).
  Filter by text and by tag.
- A form for the selected command. Every field of every action type is kept while editing, so switching the type
  does not lose what was typed; only the fields of the chosen type are saved.
- Form / YAML views. Switching to YAML shows exactly the text that saving the form would write; switching back reads
  the YAML into the form.
- New: the file name (id) defaults to the title in lowercase with hyphens. An id that is already taken is refused,
  so a new command never overwrites a file.
- Try runs the saved command once without counting it as a use. Show in Finder reveals the file. Delete asks for a
  second click.

Protecting the files:

- A save sends the file text the window loaded. If the file changed since (an editor, `git pull`), the save is
  refused instead of putting the window's old values back. Revert / Reload loads the file again.
- If `settings.yaml` cannot be read, General opens it as YAML, and the form cannot be saved: the form would show the
  last settings that could be read, not what the file says.
- Switching from YAML to the form is refused while the YAML has unsaved edits, because the form cannot hold
  comments or layout typed there.
- Reloading when the window is shown again keeps unsaved edits.
- Saving keeps the file's permissions (a private file stays private).
- A command hotkey without a modifier is refused when saving, not only shown as an error.

Limitations:

- The window reloads when the files change on disk. Unsaved edits are kept, and a banner says the files changed;
  if the edited file was one of them, Save is refused and Revert loads it again.
- Renaming a command (its file) is not supported in the window.
