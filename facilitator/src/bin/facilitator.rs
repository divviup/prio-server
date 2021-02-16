use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, NaiveDateTime};
use clap::{value_t, App, Arg, ArgMatches, SubCommand};
use log::{error, info};
use prio::encrypt::PrivateKey;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING,
};
use std::{collections::HashMap, fs, fs::File, io::Read, str::FromStr, time::Instant};
use uuid::Uuid;

use facilitator::{
    aggregation::BatchAggregator,
    config::{Identity, ManifestKind, StoragePath, TaskQueueKind},
    intake::BatchIntaker,
    manifest::{
        DataShareProcessorGlobalManifest, IngestionServerManifest, PortalServerGlobalManifest,
        SpecificManifest,
    },
    metrics::{start_metrics_scrape_endpoint, AggregateMetricsCollector, IntakeMetricsCollector},
    sample::{generate_ingestion_sample, SampleOutput},
    task::{AggregationTask, AwsSqsTaskQueue, GcpPubSubTaskQueue, IntakeBatchTask, TaskQueue},
    transport::{
        GCSTransport, LocalFileTransport, S3Transport, SignableTransport, Transport,
        VerifiableAndDecryptableTransport, VerifiableTransport,
    },
    BatchSigningKey, Error, DATE_FORMAT,
};

fn num_validator<F: FromStr>(s: String) -> Result<(), String> {
    s.parse::<F>()
        .map(|_| ())
        .map_err(|_| "could not parse value as number".to_owned())
}

fn date_validator(s: String) -> Result<(), String> {
    NaiveDateTime::parse_from_str(&s, DATE_FORMAT)
        .map(|_| ())
        .map_err(|e| format!("{} {}", s, e.to_string()))
}

fn uuid_validator(s: String) -> Result<(), String> {
    Uuid::parse_str(&s).map(|_| ()).map_err(|e| e.to_string())
}

