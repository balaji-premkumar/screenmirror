//! Structured event log.
//!
//! These entries are not developer-only diagnostics: the desktop app renders
//! them in its "Diagnostic Stream" panel, so a user reads them. That is why an
//! entry carries a `code` and its `params` rather than only a finished
//! sentence — the interface looks the wording up in the user's language. See
//! `packages/mirror-i18n` for the catalog and the reasoning.
//!
//! `message` is still populated, in English, for two consumers that have no
//! catalog: the on-disk log file, which has to be readable when pasted into a
//! bug report, and any interface that has not been localised yet.
//!
//! Emit with the [`log_event!`](crate::log_event) macro:
//!
//! ```ignore
//! log_event!(codes::USB_STREAMING_OPEN_FAILED, "error" => format!("{e:?}"));
//! ```

/// Re-exported so `log_event!` expands correctly in any module, without that
/// module needing its own `use mirror_i18n::Event`.
pub use mirror_i18n::Event;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::io::Write;
use std::sync::mpsc;
use std::sync::Mutex;

/// Every entry emitted this session, capped and trimmed from the front.
pub static LOG_BUFFER: Lazy<Mutex<Vec<LogEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// How far the interface has read into `LOG_BUFFER`.
static LOG_CURSOR: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(0));

/// One logged event, as the interface receives it.
#[derive(Serialize, Clone, Debug)]
pub struct LogEntry {
    /// Local wall-clock time, `HH:MM:SS.mmm`.
    pub timestamp: String,
    /// Stable event code, e.g. `usb.streaming.open_failed`. The interface
    /// translates on this; it is the only field safe to branch on.
    pub code: String,
    /// Severity: `INFO`, `SUCCESS`, `WARN`, `ERROR`, `FATAL`.
    pub level: String,
    /// Emitting subsystem, e.g. `USB`.
    pub component: String,
    /// Activity within that subsystem, e.g. `handshake`.
    pub action: String,
    /// Values for the `{name}` placeholders in the code's wording.
    pub params: std::collections::BTreeMap<String, String>,
    /// English rendering. A fallback, not the source of truth — an interface
    /// with a catalog should render from `code` and `params` instead.
    pub message: String,
}

/// Rotate the log once it passes this size, keeping one previous generation.
/// Without this `mirror_rust.log.json` grew without limit for the lifetime of
/// the install.
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// Bounded so a burst of errors cannot grow the channel without limit. Logs
/// are diagnostics: dropping a few under pressure beats unbounded memory on
/// the streaming threads.
const LOG_CHANNEL_CAPACITY: usize = 4096;

/// Entries kept in memory before the oldest are trimmed.
const LOG_BUFFER_MAX: usize = 500;

/// How many to drop when the cap is hit.
const LOG_BUFFER_TRIM: usize = 250;

/// Returns the platform-appropriate log directory path.
fn log_dir() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        std::path::PathBuf::from(home)
            .join(".mirror_stream")
            .join("logs")
    } else {
        std::env::temp_dir().join("mirror_stream").join("logs")
    }
}

fn open_log_file(log_path: &std::path::Path) -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok()
}

/// Dedicated log writer thread. Streaming threads only push into a channel;
/// all file I/O (open/append/flush) happens here, off the hot path.
static LOG_TX: Lazy<Mutex<mpsc::SyncSender<LogEntry>>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<LogEntry>(LOG_CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let dir = log_dir();
        let _ = std::fs::create_dir_all(&dir);
        let log_path = dir.join("mirror_rust.log.json");
        let rotated = dir.join("mirror_rust.log.json.1");

        let mut file = open_log_file(&log_path);
        let mut written: u64 = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

        while let Ok(entry) = rx.recv() {
            let Ok(json) = serde_json::to_string(&entry) else {
                continue;
            };
            if let Some(f) = file.as_mut() {
                if writeln!(f, "{}", json).is_ok() {
                    written += json.len() as u64 + 1;
                }
            }
            if written >= LOG_ROTATE_BYTES {
                // Close the handle before renaming — Windows refuses to
                // rename a file that still has an open handle.
                drop(file.take());
                let _ = std::fs::rename(&log_path, &rotated);
                file = open_log_file(&log_path);
                written = 0;
            }
        }
    });
    Mutex::new(tx)
});

