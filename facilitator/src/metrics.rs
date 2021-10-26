use anyhow::{Context, Result};
use http::Response;
use prometheus::{
    register_int_counter, register_int_counter_vec, Encoder, IntCounter, IntCounterVec, TextEncoder,
};
use slog::{error, info, o, Logger};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::runtime::Runtime;
use warp::Filter;

/// Starts listening on an HTTP endpoint so that Prometheus can scrape metrics
/// from this instance. On success, returns a Runtime value that the caller must
/// keep live, or the task that handles Prometheus scrapes will not run. Returns
/// an error if something goes wrong setting up the endpoint.
pub fn start_metrics_scrape_endpoint(port: u16, parent_logger: &Logger) -> Result<Runtime> {
    // The default, multi-threaded runtime should suffice for our needs
    let runtime = Runtime::new().context("failed to create runtime for metrics endpoint")?;

    // scrape_logger will be moved into the closure passed to `runtime.spawn()`
    let scrape_logger = parent_logger.new(o!());

    // This task will run forever, so we intentionally drop the returned handle
    runtime.spawn(async move {
        // Clone scrape_logger so it can safely be moved into the closure that
        // handles metrics scrapes.
        let scrape_logger_clone = scrape_logger.clone();
        let endpoint = warp::get().and(warp::path("metrics")).map(move || {
            match handle_scrape() {
                Ok(body) => {
                    Response::builder()
                        // https://github.com/prometheus/docs/blob/master/content/docs/instrumenting/exposition_formats.md
                        .header("Content-Type", "text/plain; version=0.0.4")
                        .body(body)
                }
                Err(err) => {
                    error!(
                        scrape_logger_clone,
                        "unable to scrape Prometheus metrics: {}", err
                    );
                    Response::builder().status(500).body(vec![])
                }
            }
        });

        info!(scrape_logger, "serving metrics scrapes on 0.0.0.0:{}", port);
        warp::serve(endpoint)
            .run(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port))
            .await;
    });

    Ok(runtime)
}

fn handle_scrape() -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    TextEncoder::new()
        .encode(&prometheus::gather(), &mut buffer)
        .context("failed to encode metrics")?;
    Ok(buffer)
}

/// A group of collectors for intake tasks.
#[derive(Clone, Debug)]
pub struct IntakeMetricsCollector {
    pub intake_tasks_started: IntCounter,
    pub intake_tasks_finished: IntCounterVec,
}

impl IntakeMetricsCollector {
    pub fn new() -> Result<Self> {
        let intake_tasks_started: IntCounter = register_int_counter!(
            "facilitator_intake_tasks_started",
            "Number of intake-batch tasks that started (on the facilitator side)"
        )
        .context("failed to register metrics counter for started intakes")?;

        let intake_tasks_finished = register_int_counter_vec!(
            "facilitator_intake_tasks_finished",
            "Number of intake-batch tasks that finished (on the facilitator side)",
            &["status"]
        )
        .context("failed to register metrics counter for finished intakes")?;

        Ok(Self {
            intake_tasks_started,
            intake_tasks_finished,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AggregateMetricsCollector {
    pub aggregate_tasks_started: IntCounter,
    pub aggregate_tasks_finished: IntCounterVec,
    pub own_validation_batches_reader_metrics: BatchReaderMetricsCollector,
    pub peer_validation_batches_reader_metrics: BatchReaderMetricsCollector,
}

impl AggregateMetricsCollector {
    pub fn new() -> Result<Self> {
        let aggregate_tasks_started: IntCounter = register_int_counter!(
            "facilitator_aggregate_tasks_started",
            "Number of aggregate tasks that started (on the facilitator side)"
        )
        .context("failed to register metrics counter for started aggregations")?;

        let aggregate_tasks_finished = register_int_counter_vec!(
            "facilitator_aggregate_tasks_finished",
            "Number of aggregate tasks that finished (on the facilitator side)",
            &["status"]
        )
        .context("failed to register metrics counter for finished aggregations")?;

        Ok(Self {
            aggregate_tasks_started,
            aggregate_tasks_finished,
            own_validation_batches_reader_metrics: BatchReaderMetricsCollector::new("own")?,
            peer_validation_batches_reader_metrics: BatchReaderMetricsCollector::new("peer")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct BatchReaderMetricsCollector {
    pub invalid_validation_batches: IntCounterVec,
}

impl BatchReaderMetricsCollector {
    pub fn new(ownership: &str) -> Result<Self> {
        let invalid_validation_batches = register_int_counter_vec!(
            format!("facilitator_invalid_{}_validation_batches", ownership),
            format!(
                "Number of invalid {} validation batches encountered during aggregation",
                ownership
            ),
            &["reason"]
        )
        .context("failed to register metrics counter for invalid own validation batches")?;

        Ok(Self {
            invalid_validation_batches,
        })
    }
}
