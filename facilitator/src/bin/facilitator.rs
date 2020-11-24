use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, NaiveDateTime};
use clap::{App, Arg, ArgMatches, SubCommand};
use prio::encrypt::PrivateKey;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING,
};
use std::{collections::HashMap, fs, fs::File, io::Read, str::FromStr};
use uuid::Uuid;

use facilitator::{
    aggregation::BatchAggregator,
    config::{Identity, ManifestKind, StoragePath},
    intake::BatchIntaker,
    manifest::{
        DataShareProcessorGlobalManifest, IngestionServerGlobalManifest,
        PortalServerGlobalManifest, SpecificManifest,
    },
    sample::generate_ingestion_sample,
    transport::{
        GCSTransport, LocalFileTransport, S3Transport, SignableTransport, Transport,
        VerifiableAndDecryptableTransport, VerifiableTransport,
    },
    BatchSigningKey, DATE_FORMAT,
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

fn b64_validator(s: String) -> Result<(), String> {
    if s == "not-a-real-key" {
        return Err("'not-a-real-key'. Check the deploy-operator for valid secrets".to_string());
    }
    base64::decode(s).map_err(|e| format!("decoding base64: {}", e))?;
    Ok(())
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
    fn add_is_first_argument(self: Self) -> Self;

    fn add_instance_name_argument(self: Self) -> Self;

    fn add_manifest_base_url_argument(self: Self, entity: Entity) -> Self;

    fn add_storage_arguments(self: Self, entity: Entity, in_out: InOut) -> Self;

    fn add_batch_public_key_arguments(self: Self, entity: Entity) -> Self;

    fn add_batch_signing_key_arguments(self: Self) -> Self;

    fn add_packet_decryption_key_argument(self: Self) -> Self;

    fn add_gcp_service_account_key_file_argument(self: Self) -> Self;
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
                )))
                .validator(b64_validator),
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
                .required(true)
                .validator(b64_validator),
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
                .validator(b64_validator)
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
}

fn main() -> Result<(), anyhow::Error> {
    use log::info;
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    info!(
        "starting {} version {}. Args: [{}]",
        "",
        option_env!("BUILD_INFO").unwrap_or("(BUILD_INFO unavailable)"),
        args[1..].join(" "),
    );
    let matches = App::new("facilitator")
        .about("Prio data share processor")
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .help("Enable verbose output to stderr"),
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
                        .required(true)
                        .validator(b64_validator),
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
                        .required(true)
                        .validator(b64_validator),
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
                .add_storage_arguments(Entity::Own, InOut::Output),
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
                    .possible_value(leak_string(ManifestKind::DataShareProcessorGlobal.to_string()))
                    .possible_value(leak_string(ManifestKind::DataShareProcessorSpecific.to_string()))
                    .possible_value(leak_string(ManifestKind::PortalServerGlobal.to_string()))
                    .required(true)
                )
            .arg(
                Arg::with_name("instance")
                    .long("instance")
                    .value_name("LOCALITY-TECH_GIANT")
                    .help("the instance name whose manifest is to be fetched")
                    .long_help(
                        leak_string(format!("the instance name whose manifest is to be fetched, \
                        e.g., \"mi-google\". Required if manifest-kind={}.",
                        ManifestKind::DataShareProcessorSpecific))
                    )
            )
        )
        .get_matches();

    let _verbose = matches.is_present("verbose");

    match matches.subcommand() {
        // The configuration of the Args above should guarantee that the
        // various parameters are present and valid, so it is safe to use
        // unwrap() here.
        ("generate-ingestion-sample", Some(sub_matches)) => generate_sample(sub_matches),
        ("intake-batch", Some(sub_matches)) => intake_batch(sub_matches),
        ("aggregate", Some(sub_matches)) => aggregate(sub_matches),
        ("lint-manifest", Some(sub_matches)) => lint_manifest(sub_matches),
        (_, _) => Ok(()),
    }
}

fn generate_sample(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let peer_output_path = StoragePath::from_str(sub_matches.value_of("peer-output").unwrap())?;
    let peer_identity = sub_matches.value_of("peer-identity");
    let mut peer_transport = transport_for_path(peer_output_path, peer_identity, sub_matches)?;

    let own_output_path = StoragePath::from_str(sub_matches.value_of("own-output").unwrap())?;
    let own_identity = sub_matches.value_of("own-identity");
    let mut own_transport = transport_for_path(own_output_path, own_identity, sub_matches)?;
    let ingestor_batch_signing_key = batch_signing_key_from_arg(sub_matches)?;

    generate_ingestion_sample(
        &mut *peer_transport,
        &mut *own_transport,
        &sub_matches
            .value_of("batch-id")
            .map_or_else(Uuid::new_v4, |v| Uuid::parse_str(v).unwrap()),
        &sub_matches.value_of("aggregation-id").unwrap(),
        &sub_matches.value_of("date").map_or_else(
            || Utc::now().naive_utc(),
            |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
        ),
        &PrivateKey::from_base64(sub_matches.value_of("pha-ecies-private-key").unwrap()).unwrap(),
        &PrivateKey::from_base64(
            sub_matches
                .value_of("facilitator-ecies-private-key")
                .unwrap(),
        )
        .unwrap(),
        &ingestor_batch_signing_key,
        sub_matches
            .value_of("dimension")
            .unwrap()
            .parse::<i32>()
            .unwrap(),
        sub_matches
            .value_of("packet-count")
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        sub_matches
            .value_of("epsilon")
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        sub_matches
            .value_of("batch-start-time")
            .unwrap()
            .parse::<i64>()
            .unwrap(),
        sub_matches
            .value_of("batch-end-time")
            .unwrap()
            .parse::<i64>()
            .unwrap(),
    )?;
    Ok(())
}

