# Flintkey

キーボードランチャー。仕様は `docs/design.md`、概要は `README.md`。

## 構成

- `crates/flintkey-core`: 描画エンジンに依存しないコア。設定の読み書き・検証・アクション・秘密情報・検索。
- `spikes/`: 設計判断の根拠にした実験。ワークスペースには含めない (`Cargo.toml` の `exclude`)。
  それぞれ独立した Cargo プロジェクトで、製品コードから参照しない。
- デスクトップアプリは `apps/desktop` に Tauri 2 + Svelte 5 + Tailwind 4 で作る (未実装)。

## コマンド

```shell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

## 決まり

- コアはウインドウ無しでテストできる状態を保つ。OS ごとの挙動は `launch::plan` のように
  「マシンの情報を引数で受け取る純粋な関数」にして、どの OS 向けの挙動も手元でテストする。
- 設定ファイルへの書き込みは `Store` を通す。意味が変わったキーだけを書き換えるという性質
  (`docs/design.md` の「保存」) を壊さない。
- 秘密情報を解決した値をログ・ディスク・`Debug` 出力に出さない。
- コードコメントは日本語。ユーザーに見せるメッセージ (エラー文など) は英語。
- 依存 crate は公開から 7 日以上経ったバージョンを使う。

## 実機確認

- ターミナル起動 (`type: terminal`) は AppleScript を使うので、変更したら Terminal.app の 3 つの状態
  (未起動 / 起動中でウインドウなし / ウインドウあり) で実際に動かして確かめる。ユニットテストは
  組み立てたコマンドラインしか見ていない。
- GUI アプリは Claude Code のサンドボックス内では描画できない。画面が要る確認はサンドボックスの外で行う。
- Windows 向けの挙動は実機で未確認。Windows 機で動かしたら `docs/design.md` の「実装状況」を更新する。
