//! Flintkey ランチャーの、描画エンジンに依存しないコア。
//!
//! ここにあるものはウインドウ無しで動く: 設定フォルダの読み書き、コマンドが何を起動するかの
//! 決定、秘密情報の解決、検索結果の順位付け。デスクトップアプリはこの上の薄い層になる。

pub mod alfred;
pub mod command;
pub mod diagnostic;
pub mod launch;
pub mod paths;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod store;
pub mod usage;

mod atomic;
mod document;

pub use command::{Action, Command, Os, parse_command};
pub use diagnostic::Diagnostic;