fn intake_batch(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
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

    let mut batch_intaker = BatchIntaker::new(
        &sub_matches.value_of("aggregation-id").unwrap(),
        &Uuid::parse_str(sub_matches.value_of("batch-id").unwrap()).unwrap(),
        &NaiveDateTime::parse_from_str(&sub_matches.value_of("date").unwrap(), DATE_FORMAT)
            .unwrap(),
        &mut intake_transport,
        &mut peer_validation_transport,
        &mut own_validation_transport,
        is_first_from_arg(sub_matches),
    )?;
    batch_intaker.generate_validation_share()
}

fn aggregate(sub_matches: &ArgMatches) -> Result<(), anyhow::Error> {
    let instance_name = sub_matches.value_of("instance-name").unwrap();
    let is_first = is_first_from_arg(sub_matches);

    let mut intake_transport = intake_transport_from_args(sub_matches)?;

    // We created the bucket to which we wrote copies of our validation
    // shares, so it is simply provided by argument.
    let own_validation_bucket = StoragePath::from_str(sub_matches.value_of("own-input").unwrap())?;
    let own_identity = sub_matches.value_of("own-identity");
    let own_validation_transport =
        transport_for_path(own_validation_bucket, own_identity, sub_matches)?;

    // To read our own validation shares, we require our own public keys
    // which we discover in our own specific manifest.
    let own_public_key_map = match (
        sub_matches.value_of("batch-signing-private-key"),
        sub_matches.value_of("batch-signing-private-key-identifier"),
        sub_matches.value_of("own-manifest-base-url"),
    ) {
        (Some(private_key), Some(private_key_identifier), _) => {
            public_key_map_from_arg(private_key, private_key_identifier)
        }
        (_, _, Some(manifest_base_url)) => {
            SpecificManifest::from_https(manifest_base_url, instance_name)?
                .batch_signing_public_keys()?
        }
        _ => {
            return Err(anyhow!(
                "batch-signing-private-key and \
                        batch-signing-private-key-identifier are required if \
                        own-manifest-base-url is not provided."
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
            public_key_map_from_arg(public_key, public_key_identifier)
        }
        _ => {
            return Err(anyhow!(
                "peer-public-key and peer-public-key-identifier are \
                        required if peer-manifest-base-url is not provided."
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

    let batch_ids: Vec<Uuid> = sub_matches
        .values_of("batch-id")
        .context("no batch-id")?
        .map(|v| Uuid::parse_str(v).unwrap())
        .collect();
    let batch_dates: Vec<NaiveDateTime> = sub_matches
        .values_of("batch-time")
        .context("no batch-time")?
        .map(|s| NaiveDateTime::parse_from_str(&s, DATE_FORMAT).unwrap())
        .collect();
    if batch_ids.len() != batch_dates.len() {
        return Err(anyhow!(
            "must provide same number of batch-id and batch-date values"
        ));
    }

    let batch_info: Vec<_> = batch_ids.into_iter().zip(batch_dates).collect();
    BatchAggregator::new(
        instance_name,
        &sub_matches.value_of("aggregation-id").unwrap(),
        &NaiveDateTime::parse_from_str(
            &sub_matches.value_of("aggregation-start").unwrap(),
            DATE_FORMAT,
        )
        .unwrap(),
        &NaiveDateTime::parse_from_str(
            &sub_matches.value_of("aggregation-end").unwrap(),
            DATE_FORMAT,
        )
        .unwrap(),
        is_first,
        &mut intake_transport,
        &mut VerifiableTransport {
            transport: own_validation_transport,
            batch_signing_public_keys: own_public_key_map,
        },
        &mut VerifiableTransport {
            transport: peer_validation_transport,
            batch_signing_public_keys: peer_share_processor_pub_key_map,
        },
        &mut SignableTransport {
            transport: aggregation_transport,
            batch_signing_key,
        },
    )?
    .generate_sum_part(&batch_info)?;
    Ok(())
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
        ManifestKind::IngestorGlobal => {
            let manifest = if let Some(base_url) = manifest_base_url {
                IngestionServerGlobalManifest::from_https(base_url)?
            } else if let Some(body) = manifest_body {
                IngestionServerGlobalManifest::from_slice(body.as_bytes())?
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
) -> HashMap<String, UnparsedPublicKey<Vec<u8>>> {
    // UnparsedPublicKey::new doesn't return an error, so try parsing the
    // argument as a private key first.
    let key_bytes = base64::decode(key).unwrap();
    let public_key = match EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_bytes) {
        Ok(priv_key) => UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            Vec::from(priv_key.public_key().as_ref()),
        ),
        Err(_) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key_bytes),
    };

    let mut key_map = HashMap::new();
    key_map.insert(key_identifier.to_owned(), public_key);
    key_map
}

fn batch_signing_key_from_arg(matches: &ArgMatches) -> Result<BatchSigningKey> {
    let key_bytes = base64::decode(matches.value_of("batch-signing-private-key").unwrap()).unwrap();
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
            public_key_map_from_arg(public_key, public_key_identifier)
        }
        (_, _, Some(manifest_base_url)) => {
            IngestionServerGlobalManifest::from_https(manifest_base_url)?
                .batch_signing_public_keys()?
        }
        _ => {
            return Err(anyhow!(
                "ingestor-public-key and ingestor-public-key-identifier are \
                required if ingestor-manifest-base-url is not provided."
            ))
        }
    };

    // Get the keys we will use to decrypt packets in the ingestion
    // batch
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
