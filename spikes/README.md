# Spikes

Experiments that answer two design questions. They are not part of the product.

| Folder | Question |
|---|---|
| `yaml-edit/` | Can the settings UI rewrite hand-written YAML without breaking its comments and formatting? |
| `gpui-launcher/` | How fast is a launcher window built with GPUI (`gpui-kit`)? |
| `tauri-launcher/` | How fast is the same window built with Tauri 2 (the OS WebView)? |
| `common/` | The workload both launchers share: 1000 candidates, fuzzy search, the keys to type |
| `results/` | Raw data measured on macOS |

## YAML editing

```shell
cd spikes/yaml-edit && cargo run          # add -- -v to print the output of every case
```

Each case edits a commented command file with `yaml-edit`, then checks that the lines it did not touch are
byte-for-byte identical and that reading the result back with `serde-saphyr` gives the expected values.

## Engine benchmark

Both apps accept `--bench <out.json>`, measure themselves and exit. They need a screen, so run them
from a normal terminal.

```shell
(cd spikes/gpui-launcher && cargo build --release)
(cd spikes/tauri-launcher && cargo build --release)
python3 spikes/run-engine-bench.py
```

Do not touch the keyboard or mouse while the windows flash. The results go to `spikes/results/`.

What is measured:

| Field | Meaning |
|---|---|
| `cold_start_ms` | From process start to the first frame of the launcher window |
| `show_ms` | From the request to show the hidden window to the first frame after it reappears (20 times) |
| `reopen_ms` | From closing the window and opening a new one to its first frame (GPUI only) |
| `type_total_ms` | From one key to the frame that draws the new result list (11 keys, 1000 candidates) |
| `type_filter_us` | The part of that spent in the fuzzy search |
| `processes_while_idle` | Memory of the app and its WebView helper processes while hidden and idle |

Limits of this method:

- Neither measures the moment the pixels reach the screen.
  - Tauri: the point where the main thread's rendering work is done (the task right after
    `requestAnimationFrame`). Compositing in the GPU process is not included. `show_ms` and `cold_start_ms`
    are taken when that notification reaches the Rust side over IPC, so they include one IPC round trip.
  - GPUI: the `on_next_frame` callback runs before the frame is drawn, so the end of drawing cannot be
    observed directly. The current code measures at "the start of the following frame". Drawing and
    presenting are done by then, but it can be up to one refresh late (an upper bound).
- The app issues the show request itself. The time to handle a global hotkey is not included
  (both engines would use the same OS API for it).
- Tauri draws the top 50 rows. GPUI uses a virtual list and draws only the rows that fit in the window.
- Input is simulated. GPUI is sent synthesized keystrokes; for the WebView, the input field's value is
  rewritten. Neither goes through the OS IME.

How to read `results/`:

| Folder | GPUI measurement point | GPUI window |
|---|---|---|
| `run1/` `run2/` | Start of the frame that draws the change (excludes drawing time; a lower bound) | With title bar |
| `run3/` (GPUI only) | Start of the following frame (an upper bound) | Without title bar |

In `run1/`, GPUI's `cold_start_ms` is too high (about 2000 ms) because the app was not activated before the
first window was shown. Fixed from `run2/` on.

Only the fuzzy search can be checked without a screen:

```shell
cd spikes/common && cargo run --release --bin search-bench
```

## Japanese input (manual)

Start each app with no arguments, type Japanese into the search field and check that:

1. The text being composed appears underlined inside the field
2. The candidate window appears right below the cursor
3. Enter only commits the composition
4. No characters are dropped or doubled

```shell
spikes/gpui-launcher/target/release/spike-gpui-launcher     # Esc or Cmd+Q closes it
spikes/tauri-launcher/target/release/spike-tauri-launcher   # Esc closes it
```