/// Records an event.
///
/// Prefer the [`log_event!`](crate::log_event) macro, which builds the
/// [`Event`] for you and keeps the code and its parameters on one line.
pub fn record(event: Event) {
    let entry = LogEntry {
        timestamp: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        code: event.code().to_string(),
        level: event.level().to_string(),
        component: event.component().to_string(),
        action: event.action().to_string(),
        params: event
            .params()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        message: event.render_en(),
    };

    // Hand off to the writer thread — no file I/O here, and try_send so a
    // backed-up writer can never block a streaming thread.
    if let Ok(tx) = LOG_TX.lock() {
        let _ = tx.try_send(entry.clone());
    }

    if let Ok(mut logs) = LOG_BUFFER.lock() {
        logs.push(entry);
        if logs.len() > LOG_BUFFER_MAX {
            logs.drain(0..LOG_BUFFER_TRIM);
            if let Ok(mut cursor) = LOG_CURSOR.lock() {
                *cursor = cursor.saturating_sub(LOG_BUFFER_TRIM);
            }
        }
    }
}

/// Builds and records an event in one statement.
///
/// ```ignore
/// log_event!(codes::USB_STREAMING_THREAD_STARTED);
/// log_event!(codes::USB_STREAMING_CONFIG_SENT, "bytes" => n);
/// log_event!(codes::AOA_HANDSHAKE_ATTEMPT_FAILED, "attempt" => i + 1, "error" => format!("{e:?}"));
/// ```
///
/// Parameter names must match the `{name}` placeholders in the catalog. A
/// mismatch is not a compile error — the placeholder is left visible in the
/// rendered message, which is loud enough to notice in a log.
#[macro_export]
macro_rules! log_event {
    ($code:expr) => {
        $crate::telemetry::log::record($crate::telemetry::log::Event::new($code))
    };
    ($code:expr, $($name:literal => $value:expr),+ $(,)?) => {
        $crate::telemetry::log::record(
            $crate::telemetry::log::Event::new($code)
                $(.with($name, $value))+
        )
    };
}

/// Returns only entries added since the last call, so the interface can poll
/// without re-rendering the whole buffer.
pub fn take_new() -> Vec<LogEntry> {
    let logs = LOG_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    let mut cursor = LOG_CURSOR.lock().unwrap_or_else(|e| e.into_inner());
    let start = *cursor;
    let end = logs.len();
    if start >= end {
        return Vec::new();
    }
    let new_logs = logs[start..end].to_vec();
    *cursor = end;
    new_logs
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirror_i18n::codes;

    #[test]
    fn an_entry_carries_the_code_and_its_params_not_just_prose() {
        // The interface translates on `code`; if only `message` were populated
        // there would be nothing to translate against.
        let mut entry_params = std::collections::BTreeMap::new();
        entry_params.insert("bytes".to_string(), "412".to_string());

        let event = Event::new(codes::USB_STREAMING_CONFIG_SENT).with("bytes", 412);
        assert_eq!(event.code(), "usb.streaming.config_sent");
        assert_eq!(event.params()[0], ("bytes".to_string(), "412".to_string()));
        assert_eq!(event.render_en(), "Settings sent (412 bytes).");
        assert_eq!(event.level(), "SUCCESS");
    }

    #[test]
    fn take_new_never_returns_the_same_entry_twice() {
        // LOG_BUFFER is process-global and the rest of the suite runs in
        // parallel, so this cannot assume it is the only writer. It asserts
        // the cursor property instead: what one poll yields, the next does
        // not.
        let marker = "usb.streaming.session_reset";
        let count = |batch: &[LogEntry]| batch.iter().filter(|e| e.code == marker).count();

        // Drain whatever is pending from other tests.
        let _ = take_new();

        record(Event::new(codes::USB_STREAMING_SESSION_RESET));
        record(Event::new(codes::USB_STREAMING_SESSION_RESET));

        let first = take_new();
        assert_eq!(count(&first), 2, "both records should arrive in one poll");

        let second = take_new();
        assert_eq!(count(&second), 0, "a second poll must not repeat them");
    }
}
