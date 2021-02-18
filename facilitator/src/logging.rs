use env_logger::fmt::Formatter;
use log::{Level, Record};
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Serialize)]
#[serde()]
enum Severity {
    DEBUG,
    INFO,
    WARNING,
    ERROR,
}

impl Severity {
    fn from_log_level(level: log::Level) -> Severity {
        match level {
            Level::Error => Severity::ERROR,
            Level::Warn => Severity::WARNING,
            Level::Info => Severity::INFO,
            Level::Debug => Severity::DEBUG,
            Level::Trace => Severity::DEBUG,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    message: String,
    severity: Severity,
    time: String,
}

impl LogEntry {
    fn from_record(fmt: &Formatter, record: &Record) -> LogEntry {
        LogEntry {
            severity: Severity::from_log_level(record.level()),
            time: fmt.timestamp().to_string(),
            message: record.args().to_string(),
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
