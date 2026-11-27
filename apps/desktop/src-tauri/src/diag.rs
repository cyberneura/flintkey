//! A log of where keyboard input goes, for tracking down a launcher that stops taking typing. On only when a file
//! named `debug-focus` exists in the state folder at startup; then lines are appended to `focus.log` there.
//!
//! Key values are never written, only that a key arrived and where focus was. Lines are written on a thread of their
//! own, so the log cannot hold up key handling on the main thread.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

const FLAG: &str = "debug-focus";
const LOG: &str = "focus.log";
/// The log stops growing past this size.
const LIMIT: u64 = 1 << 20;

static WRITER: OnceLock<Option<Sender<String>>> = OnceLock::new();

pub fn init(state_dir: &Path) {
    let writer = state_dir.join(FLAG).exists().then(|| start_writer(state_dir.join(LOG)));
    let _ = WRITER.set(writer);
}

fn start_writer(path: PathBuf) -> Sender<String> {
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in receiver {
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LIMIT) {
                continue;
            }
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(file, "{line}");
            }
        }
    });
    sender
}

pub fn enabled() -> bool {
    WRITER.get().is_some_and(Option::is_some)
}

pub fn log(message: &str) {
    let Some(Some(writer)) = WRITER.get() else {
        return;
    };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let _ = writer.send(format!("{millis} {message}"));
}
