//! Structured failure logging, shared by the client and the remote server.
//!
//! Failures are the events most worth capturing and the ones this tool was
//! worst at reporting: a fatal client error was printed as plain text even with
//! `--progress-json` active, and the remote server's diagnostics were
//! unstructured lines interleaved into the client's stderr.
//!
//! One record shape serves both sides so a consumer parses a single schema. The
//! `origin` field says which end produced a record, because "the transfer
//! failed" and "the far end failed" call for different responses.
//!
//! The server's stdout carries the binary protocol, so its sink can only ever
//! be stderr or a file — never stdout. `-` selects stderr for exactly that
//! reason.

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Schema version for records emitted by this module.
pub const FAILURE_LOG_SCHEMA_VERSION: u32 = 1;

/// Which end of the connection produced a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The local `xs` process driving the transfer.
    Client,
    /// The remote `xs --server` process.
    Server,
}

impl Origin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// Severity of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Informational lifecycle detail. Carried so a JSON sink stays a single
    /// parseable stream rather than a mix of JSON and bare text.
    Info,
    /// One entry failed while unrelated work continued.
    Failed,
    /// The run cannot continue.
    Fatal,
}

impl Severity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Failed => "failed",
            Self::Fatal => "fatal",
        }
    }
}

enum Sink {
    Stderr,
    File(std::fs::File),
}

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

fn sink() -> &'static Mutex<Option<Sink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

/// Errors from configuring the failure log.
#[derive(Debug, thiserror::Error)]
pub enum FailureLogError {
    /// The log file could not be opened for appending.
    #[error("cannot open the failure log at {path}: {source}")]
    Open {
        /// Path that could not be opened.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },
}

/// Point the failure log at `spec`: `-` for stderr, anything else a file path.
///
/// Records append rather than truncate, so a log survives across runs and a
/// post-mortem is not destroyed by the next invocation.
///
/// # Errors
///
/// Returns [`FailureLogError::Open`] when the path cannot be opened. This is
/// reported rather than ignored: a caller that asked for a failure log and
/// silently did not get one is worse off than one told immediately.
pub fn enable(spec: &str) -> Result<(), FailureLogError> {
    let new = if spec == "-" {
        Sink::Stderr
    } else {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(spec))
            .map_err(|source| FailureLogError::Open {
                path: spec.to_owned(),
                source,
            })?;
        Sink::File(file)
    };
    if let Ok(mut guard) = sink().lock() {
        *guard = Some(new);
    }
    Ok(())
}

/// Whether a sink is configured. Callers use this to skip building a record.
#[must_use]
pub fn is_enabled() -> bool {
    sink().lock().is_ok_and(|guard| guard.is_some())
}

/// One structured record.
#[derive(Debug, Clone)]
pub struct Record<'a> {
    /// Severity.
    pub severity: Severity,
    /// Which end produced it.
    pub origin: Origin,
    /// Stable machine-readable error family, e.g. `transport` or `io`.
    pub kind: &'a str,
    /// Destination-relative path, when the failure concerns one entry.
    pub path: Option<&'a str>,
    /// Remote authority this concerns, when the failure is about a peer.
    pub host: Option<&'a str>,
    /// Human-readable detail.
    pub message: &'a str,
}

/// Render a record as one JSON object.
///
/// Split from writing so the shape can be asserted in tests without touching a
/// file or stderr.
#[must_use]
pub fn render(record: &Record<'_>) -> String {
    let mut value = serde_json::Map::new();
    value.insert(
        "schema_version".to_owned(),
        serde_json::json!(FAILURE_LOG_SCHEMA_VERSION),
    );
    value.insert(
        "type".to_owned(),
        serde_json::json!(record.severity.as_str()),
    );
    value.insert(
        "timestamp_unix_nanos".to_owned(),
        serde_json::json!(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()),
    );
    value.insert(
        "origin".to_owned(),
        serde_json::json!(record.origin.as_str()),
    );
    value.insert("kind".to_owned(), serde_json::json!(record.kind));
    value.insert("pid".to_owned(), serde_json::json!(std::process::id()));
    if let Some(path) = record.path {
        value.insert("path".to_owned(), serde_json::json!(path));
    }
    if let Some(host) = record.host {
        value.insert("host".to_owned(), serde_json::json!(host));
    }
    value.insert("message".to_owned(), serde_json::json!(record.message));
    serde_json::Value::Object(value).to_string()
}

/// Write one record, if a sink is configured.
///
/// Failures to write are swallowed deliberately: losing a log line must never
/// turn a completed transfer into a failed one, and the alternative -- a panic
/// or an error path inside error reporting -- is worse than a missing line.
pub fn write(record: &Record<'_>) {
    let Ok(mut guard) = sink().lock() else {
        return;
    };
    let Some(target) = guard.as_mut() else {
        return;
    };
    let line = render(record);
    match target {
        Sink::Stderr => {
            let stderr = std::io::stderr();
            let mut handle = stderr.lock();
            let _ = writeln!(handle, "{line}");
            let _ = handle.flush();
        }
        Sink::File(file) => {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

/// Forward a line already rendered elsewhere, used for records relayed from the
/// remote so they reach the same sink without being re-encoded.
pub fn write_raw(line: &str) {
    let Ok(mut guard) = sink().lock() else {
        return;
    };
    let Some(target) = guard.as_mut() else {
        return;
    };
    match target {
        Sink::Stderr => {
            let stderr = std::io::stderr();
            let mut handle = stderr.lock();
            let _ = writeln!(handle, "{line}");
            let _ = handle.flush();
        }
        Sink::File(file) => {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> serde_json::Value {
        serde_json::from_str(line).expect("record must be valid JSON")
    }

    #[test]
    fn a_record_carries_the_fields_a_consumer_needs() {
        let line = render(&Record {
            severity: Severity::Fatal,
            origin: Origin::Server,
            kind: "transport",
            path: Some("nested/file.txt"),
            host: Some("user@example"),
            message: "peer disconnected",
        });
        let value = parse(&line);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["type"], "fatal");
        assert_eq!(value["origin"], "server");
        assert_eq!(value["kind"], "transport");
        assert_eq!(value["path"], "nested/file.txt");
        assert_eq!(value["host"], "user@example");
        assert_eq!(value["message"], "peer disconnected");
        assert!(value["timestamp_unix_nanos"].as_u64().is_some());
        assert!(value["pid"].as_u64().is_some());
    }

    #[test]
    fn optional_fields_are_omitted_rather_than_null() {
        // A consumer distinguishing "no path" from "path was null" should not
        // have to; absent means absent.
        let line = render(&Record {
            severity: Severity::Failed,
            origin: Origin::Client,
            kind: "io",
            path: None,
            host: None,
            message: "disk full",
        });
        let value = parse(&line);
        assert!(value.get("path").is_none());
        assert!(value.get("host").is_none());
        assert_eq!(value["origin"], "client");
    }

    #[test]
    fn messages_with_json_metacharacters_stay_parseable() {
        // Error text is not controlled by us: it can contain quotes, braces,
        // newlines, and backslashes from paths.
        let hostile = "cannot open \"C:\\a\\b\": {\"unterminated\n";
        let line = render(&Record {
            severity: Severity::Failed,
            origin: Origin::Client,
            kind: "io",
            path: Some(hostile),
            host: None,
            message: hostile,
        });
        assert!(!line.contains('\n'), "a record must be exactly one line");
        let value = parse(&line);
        assert_eq!(value["message"], hostile);
        assert_eq!(value["path"], hostile);
    }
}
