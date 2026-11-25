# Flintkey

キーボードランチャー。設定をファイルで持ち、Git で複数マシンに共有でき、同じ内容を UI からも編集できる。

対応 OS は macOS (主) と Windows。設計は [docs/design.md](docs/design.md) にある。

現在あるのはコア (`crates/flintkey-core`) だけで、デスクトップアプリはまだ無い。

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
cargo test                                   # ユニットテスト
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

デスクトップアプリ無しで設定フォルダを試す:

```shell
cargo run -p flintkey-core --example flintkey -- list [query]   # 順位付けした一覧と、壊れているファイル
cargo run -p flintkey-core --example flintkey -- plan <id>      # 実行すると何が起きるか
cargo run -p flintkey-core --example flintkey -- run <id>       # 実行する
```

`FLINTKEY_CONFIG_DIR` と `FLINTKEY_STATE_DIR` で、本物の設定に触れずに別のフォルダを使える。

## フォルダ

| フォルダ | 内容 |
|---|---|
| `crates/flintkey-core` | 描画エンジンに依存しないコア |
| `docs` | 設計 |
| `spikes` | 設計判断の根拠にした実験 (YAML の編集、UI エンジンの比較)。製品には入らない |