fn path_validator(s: String) -> Result<(), String> {
    StoragePath::from_str(&s)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// Trait applied to clap::App to extend its builder pattern with some helpers
// specific to our use case.
trait AppArgumentAdder {
    fn add_is_first_argument(self) -> Self;

    fn add_instance_name_argument(self) -> Self;

    fn add_manifest_base_url_argument(self, entity: Entity) -> Self;

    fn add_storage_arguments(self, entity: Entity, in_out: InOut) -> Self;

    fn add_batch_public_key_arguments(self, entity: Entity) -> Self;

    fn add_batch_signing_key_arguments(self) -> Self;

    fn add_packet_decryption_key_argument(self) -> Self;

    fn add_gcp_service_account_key_file_argument(self) -> Self;

    fn add_task_queue_arguments(self) -> Self;

    fn add_metrics_scrape_port_argument(self) -> Self;

    fn add_use_bogus_packet_file_digest_argument(self) -> Self;
}

const SHARED_HELP: &str = "Storage arguments: Any flag ending in -input or -output can take an \
     S3 bucket (s3://<region>/<bucket>), a Google Storage bucket (gs://), \
     or a local directory name. The corresponding -identity flag specifies \
     what identity to use with a bucket.
     
     For S3 buckets: An identity flag may contain an AWS IAM role, specified \
     using an ARN (i.e. \"arn:...\"). Facilitator will assume that role \
     using an OIDC auth token obtained from the GKE metadata service. \
     Appropriate mappings need to be in place from Facilitator's k8s \
     service account to its GCP service account to the IAM role. If \
     the identity flag is omitted or is the empty string, use credentials from \
     ~/.aws.

     For GS buckets: An identity flag may contain a GCP service account \
     (identified by an email address). Requests to Google Storage (gs://) \
     are always authenticated as one of our service accounts by GKE's \
     Workload Identity feature: \
     https://cloud.google.com/kubernetes-engine/docs/how-to/workload-identity. \
     If an identity flag is set, facilitator will use its default service account \
     to impersonate a different account, which should have permissions to write \
     to or read from the named bucket. If an identity flag is omitted or is \
     the empty string, the default service account exposed by the GKE metadata \
     service is used. \
     \
     Keys: All keys are P-256. Public keys are base64-encoded DER SPKI. Private \
     keys are in the base64 encoded format expected by libprio-rs, or base64-encoded \
     PKCS#8, as documented. \
    ";

/// The string "-input" or "-output", for appending to arg names.
enum InOut {
    Input,
    Output,
}

impl InOut {
    fn str(&self) -> &'static str {
        match self {
            InOut::Input => "-input",
            InOut::Output => "-output",
        }
    }
}

/// One of the organizations participating in the Prio system.
enum Entity {
    Ingestor,
    Peer,
    Own,
    Portal,
}

/// We need to be able to give &'static strs to `clap`, but sometimes we want to generate them
/// with format!(), which generates a String. This leaks a String in order to give us a &'static str.
fn leak_string(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Entity {
    fn str(&self) -> &'static str {
        match self {
            Entity::Ingestor => "ingestor",
            Entity::Peer => "peer",
            Entity::Own => "own",
            Entity::Portal => "portal",
        }
    }

    /// Return the lowercase name of this entity, plus a suffix.
    /// Intentionally leak the resulting string so it can be used
    /// as a &'static str by clap.
    fn suffix(&self, s: &str) -> &'static str {
        leak_string(format!("{}{}", self.str(), s))
    }
}

fn upper_snake_case(s: &str) -> String {
    s.to_uppercase().replace("-", "_")
}

impl<'a, 'b> AppArgumentAdder for App<'a, 'b> {
    fn add_is_first_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("is-first")
                .env("IS_FIRST")
                .long("is-first")
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .required(true)
                .help(
                    "Whether this is the \"first\" server receiving a share, \
                    i.e., the PHA.",
                ),
        )
    }
    fn add_instance_name_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("instance-name")
                .long("instance-name")
                .env("INSTANCE_NAME")
                .value_name("NAME")
                .required(true)
                .help("Name of this data share processor")
                .long_help(
                    "Name of this data share processor instance, to be used to \
                    look up manifests to discover resources owned by this \
                    server and peers. e.g., the instance for the state \"zc\" \
                    and ingestor server \"megacorp\" would be \"zc-megacorp\".",
                ),
        )
    }

    fn add_manifest_base_url_argument(self: App<'a, 'b>, entity: Entity) -> App<'a, 'b> {
        let name = entity.suffix("-manifest-base-url");
        let name_env = leak_string(upper_snake_case(name));
        self.arg(
            Arg::with_name(name)
                .long(name)
                .env(name_env)
                .value_name("BASE_URL")
                .help("Base URL relative to which manifests should be fetched")
                .long_help(leak_string(format!(
                    "Base URL from which the {} vends manifests, \
                    enabling this data share processor to retrieve the global \
                    or specific manifest for the server and obtain storage \
                    buckets and batch signing public keys.",
                    entity.str()
                ))),
        )
    }

    fn add_storage_arguments(self: App<'a, 'b>, entity: Entity, in_out: InOut) -> App<'a, 'b> {
        let name = entity.suffix(in_out.str());
        let name_env = leak_string(upper_snake_case(name));
        let id = entity.suffix("-identity");
        let id_env = leak_string(upper_snake_case(id));
        self.arg(
            Arg::with_name(name)
                .long(name)
                .env(name_env)
                .value_name("PATH")
                .validator(path_validator)
                .help("Storage path (gs://, s3:// or local dir name)"),
        )
        .arg(
            Arg::with_name(id)
                .long(id)
                .env(id_env)
                .value_name("IAM_ROLE_OR_SERVICE_ACCOUNT")
                .help(leak_string(format!(
                    "Identity to assume when using S3 or GS storage APIs for {} bucket.",
                    entity.str()
                ))),
        )
    }

    fn add_batch_public_key_arguments(self: App<'a, 'b>, entity: Entity) -> App<'a, 'b> {
        self.arg(
            Arg::with_name(entity.suffix("-public-key"))
                .long(entity.suffix("-public-key"))
                .value_name("B64")
                .help(leak_string(format!(
                    "Batch signing public key for the {}",
                    entity.str()
                ))),
        )
        .arg(
            Arg::with_name(entity.suffix("-public-key-identifier"))
                .long(entity.suffix("-public-key-identifier"))
                .value_name("KEY_ID")
                .help(leak_string(format!(
                    "Identifier for the {}'s batch keypair",
                    entity.str()
                ))),
        )
    }

    fn add_batch_signing_key_arguments(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("batch-signing-private-key")
                .long("batch-signing-private-key")
                .env("BATCH_SIGNING_PRIVATE_KEY")
                .value_name("B64_PKCS8")
                .help("Batch signing private key for this server")
                .long_help(
                    "Base64 encoded PKCS#8 document containing P-256 \
                    batch signing private key to be used by this server when \
                    sending messages to other servers.",
                )
                .required(true),
        )
        .arg(
            Arg::with_name("batch-signing-private-key-identifier")
                .long("batch-signing-private-key-identifier")
                .env("BATCH_SIGNING_PRIVATE_KEY_IDENTIFIER")
                .value_name("ID")
                .help("Batch signing private key identifier")
                .long_help(
                    "Identifier for the batch signing keypair to use, \
                    corresponding to an entry in this server's global \
                    or specific manifest. Used to construct \
                    PrioBatchSignature messages.",
                )
                .required(true),
        )
    }

    fn add_packet_decryption_key_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("packet-decryption-keys")
                .long("packet-decryption-keys")
                .value_name("B64")
                .env("PACKET_DECRYPTION_KEYS")
                .long_help(
                    "List of packet decryption private keys, comma separated. \
                    When decrypting packets, all provided keys will be tried \
                    until one works.",
                )
                .multiple(true)
                .min_values(1)
                .use_delimiter(true)
                .required(true),
        )
    }

    fn add_gcp_service_account_key_file_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("gcp-service-account-key-file")
                .long("gcp-service-account-key-file")
                .env("GCP_SERVICE_ACCOUNT_KEY_FILE")
                .help("Path to key file for GCP service account")
                .long_help(
                    "Path to the JSON key file for the GCP service account \
                    that should be used by for accessing GCS or impersonating \
                    other GCP service accounts. If omitted, the default \
                    account found in the GKE metadata service will be used for \
                    authentication or impersonation.",
                ),
        )
    }

    fn add_task_queue_arguments(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("task-queue-kind")
                .long("task-queue-kind")
                .env("TASK_QUEUE_KIND")
                .help("kind of task queue to use")
                .possible_value(leak_string(TaskQueueKind::GcpPubSub.to_string()))
                .possible_value(leak_string(TaskQueueKind::AwsSqs.to_string()))
                .required(true),
        )
        .arg(
            Arg::with_name("task-queue-name")
                .long("task-queue-name")
                .env("TASK_QUEUE_NAME")
                .help("Name of queue from which tasks should be pulled.")
                .long_help(
                    "Name of queue from which tasks should be pulled. On GCP, \
                    a PubSub subscription ID. On AWS, an SQS queue URL.",
                )
                .required(true),
        )
        .arg(
            Arg::with_name("task-queue-identity")
                .long("task-queue-identity")
                .env("TASK_QUEUE_IDENTITY")
                .help("Identity to assume when accessing task queue"),
        )
        .arg(
            Arg::with_name("gcp-project-id")
                .long("gcp-project-id")
                .env("GCP_PROJECT_ID")
                .help("Project ID for GCP PubSub topic")
                .long_help(
                    "The GCP Project ID in which the PubSub topic implementing \
                    the work queue was created. Required if the task queue is \
                    a PubSub topic.",
                ),
        )
        .arg(
            Arg::with_name("pubsub-api-endpoint")
                .long("pubsub-api-endpoint")
                .env("PUBSUB_API_ENDPOINT")
                .help("API endpoint for GCP PubSub")
                .default_value("https://pubsub.googleapis.com")
                .help("API endpoint for GCP PubSub. Optional."),
        )
        .arg(
            Arg::with_name("aws-sqs-region")
                .long("aws-sqs-region")
                .env("AWS_SQS_REGION")
                .help("AWS region in which to use SQS"),
        )
    }

    fn add_metrics_scrape_port_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("metrics-scrape-port")
                .long("metrics-scrape-port")
                .env("METRICS_SCRAPE_PORT")
                .help("TCP port on which to expose Prometheus /metrics endpoint")
                .default_value("8080")
                .validator(num_validator::<u16>),
        )
    }

    fn add_use_bogus_packet_file_digest_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("use-bogus-packet-file-digest")
                .long("use-bogus-packet-file-digest")
                .env("USE_BOGUS_PACKET_FILE_DIGEST")
                .help("whether to tamper with validation batch headers")
                .long_help(
                    "If set, then instead of the computed digest of the packet \
                    file, a fixed, bogusddigest will be inserted into the own \
                    and peer validation batch headers written out during \
                    intake tasks. This should only be used in test scenarios \
                    to simulate buggy data share processors.",
                )
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false"),
        )
    }
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::builder().format_timestamp_millis().init();

    let args: Vec<String> = std::env::args().collect();
    info!(
        "starting {} version {}. Args: [{}]",
        args[0],
        option_env!("BUILD_INFO").unwrap_or("(BUILD_INFO unavailable)"),
        args[1..].join(" "),
    );
    let matches = App::new("facilitator")
        .about("Prio data share processor")
        .arg(
            Arg::with_name("pushgateway")
                .long("pushgateway")
                .env("PUSHGATEWAY")
                .help("Address of a Prometheus pushgateway to push metrics to, in host:port form"),
        )
        .subcommand(
            SubCommand::with_name("generate-ingestion-sample")
                .about("Generate sample data files")
                .add_gcp_service_account_key_file_argument()
                .add_storage_arguments(Entity::Peer, InOut::Output)
                .add_storage_arguments(Entity::Own, InOut::Output)
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .required(true)
                        .help("Name of the aggregation"),
                )
                .arg(
                    Arg::with_name("batch-id")
                        .long("batch-id")
                        .value_name("UUID")
                        .help(
                            "UUID of the batch. If omitted, a UUID is \
                            randomly generated.",
                        )
                        .validator(uuid_validator),
                )
                .arg(
                    Arg::with_name("date")
                        .long("date")
                        .value_name("DATE")
                        .help("Date for the batch in YYYY/mm/dd/HH/MM format")
                        .long_help(
                            "Date for the batch in YYYY/mm/dd/HH/MM format. If \
                            omitted, the current date is used.",
                        )
                        .validator(date_validator),
                )
                .arg(
                    Arg::with_name("dimension")
                        .long("dimension")
                        .short("d")
                        .value_name("INT")
                        .required(true)
                        .validator(num_validator::<i32>)
                        .help(
                            "Length in bits of the data packets to generate \
                            (a.k.a. \"bins\" in some contexts). Must be a \
                            natural number.",
                        ),
                )
                .arg(
                    Arg::with_name("packet-count")
                        .long("packet-count")
                        .short("p")
                        .value_name("INT")
                        .required(true)
                        .validator(num_validator::<usize>)
                        .help("Number of data packets to generate"),
                )
                .arg(
                    Arg::with_name("pha-ecies-private-key")
                        .long("pha-ecies-private-key")
                        .env("PHA_ECIES_PRIVATE_KEY")
                        .value_name("B64")
                        .help(
                            "Base64 encoded ECIES private key for the PHA \
                            server",
                        )
                        .required(true),
                )
                .arg(
                    Arg::with_name("facilitator-ecies-private-key")
                        .long("facilitator-ecies-private-key")
                        .env("FACILITATOR_ECIES_PRIVATE_KEY")
                        .value_name("B64")
                        .help(
                            "Base64 encoded ECIES private key for the \
                            facilitator server",
                        )
                        .required(true),
                )
                .add_batch_signing_key_arguments()
                .arg(
                    Arg::with_name("epsilon")
                        .long("epsilon")
                        .value_name("DOUBLE")
                        .help(
                            "Differential privacy parameter for local \
                            randomization before aggregation",
                        )
                        .required(true)
                        .validator(num_validator::<f64>),
                )
                .arg(
                    Arg::with_name("batch-start-time")
                        .long("batch-start-time")
                        .value_name("MILLIS")
                        .help("Start of timespan covered by the batch, in milliseconds since epoch")
                        .required(true)
                        .validator(num_validator::<i64>),
                )
                .arg(
                    Arg::with_name("batch-end-time")
                        .long("batch-end-time")
                        .value_name("MILLIS")
                        .help("End of timespan covered by the batch, in milliseconds since epoch")
                        .required(true)
                        .validator(num_validator::<i64>),
                ),
        )
        .subcommand(
            SubCommand::with_name("intake-batch")
                .about(format!("Validate an input share (from an ingestor's bucket) and emit a validation share.\n\n{}", SHARED_HELP).as_str())
                .add_instance_name_argument()
                .add_is_first_argument()
                .add_gcp_service_account_key_file_argument()
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .required(true)
                        .help("Name of the aggregation"),
                )
                .arg(
                    Arg::with_name("batch-id")
                        .long("batch-id")
                        .value_name("UUID")
                        .help("UUID of the batch.")
                        .required(true)
                        .validator(uuid_validator),
                )
                .arg(
                    Arg::with_name("date")
                        .long("date")
                        .value_name("DATE")
                        .help("Date for the batch in YYYY/mm/dd/HH/MM format")
                        .validator(date_validator)
                        .required(true),
                )
                .add_packet_decryption_key_argument()
                .add_batch_public_key_arguments(Entity::Ingestor)
                .add_batch_signing_key_arguments()
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Output)
                .add_storage_arguments(Entity::Own, InOut::Output)
                .add_use_bogus_packet_file_digest_argument()
        )
        .subcommand(
            SubCommand::with_name("aggregate")
                .about(format!("Verify peer validation share and emit sum part.\n\n{}", SHARED_HELP).as_str())
                .add_instance_name_argument()
                .add_is_first_argument()
                .add_gcp_service_account_key_file_argument()
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .required(true)
                        .help("Name of the aggregation"),
                )
                .arg(
                    Arg::with_name("batch-id")
                        .long("batch-id")
                        .multiple(true)
                        .value_name("UUID")
                        .help(
                            "Batch IDs being aggregated. May be specified \
                            multiple times.",
                        )
                        .long_help(
                            "Batch IDs being aggregated. May be specified \
                            multiple times. Must be specified in the same \
                            order as batch-time values.",
                        )
                        .min_values(1)
                        .validator(uuid_validator),
                )
                .arg(
                    Arg::with_name("batch-time")
                        .long("batch-time")
                        .multiple(true)
                        .value_name("DATE")
                        .help("Date for the batches in YYYY/mm/dd/HH/MM format")
                        .long_help(
                            "Date for the batches in YYYY/mm/dd/HH/MM format. \
                            Must be specified in the same order as batch-id \
                            values.",
                        )
                        .min_values(1)
                        .validator(date_validator),
                )
                .arg(
                    Arg::with_name("aggregation-start")
                        .long("aggregation-start")
                        .value_name("DATE")
                        .help("Beginning of the timespan covered by the aggregation.")
                        .required(true)
                        .validator(date_validator),
                )
                .arg(
                    Arg::with_name("aggregation-end")
                        .long("aggregation-end")
                        .value_name("DATE")
                        .help("End of the timespan covered by the aggregation.")
                        .required(true)
                        .validator(date_validator),
                )
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_batch_public_key_arguments(Entity::Ingestor)
                .add_manifest_base_url_argument(Entity::Own)
                .add_storage_arguments(Entity::Own, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Input)
                .add_batch_public_key_arguments(Entity::Peer)
                .add_manifest_base_url_argument(Entity::Portal)
                .add_storage_arguments(Entity::Portal, InOut::Output)
                .add_packet_decryption_key_argument()
                .add_batch_signing_key_arguments()
        )
        .subcommand(
            SubCommand::with_name("lint-manifest")
            .about("Validate and print out global or specific manifests")
            .arg(
                Arg::with_name("manifest-base-url")
                    .long("manifest-base-url")
                    .value_name("URL")
                    .help("base URL relative to which manifests may be fetched")
                    .long_help(
                        "base URL relative to which manifests may be fetched \
                        over HTTPS. Should be in the form \"https://foo.com\"."
                    )
                    .required_unless("manifest-path")
            )
            .arg(
                Arg::with_name("manifest-path")
                    .long("manifest-path")
                    .value_name("PATH")
                    .help("path to local manifest file to lint")
                    .required_unless("manifest-base-url")
            )
            .arg(
                Arg::with_name("manifest-kind")
                    .long("manifest-kind")
                    .value_name("KIND")
                    .help("kind of manifest to locate and parse")
                    .possible_value(leak_string(ManifestKind::IngestorGlobal.to_string()))
                    .possible_value(leak_string(ManifestKind::IngestorSpecific.to_string()))
                    .possible_value(leak_string(ManifestKind::DataShareProcessorGlobal.to_string()))
                    .possible_value(leak_string(ManifestKind::DataShareProcessorSpecific.to_string()))
                    .possible_value(leak_string(ManifestKind::PortalServerGlobal.to_string()))
                    .required(true)
                )
            .arg(
                Arg::with_name("instance")
                    .long("instance")
                    .value_name("INSTANCE_NAME")
                    .help("the instance name whose manifest is to be fetched")
                    .long_help(
                        leak_string(format!("the instance name whose manifest is to be fetched, \
                        e.g., \"mi-google\" for a data share processor specific manifest or \"mi\" \
                        for an ingestor specific manifest. Required if manifest-kind={} or {}.",
                        ManifestKind::DataShareProcessorSpecific, ManifestKind::IngestorSpecific))
                    )
            )
        )
        .subcommand(
            SubCommand::with_name("intake-batch-worker")
                .about(format!("Consume intake batch tasks from a queue, validating an input share (from an ingestor's bucket) and emit a validation share.\n\n{}", SHARED_HELP).as_str())
                .add_instance_name_argument()
                .add_is_first_argument()
                .add_gcp_service_account_key_file_argument()
                .add_packet_decryption_key_argument()
                .add_batch_public_key_arguments(Entity::Ingestor)
                .add_batch_signing_key_arguments()
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Output)
                .add_storage_arguments(Entity::Own, InOut::Output)
                .add_task_queue_arguments()
                .add_metrics_scrape_port_argument()
                .add_use_bogus_packet_file_digest_argument()
        )
        .subcommand(
            SubCommand::with_name("aggregate-worker")
                .about(format!("Consume aggregate tasks from a queue.\n\n{}", SHARED_HELP).as_str())
                .add_instance_name_argument()
                .add_is_first_argument()
                .add_gcp_service_account_key_file_argument()
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_batch_public_key_arguments(Entity::Ingestor)
                .add_manifest_base_url_argument(Entity::Own)
                .add_storage_arguments(Entity::Own, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Input)
                .add_batch_public_key_arguments(Entity::Peer)
                .add_manifest_base_url_argument(Entity::Portal)
                .add_storage_arguments(Entity::Portal, InOut::Output)
                .add_packet_decryption_key_argument()
                .add_batch_signing_key_arguments()
                .add_task_queue_arguments()
                .add_metrics_scrape_port_argument()
        )
        .get_matches();

    let result = match matches.subcommand() {
        // The configuration of the Args above should guarantee that the
        // various parameters are present and valid, so it is safe to use
        // unwrap() here.
        ("generate-ingestion-sample", Some(sub_matches)) => generate_sample(sub_matches),
        ("intake-batch", Some(sub_matches)) => intake_batch_subcommand(sub_matches),
        ("intake-batch-worker", Some(sub_matches)) => intake_batch_worker(sub_matches),
        ("aggregate", Some(sub_matches)) => aggregate_subcommand(sub_matches),
        ("aggregate-worker", Some(sub_matches)) => aggregate_worker(sub_matches),
        ("lint-manifest", Some(sub_matches)) => lint_manifest(sub_matches),
        (_, _) => Ok(()),
    };

    result
}

