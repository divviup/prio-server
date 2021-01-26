use anyhow::{Context, Result};
use http::Response;
use log::{error, info};
use prometheus::{
    register_int_counter, register_int_counter_vec, Encoder, IntCounter, IntCounterVec, TextEncoder,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::runtime::Runtime;
use warp::Filter;

/// Starts listening on an HTTP endpoint so that Prometheus can scrape metrics
/// from this instance. On success, returns a Runtime value that the caller must
/// keep live, or the task that handles Prometheus scrapes will not run. Returns
/// an error if something goes wrong setting up the endpoint.
pub fn start_metrics_scrape_endpoint(port: u16) -> Result<Runtime> {
    // The default, multi-threaded runtime should suffice for our needs
    let runtime = Runtime::new().context("failed to create runtime for metrics endpoint")?;

    // This task will run forever, so we intentionally drop the returned handle
    runtime.spawn(async move {
        let endpoint = warp::get().and(warp::path("metrics")).map(|| {
            match handle_scrape() {
                Ok(body) => {
                    Response::builder()
                        // https://github.com/prometheus/docs/blob/master/content/docs/instrumenting/exposition_formats.md
                        .header("Content-Type", "text/plain; version=0.0.4")
                        .body(body)
                }
                Err(err) => {
                    error!("unable to scrape Prometheus metrics: {}", err);
                    Response::builder().status(500).body(vec![])
                }
            }
        });

        info!("serving metrics scrapes on 0.0.0.0:{}", port);
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
#[derive(Debug)]
pub struct IntakeMetricsCollector {
    pub intake_tasks_started: IntCounter,
    pub intake_tasks_finished: IntCounterVec,
}

impl IntakeMetricsCollector {
    pub fn new() -> Result<IntakeMetricsCollector> {
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

        Ok(IntakeMetricsCollector {
            intake_tasks_started,
            intake_tasks_finished,
        })
    }
}

#[derive(Debug)]
pub struct AggregateMetricsCollector {
    pub aggregate_tasks_started: IntCounter,
    pub aggregate_tasks_finished: IntCounterVec,
}

impl AggregateMetricsCollector {
    pub fn new() -> Result<AggregateMetricsCollector> {
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

        Ok(AggregateMetricsCollector {
            aggregate_tasks_started,
            aggregate_tasks_finished,
        })
    }
}
