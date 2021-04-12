use env_logger::fmt::Formatter;
use log::{Level, Record};
use serde::Serialize;
use std::{io::Write, thread};

/// Severity maps `log::Level` to Google Cloud Platform's notion of Severity.
/// https://cloud.google.com/logging/docs/reference/v2/rest/v2/LogEntry#LogSeverity
#[derive(Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Severity {
    Debug,
    Info,
    Warning,
    Error,
}

impl Severity {
    fn from_log_level(level: Level) -> Severity {
        match level {
            Level::Error => Severity::Error,
            Level::Warn => Severity::Warning,
            Level::Info => Severity::Info,
            Level::Debug => Severity::Debug,
            Level::Trace => Severity::Debug,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry<'a> {
    message: String,
    severity: Severity,
    time: String,
    module_path: Option<&'a str>,
    file: Option<&'a str>,
    line: Option<u32>,
    thread_id: String,
}

impl<'a> LogEntry<'a> {
    fn from_record(fmt: &Formatter, record: &'a Record) -> LogEntry<'a> {
        LogEntry {
            severity: Severity::from_log_level(record.level()),
            time: fmt.timestamp().to_string(),
            message: record.args().to_string(),
            module_path: record.module_path(),
            file: record.file(),
            line: record.line(),
            thread_id: format!("{:?}", thread::current().id()),
        }
    }
}

/// Sets up environment logging with GCP's expected format
pub fn setup_env_logging() {
    env_logger::builder()
        .format_timestamp_nanos()
        .format(|fmt, record| {
            let log_entry = LogEntry::from_record(fmt, record);
            match serde_json::to_string(&log_entry) {
                Ok(json_value) => writeln!(fmt, "{}", json_value),
                _ => writeln!(fmt, "{} - {}", record.level(), record.args()),
            }
        })
        .init();
}