fn generate_sample(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let peer_output_path = StoragePath::from_str(sub_matches.value_of("peer-output").unwrap())?;
    let peer_identity = sub_matches.value_of("peer-identity");
    let packet_encryption_key = PrivateKey::from_base64(
        sub_matches
            .value_of("facilitator-ecies-private-key")
            .unwrap(),
    )
    .unwrap();
    let mut peer_transport = SampleOutput {
        transport: SignableTransport {
            transport: transport_for_path(peer_output_path, peer_identity, sub_matches)?,
            batch_signing_key: batch_signing_key_from_arg(sub_matches)?,
        },
        packet_encryption_key,
        drop_nth_packet: None,
    };

    let own_output_path = StoragePath::from_str(sub_matches.value_of("own-output").unwrap())?;
    let own_identity = sub_matches.value_of("own-identity");
    let mut own_transport = SampleOutput {
        transport: SignableTransport {
            transport: transport_for_path(own_output_path, own_identity, sub_matches)?,
            batch_signing_key: batch_signing_key_from_arg(sub_matches)?,
        },
        packet_encryption_key: PrivateKey::from_base64(
            sub_matches.value_of("pha-ecies-private-key").unwrap(),
        )
        .unwrap(),
        drop_nth_packet: None,
    };

    generate_ingestion_sample(
        &value_t!(sub_matches.value_of("batch-id"), Uuid).unwrap_or_else(|_| Uuid::new_v4()),
        &sub_matches.value_of("aggregation-id").unwrap(),
        &sub_matches.value_of("date").map_or_else(
            || Utc::now().naive_utc(),
            |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
        ),
        value_t!(sub_matches.value_of("dimension"), i32)?,
        value_t!(sub_matches.value_of("packet-count"), usize)?,
        value_t!(sub_matches.value_of("epsilon"), f64)?,
        value_t!(sub_matches.value_of("batch-start-time"), i64)?,
        value_t!(sub_matches.value_of("batch-end-time"), i64)?,
        &mut own_transport,
        &mut peer_transport,
    )?;
    Ok(())
}

