# Flintkey

キーボードランチャー。仕様は `docs/design.md`、概要は `README.md`。

## 構成

- `crates/flintkey-core`: 描画エンジンに依存しないコア。設定の読み書き・検証・アクション・秘密情報・検索。
- `spikes/`: 設計判断の根拠にした実験。ワークスペースには含めない (`Cargo.toml` の `exclude`)。
  それぞれ独立した Cargo プロジェクトで、製品コードから参照しない。
- `apps/desktop`: デスクトップアプリ (Tauri 2 + Svelte 5 + Tailwind 4)。コアを呼ぶ薄い層。
  - `src-tauri/src/main.rs`: 常駐、トレイ、ホットキー、窓を出す / 隠す。
  - `src-tauri/src/placement.rs`: 窓を置く位置の計算 (純粋な関数)。
  - `src-tauri/src/session.rs`: 窓に依存しない状態 (読み込み、検索、実行の計画)。
  - `src-tauri/src/showing.rs`: 表示ごとの番号と、実行要求の受け付け (純粋な状態)。
  - `src-tauri/src/panel.rs`: macOS で窓を NSPanel に差し替える。
  - `src/`: ランチャー窓のページ。キーの解釈は `src/lib/keys.ts`。

## コマンド

```shell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
pnpm -C apps/desktop test
pnpm -C apps/desktop check
```

`cargo` のコマンドはページをビルドしていなくても通る (開発用ビルドはページを埋め込まない)。

`.j-menu.yaml` に Tauri dev / Local deploy / CLI のインストール / clippy を登録してある。
Local deploy (`apps/desktop/scripts/local-deploy.sh`) は `.app` をビルドし、Developer ID 署名・Apple 公証・staple の
検証後に `/Applications` に置く。公証には Keychain の `cyberneura-notary` (`NOTARY_PROFILE` で変更可) を使い、
失敗時は既存のアプリを置き換えない。置き場所は `DEST_DIR` で変えられる。署名は Cyberneura の Developer ID に
固定で、`notarize-app.sh` が公証の前に検証する。

## 決まり

- コアはウインドウ無しでテストできる状態を保つ。OS ごとの挙動は `launch::plan` のように
  「マシンの情報を引数で受け取る純粋な関数」にして、どの OS 向けの挙動も手元でテストする。
- 設定ファイルへの書き込みは `Store` を通す。意味が変わったキーだけを書き換えるという性質
  (`docs/design.md` の「保存」) を壊さない。
- 秘密情報を解決した値をログ・ディスク・`Debug` 出力に出さない。
- コードコメントは日本語。ユーザーに見せるメッセージ (エラー文など) は英語。
- 依存 crate は公開から 7 日以上経ったバージョンを使う。`Cargo.lock` に入る間接依存も対象
  (crate を足すと、その時点の最新版で解決される)。npm 側は `apps/desktop/pnpm-workspace.yaml` の
  `minimumReleaseAge` が守る。
- 配色はダークテーマのみ。Tailwind の `dark:` は使わず、暗い色を直接書く。
- 白い背景を一瞬も見せない。窓を足すときは、隠したまま作り、背景を透明か暗い色にする
  (ページが描かれる前に出ても白くならないようにする)。
- 窓の座標は OS ごとに意味が違う。位置の計算は `placement::place` に閉じ込め、窓の API が返した値を
  そのまま比べない。

## 実機確認

- ターミナル起動 (`type: terminal`) は AppleScript を使うので、変更したら Terminal.app の 3 つの状態
  (未起動 / 起動中でウインドウなし / ウインドウあり) で実際に動かして確かめる。ユニットテストは
  組み立てたコマンドラインしか見ていない。
- GUI アプリは Claude Code のサンドボックス内では描画できない。画面が要る確認はサンドボックスの外で行う。
- アプリを手元で試すときは、`FLINTKEY_CONFIG_DIR` / `FLINTKEY_STATE_DIR` で別のフォルダを使い、
  `settings.yaml` の `hotkey` を普段使わない組み合わせにする (常用のホットキーを奪わないため)。
  起動中のアプリをもう一度起動すると窓が出るので、ホットキーを押さずに窓を出せる。
  多重起動の判定は設定フォルダと無関係なので、常用の Flintkey が起動中なら先に終了する
  (終了しないと、常用の方の窓が出るだけでテスト用の設定は使われない)。
- Windows 向けの挙動は実機で未確認。Windows 機で動かしたら `docs/design.md` の「実装状況」を更新する。
