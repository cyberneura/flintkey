# Flintkey

A keyboard launcher. The specification is `docs/design.md`; the overview is `README.md`.

## Layout

- `crates/flintkey-core`: Core that does not depend on a rendering engine. Reading, writing and validating settings,
  actions, secrets, search, and conversion from Alfred (`alfred.rs`; the CLI turns the plist into JSON with `plutil`
  and passes that in).
  - `examples/flintkey.rs`: CLI (`list` / `plan` / `run` / `import-alfred`).
- `spikes/`: Experiments behind design decisions. Not part of the workspace (`exclude` in `Cargo.toml`).
  Each is an independent Cargo project, and product code does not reference them.
- `apps/desktop`: Desktop app (Tauri 2 + Svelte 5 + Tailwind 4). A thin layer that calls the core.
  - `src-tauri/src/main.rs`: Background process, tray, hotkeys, showing / hiding the window.
  - `src-tauri/src/placement.rs`: Computes where to put the window (pure functions).
  - `src-tauri/src/session.rs`: State that does not depend on the window (loading, search, planning a run).
  - `src-tauri/src/showing.rs`: A number per showing of the window, and accepting run requests (pure state).
  - `src-tauri/src/hotkeys.rs`: Decides the set of shortcuts to register from the hotkeys in the settings (pure functions).
  - `src-tauri/src/panel.rs`: Replaces the window with an NSPanel on macOS.
  - `src-tauri/src/watch.rs`: Watches the configuration folder; a change reloads the files and the hotkeys.
  - `src-tauri/src/settings_ui.rs`: Commands for the settings window. `forms.rs` converts its forms to and from
    the core types (pure functions).
  - `src/`: The launcher window's page. Key handling is in `src/lib/keys.ts`.
  - `src/settings/` and `settings.html`: The settings window's page. Hotkey strings are read and written in
    `src/lib/hotkey.ts`.

## Commands

```shell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
pnpm -C apps/desktop test
pnpm -C apps/desktop check
```

The `cargo` commands pass even if the page has not been built (development builds do not embed the page).

`.j-menu.yaml` has entries for Tauri dev / Local deploy / installing the CLI / migrating from Alfred (a list that writes nothing) / clippy.
Local deploy (`apps/desktop/scripts/local-deploy.sh`) builds the `.app` and, after verifying the Developer ID signature,
Apple notarization and stapling, puts it in `/Applications`. Notarization uses `cyberneura-notary` in the Keychain
(override with `NOTARY_PROFILE`); on failure the existing app is not replaced. `DEST_DIR` changes the destination.
Signing is fixed to Cyberneura's Developer ID, and `notarize-app.sh` checks it before notarizing.

## Rules

- Keep the core testable without a window. Make OS-specific behavior a "pure function that takes the machine's
  information as arguments", like `launch::plan`, and test the behavior for every OS locally.
- Write to settings files through `Store`. Do not break the property that only keys whose meaning changed are
  rewritten ("Saving" in `docs/design.md`).
- Never put resolved secret values into logs, on disk, or in `Debug` output.
- Everything in the repository is written in English: code comments, documentation, messages shown to users
  (error text etc.), commit messages and PR descriptions. Japanese is fine as test input data.
- Use dependency crate versions that were published at least 7 days ago. This includes indirect dependencies that
  end up in `Cargo.lock` (adding a crate resolves to the latest versions at that moment). On the npm side,
  `minimumReleaseAge` in `apps/desktop/pnpm-workspace.yaml` enforces this.
- Dark theme only. Do not use Tailwind's `dark:`; write the dark colors directly.
- Never show a white background, even for a moment. When adding a window, create it hidden and give it a
  transparent or dark background (so it does not turn white if it appears before the page is drawn).
- Do not put personal names, company names or other data from a real setup into tests and documents; use neutral
  examples (the project is open source).
- Window coordinates mean different things on each OS. Keep position calculations inside `placement::place`, and
  do not compare values returned by the window API directly.

## Checking on a real machine

- Launching a terminal (`type: terminal`) uses AppleScript, so after changing it, run it for real in the three states
  of Terminal.app (not running / running with no windows / with a window). The unit tests only look at the assembled
  command line.
- GUI apps cannot draw inside the Claude Code sandbox. Do checks that need the screen outside the sandbox.
- When trying the app locally, use separate folders via `FLINTKEY_CONFIG_DIR` / `FLINTKEY_STATE_DIR`, and set
  `hotkey` in `settings.yaml` to a combination you do not normally use (so the everyday hotkey is not taken).
  Launching the app again while it is running shows the window, so you can open the window without pressing the hotkey.
  Single-instance detection does not depend on the settings folder, so if your everyday Flintkey is running, quit it
  first (otherwise only the everyday instance's window appears and the test settings are not used).
- Windows behavior has not been checked on a real machine. After running it on a Windows machine, update "Status" in
  `docs/design.md`.