fn intake_batch<F: FnMut()>(
    aggregation_id: &str,
    batch_id: &str,
    date: &str,
    sub_matches: &ArgMatches,
    metrics_collector: Option<&IntakeMetricsCollector>,
    callback: F,
) -> Result<(), anyhow::Error> {
    let mut intake_transport = intake_transport_from_args(sub_matches)?;

    // We need the bucket to which we will write validations for the
    // peer data share processor, which can either be fetched from the
    // peer manifest or provided directly via command line argument.
    let peer_validation_bucket =
        if let Some(base_url) = sub_matches.value_of("peer-manifest-base-url") {
            SpecificManifest::from_https(base_url, sub_matches.value_of("instance-name").unwrap())?
                .validation_bucket()
        } else if let Some(path) = sub_matches.value_of("peer-output") {
            StoragePath::from_str(path)
        } else {
            Err(anyhow!("peer-output or peer-manifest-base-url required."))
        }?;

    let peer_identity = sub_matches.value_of("peer-identity");
    let mut peer_validation_transport = SignableTransport {
        transport: transport_for_path(peer_validation_bucket, peer_identity, sub_matches)?,
        batch_signing_key: batch_signing_key_from_arg(sub_matches)?,
    };

    // We created the bucket to which we write copies of our validation
    // shares, so it is simply provided by argument.
    let own_validation_bucket = StoragePath::from_str(sub_matches.value_of("own-output").unwrap())?;
    let own_identity = sub_matches.value_of("own-identity");
    let mut own_validation_transport = SignableTransport {
        transport: transport_for_path(own_validation_bucket, own_identity, sub_matches)?,
        batch_signing_key: batch_signing_key_from_arg(sub_matches)?,
    };

    let batch_id: Uuid = Uuid::parse_str(batch_id).unwrap();

    let date: NaiveDateTime = NaiveDateTime::parse_from_str(date, DATE_FORMAT).unwrap();

    let mut batch_intaker = BatchIntaker::new(
        &aggregation_id,
        &batch_id,
        &date,
        &mut intake_transport,
        &mut peer_validation_transport,
        &mut own_validation_transport,
        is_first_from_arg(sub_matches),
    )?;

    if let Some("true") = sub_matches.value_of("use-bogus-packet-file-digest") {
        batch_intaker.set_use_bogus_packet_file_digest(true);
    }

    if let Some(collector) = metrics_collector {
        batch_intaker.set_metrics_collector(collector);
        collector.intake_tasks_started.inc();
    }

    let result = batch_intaker.generate_validation_share(callback);

    if let Some(collector) = metrics_collector {
        match result {
            Ok(()) => collector
                .intake_tasks_finished
                .with_label_values(&["success"])
                .inc(),
            Err(_) => collector
                .intake_tasks_finished
                .with_label_values(&["error"])
                .inc(),
        }
    }

    result
}

