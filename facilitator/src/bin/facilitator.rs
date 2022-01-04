use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, NaiveDateTime};
use clap::{value_t, App, Arg, ArgGroup, ArgMatches, SubCommand};
use kube::api::ResourceExt;
use prio::encrypt::{PrivateKey, PublicKey};
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING,
};
use slog::{debug, error, info, Logger};
use std::{
    collections::HashMap, env, fs, fs::File, io::Read, str::FromStr, time::Duration, time::Instant,
};
use tokio::runtime::{self, Handle};
use uuid::Uuid;

use facilitator::{
    aggregation::BatchAggregator,
    aws_credentials,
    config::{
        leak_string, Entity, Identity, InOut, ManifestKind, StoragePath, TaskQueueKind,
        WorkloadIdentityPoolParameters,
    },
    gcp_oauth::GcpAccessTokenProviderFactory,
    intake::BatchIntaker,
    kubernetes::KubernetesClient,
    logging::{event, setup_logging, LoggingConfiguration},
    manifest::{
        DataShareProcessorGlobalManifest, DataShareProcessorSpecificManifest,
        IngestionServerManifest, PortalServerGlobalManifest,
    },
    metrics::{
        start_metrics_scrape_endpoint, AggregateMetricsCollector, ApiClientMetricsCollector,
        IntakeMetricsCollector,
    },
    sample::{SampleGenerator, SampleOutput},
    task::{AggregationTask, AwsSqsTaskQueue, GcpPubSubTaskQueue, IntakeBatchTask, TaskQueue},
    transport::{
        GcsTransport, LocalFileTransport, S3Transport, SignableTransport, Transport,
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

    fn add_batch_signing_key_arguments(self, required: bool) -> Self;

    fn add_packet_decryption_key_argument(self) -> Self;

    fn add_gcp_service_account_key_file_argument(self) -> Self;

    fn add_gcp_workload_identity_pool_provider_argument(self) -> Self;

    fn add_task_queue_arguments(self) -> Self;

    fn add_metrics_scrape_port_argument(self) -> Self;

    fn add_common_sample_maker_arguments(self) -> Self;

    fn add_permit_malformed_batch_argument(self) -> Self;

    fn add_worker_lifetime_argument(self) -> Self;

    fn add_intake_batch_common_arguments(self) -> Self;
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
        let use_default_aws_credentials_provider =
            entity.suffix("-use-default-aws-credentials-provider");
        let use_default_aws_credentials_provider_env =
            leak_string(upper_snake_case(use_default_aws_credentials_provider));
        let gcp_sa_to_impersonate_before_assuming_role =
            entity.suffix("-gcp-sa-to-impersonate-before-assuming-role");
        let gcp_sa_to_impersonate_before_assuming_role_env =
            leak_string(upper_snake_case(gcp_sa_to_impersonate_before_assuming_role));
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
                    "Identity to assume when using S3 or GS APIs for {} bucket.",
                    entity.str()
                )))
                .long_help(leak_string(format!(
                    "Identity to assume when using S3 or GS APIs for bucket \
                    {}. May not be set if {} is true. Should only be set when \
                    running in GKE.",
                    entity.str(),
                    use_default_aws_credentials_provider
                )))
                .default_value("")
                .hide_default_value(true),
        )
        // It's counterintuitive that users must explicitly opt into the
        // default credentials provider. This was done to preserve backward
        // compatibility with previous versions, which defaulted to a provider
        // that would use web identity from Kubernetes environment.
        .arg(
            Arg::with_name(use_default_aws_credentials_provider)
                .long(use_default_aws_credentials_provider)
                .env(use_default_aws_credentials_provider_env)
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false")
                .help(leak_string(format!(
                    "Whether to use the default AWS credentials provider when \
                    using S3 APIs for {} bucket.",
                    entity.str(),
                )))
                .long_help(leak_string(format!(
                    "If true and {} is unset, the default AWS credentials \
                    provider will be used when using S3 APIs for {} bucket. If \
                    false or unset and {} is unset, a web identity provider \
                    configured from the Kubernetes environment will be used. \
                    If false or unset and {} is set, a web identity provider \
                    configured from the GKE metadata service is used. May not \
                    be set to true if {} is set. Should only be set to true if \
                    running in AWS.",
                    id,
                    entity.str(),
                    id,
                    id,
                    id,
                ))),
        )
        .arg(
            Arg::with_name(gcp_sa_to_impersonate_before_assuming_role)
                .long(gcp_sa_to_impersonate_before_assuming_role)
                .env(gcp_sa_to_impersonate_before_assuming_role_env)
                .value_name("SERVICE_ACCOUNT")
                .long_help(leak_string(format!(
                    "If {} is an AWS IAM role and running in GCP, an identity \
                    token will be obtained for the specified GCP service \
                    account to then assume the AWS IAM role using STS \
                    AssumeRoleWithWebIdentity.",
                    id
                )))
                .default_value("")
                .hide_default_value(true),
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

    fn add_batch_signing_key_arguments(self: App<'a, 'b>, required: bool) -> App<'a, 'b> {
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
                .required(required),
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
                ),
        )
        .arg(
            Arg::with_name("batch-signing-private-key-default-identifier")
                .long("batch-signing-private-key-default-identifier")
                .env("BATCH_SIGNING_PRIVATE_KEY_DEFAULT_IDENTIFIER")
                .value_name("ID")
                .help("Batch signing private key default identifier")
                .long_help(
                    "Identifier for the default batch signing keypair to use, \
                    corresponding to an entry in this server's global or \
                    specific manifest. Used only if \
                    --batch-signing-private-key-identifier is not specified. \
                    Used to construct PrioBatchSignature messages.",
                )
                .required(required),
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
                    that should be used by for accessing GCP or impersonating \
                    other GCP service accounts. If omitted, the default \
                    account found in the GKE metadata service will be used for \
                    authentication or impersonation.",
                ),
        )
    }

    fn add_gcp_workload_identity_pool_provider_argument(self) -> Self {
        self.arg(
            Arg::with_name("gcp-workload-identity-pool-provider")
                .long("gcp-workload-identity-pool-provider")
                .env("GCP_WORKLOAD_IDENTITY_POOL_PROVIDER")
                .help("Full resource name of a GCP workload identity pool provider")
                .long_help(
                    "Full resource name of a GCP workload identity pool \
                    provider that should be used for accessing GCP or \
                    impersonating other GCP service accounts when running in \
                    Amazon EKS. The pool provider should be configured to \
                    permit service account impersonation by the AWS IAM role or \
                    user that this facilitator authenticates as.",
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
                .help("Identity to assume when accessing task queue")
                .long_help(
                    "Identity to assume when accessing task queue. Should only \
                    be set when running under GKE and task-queue-kind is PubSub.",
                )
                .default_value("")
                .hide_default_value(true),
        )
        // It's counterintuitive that users must explicitly opt into the
        // default credentials provider. This was done to preserve backward
        // compatibility with previous versions, which defaulted to a provider
        // that would use web identity from Kubernetes environment.
        .arg(
            Arg::with_name("task-queue-use-default-aws-credentials-provider")
                .long("task-queue-use-default-aws-credentials-provider")
                .env("TASK_QUEUE_USE_DEFAULT_AWS_CREDENTIALS_PROVIDER")
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false")
                .help(
                    "Whether to use the default AWS credentials provider when \
                    using SQS APIs.",
                )
                .long_help(
                    "Whether to use the default AWS credentials provider when \
                    using SQS APIs. If unset and task-queue-kind is SQS, uses \
                    a web identity provider configured from Kubernetes \
                    environment. Should not be set unless task-queue-kind is \
                    SQS.",
                ),
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

    fn add_common_sample_maker_arguments(self: App<'a, 'b>) -> App<'a, 'b> {
        self.add_gcp_service_account_key_file_argument()
            .add_storage_arguments(Entity::Peer, InOut::Output)
            .add_storage_arguments(Entity::Facilitator, InOut::Output)
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
                Arg::with_name("pha-ecies-public-key")
                    .long("pha-ecies-public-key")
                    .env("PHA_ECIES_PUBLIC_KEY")
                    .value_name("B64")
                    .help(
                        "Base64 encoded X9.62 uncompressed public key for the PHA \
                            server",
                    ),
            )
            .arg(
                Arg::with_name("pha-manifest-base-url")
                    .long("pha-manifest-base-url")
                    .env("PHA_MANIFEST_BASE_URL")
                    .value_name("URL")
                    .help("Base URL of the Public Health Authority manifest"),
            )
            .group(
                ArgGroup::with_name("public_health_authority_information")
                    .args(&["pha-ecies-public-key", "pha-manifest-base-url"])
                    .required(true),
            )
            .arg(
                Arg::with_name("facilitator-ecies-public-key")
                    .long("facilitator-ecies-public-key")
                    .env("FACILITATOR_ECIES_PUBLIC_KEY")
                    .value_name("B64")
                    .help(
                        "Base64 encoded X9.62 uncompressed public key for the \
                            facilitator server",
                    ),
            )
            .arg(
                Arg::with_name("facilitator-manifest-base-url")
                    .long("facilitator-manifest-base-url")
                    .env("FACILITATOR_MANIFEST_BASE_URL")
                    .value_name("URL")
                    .help("Base URL of the Facilitator manifest"),
            )
            .group(
                ArgGroup::with_name("facilitator_information")
                    .args(&[
                        "facilitator-ecies-public-key",
                        "facilitator-manifest-base-url",
                    ])
                    .required(true),
            )
            .arg(
                Arg::with_name("ingestor-manifest-base-url")
                    .long("ingestor-manifest-base-url")
                    .env("INGESTOR_MANIFEST_BASE_URL")
                    .value_name("URL")
                    .help("Base URL of this ingestor's manifest"),
            )
            .group(
                ArgGroup::with_name("ingestor_information")
                    .args(&["ingestor-manifest-base-url", "batch-signing-private-key"])
                    .required(true),
            )
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
            )
            .arg(
                Arg::with_name("locality-name")
                    .long("locality-name")
                    .value_name("STRING")
                    .help("Name of the locality this ingestor is targeting"),
            )
            .arg(
                Arg::with_name("ingestor-name")
                    .long("ingestor-name")
                    .value_name("STRING")
                    .help("Name of this ingestor"),
            )
            .add_batch_signing_key_arguments(false)
    }

    fn add_permit_malformed_batch_argument(self: App<'a, 'b>) -> App<'a, 'b> {
        self.arg(
            Arg::with_name("permit-malformed-batch")
                .long("permit-malformed-batch")
                .env("PERMIT_MALFORMED_BATCH")
                .help("Permit intake or aggregation of malformed batches")
                .long_help(
                    "Whether to permit malformed batches. When malformed \
                    batches are permitted, facilitator does not abort batch \
                    intake or aggregation if an a batch with an invalid \
                    signature or an incorrect packet file digest is \
                    encountered. If the batch can still be parsed and is \
                    otherwise valid, it will be processed.",
                )
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false"),
        )
    }

    fn add_worker_lifetime_argument(self) -> Self {
        self.arg(
            Arg::with_name("worker-maximum-lifetime")
                .long("worker-maximum-lifetime")
                .env("WORKER_MAXIMUM_LIFETIME")
                .value_name("SECONDS")
                .help("Specifies the maximum lifetime of a worker in seconds.")
                .validator(num_validator::<u64>)
                .long_help(
                    "Specifies the maximum lifetime of a worker in seconds. \
                    After this amount of time, workers will terminate \
                    successfully. Termination may not be immediate: workers \
                    may choose to complete their current work item before \
                    terminating, but in all cases workers will not start a \
                    new work item after their lifetime is complete.",
                ),
        )
    }

    fn add_intake_batch_common_arguments(self) -> Self {
        self.arg(
            Arg::with_name("write-single-object-validation-batches")
                .long("write-single-object-validation-batches")
                .env("WRITE_SINGLE_OBJECT_VALIDATION_BATCHES")
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false")
                .help("Write single-object validation batches (atomically)")
                .long_help(
                    "If set, validation batches will be written in a \
                    single-object format. (This makes writing a batch an \
                    atomic operation.) If not set, validation batches will be \
                    written using the same three-object format used for \
                    ingestion & sum part batches.",
                ),
        )
    }
}

