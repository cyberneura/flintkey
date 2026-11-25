//! Secret references.
//!
//! A config value like `op://Private/GitHub/token` is not the secret but a reference to it.
//! It is resolved with the 1Password CLI right before the command runs and passed to the child process.
//! The value is never written to disk or kept in memory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const REFERENCE_PREFIX: &str = "op://";

pub fn is_reference(value: &str) -> bool {
    value.starts_with(REFERENCE_PREFIX)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("`{0}` is not a usable secret reference")]
    InvalidReference(String),
    #[error("could not run the 1Password CLI (`{program}`): {message}")]
    CliUnavailable { program: String, message: String },
    #[error("the 1Password CLI could not resolve the secrets: {0}")]
    Rejected(String),
    #[error("the 1Password CLI returned output in an unexpected shape")]
    UnexpectedOutput,
    #[error("the secret `{0}` is empty")]
    Empty(String),
}

pub trait SecretResolver {
    /// Returns one value per reference, in the same order.
    fn resolve(&self, references: &[String]) -> Result<Vec<String>, SecretError>;
}

/// Resolves references with `op inject`.
pub struct OnePassword {
    program: PathBuf,
}

impl OnePassword {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Looks for `op` where package managers install it. Apps started from the Dock or Start menu
    /// do not inherit the shell's PATH, so running it by bare name is the last resort.
    pub fn locate() -> Self {
        let known = ["/opt/homebrew/bin/op", "/usr/local/bin/op"];
        let found = known.iter().map(Path::new).find(|path| path.is_file());
        Self::new(found.unwrap_or(Path::new("op")))
    }
}

static BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A line that practically never appears in a secret. Separates the values in the batched output.
fn boundary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let count = BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("flintkey-boundary-{:x}-{nanos:x}-{count:x}", std::process::id())
}

