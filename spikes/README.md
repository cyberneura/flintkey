# スパイク

設計上の 2 つの問いに答えるための実験。製品には入らない。

| フォルダ | 問い |
|---|---|
| `yaml-edit/` | 設定 UI は、手書きの YAML をコメントと書式を壊さずに書き換えられるか |
| `gpui-launcher/` | GPUI (`gpui-kit`) のランチャー窓はどのくらい速いか |
| `tauri-launcher/` | 同じ窓を Tauri 2 (OS の WebView) で作るとどのくらい速いか |
| `common/` | 2 つのランチャーが共有する負荷: 候補 1000 件、あいまい検索、入力するキー |
| `results/` | macOS で計測した生データ |

## YAML の編集

```shell
cd spikes/yaml-edit && cargo run          # -- -v を付けると全ケースの出力を表示
```

各ケースは、コメント付きのコマンドファイルを `yaml-edit` で編集し、触れていない行がバイト単位で
同じであることと、結果を `serde-saphyr` で読み直すと期待した値になることを確かめる。

## エンジンのベンチマーク

どちらのアプリも `--bench <out.json>` を受け付け、自分で計測して終了する。画面が必要なので、
通常のターミナルから実行する。

```shell
(cd spikes/gpui-launcher && cargo build --release)
(cd spikes/tauri-launcher && cargo build --release)
python3 spikes/run-engine-bench.py
```

ウインドウが点滅する間は、キーボードとマウスに触らない。結果は `spikes/results/` に出る。

計測する項目:

| フィールド | 意味 |
|---|---|
| `cold_start_ms` | プロセスの開始から、ランチャー窓の最初のフレームまで |
| `show_ms` | 隠した窓の表示要求から、再表示後の最初のフレームまで (20 回) |
| `reopen_ms` | 窓を閉じて新しく開き、その最初のフレームまで (GPUI のみ) |
| `type_total_ms` | キー 1 つから、新しい結果一覧を描いたフレームまで (11 キー、候補 1000 件) |
| `type_filter_us` | そのうち、あいまい検索にかかった時間 |
| `processes_while_idle` | 隠してアイドルにした状態での、アプリと WebView のヘルパープロセスのメモリ |

この方法の限界:

- どちらも、画素が画面に出た時点は測れていない。
  - Tauri: メインスレッドの描画処理が済んだ時点 (`requestAnimationFrame` の直後のタスク)。
    GPU プロセスでの合成は含まない。`show_ms` と `cold_start_ms` は、その通知が IPC で
    Rust 側に届いた時点なので、IPC の往復 1 回分を含む。
  - GPUI: `on_next_frame` のコールバックはフレームを描く前に呼ばれるので、描画の完了を直接は
    観測できない。現在のコードは「その次のフレームの開始」で測る。描画と表示は済んでいるが、
    最大 1 リフレッシュ分遅い (上限値)。
- 表示要求はアプリ自身が出す。グローバルホットキーの処理時間は含まない
  (どちらのエンジンでも同じ OS の API を使うことになる)。
- Tauri は上位 50 行を描く。GPUI は仮想リストで、窓に収まる行だけを描く。
- 入力は模擬。GPUI には合成したキー入力を送り、WebView は入力欄の値を書き換える。
  どちらも OS の IME を通らない。

`results/` の読み方:

| フォルダ | GPUI の計測点 | GPUI の窓 |
|---|---|---|
| `run1/` `run2/` | 変更を描くフレームの開始 (描画時間を含まない。下限値) | タイトルバーあり |
| `run3/` (GPUI のみ) | その次のフレームの開始 (上限値) | タイトルバーなし |

`run1/` の GPUI の `cold_start_ms` は、最初の窓を出す前にアプリをアクティブ化していなかったため
過大 (約 2000 ms)。`run2/` 以降は修正済み。

画面なしで確かめられるのは、あいまい検索だけ:

```shell
cd spikes/common && cargo run --release --bin search-bench
```

## 日本語入力 (手動)

各アプリを引数なしで起動し、検索欄に日本語を入力して次を確かめる。

1. 変換中の文字が、欄の中に下線付きで出る
2. 変換候補のウインドウが、カーソルの真下に出る
3. Enter で確定だけされる
4. 文字が欠けたり二重になったりしない

```shell
spikes/gpui-launcher/target/release/spike-gpui-launcher     # Esc か Cmd+Q で閉じる
spikes/tauri-launcher/target/release/spike-tauri-launcher   # Esc で閉じる
```
