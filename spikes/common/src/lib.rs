//! 2 つのエンジンのスパイクが共有するコード。同じ負荷を測るためのもの:
//! 同じ 1000 件の候補、同じあいまい検索、同じ模擬入力。

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use serde::Serialize;
use std::path::Path;
use std::time::{Duration, Instant};

pub const ITEM_COUNT: usize = 1000;
pub const SHOW_CYCLES: usize = 20;
pub const REOPEN_CYCLES: usize = 10;
/// 表示の計測の前に、窓を隠しておく時間。
pub const HIDDEN_PAUSE: Duration = Duration::from_millis(500);
/// 1 手順に許す時間。超えたら計測全体を止めずに、欠損として記録する。
pub const STEP_TIMEOUT: Duration = Duration::from_secs(3);

/// 検索欄に 1 つずつ入力するキー。クエリは 1 文字から 11 文字に伸びるので、
/// 最初のうちは一覧の大半に一致し、最後はほとんど一致しない。
pub const TYPED_KEYS: &[&str] = &["d", "e", "p", "l", "o", "y", "b", "l", "o", "g", "s"];

#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
}

/// 実際のランチャーの項目に似せた、毎回同じ内容の候補。アプリ・URL・スクリプト・
/// プロジェクトのフォルダ。一部は日本語。
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

    /// 一致した項目の添字。スコアの高い順。空のクエリは全件を元の順で返す。
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
    /// 計測中は "running"、メモリの計測待ちは "idle"、終了時は "done"。
    pub phase: String,
    pub pid: u32,
    pub item_count: usize,
    /// プロセスの開始から、ランチャー窓の最初のフレームまで。
    pub cold_start_ms: Option<f64>,
    /// キーごと: キー入力から、新しい結果一覧を描いたフレームまで。
    pub type_total_ms: Vec<Option<f64>>,
    /// キーごと: あいまい検索だけにかかった時間。
    pub type_filter_us: Vec<f64>,
    /// 入力後に検索欄に入っていた文字列。入力したキーを連結したものと一致しなければならない。
    pub typed_query: String,
    /// 1 回ごと: 表示要求から、窓が再び見えた後の最初のフレームまで。
    pub show_ms: Vec<Option<f64>>,
    /// 1 回ごと: 表示要求から、OS が窓を可視と報告するまで (GPUI のみ)。
    pub show_visible_ms: Vec<Option<f64>>,
    /// 1 回ごと: 窓を閉じて新しく開き、その最初のフレームまで (GPUI のみ)。
    pub reopen_ms: Vec<Option<f64>>,
    /// 隠した状態でアイドルにした後の、このプロセスだけの physical footprint。
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

    /// 一時ファイル経由で書く。ランナースクリプトが書きかけのファイルを読まないようにするため。
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

/// コマンドラインの `--bench <path>`。あれば返す。
pub fn bench_output_from_args() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--bench")
        .and_then(|i| args.get(i + 1))
        .map(Into::into)
}

#[cfg(target_os = "macos")]
pub fn self_footprint_kb() -> Option<u64> {
    // phys_footprint はアクティビティモニタの「メモリ」に出る値。RSS は共有ページを多く数えすぎる。
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

/// あいまい検索だけの中央値と最悪値。画面なしの確認用。
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
