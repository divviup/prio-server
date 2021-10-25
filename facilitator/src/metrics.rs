use anyhow::{Context, Result};
use http::Response;
use prometheus::{
    register_int_counter_vec, register_int_gauge_vec, Encoder, IntCounterVec, IntGaugeVec,
    TextEncoder,
};
use slog::{error, info, o, Logger};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::runtime::Handle;
use warp::Filter;

/// Starts listening on an HTTP endpoint so that Prometheus can scrape metrics
/// from this instance. Returns an error if something goes wrong setting up the
/// endpoint.
pub fn start_metrics_scrape_endpoint(
    port: u16,
    runtime_handle: &Handle,
    parent_logger: &Logger,
) -> Result<()> {
    // scrape_logger will be moved into the closure passed to `Handle::spawn()`
    let scrape_logger = parent_logger.new(o!());

    // This task will run forever, so we intentionally drop the returned handle
    runtime_handle.spawn(async move {
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

    Ok(())
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
    pub intake_tasks_started: IntCounterVec,
    pub intake_tasks_finished: IntCounterVec,
    pub packets_per_batch: IntGaugeVec,
    pub packets_processed: IntCounterVec,
    pub bytes_processed: IntCounterVec,
    pub ingestion_batches_reader_metrics: BatchReaderMetricsCollector,
}

impl IntakeMetricsCollector {
    pub fn new() -> Result<Self> {
        let intake_tasks_started = register_int_counter_vec!(
            "facilitator_intake_tasks_started",
            "Number of intake-batch tasks that started (on the facilitator side)",
            &["aggregation_id"]
        )
        .context("failed to register metrics counter for started intakes")?;

        let intake_tasks_finished = register_int_counter_vec!(
            "facilitator_intake_tasks_finished",
            "Number of intake-batch tasks that finished (on the facilitator side)",
            &["status", "aggregation_id"]
        )
        .context("failed to register metrics counter for finished intakes")?;

        let packets_per_batch = register_int_gauge_vec!(
            "facilitator_intake_packets_per_ingestion_batch",
            "Number of packets in each ingestion batch",
            &["aggregation_id"]
        )
        .context("failed to register gauge for ingestion batch size")?;

        let packets_processed = register_int_counter_vec!(
            "facilitator_intake_ingestion_packets_processed",
            "Total number of ingestion packets processed during intake",
            &["aggregation_id"]
        )
        .context("failed to register counters for ingestion packets processed")?;

        let bytes_processed = register_int_counter_vec!(
            "facilitator_intake_bytes_processed",
            "Total number of bytes of real data processed (excluding encoding overhead)",
            &["aggregation_id"]
        )
        .context("failed to register counters for bytes of data processed")?;

        Ok(Self {
            intake_tasks_started,
            intake_tasks_finished,
            packets_per_batch,
            packets_processed,
            bytes_processed,
            ingestion_batches_reader_metrics: BatchReaderMetricsCollector::new("ingestion")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AggregateMetricsCollector {
    pub aggregate_tasks_started: IntCounterVec,
    pub aggregate_tasks_finished: IntCounterVec,
    pub peer_validation_batches_reader_metrics: BatchReaderMetricsCollector,
    pub packets_per_batch: IntGaugeVec,
    pub packets_processed: IntCounterVec,
    pub bytes_processed: IntCounterVec,
    pub packets_per_sum_part: IntGaugeVec,
}

impl AggregateMetricsCollector {
    pub fn new() -> Result<Self> {
        let aggregate_tasks_started = register_int_counter_vec!(
            "facilitator_aggregate_tasks_started",
            "Number of aggregate tasks that started (on the facilitator side)",
            &["aggregation_id"]
        )
        .context("failed to register metrics counter")?;

        let aggregate_tasks_finished = register_int_counter_vec!(
            "facilitator_aggregate_tasks_finished",
            "Number of aggregate tasks that finished (on the facilitator side)",
            &["status", "aggregation_id"]
        )
        .context("failed to register metrics counter")?;

        let packets_per_batch = register_int_gauge_vec!(
            "facilitator_aggregate_packets_per_batch",
            "Number of aggregated packets in each batch",
            &["aggregation_id"]
        )
        .context("failed to register gauge")?;

        let packets_processed = register_int_counter_vec!(
            "facilitator_aggregate_packets_processed",
            "Total number of packets aggregated",
            &["aggregation_id"]
        )
        .context("failed to register counters")?;

        let bytes_processed = register_int_counter_vec!(
            "facilitator_aggregate_bytes_processed",
            "Total number of bytes of real data aggregated (excluding encoding overhead)",
            &["aggregation_id"]
        )
        .context("failed to register counters")?;

        let packets_per_sum_part = register_int_gauge_vec!(
            "facilitator_aggregate_packets_per_sum_part",
            "Number of packets/contributions included in sum parts",
            &["aggregation_id"]
        )
        .context("failed to register gauges")?;

        Ok(Self {
            aggregate_tasks_started,
            aggregate_tasks_finished,
            peer_validation_batches_reader_metrics: BatchReaderMetricsCollector::new("peer")?,
            packets_per_batch,
            packets_processed,
            bytes_processed,
            packets_per_sum_part,
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
