use std::fmt;

/// One problem in a configuration file, with its location when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Line in the file (1-based).
    pub line: Option<u64>,
    /// Column in the line (1-based).
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
        // The default formatting appends a source excerpt. Callers show the line themselves.
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