fn intake_batch_subcommand(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    intake_batch(
        sub_matches.value_of("aggregation-id").unwrap(),
        sub_matches.value_of("batch-id").unwrap(),
        sub_matches.value_of("date").unwrap(),
        sub_matches,
        None,
        || {}, // no-op callback
    )
}

fn intake_batch_worker(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let metrics_collector = IntakeMetricsCollector::new()?;
    let scrape_port = value_t!(sub_matches.value_of("metrics-scrape-port"), u16)?;
    let _runtime = start_metrics_scrape_endpoint(scrape_port)?;
    let mut queue = intake_task_queue_from_args(sub_matches)?;

    loop {
        if let Some(task_handle) = queue.dequeue()? {
            info!("dequeued task: {}", task_handle);
            let task_start = Instant::now();

            let result = intake_batch(
                &task_handle.task.aggregation_id,
                &task_handle.task.batch_id,
                &task_handle.task.date,
                sub_matches,
                Some(&metrics_collector),
                || {
                    if let Err(e) =
                        queue.maybe_extend_task_deadline(&task_handle, &task_start.elapsed())
                    {
                        error!("{}", e);
                    }
                },
            );

            match result {
                Ok(_) => queue.acknowledge_task(task_handle)?,
                Err(err) => {
                    error!("error while processing task {}: {:?}", task_handle, err);
                    queue.nacknowledge_task(task_handle)?;
                }
            }
        }
    }

    // unreachable
}

