//! スパイク: GPUI (gpui-kit) で作る最小のランチャー窓。
//!
//! `spike-gpui-launcher`                 窓を開く。人が IME を試すためのもの (Esc か Cmd+Q で終了)。
//! `spike-gpui-launcher --bench out.json` 自分で計測して終了する。

use futures::channel::oneshot;
use futures::future::{select, Either};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    actions, div, px, rgb, size, uniform_list, AnyWindowHandle, App, AsyncApp, Bounds, Context, Entity, KeyBinding,
    Keystroke, Subscription, Window, WindowBounds, WindowKind, WindowOptions, WindowVisibility,
};
use spike_common::{self as common, ms, us, Item, Report, Searcher};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ROW_HEIGHT: f32 = 44.0;

actions!(spike, [Quit]);

struct Launcher {
    input: Entity<InputState>,
    items: Arc<Vec<Item>>,
    searcher: Searcher,
    results: Arc<Vec<u32>>,
    last_filter: Duration,
    _subscriptions: Vec<Subscription>,
}

impl Launcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let items = Arc::new(common::items(common::ITEM_COUNT));
        let mut searcher = Searcher::new(&items);
        let results = Arc::new(searcher.search(""));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search commands…"));
        let subscriptions = vec![cx.subscribe_in(&input, window, Self::on_input_event)];
        input.update(cx, |state, cx| state.focus(window, cx));
        Self {
            input,
            items,
            searcher,
            results,
            last_filter: Duration::ZERO,
            _subscriptions: subscriptions,
        }
    }

    fn on_input_event(&mut self, _: &Entity<InputState>, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let query = self.input.read(cx).value();
        let started = Instant::now();
        self.results = Arc::new(self.searcher.search(&query));
        self.last_filter = started.elapsed();
        cx.notify();
    }
}

impl Render for Launcher {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let items = self.items.clone();
        let results = self.results.clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1f22))
            .text_color(rgb(0xe6e6e6))
            .child(div().p_2().child(Input::new(&self.input)))
            .child(
                uniform_list("results", results.len(), move |range, _, _| {
                    range
                        .map(|row| {
                            let item = &items[results[row] as usize];
                            div()
                                .id(row)
                                .h(px(ROW_HEIGHT))
                                .px_3()
                                .flex()
                                .flex_col()
                                .justify_center()
                                .when(row == 0, |el| el.bg(rgb(0x2f65ca)))
                                .child(div().text_sm().child(item.title.clone()))
                                .child(div().text_xs().text_color(rgb(0x9a9a9a)).child(item.subtitle.clone()))
                        })
                        .collect::<Vec<_>>()
                })
                .flex_1(),
            )
    }
}

/// 表示の計測中に、ランチャー窓の可視状態のオブザーバーが埋める。
#[derive(Default)]
struct ShowProbe {
    requested_at: Option<Instant>,
    visible_ms: Option<f64>,
    frame_done: Option<oneshot::Sender<f64>>,
}

struct OpenedWindow {
    handle: AnyWindowHandle,
    view: Entity<Launcher>,
    first_frame: oneshot::Receiver<Instant>,
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(720.0), px(480.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // ランチャーは枠なしのパネル。タイトルバーを出さないので、信号機ボタンも出ない。
        titlebar: None,
        kind: WindowKind::PopUp,
        ..Default::default()
    }
}

/// 次に描かれるフレームの描画と表示が終わった後に `done` を呼ぶ。
///
/// GPUI は `on_next_frame` のコールバックを、そのフレームを描く前に呼ぶ。1 つ目の
/// コールバックの中でもう一度登録すると、2 つ目はその次のフレームの先頭、つまり 1 つ目の
/// フレームを描いて表示した後に呼ばれる。実際の描画完了より最大 1 リフレッシュ分遅いので、
/// この計測値は上限になる。
fn after_frame_drawn(window: &Window, done: impl FnOnce() + 'static) {
    window.on_next_frame(move |window, _| window.on_next_frame(move |_, _| done()));
}

fn open_launcher(probe: Rc<RefCell<ShowProbe>>, cx: &mut App) -> OpenedWindow {
    let (first_frame_tx, first_frame) = oneshot::channel();
    let (handle, view) = gpui_kit::open_window(window_options(cx), cx, |window, cx| {
        after_frame_drawn(window, move || {
            let _ = first_frame_tx.send(Instant::now());
        });
        window
            .observe_window_visibility(move |visibility, window, _| {
                let mut state = probe.borrow_mut();
                let Some(requested_at) = state.requested_at else { return };
                if visibility != WindowVisibility::Visible {
                    return;
                }
                state.requested_at = None;
                state.visible_ms = Some(ms(requested_at.elapsed()));
                if let Some(done) = state.frame_done.take() {
                    after_frame_drawn(window, move || {
                        let _ = done.send(ms(requested_at.elapsed()));
                    });
                }
            })
            .detach();
        cx.new(|cx| Launcher::new(window, cx))
    })
    .expect("open launcher window");
    OpenedWindow {
        handle,
        view,
        first_frame,
    }
}

