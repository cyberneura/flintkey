//! Code shared by the two engine spikes, so that both measure the same workload:
//! the same 1000 candidates, the same fuzzy search, the same simulated input.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant};

pub const ITEM_COUNT: usize = 1000;
pub const SHOW_CYCLES: usize = 20;
pub const REOPEN_CYCLES: usize = 10;
/// How long the window stays hidden before a show is measured.
pub const HIDDEN_PAUSE: Duration = Duration::from_millis(500);
/// Time allowed for one step. When exceeded, the step is recorded as missing instead of stopping the whole run.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(3);

/// Keys typed into the search field one at a time. The query grows from 1 to 11 characters,
/// so at first it matches most of the list and at the end it matches almost nothing.
pub const TYPED_KEYS: &[&str] = &["d", "e", "p", "l", "o", "y", "b", "l", "o", "g", "s"];

#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
}

/// Deterministic candidates that resemble real launcher items: apps, URLs, scripts and
/// project folders. Some are in Japanese.
pub fn items(count: usize) -> Vec<Item> {
    const VERBS: &[&str] = &[
        "Open",
        "Deploy",
        "Build",
        "Restart",
        "Tail logs of",
        "SSH to",
        "Search",
        "開く",
        "デプロイ",
        "再起動",
    ];
    const NOUNS: &[&str] = &[
        "blog",
        "shop",
        "admin",
        "api",
        "worker",
        "docs",
        "dashboard",
        "staging",
        "ブログ",
        "管理画面",
        "注文",
        "billing",
        "search",
        "mail",
        "calendar",
        "wiki",
    ];
    const KINDS: &[(&str, &str)] = &[
        ("url", "https://example.com/"),
        ("app", "/Applications/"),
        ("script", "~/workspace/"),
        ("terminal", "ssh "),
    ];
    (0..count)
        .map(|i| {
            let verb = VERBS[i % VERBS.len()];
            let noun = NOUNS[(i / VERBS.len()) % NOUNS.len()];
            let (kind, prefix) = KINDS[(i / 7) % KINDS.len()];
            Item {
                title: format!("{verb} {noun} {i}"),
                subtitle: format!("{kind} · {prefix}{noun}-{i}"),
            }
        })
        .collect()
}

pub struct Searcher {
    matcher: Matcher,
    haystacks: Vec<Utf32String>,
}

impl Searcher {
    pub fn new(items: &[Item]) -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            haystacks: items.iter().map(|i| Utf32String::from(i.title.as_str())).collect(),
        }
    }

    /// Indices of the matching items, highest score first. An empty query returns everything in the original order.
    pub fn search(&mut self, query: &str) -> Vec<u32> {
        if query.is_empty() {
            return (0..self.haystacks.len() as u32).collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut scored: Vec<(u32, u32)> = self
            .haystacks
            .iter()
            .enumerate()
            .filter_map(|(i, h)| pattern.score(h.slice(..), &mut self.matcher).map(|s| (s, i as u32)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, i)| i).collect()
    }
}

#[derive(Default, Debug, Serialize)]
pub struct Report {
    pub engine: String,
    /// "running" while measuring, "idle" while waiting for the memory measurement, "done" when finished.
    pub phase: String,
    pub pid: u32,
    pub item_count: usize,
    /// From process start to the first frame of the launcher window.
    pub cold_start_ms: Option<f64>,
    /// Per key: from the key press to the frame that draws the new result list.
    pub type_total_ms: Vec<Option<f64>>,
    /// Per key: time spent in the fuzzy search alone.
    pub type_filter_us: Vec<f64>,
    /// The text in the search field after typing. Must equal the typed keys concatenated.
    pub typed_query: String,
    /// Per cycle: from the show request to the first frame after the window is visible again.
    pub show_ms: Vec<Option<f64>>,
    /// Per cycle: from the show request until the OS reports the window as visible (GPUI only).
    pub show_visible_ms: Vec<Option<f64>>,
    /// Per cycle: from closing the window and opening a new one to its first frame (GPUI only).
    pub reopen_ms: Vec<Option<f64>>,
    /// Physical footprint of this process alone, after it has gone idle while hidden.
    pub self_footprint_kb: Option<u64>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn new(engine: &str) -> Self {
        Self {
            engine: engine.to_string(),
            phase: "running".to_string(),
            pid: std::process::id(),
            item_count: ITEM_COUNT,
            ..Default::default()
        }
    }

