//! 入力された文字に対するコマンドの順位付け。
//!
//! 並び順は説明できるものにしている。設定には並び順を保存しない (順序のリストは Git で
//! 真っ先にコンフリクトする)。キーワードの一致、一致の質、ピン留め、このマシンでの
//! 使用回数で並べる。

use crate::store::StoredCommand;
use crate::usage::Usage;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::cmp::Reverse;

pub struct Searcher {
    matcher: Matcher,
    buffer: Vec<char>,
}

impl Default for Searcher {
    fn default() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buffer: Vec::new(),
        }
    }
}

/// コマンドがどう一致したか。良い順。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    KeywordExact,
    KeywordPrefix,
    Fuzzy,
}

impl Searcher {
    /// `commands` の添字を、良く一致した順に返す。一致しないコマンドは含めない。
    ///
    /// 空のクエリは全件を返す。ピン留め、次に使用回数の多い順。
    pub fn rank(&mut self, commands: &[StoredCommand], usage: &Usage, query: &str) -> Vec<usize> {
        let query = query.trim();
        let title_key = |index: usize| (commands[index].command.title.to_lowercase(), commands[index].id.clone());
        let uses = |index: usize| Reverse(usage.count(&commands[index].id));

        if query.is_empty() {
            let mut all: Vec<usize> = (0..commands.len()).collect();
            all.sort_by_cached_key(|&index| (Reverse(commands[index].command.pin), uses(index), title_key(index)));
            return all;
        }

        // 入力された文字はそのまま照合する。単語の順序は問わないが、fzf の構文のように
        // `!` や `^` に特別な意味は持たせない。
        let pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
        let lowered_query = query.to_lowercase();
        let mut matched: Vec<(Tier, Reverse<u32>, usize)> = Vec::new();
        for (index, stored) in commands.iter().enumerate() {
            let keywords: Vec<String> = stored.command.keywords.iter().map(|k| k.to_lowercase()).collect();
            let mut score = |text: &str| pattern.score(Utf32Str::new(text, &mut self.buffer), &mut self.matcher);
            let title_score = score(&stored.command.title);
            let fuzzy = keywords
                .iter()
                .filter_map(|keyword| score(keyword))
                .chain(title_score)
                .max();
            let tier = if keywords.contains(&lowered_query) {
                Some(Tier::KeywordExact)
            } else if keywords.iter().any(|keyword| keyword.starts_with(&lowered_query)) {
                Some(Tier::KeywordPrefix)
            } else {
                fuzzy.map(|_| Tier::Fuzzy)
            };
            if let Some(tier) = tier {
                // 一致の質で並べるのは、あいまい一致の層だけ。キーワード一致の層は使用回数で決まる。
                let quality = if tier == Tier::Fuzzy { fuzzy.unwrap_or(0) } else { 0 };
                matched.push((tier, Reverse(quality), index));
            }
        }
        matched.sort_by_cached_key(|&(tier, quality, index)| (tier, quality, uses(index), title_key(index)));
        matched.into_iter().map(|(_, _, index)| index).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Action, Command};

    fn command(id: &str, title: &str, keyword: Option<&str>, pin: bool) -> StoredCommand {
        StoredCommand {
            id: id.into(),
            command: Command {
                title: title.into(),
                keywords: keyword
                    .map(|keyword| keyword.split('|').map(Into::into).collect())
                    .unwrap_or_default(),
                hotkey: None,
                pin,
                tags: vec![],
                os: [].into(),
                action: Action::Url {
                    url: "https://example.com".into(),
                },
            },
        }
    }

    fn ranked(commands: &[StoredCommand], usage: &Usage, query: &str) -> Vec<String> {
        Searcher::default()
            .rank(commands, usage, query)
            .into_iter()
            .map(|index| commands[index].id.clone())
            .collect()
    }

    fn used(counts: &[(&str, u32)]) -> Usage {
        let mut usage = Usage::default();
        for (id, times) in counts {
            for _ in 0..*times {
                usage.record(id);
            }
        }
        usage
    }