fn aggregate<F: FnMut()>(
    aggregation_id: &str,
    start: &str,
    end: &str,
    batches: Vec<(&str, &str)>,
    sub_matches: &ArgMatches,
    metrics_collector: Option<&AggregateMetricsCollector>,
    callback: F,
) -> Result<(), Error> {
    let instance_name = sub_matches.value_of("instance-name").unwrap();
    let is_first = is_first_from_arg(sub_matches);

    let mut intake_transport = intake_transport_from_args(sub_matches)?;

    // We created the bucket to which we wrote copies of our validation
    // shares, so it is simply provided by argument.
    let own_validation_bucket = StoragePath::from_str(sub_matches.value_of("own-input").unwrap())?;
    let own_identity = sub_matches.value_of("own-identity");
    let own_validation_transport =
        transport_for_path(own_validation_bucket, own_identity, sub_matches)?;

    // To read our own validation shares, we require our own public keys which
    // we discover in our own specific manifest. If no manifest is provided, use
    // the public portion of the provided batch signing private key.
    let own_public_key_map = match (
        sub_matches.value_of("own-manifest-base-url"),
        sub_matches.value_of("batch-signing-private-key"),
        sub_matches.value_of("batch-signing-private-key-identifier"),
    ) {
        (Some(manifest_base_url), _, _) => {
            SpecificManifest::from_https(manifest_base_url, instance_name)?
                .batch_signing_public_keys()?
        }
        (_, Some(private_key), Some(private_key_identifier)) => {
            public_key_map_from_arg(private_key, private_key_identifier)?
        }
        _ => {
            return Err(Error::InvalidArgument(
                "batch-signing-private-key and \
                batch-signing-private-key-identifier are required if \
                own-manifest-base-url is not provided."
                    .to_string(),
            ))
        }
    };

    // We created the bucket that peers wrote validations into, and so
    // it is simply provided via argument.
    let peer_validation_bucket =
        StoragePath::from_str(sub_matches.value_of("peer-input").unwrap())?;
    let peer_identity = sub_matches.value_of("peer-identity");

    let peer_validation_transport =
        transport_for_path(peer_validation_bucket, peer_identity, sub_matches)?;

    // We need the public keys the peer data share processor used to
    // sign messages, which we can obtain by argument or by discovering
    // their specific manifest.
    let peer_share_processor_pub_key_map = match (
        sub_matches.value_of("peer-public-key"),
        sub_matches.value_of("peer-public-key-identifier"),
        sub_matches.value_of("peer-manifest-base-url"),
    ) {
        (_, _, Some(manifest_base_url)) => {
            SpecificManifest::from_https(manifest_base_url, instance_name)?
                .batch_signing_public_keys()?
        }
        (Some(public_key), Some(public_key_identifier), _) => {
            public_key_map_from_arg(public_key, public_key_identifier)?
        }
        _ => {
            return Err(Error::InvalidArgument(
                "peer-public-key and peer-public-key-identifier are required \
                if peer-manifest-base-url is not provided."
                    .to_string(),
            ))
        }
    };

    // We need the portal server owned bucket to which to write sum part
    // messages aka aggregations. We can discover it from the portal
    // server global manifest, or we can get that from an argument.
    let portal_bucket = match (
        sub_matches.value_of("portal-manifest-base-url"),
        sub_matches.value_of("portal-output"),
    ) {
        (Some(manifest_base_url), _) => {
            PortalServerGlobalManifest::from_https(manifest_base_url)?.sum_part_bucket(is_first)
        }
        (_, Some(path)) => StoragePath::from_str(path),
        _ => Err(anyhow!(
            "portal-output or portal-manifest-base-url required"
        )),
    }?;
    let portal_identity = sub_matches.value_of("portal-identity");
    let aggregation_transport = transport_for_path(portal_bucket, portal_identity, sub_matches)?;

    // Get the key we will use to sign sum part messages sent to the
    // portal server.
    let batch_signing_key = batch_signing_key_from_arg(sub_matches)?;

    let start: NaiveDateTime = NaiveDateTime::parse_from_str(start, DATE_FORMAT).unwrap();
    let end: NaiveDateTime = NaiveDateTime::parse_from_str(end, DATE_FORMAT).unwrap();

    let mut own_validation_transport = VerifiableTransport {
        transport: own_validation_transport,
        batch_signing_public_keys: own_public_key_map,
    };
    let mut peer_validation_transport = VerifiableTransport {
        transport: peer_validation_transport,
        batch_signing_public_keys: peer_share_processor_pub_key_map,
    };
    let mut aggregation_transport = SignableTransport {
        transport: aggregation_transport,
        batch_signing_key,
    };

    let mut parsed_batches: Vec<(Uuid, NaiveDateTime)> = Vec::new();
    for raw_batch in batches.iter() {
        let uuid = Uuid::parse_str(raw_batch.0).context("batch ID is not a UUID")?;
        let date = NaiveDateTime::parse_from_str(raw_batch.1, DATE_FORMAT)
            .context("batch date is not in expected format")?;
        parsed_batches.push((uuid, date));
    }

    let mut aggregator = BatchAggregator::new(
        instance_name,
        aggregation_id,
        &start,
        &end,
        is_first,
        &mut intake_transport,
        &mut own_validation_transport,
        &mut peer_validation_transport,
        &mut aggregation_transport,
    )?;

    if let Some(collector) = metrics_collector {
        aggregator.set_metrics_collector(collector);
        collector.aggregate_tasks_started.inc();
    }

    let result = aggregator.generate_sum_part(&parsed_batches, callback);

    if let Some(collector) = metrics_collector {
        match result {
            Ok(()) => collector
                .aggregate_tasks_finished
                .with_label_values(&["success"])
                .inc(),
            Err(_) => collector
                .aggregate_tasks_finished
                .with_label_values(&["error"])
                .inc(),
        }
    }

    result
}

fn aggregate_subcommand(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let batch_ids: Vec<&str> = sub_matches
        .values_of("batch-id")
        .context("no batch-id")?
        .collect();
    let batch_dates: Vec<&str> = sub_matches
        .values_of("batch-time")
        .context("no batch-time")?
        .collect();

    if batch_ids.len() != batch_dates.len() {
        return Err(anyhow!(
            "must provide same number of batch-id and batch-date values"
        ));
    }
    let batch_info: Vec<_> = batch_ids.into_iter().zip(batch_dates).collect();

    aggregate(
        &sub_matches.value_of("aggregation-id").unwrap(),
        sub_matches.value_of("aggregation-start").unwrap(),
        sub_matches.value_of("aggregation-end").unwrap(),
        batch_info,
        sub_matches,
        None,
        || {}, // no-op callback
    )
    .context("aggregation failed")
}