fn main() -> Result<(), anyhow::Error> {
    let matches = App::new("facilitator")
        .about("Prio data share processor")
        .arg(
            // TODO(brandon): remove this flag once it is no longer specified anywhere
            Arg::with_name("pushgateway")
                .long("pushgateway")
                .env("PUSHGATEWAY")
                .help("Deprecated: does nothing")
                .hidden(true),
        )
        .arg(
            Arg::with_name("force-json-log-output")
                .long("force-json-log-output")
                .env("FORCE_JSON_LOG_OUTPUT")
                .help("Force log output to JSON format")
                .value_name("BOOL")
                .possible_value("true")
                .possible_value("false")
                .default_value("false"),
        )
        .subcommand(
            SubCommand::with_name("generate-ingestion-sample")
                .about("Generate sample data files")
                .add_common_sample_maker_arguments()
        )
        .subcommand(
            SubCommand::with_name("generate-ingestion-sample-worker")
                .about("Spawn a worker to generate sample data files")
                .add_common_sample_maker_arguments()
                .add_worker_lifetime_argument()
                .arg(
                    Arg::with_name("kube-namespace")
                        .long("kube-namespace")
                        .env("KUBE_NAMESPACE")
                        .value_name("STRING")
                        .help(
                            "Name of the kubernetes namespace"
                        )
                        .required(true)
                )
                .arg(
                    Arg::with_name("generation-interval")
                        .long("generation-interval")
                        .value_name("INTERVAL")
                        .help(
                            "How often should samples be generated in seconds"
                        )
                        .required(true)
                )
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
                .add_batch_signing_key_arguments(true)
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Output)
                .add_permit_malformed_batch_argument()
                .add_gcp_workload_identity_pool_provider_argument()
                .add_intake_batch_common_arguments()
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
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Input)
                .add_batch_public_key_arguments(Entity::Peer)
                .add_manifest_base_url_argument(Entity::Portal)
                .add_storage_arguments(Entity::Portal, InOut::Output)
                .add_packet_decryption_key_argument()
                .add_batch_signing_key_arguments(true)
                .add_permit_malformed_batch_argument()
                .add_gcp_workload_identity_pool_provider_argument()
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
                .add_batch_signing_key_arguments(true)
                .add_manifest_base_url_argument(Entity::Ingestor)
                .add_storage_arguments(Entity::Ingestor, InOut::Input)
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Output)
                .add_task_queue_arguments()
                .add_metrics_scrape_port_argument()
                .add_permit_malformed_batch_argument()
                .add_gcp_workload_identity_pool_provider_argument()
                .add_worker_lifetime_argument()
                .add_intake_batch_common_arguments()
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
                .add_manifest_base_url_argument(Entity::Peer)
                .add_storage_arguments(Entity::Peer, InOut::Input)
                .add_batch_public_key_arguments(Entity::Peer)
                .add_manifest_base_url_argument(Entity::Portal)
                .add_storage_arguments(Entity::Portal, InOut::Output)
                .add_packet_decryption_key_argument()
                .add_batch_signing_key_arguments(true)
                .add_task_queue_arguments()
                .add_metrics_scrape_port_argument()
                .add_permit_malformed_batch_argument()
                .add_gcp_workload_identity_pool_provider_argument()
                .add_worker_lifetime_argument()
        )
        .get_matches();

    let force_json_log_output = value_t!(matches.value_of("force-json-log-output"), bool)?;
    let log_level = &env::var("RUST_LOG")
        .unwrap_or_else(|_| "INFO".to_owned())
        .to_uppercase();
    let root_logger = setup_logging(&LoggingConfiguration {
        force_json_output: force_json_log_output,
        version_string: option_env!("BUILD_INFO").unwrap_or("(BUILD_INFO unavailable)"),
        log_level,
    })?;
    let args: Vec<String> = std::env::args().collect();
    info!(
        root_logger,
        "starting {}. Args: [{}]",
        args[0],
        args[1..].join(" "),
    );
    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let api_metrics = ApiClientMetricsCollector::new()?;

    let mut gcp_access_token_provider_factory =
        GcpAccessTokenProviderFactory::new(runtime.handle(), &api_metrics, &root_logger);
    let mut aws_provider_factory = aws_credentials::ProviderFactory::new(
        runtime.handle(),
        &gcp_access_token_provider_factory,
        &api_metrics,
        &root_logger,
    );

    let result = match matches.subcommand() {
        // The configuration of the Args above should guarantee that the
        // various parameters are present and valid, so it is safe to use
        // unwrap() here.
        ("generate-ingestion-sample", Some(sub_matches)) => generate_sample(
            &Uuid::new_v4(),
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("generate-ingestion-sample-worker", Some(sub_matches)) => generate_sample_worker(
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("intake-batch", Some(sub_matches)) => intake_batch_subcommand(
            &Uuid::new_v4(),
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("intake-batch-worker", Some(sub_matches)) => intake_batch_worker(
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("aggregate", Some(sub_matches)) => aggregate_subcommand(
            &Uuid::new_v4(),
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("aggregate-worker", Some(sub_matches)) => aggregate_worker(
            sub_matches,
            runtime.handle(),
            &mut aws_provider_factory,
            &mut gcp_access_token_provider_factory,
            &api_metrics,
            &root_logger,
        ),
        ("lint-manifest", Some(sub_matches)) => {
            lint_manifest(sub_matches, &root_logger, &api_metrics)
        }
        (_, _) => Ok(()),
    };

    result
}

/// Check batch signing and packet encryption public keys in this instance's
/// specific manifests against the corresponding private keys provided. Returns
/// an error unless each advertised public key matches up with an available
/// private key.
fn crypto_self_check(
    matches: &ArgMatches,
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<()> {
    let instance_name = matches.value_of("instance-name").unwrap();
    let own_manifest = match matches.value_of("own-manifest-base-url") {
        Some(manifest_base_url) => match DataShareProcessorSpecificManifest::from_https(
            manifest_base_url,
            instance_name,
            logger,
            api_metrics,
        ) {
            Ok(manifest) => manifest,
            // At deploy time, the manifest won't exist yet, and we will get a
            // 404 Not Found or 403 Not Authorized response. Skip the crypto
            // self check and move on because otherwise the deployment will
            // never become healthy and the deploy will fail (#834).
            Err(Error::HttpError(ureq::Error::Status(404, _)))
            | Err(Error::HttpError(ureq::Error::Status(403, _))) => return Ok(()),
            v => v?,
        },
        // Skip crypto self check if no own manifest is provided
        None => return Ok(()),
    };

    let batch_signing_key = batch_signing_key_from_arg(matches)?;
    own_manifest.verify_batch_signing_key(&batch_signing_key)?;
    debug!(logger, "batch singing key self check OK!");

    let packet_decryption_keys: Vec<PrivateKey> = matches
        .values_of("packet-decryption-keys")
        .unwrap()
        .map(|k| {
            PrivateKey::from_base64(k)
                .context("could not parse encoded packet encryption key")
                .unwrap()
        })
        .collect();

    own_manifest.verify_packet_encryption_keys(&packet_decryption_keys)?;
    debug!(logger, "packet decryption key self check OK!");

    Ok(())
}

fn generate_sample_worker(
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    root_logger: &Logger,
) -> Result<(), anyhow::Error> {
    let termination_instant = termination_instant_from_args(sub_matches)?;
    let interval = value_t!(sub_matches.value_of("generation-interval"), u64)?;

    while !should_terminate(termination_instant) {
        let trace_id = Uuid::new_v4();
        let result = generate_sample(
            &trace_id,
            sub_matches,
            runtime_handle,
            aws_provider_factory,
            gcp_access_token_provider_factory,
            api_metrics,
            root_logger,
        );

        if let Err(e) = result {
            error!(
                root_logger, "Error: {:?}", e;
                event::TRACE_ID => trace_id.to_string(),
            );
        }
        std::thread::sleep(Duration::from_secs(interval));
    }
    Ok(())
}

fn get_ecies_public_key(
    key_option: Option<&str>,
    manifest_url: Option<&str>,
    ingestor_name: Option<&str>,
    locality_name: Option<&str>,
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<PublicKey> {
    match key_option {
        Some(key) => {
            // Try to parse the provided base64 as a private key from which we
            // extract the public portion. If that fails, fall back to parsing
            // the base64 as a public key.
            match PrivateKey::from_base64(key) {
                Ok(key) => Ok(PublicKey::from(&key)),
                Err(_) => PublicKey::from_base64(key)
                    .context("unable to create public key from base64 ecies key"),
            }
        }
        None => match manifest_url {
            Some(manifest_url) => {
                let ingestor_name = ingestor_name.ok_or_else(|| {
                    anyhow!("ingestor-name must be provided with ingestor-manifest-base-url")
                })?;
                let locality_name = locality_name.ok_or_else(|| {
                    anyhow!("locality-name must be provided with ingestor-manifest-base-url")
                })?;
                let peer_name = &format!("{}-{}", locality_name, ingestor_name);
                let manifest = DataShareProcessorSpecificManifest::from_https(
                    manifest_url,
                    peer_name,
                    logger,
                    api_metrics,
                )
                .context(format!(
                    "unable to read DataShareProcessorSpecificManifest from {}",
                    manifest_url
                ))?;

                let (key_identifier, packet_decryption_key) = manifest
                    .packet_encryption_keys()
                    .iter()
                    .next()
                    .context("No packet encryption keys in manifest")?;

                let public_key =
                    PublicKey::from_base64(&packet_decryption_key.base64_public_key()?)
                        .context("unable to create public key from base64 ecies key")?;

                debug!(
                    logger,
                    "Picked packet decryption key with ID: {} - public key {:?}",
                    key_identifier,
                    &public_key
                );

                Ok(public_key)
            }
            None => Err(anyhow!(
                "Neither manifest_option or key_option were specified. This error shouldn't happen."
            )),
        },
    }
}

fn get_ingestion_identity_and_bucket(
    identity: Identity,
    bucket: Option<&str>,
    manifest_url: Option<&str>,
    ingestor_name: Option<&str>,
    locality_name: Option<&str>,
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<(Identity, StoragePath)> {
    match bucket {
        Some(bucket) => Ok((identity, StoragePath::from_str(bucket)?)),
        None => {
            let ingestor_name = ingestor_name
                .ok_or_else(|| anyhow!("ingestor-name must be provided with manifest-base-url"))?;
            let locality_name = locality_name
                .ok_or_else(|| anyhow!("locality-name must be provided with manifest-base-url"))?;
            let peer_name = &format!("{}-{}", locality_name, ingestor_name);
            let manifest_url = manifest_url.ok_or_else(|| {
                anyhow!("If bucket is not provided, manifest_url must be provided")
            })?;

            let manifest = DataShareProcessorSpecificManifest::from_https(
                manifest_url,
                peer_name,
                logger,
                api_metrics,
            )
            .context(format!(
                "unable to read DataShareProcessorSpecificManifest from {}",
                manifest_url
            ))?;

            Ok((
                manifest.ingestion_identity().to_owned(),
                manifest.ingestion_bucket().to_owned(),
            ))
        }
    }
}

fn get_valid_batch_signing_key(
    namespace: Option<&str>,
    ingestor_manifest_url: Option<&str>,
    matches: &ArgMatches,
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<BatchSigningKey> {
    match ingestor_manifest_url {
        Some(own_manifest_url) => {
            let namespace = namespace.ok_or_else(|| {
                anyhow!("If manifest URLs are used, kubernetes namespace must be provided")
            })?;

            let manifest =
                IngestionServerManifest::from_https(own_manifest_url, None, logger, api_metrics)
                    .context(format!(
                        "unable to get ingestion server manifest from url: {}",
                        own_manifest_url
                    ))?;

            let kubernetes = KubernetesClient::new(String::from(namespace));
            let secrets = kubernetes.get_sorted_secrets("isrg-prio.org/type=batch-signing-key")?;

            let batch_signing_keys = manifest.batch_signing_public_keys().unwrap();

            let secret = secrets
                .into_iter()
                .find(|secret| batch_signing_keys.contains_key(&secret.name()));

            match secret {
                None => Err(anyhow!(
                    "unable to find a batch signing key from the manifest and kubernetes secret store"
                )),
                Some(secret) => {
                    let secret_bytestring = secret.data
                        .as_ref()
                        .ok_or_else(|| anyhow!("no data in Kubernetes secret"))?
                        .get("secret_key")
                        .ok_or_else(|| anyhow!("no secret_key in Kubernetes secret"))?;
                    let secret_data = base64::decode(&secret_bytestring.0)?;
                    let key =
                        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &secret_data)
                            .context("decoding secret key rejected")?;

                    Ok(BatchSigningKey {
                        identifier: secret.name(),
                        key,
                    })
                }
            }
        }
        // The caller is passing key in directly
        None => batch_signing_key_from_arg(matches),
    }
}

fn generate_sample(
    trace_id: &Uuid,
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    logger: &Logger,
) -> Result<(), anyhow::Error> {
    let kube_namespace = sub_matches.value_of("kube-namespace");
    let ingestor_manifest_base_url = sub_matches.value_of("ingestor-manifest-base-url");

    let ingestor_name = sub_matches.value_of("ingestor-name");
    let locality_name = sub_matches.value_of("locality-name");

    let own_batch_signing_key = get_valid_batch_signing_key(
        kube_namespace,
        ingestor_manifest_base_url,
        sub_matches,
        logger,
        api_metrics,
    )?;

    let (peer_identity, peer_output_path) = get_ingestion_identity_and_bucket(
        value_t!(sub_matches.value_of("peer-identity"), Identity)?,
        sub_matches.value_of("peer-output"),
        sub_matches.value_of("pha-manifest-base-url"),
        ingestor_name,
        locality_name,
        logger,
        api_metrics,
    )?;

    let packet_encryption_public_key = get_ecies_public_key(
        sub_matches.value_of("pha-ecies-public-key"),
        sub_matches.value_of("pha-manifest-base-url"),
        ingestor_name,
        locality_name,
        logger,
        api_metrics,
    )?;

    let mut peer_transport = SampleOutput {
        transport: SignableTransport {
            transport: transport_for_path(
                peer_output_path,
                peer_identity,
                Entity::Peer,
                sub_matches,
                runtime_handle,
                api_metrics,
                aws_provider_factory,
                gcp_access_token_provider_factory,
                logger,
            )?,
            batch_signing_key: own_batch_signing_key,
        },
        packet_encryption_public_key,
        drop_nth_packet: None,
    };

    let (facilitator_identity, facilitator_output) = get_ingestion_identity_and_bucket(
        value_t!(sub_matches.value_of("facilitator-identity"), Identity)?,
        sub_matches.value_of("facilitator-output"),
        sub_matches.value_of("facilitator-manifest-base-url"),
        ingestor_name,
        locality_name,
        logger,
        api_metrics,
    )?;

    let packet_encryption_public_key = get_ecies_public_key(
        sub_matches.value_of("facilitator-ecies-public-key"),
        sub_matches.value_of("facilitator-manifest-base-url"),
        ingestor_name,
        locality_name,
        logger,
        api_metrics,
    )
    .unwrap();

    let own_batch_signing_key = get_valid_batch_signing_key(
        kube_namespace,
        ingestor_manifest_base_url,
        sub_matches,
        logger,
        api_metrics,
    )?;

    let mut facilitator_transport = SampleOutput {
        transport: SignableTransport {
            transport: transport_for_path(
                facilitator_output,
                facilitator_identity,
                Entity::Facilitator,
                sub_matches,
                runtime_handle,
                api_metrics,
                aws_provider_factory,
                gcp_access_token_provider_factory,
                logger,
            )?,
            batch_signing_key: own_batch_signing_key,
        },
        packet_encryption_public_key,
        drop_nth_packet: None,
    };

    let sample_generator = SampleGenerator::new(
        sub_matches.value_of("aggregation-id").unwrap(),
        value_t!(sub_matches.value_of("dimension"), i32)?,
        value_t!(sub_matches.value_of("epsilon"), f64)?,
        value_t!(sub_matches.value_of("batch-start-time"), i64)?,
        value_t!(sub_matches.value_of("batch-end-time"), i64)?,
        &mut peer_transport,
        &mut facilitator_transport,
        logger,
    );

    sample_generator.generate_ingestion_sample(
        trace_id,
        &value_t!(sub_matches.value_of("batch-id"), Uuid).unwrap_or_else(|_| Uuid::new_v4()),
        &sub_matches.value_of("date").map_or_else(
            || Utc::now().naive_utc(),
            |v| NaiveDateTime::parse_from_str(v, DATE_FORMAT).unwrap(),
        ),
        value_t!(sub_matches.value_of("packet-count"), usize)?,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn intake_batch<F>(
    trace_id: &Uuid,
    aggregation_id: &str,
    batch_id: &str,
    date: &str,
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    metrics_collector: Option<&IntakeMetricsCollector>,
    api_metrics: &ApiClientMetricsCollector,
    parent_logger: &Logger,
    callback: F,
) -> Result<(), anyhow::Error>
where
    F: FnMut(&Logger),
{
    let mut intake_transport = intake_transport_from_args(
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        parent_logger,
    )?;

    // We need the bucket to which we will write validations for the
    // peer data share processor, which can either be fetched from the
    // peer manifest or provided directly via command line argument.
    let mut peer_validation_identity = value_t!(
        sub_matches.value_of(Entity::Peer.suffix("-identity")),
        Identity
    )?;
    let peer_validation_bucket =
        if let Some(base_url) = sub_matches.value_of("peer-manifest-base-url") {
            let peer_manifest = DataShareProcessorSpecificManifest::from_https(
                base_url,
                sub_matches.value_of("instance-name").unwrap(),
                parent_logger,
                api_metrics,
            )?;

            // Allow peer manifest to override `peer-identity` parameter, if it
            // contains an identity.
            let peer_manifest_identity = peer_manifest.peer_validation_identity();
            if peer_manifest_identity.is_some() {
                peer_validation_identity = peer_manifest_identity;
            }

            peer_manifest.peer_validation_bucket().to_owned()
        } else if let Some(path) = sub_matches.value_of(Entity::Peer.suffix(InOut::Output.str())) {
            StoragePath::from_str(path)?
        } else {
            return Err(anyhow!("peer-output or peer-manifest-base-url required."));
        };

    let mut peer_validation_transport = SignableTransport {
        transport: transport_from_args(
            peer_validation_identity,
            Entity::Peer,
            PathOrInOut::Path(peer_validation_bucket),
            sub_matches,
            runtime_handle,
            api_metrics,
            aws_provider_factory,
            gcp_access_token_provider_factory,
            parent_logger,
        )?,
        batch_signing_key: batch_signing_key_from_arg(sub_matches)?,
    };

    let batch_id: Uuid = Uuid::parse_str(batch_id).unwrap();

    let date: NaiveDateTime = NaiveDateTime::parse_from_str(date, DATE_FORMAT).unwrap();

    let mut batch_intaker = BatchIntaker::new(
        trace_id,
        aggregation_id,
        &batch_id,
        &date,
        &mut intake_transport,
        &mut peer_validation_transport,
        is_first_from_arg(sub_matches),
        Some("true") == sub_matches.value_of("permit-malformed-batch"),
        parent_logger,
    )?;

    if Some("true") == sub_matches.value_of("write-single-object-validation-batches") {
        batch_intaker.set_use_single_object_write(true);
    }

    if let Some(collector) = metrics_collector {
        batch_intaker.set_metrics_collector(collector);
        collector
            .intake_tasks_started
            .with_label_values(&[aggregation_id])
            .inc();
    }

    let result = batch_intaker.generate_validation_share(callback);

    if let Some(collector) = metrics_collector {
        match result {
            Ok(()) => collector
                .intake_tasks_finished
                .with_label_values(&["success", aggregation_id])
                .inc(),
            Err(_) => collector
                .intake_tasks_finished
                .with_label_values(&["error", aggregation_id])
                .inc(),
        }
    }

    result
}

fn intake_batch_subcommand(
    trace_id: &Uuid,
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    parent_logger: &Logger,
) -> Result<(), anyhow::Error> {
    crypto_self_check(sub_matches, parent_logger, api_metrics)
        .context("crypto self check failed")?;
    intake_batch(
        trace_id,
        sub_matches.value_of("aggregation-id").unwrap(),
        sub_matches.value_of("batch-id").unwrap(),
        sub_matches.value_of("date").unwrap(),
        sub_matches,
        runtime_handle,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        None,
        api_metrics,
        parent_logger,
        |_| {}, // no-op callback
    )
}

fn intake_batch_worker(
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    parent_logger: &Logger,
) -> Result<(), anyhow::Error> {
    let termination_instant = termination_instant_from_args(sub_matches)?;
    let metrics_collector = IntakeMetricsCollector::new()?;
    let scrape_port = value_t!(sub_matches.value_of("metrics-scrape-port"), u16)?;
    start_metrics_scrape_endpoint(scrape_port, runtime_handle, parent_logger)?;

    let queue = intake_task_queue_from_args(
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        parent_logger,
    )?;

    crypto_self_check(sub_matches, parent_logger, api_metrics)
        .context("crypto self check failed")?;

    while !should_terminate(termination_instant) {
        if let Some(task_handle) = queue.dequeue()? {
            info!(parent_logger, "dequeued intake task";
                event::TASK_HANDLE => task_handle.clone(),
            );
            let task_start = Instant::now();

            let trace_id = task_handle.task.trace_id;

            let result = intake_batch(
                &trace_id,
                &task_handle.task.aggregation_id,
                &task_handle.task.batch_id,
                &task_handle.task.date,
                sub_matches,
                runtime_handle,
                aws_provider_factory,
                gcp_access_token_provider_factory,
                Some(&metrics_collector),
                api_metrics,
                parent_logger,
                |logger| {
                    if let Err(e) =
                        queue.maybe_extend_task_deadline(&task_handle, &task_start.elapsed())
                    {
                        error!(
                            logger, "{}", e;
                            event::TRACE_ID => trace_id.to_string(),
                            event::TASK_HANDLE => task_handle.clone(),
                        );
                    }
                },
            );

            match result {
                Ok(_) => queue.acknowledge_task(task_handle)?,
                Err(err) => {
                    error!(
                        parent_logger, "error while processing intake task: {:?}", err;
                        event::TASK_HANDLE => task_handle.clone(),
                        event::TRACE_ID => trace_id.to_string(),
                    );
                    queue.nacknowledge_task(task_handle)?;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn aggregate<F>(
    trace_id: &Uuid,
    aggregation_id: &str,
    start: &str,
    end: &str,
    batches: Vec<(&str, &str)>,
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    metrics_collector: Option<&AggregateMetricsCollector>,
    api_metrics: &ApiClientMetricsCollector,
    logger: &Logger,
    callback: F,
) -> Result<()>
where
    F: FnMut(&Logger),
{
    let instance_name = sub_matches.value_of("instance-name").unwrap();
    let is_first = is_first_from_arg(sub_matches);

    let mut intake_transport = intake_transport_from_args(
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        logger,
    )?;

    // We created the bucket that peers wrote validations into, and so
    // it is simply provided via argument.
    let peer_validation_transport = transport_from_args(
        value_t!(
            sub_matches.value_of(Entity::Peer.suffix("-identity")),
            Identity
        )?,
        Entity::Peer,
        PathOrInOut::InOut(InOut::Input),
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        logger,
    )?;

    // We need the public keys the peer data share processor used to
    // sign messages, which we can obtain by argument or by discovering
    // their specific manifest.
    let peer_share_processor_pub_key_map = match (
        sub_matches.value_of("peer-public-key"),
        sub_matches.value_of("peer-public-key-identifier"),
        sub_matches.value_of("peer-manifest-base-url"),
    ) {
        (_, _, Some(manifest_base_url)) => DataShareProcessorSpecificManifest::from_https(
            manifest_base_url,
            instance_name,
            logger,
            api_metrics,
        )?
        .batch_signing_public_keys()?,
        (Some(public_key), Some(public_key_identifier), _) => {
            public_key_map_from_arg(public_key, public_key_identifier)?
        }
        _ => {
            return Err(anyhow!(
                "peer-public-key and peer-public-key-identifier are \
                        required if peer-manifest-base-url is not provided."
            ));
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
            PortalServerGlobalManifest::from_https(manifest_base_url, logger, api_metrics)?
                .sum_part_bucket(is_first)
                .to_owned()
        }
        (_, Some(path)) => StoragePath::from_str(path)?,
        _ => {
            return Err(anyhow!(
                "portal-output or portal-manifest-base-url required"
            ))
        }
    };
    let aggregation_transport = transport_from_args(
        value_t!(
            sub_matches.value_of(Entity::Portal.suffix("-identity")),
            Identity
        )?,
        Entity::Portal,
        PathOrInOut::Path(portal_bucket),
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        logger,
    )?;

    // Get the key we will use to sign sum part messages sent to the
    // portal server.
    let batch_signing_key = batch_signing_key_from_arg(sub_matches)?;

    let start: NaiveDateTime = NaiveDateTime::parse_from_str(start, DATE_FORMAT).unwrap();
    let end: NaiveDateTime = NaiveDateTime::parse_from_str(end, DATE_FORMAT).unwrap();

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
        trace_id,
        instance_name,
        aggregation_id,
        &start,
        &end,
        is_first,
        Some("true") == sub_matches.value_of("permit-malformed-batch"),
        &mut intake_transport,
        &mut peer_validation_transport,
        &mut aggregation_transport,
        logger,
    )?;

    if let Some(collector) = metrics_collector {
        aggregator.set_metrics_collector(collector);
        collector
            .aggregate_tasks_started
            .with_label_values(&[aggregation_id])
            .inc();
    }

    let result = aggregator.generate_sum_part(&parsed_batches, callback);

    if let Some(collector) = metrics_collector {
        match result {
            Ok(()) => collector
                .aggregate_tasks_finished
                .with_label_values(&["success", aggregation_id])
                .inc(),
            Err(_) => collector
                .aggregate_tasks_finished
                .with_label_values(&["error", aggregation_id])
                .inc(),
        }
    }

    result
}

fn aggregate_subcommand(
    trace_id: &Uuid,
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    parent_logger: &Logger,
) -> Result<(), anyhow::Error> {
    crypto_self_check(sub_matches, parent_logger, api_metrics)
        .context("crypto self check failed")?;

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
        trace_id,
        sub_matches.value_of("aggregation-id").unwrap(),
        sub_matches.value_of("aggregation-start").unwrap(),
        sub_matches.value_of("aggregation-end").unwrap(),
        batch_info,
        sub_matches,
        runtime_handle,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        None,
        api_metrics,
        parent_logger,
        |_| {}, // no-op callback
    )
}

fn aggregate_worker(
    sub_matches: &ArgMatches,
    runtime_handle: &Handle,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    api_metrics: &ApiClientMetricsCollector,
    parent_logger: &Logger,
) -> Result<(), anyhow::Error> {
    let termination_instant = termination_instant_from_args(sub_matches)?;
    let queue = aggregation_task_queue_from_args(
        sub_matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        parent_logger,
    )?;
    let metrics_collector = AggregateMetricsCollector::new()?;
    let scrape_port = value_t!(sub_matches.value_of("metrics-scrape-port"), u16)?;
    start_metrics_scrape_endpoint(scrape_port, runtime_handle, parent_logger)?;
    crypto_self_check(sub_matches, parent_logger, api_metrics)
        .context("crypto self check failed")?;

    while !should_terminate(termination_instant) {
        if let Some(task_handle) = queue.dequeue()? {
            info!(
                parent_logger, "dequeued aggregate task";
                event::TASK_HANDLE => task_handle.clone(),
            );
            let task_start = Instant::now();

            let batches: Vec<(&str, &str)> = task_handle
                .task
                .batches
                .iter()
                .map(|b| (b.id.as_str(), b.time.as_str()))
                .collect();

            let trace_id = task_handle.task.trace_id;

            let result = aggregate(
                &trace_id,
                &task_handle.task.aggregation_id,
                &task_handle.task.aggregation_start,
                &task_handle.task.aggregation_end,
                batches,
                sub_matches,
                runtime_handle,
                aws_provider_factory,
                gcp_access_token_provider_factory,
                Some(&metrics_collector),
                api_metrics,
                parent_logger,
                |logger| {
                    if let Err(e) =
                        queue.maybe_extend_task_deadline(&task_handle, &task_start.elapsed())
                    {
                        error!(
                            logger, "{}", e;
                            event::TRACE_ID => trace_id.to_string(),
                            event::TASK_HANDLE => task_handle.clone(),
                        );
                    }
                },
            );

            match result {
                Ok(_) => queue.acknowledge_task(task_handle)?,
                Err(err) => {
                    error!(
                        parent_logger, "error while processing task: {:?}", err;
                        event::TRACE_ID => trace_id.to_string(),
                        event::TASK_HANDLE => task_handle.clone(),
                    );
                    queue.nacknowledge_task(task_handle)?;
                }
            }
        }
    }
    Ok(())
}

fn lint_manifest(
    sub_matches: &ArgMatches,
    logger: &Logger,
    api_metrics: &ApiClientMetricsCollector,
) -> Result<(), anyhow::Error> {
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
                IngestionServerManifest::from_https(
                    base_url,
                    sub_matches.value_of("instance"),
                    logger,
                    api_metrics,
                )?
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
                DataShareProcessorGlobalManifest::from_https(base_url, logger, api_metrics)?
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
                DataShareProcessorSpecificManifest::from_https(
                    base_url,
                    instance,
                    logger,
                    api_metrics,
                )?
            } else if let Some(body) = manifest_body {
                DataShareProcessorSpecificManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("Valid: {:?}\n{:#?}", manifest.validate(), manifest);
        }
        ManifestKind::PortalServerGlobal => {
            let manifest = if let Some(base_url) = manifest_base_url {
                PortalServerGlobalManifest::from_https(base_url, logger, api_metrics)?
            } else if let Some(body) = manifest_body {
                PortalServerGlobalManifest::from_slice(body.as_bytes())?
            } else {
                return Err(anyhow!(
                    "one of manifest-base-url or manifest-path is required"
                ));
            };
            println!("Valid: Ok\n{:#?}", manifest);
        }
    }

    Ok(())
}

fn is_first_from_arg(matches: &ArgMatches) -> bool {
    Some("true") == matches.value_of("is-first")
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
        .unwrap_or_else(|| {
            matches
                .value_of("batch-signing-private-key-default-identifier")
                .unwrap()
        });
    Ok(BatchSigningKey {
        key: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_bytes)
            .context("failed to parse pkcs8 key for batch signing key")?,
        identifier: key_identifier.to_owned(),
    })
}

fn intake_transport_from_args(
    matches: &ArgMatches,
    runtime_handle: &Handle,
    api_metrics: &ApiClientMetricsCollector,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    logger: &Logger,
) -> Result<VerifiableAndDecryptableTransport> {
    let identity = value_t!(
        matches.value_of(Entity::Ingestor.suffix("-identity")),
        Identity
    )?;

    // To read (intake) content from an ingestor's bucket, we need the bucket, which we
    // know because our deployment created it, so it is always provided via the
    // ingestor-input argument.
    let intake_transport = transport_from_args(
        identity,
        Entity::Ingestor,
        PathOrInOut::InOut(InOut::Input),
        matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        logger,
    )?;

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
            logger,
            api_metrics,
        )?
        .batch_signing_public_keys()?,
        _ => {
            return Err(anyhow!(
                "ingestor-public-key and ingestor-public-key-identifier are \
                required if ingestor-manifest-base-url is not provided."
            ));
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

// transport_from_args can either be passed the StoragePath to construct the
// Transport around, or an Entity and InOut with which to interpolate an
// argument string so that it can extract the necessary argument from matches
// and construct a StoragePath. Ideally we would always just pass Entity and
// InOut to transport_from_args and let it figure out the rest, but in some cases
// a manifest must be consulted to figure out the storage path, and it would
// take more work to enable transport_from_args to generically handle the
// various kinds of manifest.
enum PathOrInOut {
    Path(StoragePath),
    InOut(InOut),
}

#[allow(clippy::too_many_arguments)]
fn transport_from_args(
    identity: Identity,
    entity: Entity,
    path_or_in_out: PathOrInOut,
    matches: &ArgMatches,
    runtime_handle: &Handle,
    api_metrics: &ApiClientMetricsCollector,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    logger: &Logger,
) -> Result<Box<dyn Transport>> {
    let path = match path_or_in_out {
        PathOrInOut::Path(path) => path,
        PathOrInOut::InOut(in_out) => {
            let path_arg = entity.suffix(in_out.str());
            StoragePath::from_str(
                matches
                    .value_of(path_arg)
                    .context(format!("{} is required", path_arg))?,
            )?
        }
    };

    transport_for_path(
        path,
        identity,
        entity,
        matches,
        runtime_handle,
        api_metrics,
        aws_provider_factory,
        gcp_access_token_provider_factory,
        logger,
    )
}

#[allow(clippy::too_many_arguments)]
fn transport_for_path(
    path: StoragePath,
    identity: Identity,
    entity: Entity,
    matches: &ArgMatches,
    runtime_handle: &Handle,
    api_metrics: &ApiClientMetricsCollector,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    logger: &Logger,
) -> Result<Box<dyn Transport>> {
    let use_default_aws_credentials_provider = value_t!(
        matches.value_of(entity.suffix("-use-default-aws-credentials-provider")),
        bool
    )?;

    let sa_to_impersonate = value_t!(
        matches.value_of(entity.suffix("-gcp-sa-to-impersonate-before-assuming-role")),
        Identity
    )?;

    match path {
        StoragePath::S3Path(path) => {
            let credentials_provider = aws_provider_factory.get(
                identity,
                sa_to_impersonate,
                use_default_aws_credentials_provider,
                "s3",
            )?;
            Ok(Box::new(S3Transport::new(
                path,
                credentials_provider,
                runtime_handle,
                logger,
                api_metrics,
            )))
        }
        StoragePath::GcsPath(path) => {
            let key_file_reader = match matches.value_of("gcp-service-account-key-file") {
                Some(path) => Some(
                    Box::new(File::open(path).context("failed to open key file")?) as Box<dyn Read>,
                ),
                None => None,
            };

            Ok(Box::new(GcsTransport::new(
                path,
                identity,
                key_file_reader,
                WorkloadIdentityPoolParameters::new(
                    matches.value_of("gcp-workload-identity-pool-provider"),
                    use_default_aws_credentials_provider,
                    aws_provider_factory,
                )?,
                gcp_access_token_provider_factory,
                logger,
                api_metrics,
            )?))
        }
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
    runtime_handle: &Handle,
    api_metrics: &ApiClientMetricsCollector,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    logger: &Logger,
) -> Result<Box<dyn TaskQueue<IntakeBatchTask>>> {
    let task_queue_kind = TaskQueueKind::from_str(
        matches
            .value_of("task-queue-kind")
            .ok_or_else(|| anyhow!("task-queue-kind is required"))?,
    )?;
    let identity = value_t!(matches.value_of("task-queue-identity"), Identity)?;
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
                gcp_access_token_provider_factory,
                logger,
                api_metrics,
            )?))
        }
        TaskQueueKind::AwsSqs => {
            let sqs_region = matches
                .value_of("aws-sqs-region")
                .ok_or_else(|| anyhow!("aws-sqs-region is required"))?;
            let credentials_provider = aws_provider_factory.get(
                identity,
                Identity::none(),
                value_t!(
                    matches.value_of("task-queue-use-default-aws-credentials-provider"),
                    bool
                )?,
                "sqs",
            )?;
            Ok(Box::new(AwsSqsTaskQueue::new(
                sqs_region,
                queue_name,
                runtime_handle,
                credentials_provider,
                logger,
                api_metrics,
            )?))
        }
    }
}

fn aggregation_task_queue_from_args(
    matches: &ArgMatches,
    runtime_handle: &Handle,
    api_metrics: &ApiClientMetricsCollector,
    aws_provider_factory: &mut aws_credentials::ProviderFactory,
    gcp_access_token_provider_factory: &mut GcpAccessTokenProviderFactory,
    logger: &Logger,
) -> Result<Box<dyn TaskQueue<AggregationTask>>> {
    let task_queue_kind = TaskQueueKind::from_str(
        matches
            .value_of("task-queue-kind")
            .ok_or_else(|| anyhow!("task-queue-kind is required"))?,
    )?;
    let identity = value_t!(matches.value_of("task-queue-identity"), Identity)?;
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
                gcp_access_token_provider_factory,
                logger,
                api_metrics,
            )?))
        }
        TaskQueueKind::AwsSqs => {
            let sqs_region = matches
                .value_of("aws-sqs-region")
                .ok_or_else(|| anyhow!("aws-sqs-region is required"))?;
            let credentials_provider = aws_provider_factory.get(
                identity,
                Identity::none(),
                value_t!(
                    matches.value_of("task-queue-use-default-aws-credentials-provider"),
                    bool
                )?,
                "sqs",
            )?;
            Ok(Box::new(AwsSqsTaskQueue::new(
                sqs_region,
                queue_name,
                runtime_handle,
                credentials_provider,
                logger,
                api_metrics,
            )?))
        }
    }
}

fn termination_instant_from_args(
    sub_matches: &ArgMatches,
) -> Result<Option<Instant>, anyhow::Error> {
    Ok(sub_matches
        .value_of("worker-maximum-lifetime")
        .map(str::parse)
        .transpose()?
        .map(Duration::from_secs)
        .map(|d| Instant::now() + d))
}

fn should_terminate(termination_instant: Option<Instant>) -> bool {
    termination_instant.map_or(false, |end| Instant::now() > end)
}