impl SecretResolver for OnePassword {
    fn resolve(&self, references: &[String]) -> Result<Vec<String>, SecretError> {
        if references.is_empty() {
            return Ok(Vec::new());
        }
        // References are pasted into the template, so they must not contain characters
        // that could close the placeholder or start a new line.
        if let Some(bad) = references
            .iter()
            .find(|r| !is_reference(r) || r.contains(['{', '}', '\n', '\r']))
        {
            return Err(SecretError::InvalidReference(bad.clone()));
        }

        // Resolve all references in one `op inject`, so the unlock prompt appears
        // once instead of once per secret.
        let boundary = boundary();
        let separator = format!("\n{boundary}\n");
        let template: String = references.iter().map(|r| format!("{{{{ {r} }}}}{separator}")).collect();

        let unavailable = |error: std::io::Error| SecretError::CliUnavailable {
            program: self.program.display().to_string(),
            message: error.to_string(),
        };
        let mut child = Command::new(&self.program)
            .arg("inject")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(unavailable)?;
        // Wait for exit whether or not the write succeeded. Otherwise the child becomes a zombie. Also,
        // if the CLI exits without reading the template, its own error message explains the cause
        // better than the write error does.
        let written = child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(template.as_bytes());
        let output = child.wait_with_output().map_err(unavailable)?;
        if !output.status.success() {
            return Err(SecretError::Rejected(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        written.map_err(unavailable)?;

        let text = String::from_utf8(output.stdout).map_err(|_| SecretError::UnexpectedOutput)?;
        let mut values: Vec<String> = text.split(&separator).map(str::to_string).collect();
        if values.len() != references.len() + 1 || values.pop().is_some_and(|rest| !rest.is_empty()) {
            return Err(SecretError::UnexpectedOutput);
        }
        // A placeholder left unreplaced must not be returned as a secret.
        if values
            .iter()
            .zip(references)
            .any(|(value, reference)| *value == format!("{{{{ {reference} }}}}"))
        {
            return Err(SecretError::UnexpectedOutput);
        }
        Ok(values)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Creates a stand-in for `op` that behaves as `op inject` according to the given script body.
    fn fake_op(dir: &Path, body: &str) -> OnePassword {
        let path = dir.join("op");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        OnePassword::new(path)
    }

    /// Replaces `{{ op://a/b/c }}` with `secret(a/b/c)`, as the real CLI replaces it with the value.
    const INJECT: &str = r#"[ "$1" = inject ] || exit 64
sed -E 's#\{\{ op://([^}]*) \}\}#secret(\1)#g'"#;

    #[test]
    fn references_are_resolved_in_order_with_one_call() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let op = fake_op(dir.path(), &format!("echo call >> '{}'\n{INJECT}", calls.display()));
        let references = vec![
            "op://Private/GitHub/token".to_string(),
            "op://Work/My Item/pass word".to_string(),
        ];

        // Act
        let values = op.resolve(&references).unwrap();

        // Assert
        assert_eq!(
            values,
            ["secret(Private/GitHub/token)", "secret(Work/My Item/pass word)"]
        );
        assert_eq!(std::fs::read_to_string(calls).unwrap(), "call\n");
    }

    #[test]
    fn multi_line_secret_is_returned_whole() {
        // Arrange: private keys span several lines.
        let dir = tempfile::tempdir().unwrap();
        let op = fake_op(
            dir.path(),
            r#"awk '{ gsub(/\{\{ op:\/\/k\/e\/y \}\}/, "line1\nline2\nline3"); print }'"#,
        );
        let references = vec!["op://k/e/y".to_string(), "op://k/e/y".to_string()];

        // Act
        let values = op.resolve(&references).unwrap();

        // Assert
        assert_eq!(values, ["line1\nline2\nline3", "line1\nline2\nline3"]);
    }

    #[test]
    fn no_references_means_no_call() {
        // Arrange: this program does not exist, so calling it fails.
        let op = OnePassword::new("/nonexistent/op");

        // Act
        let values = op.resolve(&[]).unwrap();

        // Assert
        assert!(values.is_empty());
    }

    #[test]
    fn reference_that_could_break_out_of_the_template_is_refused() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let op = fake_op(dir.path(), INJECT);

        for bad in [
            "op://a/b/c }} {{ op://x/y/z",
            "op://a/b\n/c",
            "plain text",
            "https://example.com",
        ] {
            // Act
            let result = op.resolve(&[bad.to_string()]);

            // Assert
            assert_eq!(result, Err(SecretError::InvalidReference(bad.to_string())));
        }
    }

    #[test]
    fn cli_failure_reports_its_message() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        // Exits without reading the template. Even if the write fails, the CLI's message is reported.
        let op = fake_op(dir.path(), "echo '[ERROR] item not found' >&2\nexit 1");

        // Act
        let result = op.resolve(&["op://a/b/c".to_string()]);

        // Assert
        assert_eq!(result, Err(SecretError::Rejected("[ERROR] item not found".to_string())));
    }

    #[test]
    fn missing_cli_is_reported_as_unavailable() {
        // Arrange
        let op = OnePassword::new("/nonexistent/op");

        // Act
        let result = op.resolve(&["op://a/b/c".to_string()]);

        // Assert
        assert!(matches!(result, Err(SecretError::CliUnavailable { .. })), "{result:?}");
    }

    #[test]
    fn output_that_lost_the_boundaries_is_refused() {
        // Arrange: output from a CLI that prints something else must not be accepted as secrets.
        let dir = tempfile::tempdir().unwrap();
        let op = fake_op(dir.path(), "cat > /dev/null\necho unrelated");

        // Act
        let result = op.resolve(&["op://a/b/c".to_string()]);

        // Assert
        assert_eq!(result, Err(SecretError::UnexpectedOutput));
    }

    #[test]
    fn placeholder_that_was_not_substituted_is_not_returned_as_a_secret() {
        // Arrange: a CLI that echoes the template back.
        let dir = tempfile::tempdir().unwrap();
        let op = fake_op(dir.path(), "cat");

        // Act
        let result = op.resolve(&["op://a/b/c".to_string()]);

        // Assert
        assert_eq!(result, Err(SecretError::UnexpectedOutput));
    }

    #[test]
    fn only_op_scheme_values_are_references() {
        // Act + Assert
        assert!(is_reference("op://Private/GitHub/token"));
        assert!(!is_reference("https://example.com"));
        assert!(!is_reference(" op://x/y/z"));
        assert!(!is_reference(""));
    }
}
