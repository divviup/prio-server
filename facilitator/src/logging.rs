use anyhow::{anyhow, Context, Result};
use atty::{self, Stream};
use serde::Serialize;
use slog::{o, Drain, FnValue, Level, LevelFilter, Logger, PushFnValue};
use slog_json::Json;
use slog_scope::GlobalLoggerGuard;
use slog_term::{FullFormat, PlainSyncDecorator, TermDecorator, TestStdoutWriter};
use std::{
    convert::From,
    fmt::{self, Display, Formatter},
    io::{stderr, Stderr},
    str::FromStr,
    thread,
};
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};

/// `event` defines constants for structured events
pub mod event {
    /// An event key is a key that could be encountered in the fields of a
    /// structured log message.
    type EventKey = &'static str;

    /// The trace ID of the event
    pub const TRACE_ID: EventKey = "trace_id";
    /// The task handle structure
    pub const TASK_HANDLE: EventKey = "task_handle";
    /// The name of the aggregation
    pub(crate) const AGGREGATION_NAME: EventKey = "aggregation_name";
    /// The storage path from which ingestion batches are read/written
    pub(crate) const INGESTION_PATH: EventKey = "ingestion_path";
    /// The storage path from which peer validation batches are read/written
    pub(crate) const PEER_VALIDATION_PATH: EventKey = "peer_validation_path";
    /// The UUID of a packet that something happened to
    pub(crate) const PACKET_UUID: EventKey = "packet_uuid";
    /// The ID (usually a UUID) of a batch that something happened to
    pub(crate) const BATCH_ID: EventKey = "batch_id";
    /// The date of a batch that something happened to
    pub(crate) const BATCH_DATE: EventKey = "batch_date";
    /// The path to some object store (e.g., an S3 bucket or a local directory)
    pub(crate) const STORAGE_PATH: EventKey = "path";
    /// The key for an object in some object store
    pub(crate) const STORAGE_KEY: EventKey = "key";
    /// An identity used while accessing some cloud resource (e.g., an AWS role ARN
    /// or a GCP service account email)
    pub(crate) const IDENTITY: EventKey = "identity";
    /// Acknowledgement identifier for a task in a queue
    pub(crate) const TASK_ACKNOWLEDGEMENT_ID: EventKey = "task_ack_id";
    /// Unique identifier for a task queue
    pub(crate) const TASK_QUEUE_ID: EventKey = "task_queue-id";
    /// Description of an action being retried
    pub(crate) const ACTION: EventKey = "action";
}

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
        write!(f, "{string}")
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
pub struct LoggingConfiguration<'a> {
    /// If true, logging output will be forced to JSON format using
    /// [slog-json][1]. If false, logging format will be determined by detecting
    /// whether `stderr` is a `tty`. If it is, output is formatted using
    /// [slog-term][2]. Otherwise, `slog-json` is used.
    ///
    /// [1]: https://docs.rs/slog-json
    /// [2]: https://docs.rs/slog-term
    pub force_json_output: bool,
    /// A version string which shall be attached to all log messages
    pub version_string: &'a str,
    /// Messages above this log level will be discarded
    pub log_level: &'a str,
}

/// IoErrorDrain is a supertrait that lets us work generically with
/// `slog::Drain`s.
trait IoErrorDrain: Drain<Ok = (), Err = std::io::Error> + Send {}

// Opt the `slog::Drain` implementations we use into IoErrorDrain
impl IoErrorDrain for Json<Stderr> {}
impl IoErrorDrain for FullFormat<TermDecorator> {}

/// Initialize logging resources. On success, returns a root [`slog::Logger`][1]
/// from which modules should create child loggers to add more key-value pairs
/// to the events they log, and a [`slog_scope::GlobalLoggerGuard`], which must
/// be kept live by the caller. Returns an error if `LoggingConfiguration` is
/// invalid.
///
/// [1]: https://docs.rs/slog/2.7.0/slog/struct.Logger.html
pub fn setup_logging(config: &LoggingConfiguration) -> Result<(Logger, GlobalLoggerGuard)> {
    // If stderr is not a tty, output logs as JSON structures on the assumption
    // that we are running in a cloud.
    let json_output = atty::isnt(Stream::Stderr) || config.force_json_output;

    // We have to box the Drain so that both branches return the same type
    let drain: Box<dyn IoErrorDrain> = if json_output {
        let json_drain = Json::new(stderr())
            .set_newlines(true)
            // slog_json::JsonBuilder::add_default_keys adds keys for
            // timestamp, level and the actual message, but we need the JSON
            // keys to match what the Google Cloud Logging agent expects:
            // https://cloud.google.com/logging/docs/agent/configuration#process-payload
            .add_key_value(o!(
                "time" => FnValue(|_| {
                    chrono::Local::now().to_rfc3339()
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
            "version"=> config.version_string.to_owned(),
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

    // Register the root logger in the global scope and then set it up to
    // capture messages emited by dependencies like Rusoto that use  the `log`
    // crate
    let scope_guard = slog_scope::set_global_logger(root_logger.clone());
    slog_stdlog::init().context("failed to initialize slog as log backend")?;

    // We also install a tracing subscriber to capture trace events from
    // dependencies like tokio and hyper
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_level(true)
        .with_target(true);

    let fmt_layer: Box<dyn tracing_subscriber::layer::Layer<_> + Send + Sync> = if json_output {
        Box::new(fmt_layer.json())
    } else {
        Box::new(fmt_layer.pretty())
    };

    let subscriber = Registry::default()
        .with(fmt_layer)
        // Configure filters with RUST_LOG env var. Format discussed at
        // https://docs.rs/tracing-subscriber/0.2.20/tracing_subscriber/filter/struct.EnvFilter.html
        .with(EnvFilter::from_default_env())
        .with(ErrorLayer::default());

    tracing::subscriber::set_global_default(subscriber).unwrap();

    Ok((root_logger, scope_guard))
}

/// Initialize logging for unit or integration tests. Must be public for
/// visibility in integration tests.
pub fn setup_test_logging() -> Logger {
    let decorator = PlainSyncDecorator::new(TestStdoutWriter);
    let drain = FullFormat::new(decorator).build().fuse();
    Logger::root(drain, o!())
}