    #[test]
    fn empty_query_lists_pinned_then_most_used_then_by_title() {
        // Arrange
        let commands = [
            command("c", "Charlie", None, false),
            command("a", "alpha", None, false),
            command("p", "Zulu pinned", None, true),
            command("b", "Bravo", None, false),
        ];
        let usage = used(&[("b", 3), ("p", 1)]);

        // Act
        let order = ranked(&commands, &usage, "   ");

        // Assert
        assert_eq!(order, ["p", "b", "a", "c"]);
    }

    #[test]
    fn exact_keyword_beats_keyword_prefix_beats_title_match() {
        // Arrange
        let commands = [
            command("title", "dep tracker", None, false),
            command("prefix", "Something", Some("deploy"), false),
            command("exact", "Other thing", Some("dep"), false),
        ];

        // Act
        let order = ranked(&commands, &Usage::default(), "DEP");

        // Assert
        assert_eq!(order, ["exact", "prefix", "title"]);
    }

    #[test]
    fn commands_that_do_not_match_are_left_out() {
        // Arrange
        let commands = [
            command("a", "Deploy blog", Some("deploy"), false),
            command("b", "Open Slack", None, false),
        ];

        // Act
        let order = ranked(&commands, &Usage::default(), "blog");

        // Assert
        assert_eq!(order, ["a"]);
    }

    #[test]
    fn letters_may_be_scattered_and_words_may_come_in_any_order() {
        // Arrange
        let commands = [
            command("a", "Deploy blog", None, false),
            command("b", "Open Slack", None, false),
        ];

        // Act
        let scattered = ranked(&commands, &Usage::default(), "dpblg");
        let reordered = ranked(&commands, &Usage::default(), "blog deploy");

        // Assert
        assert_eq!(scattered, ["a"]);
        assert_eq!(reordered, ["a"]);
    }

    #[test]
    fn closer_match_comes_first_and_use_breaks_ties() {
        // Arrange
        let commands = [
            command("far", "Describe every pull request", None, false),
            command("near", "Deploy", None, false),
            command("twin-a", "Open staging", None, false),
            command("twin-b", "Open staging", None, false),
        ];
        let usage = used(&[("twin-b", 2)]);

        // Act
        let by_quality = ranked(&commands, &usage, "dep");
        let by_use = ranked(&commands, &usage, "staging");

        // Assert
        assert_eq!(by_quality, ["near", "far"]);
        assert_eq!(by_use, ["twin-b", "twin-a"]);
    }

    #[test]
    fn any_of_the_keywords_counts_as_the_keyword() {
        // Arrange: Alfred の `a||b` から移した別名。2 つ目の別名でも最優先で選ばれる。
        let commands = [
            command("title", "today report", None, false),
            command("alias", "日付を貼る", Some("今日|today"), false),
            command("prefix", "Toggle", Some("tomorrow|todayish"), false),
        ];

        // Act
        let exact = ranked(&commands, &Usage::default(), "today");
        let first = ranked(&commands, &Usage::default(), "今日");

        // Assert
        assert_eq!(exact, ["alias", "prefix", "title"]);
        assert_eq!(first, ["alias"]);
    }

    #[test]
    fn among_keyword_matches_the_most_used_comes_first() {
        // Arrange
        let commands = [
            command("a", "Alpha", Some("deploy-a"), false),
            command("b", "Beta", Some("deploy-b"), false),
        ];

        // Act
        let order = ranked(&commands, &used(&[("b", 1)]), "deploy");

        // Assert
        assert_eq!(order, ["b", "a"]);
    }

    #[test]
    fn japanese_titles_are_searchable() {
        // Arrange
        let commands = [
            command("a", "ブログをデプロイ", None, false),
            command("b", "管理画面を開く", None, false),
        ];

        // Act
        let order = ranked(&commands, &Usage::default(), "デプロイ");

        // Assert
        assert_eq!(order, ["a"]);
    }

    #[test]
    fn punctuation_in_the_query_is_matched_literally() {
        // Arrange: fzf の構文では、`!blog` は「blog を含まないもの」を意味する。
        let commands = [
            command("a", "Deploy blog", None, false),
            command("b", "Run !important", None, false),
        ];

        // Act
        let order = ranked(&commands, &Usage::default(), "!imp");

        // Assert
        assert_eq!(order, ["b"]);
    }
}