async fn pause(cx: &AsyncApp, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

/// 手順が時間内に終わらなければ `None`。壊れた 1 手順で計測全体が止まらないようにするため。
async fn within_timeout<T>(cx: &AsyncApp, receiver: oneshot::Receiver<T>) -> Option<T> {
    let timer = cx.background_executor().timer(common::STEP_TIMEOUT);
    futures::pin_mut!(timer);
    match select(receiver, timer).await {
        Either::Left((value, _)) => value.ok(),
        Either::Right(_) => None,
    }
}

async fn run_bench(process_start: Instant, startup_notes: Vec<String>, out: PathBuf, cx: &mut AsyncApp) {
    let mut report = Report::new("gpui-kit 0.10.0 (gpui-pre 0.3.12)");
    report.notes = startup_notes;
    report
        .notes
        .push("frame times are upper bounds: taken at the start of the frame after the one that drew".into());
    let probe = Rc::new(RefCell::new(ShowProbe::default()));

    let opened = cx.update(|cx| {
        let opened = open_launcher(probe.clone(), cx);
        cx.activate(true);
        opened
    });
    report.notes.push(format!(
        "window object created at {:.0} ms",
        ms(process_start.elapsed())
    ));
    report.cold_start_ms = within_timeout(cx, opened.first_frame)
        .await
        .map(|at| ms(at.duration_since(process_start)));
    report.write(&out);
    pause(cx, Duration::from_secs(1)).await;

    for key in common::TYPED_KEYS {
        let (tx, rx) = oneshot::channel();
        let dispatched = opened.handle.update(cx, |_, window, cx| {
            let started = Instant::now();
            let handled = window.dispatch_keystroke(Keystroke::parse(key).expect("valid key"), cx);
            after_frame_drawn(window, move || {
                let _ = tx.send(ms(started.elapsed()));
            });
            handled
        });
        if !matches!(dispatched, Ok(true)) {
            report
                .notes
                .push(format!("keystroke {key:?} was not handled: {dispatched:?}"));
        }
        report.type_total_ms.push(within_timeout(cx, rx).await);
        report
            .type_filter_us
            .push(cx.update(|cx| us(opened.view.read(cx).last_filter)));
        pause(cx, Duration::from_millis(30)).await;
    }
    report.typed_query = cx.update(|cx| opened.view.read(cx).input.read(cx).value().to_string());
    if report.typed_query != common::expected_query() {
        report
            .notes
            .push("typed text did not reach the search field; typing numbers are not valid".into());
    }
    report.write(&out);

    for _ in 0..common::SHOW_CYCLES {
        cx.update(|cx| cx.hide());
        pause(cx, common::HIDDEN_PAUSE).await;
        let (tx, rx) = oneshot::channel();
        {
            let mut state = probe.borrow_mut();
            state.requested_at = Some(Instant::now());
            state.visible_ms = None;
            state.frame_done = Some(tx);
        }
        let activated = opened.handle.update(cx, |_, window, cx| {
            cx.activate(true);
            window.activate_window();
        });
        if let Err(error) = activated {
            report.notes.push(format!("activate failed: {error}"));
        }
        report.show_ms.push(within_timeout(cx, rx).await);
        report.show_visible_ms.push(probe.borrow_mut().visible_ms.take());
        probe.borrow_mut().requested_at = None;
        pause(cx, Duration::from_millis(200)).await;
    }
    report.write(&out);

    // GPUI にはウインドウ単位の hide が無く、アプリ単位の hide は Windows では何もしない。
    // 移植できる代替は「閉じて新しい窓を開く」なので、そのコストを測る。
    let mut current = opened.handle;
    for _ in 0..common::REOPEN_CYCLES {
        let _ = current.update(cx, |_, window, _| window.remove_window());
        pause(cx, common::HIDDEN_PAUSE).await;
        let requested_at = Instant::now();
        let reopened = cx.update(|cx| {
            let opened = open_launcher(probe.clone(), cx);
            cx.activate(true);
            opened
        });
        current = reopened.handle;
        report.reopen_ms.push(
            within_timeout(cx, reopened.first_frame)
                .await
                .map(|at| ms(at.duration_since(requested_at))),
        );
        pause(cx, Duration::from_millis(200)).await;
    }
    report.write(&out);

    cx.update(|cx| cx.hide());
    pause(cx, Duration::from_secs(3)).await;
    report.self_footprint_kb = common::self_footprint_kb();
    report.phase = "idle".into();
    report.write(&out);
    // ランナースクリプトは、phase が "idle" の間にプロセスの一覧を取る。
    pause(cx, Duration::from_secs(4)).await;
    report.phase = "done".into();
    report.write(&out);
    cx.update(|cx| cx.quit());
}

fn main() {
    let process_start = Instant::now();
    let bench_out = common::bench_output_from_args();
    gpui_kit::application().run(move |cx| {
        let mut startup_notes = vec![format!("app launched at {:.0} ms", ms(process_start.elapsed()))];
        gpui_kit::init(cx);
        startup_notes.push(format!(
            "components initialized at {:.0} ms",
            ms(process_start.elapsed())
        ));
        match bench_out {
            Some(out) => cx
                .spawn(async move |cx| run_bench(process_start, startup_notes, out, cx).await)
                .detach(),
            None => {
                // タイトルバーが無いと閉じるボタンも無いので、キーボードで終了できるようにする。
                cx.on_action(|_: &Quit, cx| cx.quit());
                cx.bind_keys([
                    KeyBinding::new("escape", Quit, None),
                    KeyBinding::new("cmd-q", Quit, None),
                ]);
                open_launcher(Rc::new(RefCell::new(ShowProbe::default())), cx);
                cx.activate(true);
            }
        }
    });
}
