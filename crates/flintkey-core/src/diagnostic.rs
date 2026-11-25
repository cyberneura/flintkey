use std::fmt;

/// 設定ファイルの問題 1 件。位置が分かる場合は位置を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// ファイル内の行 (1 始まり)。
    pub line: Option<u64>,
    /// 行内の桁 (1 始まり)。
    pub column: Option<u64>,
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn at(line: u64, column: u64, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            column: Some(column),
            message: message.into(),
        }
    }

    pub(crate) fn file(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }

    pub(crate) fn from_yaml_error(error: &serde_saphyr::Error) -> Self {
        // 既定の文字列化はソースの抜粋を付ける。該当行の表示は呼び出し側が行う。
        let message = error.without_snippet().to_string();
        match error.location() {
            Some(location) => Self::at(location.line(), location.column(), message),
            None => Self::file(message),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "line {line}, column {column}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}
