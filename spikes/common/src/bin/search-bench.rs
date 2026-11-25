//! 画面なしで測れる部分の確認: あいまい検索そのものは 1 フレームより十分に短いか。

use spike_common::{items, time_search, us, Searcher};

fn main() {
    println!(
        "{:>7} {:>12} {:>10} {:>10} {:>6}",
        "items", "query", "median_us", "max_us", "hits"
    );
    for count in [1_000usize, 10_000, 100_000] {
        let list = items(count);
        let mut searcher = Searcher::new(&list);
        for query in ["d", "dep", "deployblog", "ブログ", "zzzz"] {
            let (median, max, hits) = time_search(&mut searcher, query, 200);
            println!(
                "{count:>7} {query:>12} {:>10.1} {:>10.1} {hits:>6}",
                us(median),
                us(max)
            );
        }
    }
}
