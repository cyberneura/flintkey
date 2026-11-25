# Flintkey

キーボードランチャー。設定をファイルで持ち、Git で複数マシンに共有できる。同じ内容を UI からも編集できるようにする予定。

対応 OS は macOS (主) と Windows。設計は [docs/design.md](docs/design.md) にある。

メニューバー (Windows ではタスクトレイ) に常駐し、ホットキー (既定は `alt+space`) で入力窓を出す。
設定 UI とプラグインはまだ無く、設定は YAML ファイルを直接書く。

## 設定の例

`~/.config/flintkey/commands/deploy-blog.yaml`:

```yaml
title: Deploy blog
keyword: deploy
type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  GITHUB_TOKEN: op://Private/GitHub/token   # 実行時に 1Password CLI で解決する
```

1 コマンド 1 ファイル。`type` は `url` / `app` / `script` / `terminal`。

## 開発

```shell
cargo test --workspace                                    # Rust のユニットテスト
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all

cd apps/desktop
pnpm install
pnpm test                                                 # ページのユニットテスト
pnpm check                                                # 型チェック
pnpm tauri dev                                            # アプリを起動する
```

`FLINTKEY_CONFIG_DIR` と `FLINTKEY_STATE_DIR` で、本物の設定に触れずに別のフォルダを使える。

### 手元にインストールする (macOS)

```shell
pnpm -C apps/desktop local-deploy
```

`.app` をビルドし、Developer ID での署名・Apple の公証・staple を済ませて `/Applications` に置く
(Developer ID の証明書と、Keychain の `cyberneura-notary` プロファイルが要る)。
起動中の Flintkey は止めないので、メニューバーから終了して開き直す。

[j-menu](https://github.com/cyberneura/j-menu) を使っているなら、`j` のメニューに Tauri dev と Local deploy がある。

### コマンドラインで試す

```shell
cargo run -p flintkey-core --example flintkey -- list [query]   # 順位付けした一覧と、壊れているファイル
cargo run -p flintkey-core --example flintkey -- plan <id>      # 実行すると何が起きるか
cargo run -p flintkey-core --example flintkey -- run <id>       # 実行する
```

## フォルダ

| フォルダ | 内容 |
|---|---|
| `crates/flintkey-core` | 描画エンジンに依存しないコア |
| `apps/desktop` | デスクトップアプリ (Tauri 2 + Svelte 5 + Tailwind 4) |
| `docs` | 設計 |
| `spikes` | 設計判断の根拠にした実験 (YAML の編集、UI エンジンの比較)。製品には入らない |
