// TODO: remove once things are less stubby
#![allow(dead_code, unreachable_code, unused_variables)]

use anyhow::Result;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

mod config {
    use crate::config::{DayDuration, StoragePath};
    use anyhow::{ensure, Result};
    use chrono::Duration;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Paths {
        /// Mailbox where the ingestors will write new batches to.
        intake_input: Vec<StoragePath>,
        /// Mailboxes where our validity shares will be written to.
        validation_output: Vec<StoragePath>,
        /// Mailboxes where other share processors will write their validity shares to.
        validation_input: Vec<StoragePath>,
        /// Mailbox where aggregated sums will be written to.
        sum_output: StoragePath,

        /// Used for private intermediary storage between workflow steps.
        internal: StoragePath,
    }

    #[derive(Deserialize)]
    struct Keys {}

    #[derive(Deserialize)]
    pub struct Config {
        paths: Paths,
        keys: Keys,

        /// How often aggregations will be computed and output to the sum output bucket. If the
        /// duration does not evenly divide a day (24h) then the last period of the day will be
        /// truncated and intervals will always start aligned at the start of each day.
        aggregation_interval: DayDuration,
        /// Delays computing aggregations for a given time interval by this duration after the end
        /// of the interval, in order to help ensure delayed batches are not missing from it.
        aggregation_grace_period: DayDuration,
    }

    impl Config {
        pub fn validate(&self) -> Result<()> {
            ensure!(
                !self.paths.intake_input.is_empty(),
                "need at least one intake_input path"
            );
            ensure!(
                !self.paths.validation_output.is_empty(),
                "need at least one validation_output path"
            );
            ensure!(
                !self.paths.validation_input.is_empty(),
                "need at least one validation_input path"
            );

            ensure!(
                self.aggregation_interval.to_duration() <= Duration::days(1),
                "aggregation_interval must be at most 24 hours"
            );

            Ok(())
        }
    }
}

/// Prio share processor workflow manager
#[derive(Debug, StructOpt)]
pub struct WorkflowArgs {
    /// Enable verbose output to stderr
    #[structopt(short, long)]
    verbose: bool,

    /// Path to config file
    #[structopt(long)]
    config_path: PathBuf,
}

fn get_config(path: &Path) -> Result<config::Config> {
    // TODO load configuration files

    // TODO: since we don't have a config dpeloy step, this will probably need to hit various
    //   manifests and update its config based on those, ideally in a way amenable to moving to a
    //   separate independent step later

    let config: config::Config = todo!();
    config.validate()?;
    Ok(config)
}

fn dispatch_k8s_job_if_not_duplicate() -> Result<()> {
    // TODO: k8s functionality needed:
    //  - creds management of some kind
    //  - creating a Job based on a template (with placeholders being provided for flags to control
    //    inputs, etc.)
    //  - idempotently do nothing if a matching job already exists
    todo!()
}

fn process_ingestion() -> Result<()> {
    let file_list = todo!("list files in ingestion bucket");
    let batches_to_run: Vec<()> = todo!("scan file_list and identify complete batches");
    todo!("write .expected files in sum bucket");

    for batch in &batches_to_run {
        // TODO: Intake map job dispatch
        //  - Inputs: `.batch`, `.batch.avro`, `.batch.sig`
        //  - validate batch and produce validation data
        //  - Outputs: `.validity_N`, `.validity_N.avro`, `.validity_N.sig`
        //  - `.validity_N*` also copied to partner buckets
        //  - `.batch*` copied to next staging bucket
        //  - delete/archive inputs (at the end of launched job)

        dispatch_k8s_job_if_not_duplicate()?;
    }

    Ok(())
}

fn process_validation() -> Result<()> {
    let file_list = todo!("list files in validation bucketS");
    let batches_to_run: Vec<()> = todo!("scan file_list and identify complete batches");

    for batch in &batches_to_run {
        // TODO: Validate/sum job dispatch
        //  - Inputs: `.batch*`, `.validity_?*`
        //  - finish validation, produce sum data
        //  - Outputs: `.sum_N`, `.invalid_uuid_N.avro`
        //  At end of job:
        //  - delete matching `.expected` marker (optional, could be GCd by manager instead)
        //  - delete/archive inputs

        dispatch_k8s_job_if_not_duplicate()?;
    }

    Ok(())
}

fn process_reduce() -> Result<()> {
    // TODO: this is probably empty for now since validation will do it

    let file_list = todo!("list files in sum bucket");
    let batches_to_run: Vec<()> =
        todo!("group files by time intervals, skip batches with .expected markers");

    for batch in &batches_to_run {
        // TODO: reduce jobs
        //  - Inputs: time range, num of expected batches (just in case?)
        //  - lists Sum->Reduce bucket using the date range provided and aggregates
        //  - Outputs: `{date}-{date}.sum_N`, `.invalid_uuid_N.avro`, `.sum_N.sig`
        //  - delete/archive inputs (at the end of launched job)

        dispatch_k8s_job_if_not_duplicate()?;
    }

    Ok(())
}

pub fn workflow_main(args: WorkflowArgs) -> Result<()> {
    let config = get_config(&args.config_path)?;

    todo!("initialize environment: S3/k8s credentials, etc.");

    process_ingestion()?;
    process_validation()?;
    process_reduce()?;

    Ok(())
}
