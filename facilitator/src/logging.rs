use serde_json::Value;
use slog::Record;
use slog::{Drain, Level, KV};

use serde::Serialize;
use slog_scope::GlobalLoggerGuard;
use std::convert;
use std::{
    collections::HashMap,
    io::{stderr, Write},
    sync::Mutex,
};

/// Severity maps `log::Level` to Google Cloud Platform's notion of Severity.
/// https://cloud.google.com/logging/docs/reference/v2/rest/v2/LogEntry#LogSeverity
#[derive(Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl convert::From<Level> for Severity {
    fn from(level: Level) -> Self {
        match level {
            Level::Critical => Severity::Critical,
            Level::Error => Severity::Error,
            Level::Warning => Severity::Warning,
            Level::Info => Severity::Info,
            Level::Debug => Severity::Debug,
            Level::Trace => Severity::Debug,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    message: String,
    severity: Severity,
    time: String,

    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

impl LogEntry {
    fn new(message: String, severity: Severity, time: String) -> Self {
        LogEntry {
            message,
            severity,
            time,
            extra: HashMap::new(),
        }
    }
}

impl convert::From<&Record<'_>> for LogEntry {
    fn from(record: &Record) -> Self {
        let severity: Severity = record.level().into();
        let message: String = record.msg().to_string();
        let time = chrono::Utc::now().to_rfc3339();

        let mut entry = LogEntry::new(message, severity, time);
        let _ = record.kv().serialize(record, &mut entry);

        entry
    }
}

/// slog::Serializer allows us to handler the KV records emitted by slog.
impl slog::Serializer for LogEntry {
    fn emit_arguments(&mut self, k: slog::Key, v: &std::fmt::Arguments) -> slog::Result {
        let value = match serde_json::to_value(v) {
            Ok(value) => value,
            Err(_) => Value::String(v.to_string()),
        };

        self.extra.insert(k.to_string(), value);

        Ok(())
    }
}

/// JSONDrain is a custom slog drain implementation that implements the
/// slog::Drain trait. It acts as a final output for the log messages.
#[derive(Debug)]
struct JSONDrain {}

impl slog::Drain for JSONDrain {
    type Ok = ();
    type Err = anyhow::Error;

    fn log(
        &self,
        record: &slog::Record,
        values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        // Convert the slog::Record into a LogEntry record
        let mut log_entry: LogEntry = record.into();
        // These are the values that were provided on creation of the logger
        // using the slog::o! macro.
        let _ = values.serialize(record, &mut log_entry);

        let _ = match serde_json::to_string(&log_entry) {
            Ok(json_value) => writeln!(stderr(), "{}", json_value),
            // Just write normally to stderr
            _ => writeln!(stderr(), "{:?} - {}", log_entry.severity, log_entry.message),
        };

        Ok(())
    }
}

/// Sets up environment logging with GCP's expected format
pub fn setup_env_logging() -> GlobalLoggerGuard {
    let drain = JSONDrain {};

    let root = slog::Logger::root(Mutex::new(drain).map(slog::Fuse), slog::o!());
    // If the return value of this goes out of scope, we don't have a logger anymore
    return slog_scope::set_global_logger(root);
}
