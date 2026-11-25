//! スパイク: Tauri 2 (OS の WebView) で作る最小のランチャー窓。
//!
//! `spike-tauri-launcher`                 窓を開く。人が IME を試すためのもの (Esc で終了)。
//! `spike-tauri-launcher --bench out.json` 自分で計測して終了する。

use serde::Serialize;
use spike_common::{self as common, ms, us, Item, Report, Searcher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

/// ページに出す行数の上限。実際のランチャーも短い一覧をスクロールさせる。
const VISIBLE_ROWS: usize = 50;

enum PageEvent {
    Ready,
    Painted(u32),
    Typed {
        total_ms: Vec<f64>,
        filter_us: Vec<f64>,
        final_query: String,
    },
}

struct AppState {
    items: Vec<Item>,
    searcher: Mutex<Searcher>,
    page_events: Mutex<Sender<PageEvent>>,
}

impl AppState {
    fn send(&self, event: PageEvent) {
        let _ = self.page_events.lock().unwrap().send(event);
    }
}

#[derive(Serialize)]
struct SearchResult<'a> {
    rows: Vec<&'a Item>,
    total: usize,
    filter_us: f64,
}

#[tauri::command]
fn search(state: State<'_, AppState>, query: String) -> SearchResult<'_> {
    let state = state.inner();
    let started = Instant::now();
    let hits = state.searcher.lock().unwrap().search(&query);
    let filter_us = us(started.elapsed());
    SearchResult {
        rows: hits
            .iter()
            .take(VISIBLE_ROWS)
            .map(|&i| &state.items[i as usize])
            .collect(),
        total: hits.len(),
        filter_us,
    }
}

#[tauri::command]
fn ready(state: State<'_, AppState>) {
    state.send(PageEvent::Ready);
}

#[tauri::command]
fn painted(state: State<'_, AppState>, seq: u32) {
    state.send(PageEvent::Painted(seq));
}

#[tauri::command]
fn typed(state: State<'_, AppState>, total_ms: Vec<f64>, filter_us: Vec<f64>, final_query: String) {
    state.send(PageEvent::Typed {
        total_ms,
        filter_us,
        final_query,
    });
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// `pick` が受け付けるイベントを待つ。タイムアウトしたら `None`。壊れた 1 手順で計測全体が止まらないようにするため。
fn wait_for<T>(events: &Receiver<PageEvent>, timeout: Duration, pick: impl Fn(PageEvent) -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        if let Some(value) = pick(events.recv_timeout(remaining).ok()?) {
            return Some(value);
        }
    }
}

/// 窓を表示し、ページがフレームを描き終えるまでの時間を返す。
fn show_and_wait(app: &AppHandle, events: &Receiver<PageEvent>, seq: u32, since: Instant) -> Option<f64> {
    let window = app.get_webview_window("main")?;
    window.show().ok()?;
    window.set_focus().ok()?;
    window.eval(format!("window.__bench.show({seq})")).ok()?;
    wait_for(events, common::STEP_TIMEOUT, |e| {
        matches!(e, PageEvent::Painted(s) if s == seq).then_some(())
    })?;
    Some(ms(since.elapsed()))
}

fn run_bench(app: AppHandle, events: Receiver<PageEvent>, process_start: Instant, launched_ms: f64, out: PathBuf) {
    let mut report = Report::new("tauri 2.12.6 (system WebView)");
    report.notes.push(format!("app launched at {launched_ms:.0} ms"));
    let window = app.get_webview_window("main").expect("main window");

    if wait_for(&events, Duration::from_secs(20), |e| {
        matches!(e, PageEvent::Ready).then_some(())
    })
    .is_none()
    {
        report.notes.push("the page never reported ready".into());
    }
    report.notes.push(format!(
        "page loaded and first search done at {:.0} ms",
        ms(process_start.elapsed())
    ));
    report.cold_start_ms = show_and_wait(&app, &events, 0, process_start);
    report.write(&out);
    std::thread::sleep(Duration::from_secs(1));

    let keys = serde_json::to_string(common::TYPED_KEYS).expect("keys serialize");
    let _ = window.eval(format!("window.__bench.type({keys})"));
    let typing_timeout = common::STEP_TIMEOUT * common::TYPED_KEYS.len() as u32;
    match wait_for(&events, typing_timeout, |e| match e {
        PageEvent::Typed {
            total_ms,
            filter_us,
            final_query,
        } => Some((total_ms, filter_us, final_query)),
        _ => None,
    }) {
        Some((total_ms, filter_us, final_query)) => {
            report.type_total_ms = total_ms.into_iter().map(Some).collect();
            report.type_filter_us = filter_us;
            report.typed_query = final_query;
        }
        None => report.notes.push("typing did not finish".into()),
    }
    if report.typed_query != common::expected_query() {
        report
            .notes
            .push("typed text did not reach the search field; typing numbers are not valid".into());
    }
    report.write(&out);

    for cycle in 0..common::SHOW_CYCLES {
        let _ = window.hide();
        std::thread::sleep(common::HIDDEN_PAUSE);
        report
            .show_ms
            .push(show_and_wait(&app, &events, cycle as u32 + 1, Instant::now()));
        std::thread::sleep(Duration::from_millis(200));
    }
    report.write(&out);

    let _ = window.hide();
    std::thread::sleep(Duration::from_secs(3));
    report.self_footprint_kb = common::self_footprint_kb();
    report
        .notes
        .push("self_footprint_kb excludes the WebView helper processes; see the runner's process table".into());
    report.phase = "idle".into();
    report.write(&out);
    // ランナースクリプトは、phase が "idle" の間にプロセスの一覧を取る。
    std::thread::sleep(Duration::from_secs(4));
    report.phase = "done".into();
    report.write(&out);
    app.exit(0);
}

fn run_interactive(app: AppHandle, events: Receiver<PageEvent>) {
    let _ = wait_for(&events, Duration::from_secs(20), |e| {
        matches!(e, PageEvent::Ready).then_some(())
    });
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main() {
    let process_start = Instant::now();
    let bench_out = common::bench_output_from_args();
    let items = common::items(common::ITEM_COUNT);
    let searcher = Mutex::new(Searcher::new(&items));
    let (page_events, events) = mpsc::channel();

    tauri::Builder::default()
        .manage(AppState {
            items,
            searcher,
            page_events: Mutex::new(page_events),
        })
        .invoke_handler(tauri::generate_handler![search, ready, painted, typed, quit])
        .setup(move |app| {
            let launched_ms = ms(process_start.elapsed());
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            std::thread::spawn(move || match bench_out {
                Some(out) => run_bench(handle, events, process_start, launched_ms, out),
                None => run_interactive(handle, events),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("run tauri application");
}