fn aggregate_worker(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let mut queue = aggregation_task_queue_from_args(sub_matches)?;
    let metrics_collector = AggregateMetricsCollector::new()?;
    let scrape_port = value_t!(sub_matches.value_of("metrics-scrape-port"), u16)?;
    let _runtime = start_metrics_scrape_endpoint(scrape_port)?;

    loop {
        if let Some(task_handle) = queue.dequeue()? {
            info!("dequeued task: {}", task_handle);
            let task_start = Instant::now();

            let batches: Vec<(&str, &str)> = task_handle
                .task
                .batches
                .iter()
                .map(|b| (b.id.as_str(), b.time.as_str()))
                .collect();
            let result = aggregate(
                &task_handle.task.aggregation_id,
                &task_handle.task.aggregation_start,
                &task_handle.task.aggregation_end,
                batches,
                sub_matches,
                Some(&metrics_collector),
                || {
                    if let Err(e) =
                        queue.maybe_extend_task_deadline(&task_handle, &task_start.elapsed())
                    {
                        error!("{}", e);
                    }
                },
            );

            // If an aggregation fails due to invalid validation batches, we want to log
            // about it but we don't want to retry: an invalid signature won't become
            // valid if we verify it twice, so having the task queue redeliver the
            // message is just a waste of compute time.
            // Further, if the failure was down to invalid peer batches, we don't want
            // to send the message to the dead letter queue because then we would get an
            // alert that we can't do anything with, so just return Ok(()) in order to
            // ack the message.
            // If the aggregation fails due to a bad own validation batch, we
            // don't want to retry for the same reasons, but we do want to
            // alert, since that indicates a bug on our side. However
            // BatchAggregator::aggregate_share increments a metric when it this
            // happens, so we can alert on that message and safely ack this
            // task.
            let ack = match result {
                Ok(_) => true,
                Err(Error::BadPeerValidationBatch(e)) => {
                    error!("aggregation failed due to invalid batch from peer: {:?}", e);
                    true
                }
                Err(Error::BadOwnValidationBatch(e)) => {
                    error!("aggregation failed due to invalid batch from self: {:?}", e);
                    true
                }
                Err(e) => {
                    error!("error while processing task {}: {:?}", task_handle, e);
                    false
                }
            };

            if ack {
                queue.acknowledge_task(task_handle)?;
            } else {
                queue.nacknowledge_task(task_handle)?;
            }
        }
    }

    // unreachable
}

fn lint_manifest(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let manifest_base_url = sub_matches.value_of("manifest-base-url");
    let manifest_body: Option<String> = match sub_matches.value_of("manifest-path") {
        Some(f) => Some(fs::read_to_string(f)?),
        None => None,
    };

    let manifest_kind = ManifestKind::from_str(
        sub_matches
            .value_of("manifest-kind")
            .context("manifest-kind is required")?,
    )?;

    match manifest_kind {
        ManifestKind::IngestorGlobal | ManifestKind::IngestorSpecific => {
            if manifest_kind == ManifestKind::IngestorSpecific
                && sub_matches.value_of("instance").is_none()
            {
                return Err(anyhow!(
                    "instance is required when manifest-kind=ingestor-specific"
                ));
            }
            let manifest = if let Some(base_url) = manifest_base_url {
                IngestionServerManifest::from_https(base_url, sub_matches.value_of("instance"))?
            } else if let Some(body) = manifest_body {
                IngestionServerManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("Valid: {:?}\n{:#?}", manifest.validate(), manifest);
        }
        ManifestKind::DataShareProcessorGlobal => {
            let manifest = if let Some(base_url) = manifest_base_url {
                DataShareProcessorGlobalManifest::from_https(base_url)?
            } else if let Some(body) = manifest_body {
                DataShareProcessorGlobalManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("{:#?}", manifest);
        }
        ManifestKind::DataShareProcessorSpecific => {
            let instance = sub_matches
                .value_of("instance")
                .context("instance is required when manifest-kind=data-share-processor-specific")?;
            let manifest = if let Some(base_url) = manifest_base_url {
                SpecificManifest::from_https(base_url, instance)?
            } else if let Some(body) = manifest_body {
                SpecificManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("Valid: {:?}\n{:#?}", manifest.validate(), manifest);
        }
        ManifestKind::PortalServerGlobal => {
            let manifest = if let Some(base_url) = manifest_base_url {
                PortalServerGlobalManifest::from_https(base_url)?
            } else if let Some(body) = manifest_body {
                PortalServerGlobalManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("Valid: {:?}\n{:#?}", manifest.validate(), manifest);
        }
    }

    Ok(())
}

fn is_first_from_arg(matches: &ArgMatches) -> bool {
    let is_first = matches.value_of("is-first").unwrap();
    is_first == "true"
}

fn public_key_map_from_arg(
    key: &str,
    key_identifier: &str,
) -> Result<HashMap<String, UnparsedPublicKey<Vec<u8>>>> {
    // UnparsedPublicKey::new doesn't return an error, so try parsing the
    // argument as a private key first.
    let key_bytes = decode_base64_key(key)?;
    let public_key = match EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_bytes) {
        Ok(priv_key) => UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            Vec::from(priv_key.public_key().as_ref()),
        ),
        Err(_) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key_bytes),
    };

    let mut key_map = HashMap::new();
    key_map.insert(key_identifier.to_owned(), public_key);
    Ok(key_map)
}

fn batch_signing_key_from_arg(matches: &ArgMatches) -> Result<BatchSigningKey> {
    let key_bytes = decode_base64_key(matches.value_of("batch-signing-private-key").unwrap())?;
    let key_identifier = matches
        .value_of("batch-signing-private-key-identifier")
        .unwrap();
    Ok(BatchSigningKey {
        key: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_bytes)?,
        identifier: key_identifier.to_owned(),
    })
}