    /// Written through a temporary file so the runner script never reads a half-written file.
    pub fn write(&self, path: &Path) {
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self).expect("report serializes");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

pub fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000_000.0
}

pub fn expected_query() -> String {
    TYPED_KEYS.concat()
}

/// `--bench <path>` from the command line, if present.
pub fn bench_output_from_args() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--bench")
        .and_then(|i| args.get(i + 1))
        .map(Into::into)
}

#[cfg(target_os = "macos")]
pub fn self_footprint_kb() -> Option<u64> {
    // phys_footprint is the value Activity Monitor shows as "Memory". RSS overcounts shared pages.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            &mut info as *mut libc::rusage_info_v2 as *mut libc::rusage_info_t,
        );
        (rc == 0).then(|| info.ri_phys_footprint / 1024)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn self_footprint_kb() -> Option<u64> {
    None
}

/// Median and worst time of the fuzzy search alone. For checks without a screen.
pub fn time_search(searcher: &mut Searcher, query: &str, rounds: usize) -> (Duration, Duration, usize) {
    let mut samples = Vec::with_capacity(rounds);
    let mut hits = 0;
    for _ in 0..rounds {
        let t = Instant::now();
        hits = searcher.search(query).len();
        samples.push(t.elapsed());
    }
    samples.sort();
    (samples[samples.len() / 2], samples[samples.len() - 1], hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_deterministic_and_unique() {
        // Arrange
        let a = items(ITEM_COUNT);
        let b = items(ITEM_COUNT);

        // Act
        let unique: std::collections::HashSet<&str> = a.iter().map(|i| i.title.as_str()).collect();

        // Assert
        assert_eq!(a.len(), ITEM_COUNT);
        assert_eq!(unique.len(), ITEM_COUNT);
        assert!(a
            .iter()
            .zip(&b)
            .all(|(x, y)| x.title == y.title && x.subtitle == y.subtitle));
    }

    #[test]
    fn empty_query_returns_everything_in_original_order() {
        // Arrange
        let list = items(50);
        let mut searcher = Searcher::new(&list);

        // Act
        let hits = searcher.search("");

        // Assert
        assert_eq!(hits, (0..50).collect::<Vec<u32>>());
    }

    #[test]
    fn query_narrows_and_only_returns_matching_titles() {
        // Arrange
        let list = items(ITEM_COUNT);
        let mut searcher = Searcher::new(&list);

        // Act
        let hits = searcher.search("deployblog");

        // Assert
        assert!(!hits.is_empty());
        assert!(hits.len() < ITEM_COUNT / 10);
        for &i in &hits {
            let title = list[i as usize].title.to_lowercase();
            assert!(
                title.contains("deploy") && title.contains("blog"),
                "unexpected hit: {title}"
            );
        }
    }

    #[test]
    fn japanese_query_matches_japanese_titles() {
        // Arrange
        let list = items(ITEM_COUNT);
        let mut searcher = Searcher::new(&list);

        // Act
        let hits = searcher.search("ブログ");

        // Assert
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|&i| list[i as usize].title.contains("ブログ")));
    }

    #[test]
    fn typed_keys_still_match_something_at_every_step() {
        // Arrange: the typing benchmark is meaningless if the list goes empty early.
        let list = items(ITEM_COUNT);
        let mut searcher = Searcher::new(&list);
        let mut query = String::new();

        for key in &TYPED_KEYS[..10] {
            // Act
            query.push_str(key);
            let hits = searcher.search(&query);

            // Assert
            assert!(!hits.is_empty(), "no match for {query:?}");
        }
    }
}
