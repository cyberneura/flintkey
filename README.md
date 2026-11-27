# Flintkey

A keyboard launcher. Its settings live in files, so they can be shared across machines with Git. The same settings are planned to be editable from a UI as well.

Supported OSes are macOS (primary) and Windows. The design is in [docs/design.md](docs/design.md).

It stays in the menu bar (the system tray on Windows) and opens an input window with a hotkey (`alt+space` by default).
Settings can be edited in the settings window (tray menu → Settings…, or ⌘, in the launcher) or directly as YAML files.
Changes to the files (an editor, `git pull`) take effect immediately.
On macOS, installed apps can be searched and opened too, with their icons, without writing a command for each.
System Settings, the screen saver, locking the screen, restarting, logging out and Launchpad are built in as well.
Rows show the command's own icon, the site's favicon or app's icon, the tag's icon, or a default icon for the type.
There is no plugin support yet.

## Installing

macOS, with [Homebrew](https://brew.sh):

```shell
brew install --cask cyberneura/tap/flintkey
```

Windows: download the installer (`Flintkey_<version>_x64-setup.exe`) from
[Releases](https://github.com/cyberneura/flintkey/releases). It is not signed, so SmartScreen asks before running it.
The Windows build is made by CI but has not been tried on a real Windows machine yet.

## Example settings

`~/.config/flintkey/commands/deploy-blog.yaml`:

```yaml
title: Deploy blog
keyword: [deploy, blog]                     # can have aliases
hotkey: ctrl+alt+cmd+d                      # runs directly without opening the window
type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  GITHUB_TOKEN: op://Private/GitHub/token   # resolved with the 1Password CLI at run time
icon: blog.png                              # from ~/.config/flintkey/icons/; a URL or full path also works
tags: [personal]                            # for grouping in the settings UI
```

One command per file. `type` is `url` / `app` / `script` / `terminal`.

## Development

```shell
cargo test --workspace                                    # Rust unit tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

cd apps/desktop
pnpm install
pnpm test                                                 # unit tests for the page
pnpm check                                                # type check
pnpm tauri dev                                            # start the app
```

`FLINTKEY_CONFIG_DIR` and `FLINTKEY_STATE_DIR` point the app at other folders, so it does not touch your real settings.

### Installing locally (macOS)

```shell
pnpm -C apps/desktop local-deploy
```

Builds the `.app`, signs it with a Developer ID, has Apple notarize it, staples the ticket and puts it in `/Applications`
(this needs a Developer ID certificate and a `cyberneura-notary` profile in the Keychain).
A running Flintkey is not stopped, so quit it from the menu bar and open it again.

If you use [j-menu](https://github.com/cyberneura/j-menu), the `j` menu has Tauri dev and Local deploy.

### Releasing

Raise the version in both `Cargo.toml` (`[workspace.package]`) and `apps/desktop/package.json`, and merge it into
`main`. `.github/workflows/release.yml` sees a version that has not been released, runs the tests on macOS and
Windows, builds a signed and notarized universal dmg and an NSIS installer, and publishes them as one release.
The Homebrew tap picks the new version up within the hour.

After adding or upgrading dependencies, run `scripts/generate-third-party-notices.sh` (needs `cargo install
cargo-about --locked --features cli`) and commit `THIRD-PARTY-NOTICES.txt`; a test fails when a direct dependency is
missing from it.

### Trying it from the command line

```shell
cargo run -p flintkey-core --example flintkey -- list [query]   # ranked list, plus files that are broken
cargo run -p flintkey-core --example flintkey -- plan <id>      # what running it would do
cargo run -p flintkey-core --example flintkey -- run <id>       # run it
cargo run -p flintkey-core --example flintkey -- import-alfred  # list the commands that would be made from Alfred workflows
cargo run -p flintkey-core --example flintkey -- import-alfred --write   # actually write them
```

Hotkeys moved over from Alfred stay assigned in Alfred too. On macOS, a hotkey that Alfred already uses can still be
registered without an error (Flintkey cannot detect the overlap). Disable the Alfred workflows around the time you run
`--write`.

## Folders

| Folder | Contents |
|---|---|
| `crates/flintkey-core` | Core that does not depend on a rendering engine |
| `apps/desktop` | Desktop app (Tauri 2 + Svelte 5 + Tailwind 4) |
| `docs` | Design |
| `spikes` | Experiments behind design decisions (YAML editing, UI engine comparison). Not part of the product |

## License

MIT. See [LICENSE](LICENSE). The licenses of the bundled libraries are in [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt).