fn intake_transport_from_args(matches: &ArgMatches) -> Result<VerifiableAndDecryptableTransport> {
    // To read (intake) content from an ingestor's bucket, we need the bucket, which we
    // know because our deployment created it, so it is always provided via the
    // ingestor-input argument.
    let ingestor_bucket = StoragePath::from_str(matches.value_of("ingestor-input").unwrap())?;
    let ingestor_identity = matches.value_of("ingestor-identity");

    let intake_transport = transport_for_path(ingestor_bucket, ingestor_identity, matches)?;

    // We also need the public keys the ingestor may have used to sign the
    // the batch, which can be provided either directly via command line or must
    // be fetched from the ingestor global manifest.
    let ingestor_pub_key_map = match (
        matches.value_of("ingestor-public-key"),
        matches.value_of("ingestor-public-key-identifier"),
        matches.value_of("ingestor-manifest-base-url"),
    ) {
        (Some(public_key), Some(public_key_identifier), _) => {
            public_key_map_from_arg(public_key, public_key_identifier)?
        }
        (_, _, Some(manifest_base_url)) => IngestionServerManifest::from_https(
            manifest_base_url,
            Some(matches.value_of("instance-name").unwrap()),
        )?
        .batch_signing_public_keys()?,
        _ => {
            return Err(anyhow!(
                "ingestor-public-key and ingestor-public-key-identifier are \
                required if ingestor-manifest-base-url is not provided."
            ))
        }
    };

    // Get the keys we will use to decrypt packets in the ingestion batch
    let packet_decryption_keys = matches
        .values_of("packet-decryption-keys")
        .unwrap()
        .map(|k| {
            PrivateKey::from_base64(k)
                .context("could not parse encoded packet encryption key")
                .unwrap()
        })
        .collect();

    Ok(VerifiableAndDecryptableTransport {
        transport: VerifiableTransport {
            transport: intake_transport,
            batch_signing_public_keys: ingestor_pub_key_map,
        },
        packet_decryption_keys,
    })
}

fn transport_for_path(
    path: StoragePath,
    identity: Identity,
    matches: &ArgMatches,
) -> Result<Box<dyn Transport>> {
    // We use the value "" to indicate that either ambient AWS credentials (for
    // S3) or the default service account Oauth token (for GCS) should be used
    // so that Terraform can explicitly indicate that the default identity
    // should be used.
    let identity = if let Some("") = identity {
        None
    } else {
        identity
    };

    let key_file_reader = match matches.value_of("gcp-service-account-key-file") {
        Some(path) => {
            Some(Box::new(File::open(path).context("failed to open key file")?) as Box<dyn Read>)
        }
        None => None,
    };

    match path {
        StoragePath::S3Path(path) => Ok(Box::new(S3Transport::new(path, identity))),
        StoragePath::GCSPath(path) => Ok(Box::new(GCSTransport::new(
            path,
            identity,
            key_file_reader,
        )?)),
        StoragePath::LocalPath(path) => Ok(Box::new(LocalFileTransport::new(path))),
    }
}

fn decode_base64_key(s: &str) -> Result<Vec<u8>> {
    if s == "not-a-real-key" {
        return Err(anyhow!(
            "'not-a-real-key'. Run deploy-tool to generate secrets"
        ));
    }
    base64::decode(s).context("decoding key from base64")
}

// You can't make a trait object out of a trait that is generic in another trait
// (as would be the case for TaskQueue<T: Task>) because such traits are not
// "object safe" [1], so we can't write a function like
// fn task_queue_from_args<T: Task>() -> Result<Box<dyn TaskQueue<T>>>.
// To work around this we manually provide specializations on
// task_queue_from_args for IntakeBatchTask and AggregationTask.
//
// [1] https://doc.rust-lang.org/book/ch17-02-trait-objects.html#object-safety-is-required-for-trait-objects
fn intake_task_queue_from_args(
    matches: &ArgMatches,
) -> Result<Box<dyn TaskQueue<IntakeBatchTask>>> {
    let task_queue_kind = TaskQueueKind::from_str(
        matches
            .value_of("task-queue-kind")
            .ok_or_else(|| anyhow!("task-queue-kind is required"))?,
    )?;
    let identity = matches.value_of("task-queue-identity");
    let queue_name = matches
        .value_of("task-queue-name")
        .ok_or_else(|| anyhow!("task-queue-name is required"))?;

    match task_queue_kind {
        TaskQueueKind::GcpPubSub => {
            let gcp_project_id = matches
                .value_of("gcp-project-id")
                .ok_or_else(|| anyhow!("gcp-project-id is required"))?;
            let pubsub_api_endpoint = matches.value_of("pubsub-api-endpoint");
            Ok(Box::new(GcpPubSubTaskQueue::new(
                pubsub_api_endpoint,
                gcp_project_id,
                queue_name,
                identity,
            )?))
        }
        TaskQueueKind::AwsSqs => {
            let sqs_region = matches
                .value_of("aws-sqs-region")
                .ok_or_else(|| anyhow!("aws-sqs-region is required"))?;
            Ok(Box::new(AwsSqsTaskQueue::new(sqs_region, queue_name)?))
        }
    }
}

fn aggregation_task_queue_from_args(
    matches: &ArgMatches,
) -> Result<Box<dyn TaskQueue<AggregationTask>>> {
    let task_queue_kind = TaskQueueKind::from_str(
        matches
            .value_of("task-queue-kind")
            .ok_or_else(|| anyhow!("task-queue-kind is required"))?,
    )?;
    let identity = matches.value_of("task-queue-identity");
    let queue_name = matches
        .value_of("task-queue-name")
        .ok_or_else(|| anyhow!("task-queue-name is required"))?;

    match task_queue_kind {
        TaskQueueKind::GcpPubSub => {
            let gcp_project_id = matches
                .value_of("gcp-project-id")
                .ok_or_else(|| anyhow!("gcp-project-id is required"))?;
            let pubsub_api_endpoint = matches.value_of("pubsub-api-endpoint");
            Ok(Box::new(GcpPubSubTaskQueue::new(
                pubsub_api_endpoint,
                gcp_project_id,
                queue_name,
                identity,
            )?))
        }
        TaskQueueKind::AwsSqs => {
            let sqs_region = matches
                .value_of("aws-sqs-region")
                .ok_or_else(|| anyhow!("aws-sqs-region is required"))?;
            Ok(Box::new(AwsSqsTaskQueue::new(sqs_region, queue_name)?))
        }
    }
}
