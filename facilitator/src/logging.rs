use anyhow::{anyhow, Result};
use atty::{self, Stream};
use serde::Serialize;
use slog::{o, Drain, FnValue, Level, LevelFilter, Logger, PushFnValue};
use slog_async;
use slog_json::Json;
use slog_scope::{self, GlobalLoggerGuard};
use slog_term::{FullFormat, PlainSyncDecorator, TermDecorator, TestStdoutWriter};
use std::{
    convert::From,
    fmt::{self, Display, Formatter},
    io::stderr,
    str::FromStr,
    thread,
};

/// An event key is a key that could be encountered in the fields of a
/// structured log message.
type EventKey = &'static str;

/// The trace ID of the event
pub const TRACE_ID_EVENT_KEY: EventKey = "trace_id";
/// The task handle structure
pub const TASK_HANDLE_EVENT_KEY: EventKey = "task_handle";
/// The name of the aggregation
pub(crate) const AGGREGATION_NAME_EVENT_KEY: EventKey = "aggregation_name";
/// The storage path from which ingestion batches are read/written
pub(crate) const INGESTION_PATH_EVENT_KEY: EventKey = "ingestion_path";
/// The storage path from which own validation batches are read/written
pub(crate) const OWN_VALIDATION_PATH_EVENT_KEY: EventKey = "own_validation_path";
/// The storage path from which peer validation batches are read/written
pub(crate) const PEER_VALIDATION_PATH_EVENT_KEY: EventKey = "peer_validation_path";
/// The UUID of a packet that something happened to
pub(crate) const PACKET_UUID_EVENT_KEY: EventKey = "packet_uuid";

/// Severity maps `log::Level` to Google Cloud Platform's notion of Severity.
/// https://cloud.google.com/logging/docs/reference/v2/rest/v2/LogEntry#LogSeverity
#[derive(Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Severity {
    Critical,
    Debug,
    Info,
    Warning,
    Error,
}

impl Display for Severity {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let string = match self {
            Severity::Critical => "CRITICAL",
            Severity::Debug => "DEBUG",
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        };
        write!(f, "{}", string)
    }
}

impl From<Level> for Severity {
    fn from(slog_level: Level) -> Self {
        match slog_level {
            Level::Critical => Severity::Critical,
            Level::Error => Severity::Error,
            Level::Warning => Severity::Warning,
            Level::Info => Severity::Info,
            Level::Debug => Severity::Debug,
            Level::Trace => Severity::Debug,
        }
    }
}

/// Options for configuring logging in this application
pub struct LoggingConfiguration {
    /// If true, logging output will be forced to JSON format using
    /// [slog-json][1]. If false, logging format will be determined by detecting
    /// whether `stderr` is a `tty`. If it is, output is formatted using
    /// [slog-term][2]. Otherwise, `slog-json` is used.
    ///
    /// [1]: https://docs.rs/slog-json
    /// [2]: https://docs.rs/slog-term
    pub force_json_output: bool,
    /// A version string which shall be attached to all log messages
    pub version_string: &'static str,
    /// Messages above this log level will be discarded
    pub log_level: &'static str,
}

/// Initialize logging resources. On success, returns a root [`slog::Logger`][1]
/// from which child loggers may be built and a GlobalLoggerGuard which must be
/// kept live by the caller to enable the `slog_scope` global logger to function
/// in modules that haven't yet opted into managing their own `slog::Logger`.
/// Return an error if `LoggingConfiguration` is invalid.
///
/// [1]: https://docs.rs/slog/2.7.0/slog/struct.Logger.html
pub fn setup_logging(config: &LoggingConfiguration) -> Result<(Logger, GlobalLoggerGuard)> {
    // We have to box the Drain so that both branches return the same type
    let drain: Box<dyn Drain<Ok = (), Err = std::io::Error> + Send> =
        if atty::isnt(Stream::Stderr) || config.force_json_output {
            // If stderr is not a tty, output logs as JSON structures on the
            // assumption that we are running in a cloud deployment.
            let json_drain = Json::new(stderr())
                .set_newlines(true)
                // slog_json::JsonBuilder::add_default_keys adds keys for
                // timestamp, level and the actual message, but we need the JSON
                // keys to match what the Google Cloud Logging agent expects:
                // https://cloud.google.com/logging/docs/agent/configuration#process-payload
                .add_key_value(o!(
                    "time" => PushFnValue(|_, serializer| {
                        serializer.emit(chrono::Local::now().to_rfc3339())
                    }),
                    "severity" => FnValue(|record| {
                        Severity::from(record.level()).to_string()
                    }),
                    "message" => PushFnValue(|record, serializer| {
                        serializer.emit(record.msg())
                    }),
                ))
                .build();
            Box::new(json_drain)
        } else {
            // stderr is a tty and output was not forced to JSON, so use a
            // terminal pretty-printer
            let decorator = TermDecorator::new().stderr().build();
            Box::new(FullFormat::new(decorator).build())
        };

    // Create a filter to discard messages above desired level
    let log_level = slog::Level::from_str(config.log_level)
        .map_err(|_| anyhow!("{} is not a valid log level", config.log_level))?;
    let level_filter = LevelFilter::new(drain, log_level);

    // Use slog_async to make it safe to clone loggers across threads
    let drain = slog_async::Async::new(level_filter.fuse()).build().fuse();
    let root_logger = Logger::root(
        drain,
        o!(
            "version"=> config.version_string,
            "file" => FnValue(|record| {
                record.file()
            }),
            "line" => FnValue(|record| {
                record.line()
            }),
            "module_path" => FnValue(|record| {
                record.module()
            }),
            "thread_id" => FnValue(|_| {
                format!("{:?}", thread::current().id())
            })
        ),
    );

    // Over time, all modules in `facilitator` should begin using custom
    // `slog::Loggers` decorated with appropriate k-v pairs, and we can stop creating
    let slog_scope_guard = slog_scope::set_global_logger(root_logger.new(o!()));

    Ok((root_logger, slog_scope_guard))
}

/// Initialize logging for unit or integration tests. Must be public for
/// visibility in integration tests.
pub fn setup_test_logging() -> Logger {
    let decorator = PlainSyncDecorator::new(TestStdoutWriter);
    let drain = FullFormat::new(decorator).build().fuse();
    Logger::root(drain, o!())
}
