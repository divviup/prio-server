use anyhow::{anyhow, Result};
use chrono::{prelude::Utc, NaiveDateTime};
use clap::{App, Arg, ArgMatches, SubCommand};
use prio::encrypt::PrivateKey;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1,
    ECDSA_P256_SHA256_ASN1_SIGNING,
};
use std::str::FromStr;
use uuid::Uuid;

use facilitator::{
    aggregation::BatchAggregator,
    config::StoragePath,
    intake::BatchIntaker,
    sample::generate_ingestion_sample,
    test_utils::{
        DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY,
        DEFAULT_INGESTOR_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY,
        DEFAULT_PHA_SIGNING_PRIVATE_KEY,
    },
    transport::{GCSTransport, LocalFileTransport, S3Transport, Transport},
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
    base64::decode(s).map(|_| ()).map_err(|e| e.to_string())
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
    fn add_storage_argument(
        self: Self,
        arg_name: &'static str,
        s3_arn_arg: &'static str,
        gcs_sa_to_impersonate_arg: &'static str,
    ) -> Self;

    fn add_batch_signing_key_arguments(self: Self) -> Self;
}

impl<'a, 'b> AppArgumentAdder for App<'a, 'b> {
    fn add_storage_argument(
        self: App<'a, 'b>,
        arg_name: &'static str,
        s3_arn_arg: &'static str,
        gcs_sa_arg: &'static str,
    ) -> App<'a, 'b> {
        self.arg(
            Arg::with_name(arg_name)
                .long(arg_name)
                .value_name("PATH")
                .default_value(".")
                .validator(path_validator)
                .help("Local or cloud storage to write data into.")
                .long_help(
                    "Storage to write into or read from. May be either a local \
                    filesystem path, (no scheme), an S3 bucket (formatted as \
                    \"s3://{region}/{bucket-name}\") or a GCS bucket \
                    (formatted as \"gs://{bucket-name}\").",
                ),
        )
        .arg(
            Arg::with_name(s3_arn_arg)
                .long(s3_arn_arg)
                .help("AWS IAM role to assume when using S3.")
                .long_help(
                    "If present, and if the corresponding path is an S3 \
                    bucket, authentication to S3 will try to use an OIDC \
                    auth token obtained from the GKE metadata service to \
                    assume the role specified in the AWS_ROLE_ARN enviornment \
                    variable. If omitted, and the corresponding path is an S3 \
                    bucket, credentials found in the environment or ~/.aws/ \
                    are used. Ignored if the corresponding path is not an S3 \
                    bucket.",
                ),
        )
        .arg(
            Arg::with_name(gcs_sa_arg)
                .long(gcs_sa_arg)
                .value_name("SA_EMAIL")
                .help("GCP service account to impersonate when using GCS.")
                .long_help(
                    "If present, and if the corresponding path is a GCS \
                    bucket, authentication to GCS will try to impersonate the \
                    specified GCP service account, using the default Oauth \
                    token retrieved from the GKE metadata service to \
                    authenticate to GCP IAM. If omitted, and the corresponding \
                    path is a GCS bucket, then the default credentials will be \
                    used. Ignored if the corresponding path is not an S3 \
                    bucket.",
                ),
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
                    "Base64 encoded PKCS#8 document containing ECDSA P256 \
                    batch signing private key to be used by this server when \
                    sending messages to other servers. If not specified, a \
                    fixed private key is used.",
                )
                .default_value(DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY)
                .hide_default_value(true)
                .validator(b64_validator),
        )
        .arg(
            Arg::with_name("batch-signing-private-key-identifier")
                .long("batch-signing-private-key-identifier")
                .env("BATCH_SIGNING_PRIVATE_KEY_ID")
                .value_name("ID")
                .help("Batch signing private key identifier")
                .long_help(
                    "Identifier for the batch signing key, corresponding to an \
                    entry in this server's global or specific manifest. Used \
                    to construct PrioBatchSignature messages.",
                )
                .default_value("default-batch-signing-key-id")
                .hide_default_value(true),
        )
    }
}

fn main() -> Result<(), anyhow::Error> {
    let matches = App::new("facilitator")
        .about("Prio data share processor")
        // Environment variables are injected via build.rs
        .version(&*format!(
            "{} {} {}",
            env!("VERGEN_SEMVER"),
            env!("VERGEN_SHA_SHORT"),
            env!("VERGEN_BUILD_TIMESTAMP"),
        ))
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .help("Enable verbose output to stderr"),
        )
        .subcommand(
            SubCommand::with_name("generate-ingestion-sample")
                .about("Generate sample data files")
                .add_storage_argument("pha-output", "pha-output-s3-arn", "pha-output-gcs-sa-email")
                .add_storage_argument(
                    "facilitator-output",
                    "facilitator-output-s3-arn",
                    "facilitator-output-gcp-sa-email",
                )
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .default_value("fake-aggregation")
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
                        .default_value("123")
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
                        .default_value("10")
                        .validator(num_validator::<usize>)
                        .help("Number of data packets to generate"),
                )
                .arg(
                    Arg::with_name("pha-ecies-private-key")
                        .long("pha-ecies-private-key")
                        .value_name("B64")
                        .help(
                            "Base64 encoded ECIES private key for the PHA \
                            server",
                        )
                        .long_help(
                            "Base64 encoded private key for the PHA \
                            server. If not specified, a fixed private key will \
                            be used.",
                        )
                        .default_value(DEFAULT_PHA_ECIES_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(
                    Arg::with_name("facilitator-ecies-private-key")
                        .long("facilitator-ecies-private-key")
                        .value_name("B64")
                        .help(
                            "Base64 encoded ECIES private key for the \
                            facilitator server",
                        )
                        .long_help(
                            "Base64 encoded ECIES private key for the \
                            facilitator server. If not specified, a fixed \
                            private key will be used.",
                        )
                        .default_value(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
                        .hide_default_value(true)
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
                        .default_value("0.23")
                        .validator(num_validator::<f64>),
                )
                .arg(
                    Arg::with_name("batch-start-time")
                        .long("batch-start-time")
                        .value_name("MILLIS")
                        .help("Start of timespan covered by the batch, in milliseconds since epoch")
                        .default_value("1000000000")
                        .validator(num_validator::<i64>),
                )
                .arg(
                    Arg::with_name("batch-end-time")
                        .long("batch-end-time")
                        .value_name("MILLIS")
                        .help("End of timespan covered by the batch, in milliseconds since epoch")
                        .default_value("1000000100")
                        .validator(num_validator::<i64>),
                ),
        )
        .subcommand(
            SubCommand::with_name("batch-intake")
                .about("Validate an ingestion share and emit a validation share.")
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .default_value("fake-aggregation")
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
                    Arg::with_name("ecies-private-key")
                        .long("ecies-private-key")
                        .value_name("B64")
                        .help("Base64 encoded ECIES private key")
                        .long_help(
                            "Base64 encoded ECIES private key. If not \
                            specified, a fixed private key will be used.",
                        )
                        .default_value(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(
                    Arg::with_name("ingestor-public-key")
                        .long("ingestor-public-key")
                        .value_name("B64")
                        .help("Base64 encoded public key for the ingestor")
                        .long_help(
                            "Base64 encoded ECDSA P256 public key for the \
                            ingestor. If not specified, a default key will be \
                            used.",
                        )
                        .default_value(DEFAULT_INGESTOR_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .add_batch_signing_key_arguments()
                .arg(Arg::with_name("is-first").long("is-first").help(
                    "Whether this is the \"first\" server receiving a share, \
                    i.e., the PHA.",
                ))
                .add_storage_argument(
                    "ingestion-bucket",
                    "ingestion-bucket-s3-arn",
                    "ingestion-bucket-gcp-sa-email",
                )
                .add_storage_argument(
                    "validation-bucket",
                    "validation-bucket-s3-arn",
                    "validation-bucket-gcp-sa-email",
                ),
        )
        .subcommand(
            SubCommand::with_name("aggregate")
                .about("Verify peer validation share and emit sum part.")
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .default_value("fake-aggregation")
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
                        .long_help(
                            "Beginning of the timespan covered by the \
                            aggregation. If omitted, the current time is used.",
                        )
                        .validator(date_validator),
                )
                .arg(
                    Arg::with_name("aggregation-end")
                        .long("aggregation-end")
                        .value_name("DATE")
                        .help("End of the timespan covered by the aggregation.")
                        .long_help(
                            "End of the timespan covered by the aggregation \
                            If omitted, the current time is used.",
                        )
                        .validator(date_validator),
                )
                .add_storage_argument(
                    "ingestion-bucket",
                    "ingestion-bucket-s3-arn",
                    "ingestion-bucket-gcp-sa-email",
                )
                .add_storage_argument(
                    "own-validation-bucket",
                    "own-validation-bucket-s3-arn",
                    "own-validation-bucket-gcp-sa-email",
                )
                .add_storage_argument(
                    "peer-validation-bucket",
                    "peer-validation-bucket-s3-arn",
                    "peer-validation-bucket-gcp-sa-email",
                )
                .add_storage_argument(
                    "aggregation-bucket",
                    "aggregation-bucket-s3-arn",
                    "aggregation-bucket-gcp-sa-email",
                )
                .arg(
                    Arg::with_name("ecies-private-key")
                        .long("ecies-private-key")
                        .value_name("B64")
                        .help("Base64 encoded ECIES private key")
                        .long_help(
                            "Base64 encoded ECIES private key. If not \
                            specified, a fixed private key will be used.",
                        )
                        .default_value(DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(
                    Arg::with_name("ingestor-public-key")
                        .long("ingestor-public-key")
                        .value_name("B64")
                        .help("Base64 encoded public key for the ingestor")
                        .long_help(
                            "Base64 encoded ECDSA P256 public key for the \
                            ingestor. If not specified, a default key will be \
                            used.",
                        )
                        .default_value(DEFAULT_INGESTOR_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .add_batch_signing_key_arguments()
                .arg(
                    Arg::with_name("peer-share-processor-public-key")
                        .long("peer-share-processor-public-key")
                        .value_name("B64")
                        .help(
                            "Base64 encoded public key for the peer share \
                            processor",
                        )
                        .long_help(
                            "Base64 encoded ECDSA P256 public key for the peer \
                            share processor. If not specified, a default key \
                            will be used.",
                        )
                        .default_value(DEFAULT_PHA_SIGNING_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(Arg::with_name("is-first").long("is-first").help(
                    "Whether this is the \"first\" server receiving a share, i.e., the PHA.",
                )),
        )
        .get_matches();

    let _verbose = matches.is_present("verbose");

    match matches.subcommand() {
        // The configuration of the Args above should guarantee that the
        // various parameters are present and valid, so it is safe to use
        // unwrap() here.
        ("generate-ingestion-sample", Some(sub_matches)) => {
            let mut pha_transport = transport_for_output_path(
                "pha-output",
                "pha-output-s3-arn",
                "pha-output-gcp-sa-email",
                sub_matches,
            )?;
            let mut facilitator_transport = transport_for_output_path(
                "facilitator-output",
                "facilitator-output-s3-arn",
                "facilitator-output-gcp-sa-email",
                sub_matches,
            )?;
            let ingestor_batch_signing_key = batch_signing_key_from_arg(sub_matches)?;

            generate_ingestion_sample(
                &mut *pha_transport,
                &mut *facilitator_transport,
                &sub_matches
                    .value_of("batch-id")
                    .map_or_else(Uuid::new_v4, |v| Uuid::parse_str(v).unwrap()),
                &sub_matches.value_of("aggregation-id").unwrap(),
                &sub_matches.value_of("date").map_or_else(
                    || Utc::now().naive_utc(),
                    |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
                ),
                &PrivateKey::from_base64(sub_matches.value_of("pha-ecies-private-key").unwrap())
                    .unwrap(),
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
        ("batch-intake", Some(sub_matches)) => {
            let mut ingestion_transport = transport_for_output_path(
                "ingestion-bucket",
                "ingestion-bucket-s3-arn",
                "ingestion-bucket-gcp-sa-email",
                sub_matches,
            )?;
            let mut validation_transport = transport_for_output_path(
                "validation-bucket",
                "ingestion-bucket-s3-arn",
                "ingestion-bucket-gcp-sa-email",
                sub_matches,
            )?;

            let share_processor_ecies_key =
                PrivateKey::from_base64(sub_matches.value_of("ecies-private-key").unwrap())
                    .unwrap();

            let ingestor_pub_key = public_key_from_arg("ingestor-public-key", sub_matches);

            let batch_signing_key = batch_signing_key_from_arg(sub_matches)?;

            let mut batch_intaker = BatchIntaker::new(
                &sub_matches.value_of("aggregation-id").unwrap(),
                &sub_matches
                    .value_of("batch-id")
                    .map_or_else(Uuid::new_v4, |v| Uuid::parse_str(v).unwrap()),
                &sub_matches.value_of("date").map_or_else(
                    || Utc::now().naive_utc(),
                    |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
                ),
                &mut *ingestion_transport,
                &mut *validation_transport,
                sub_matches.is_present("is-first"),
                &share_processor_ecies_key,
                &batch_signing_key,
                &ingestor_pub_key,
            )?;
            batch_intaker.generate_validation_share()?;
            Ok(())
        }
        ("aggregate", Some(sub_matches)) => {
            let mut ingestion_transport = transport_for_output_path(
                "ingestion-bucket",
                "ingestion-bucket-s3-arn",
                "ingestion-bucket-gcp-sa-email",
                sub_matches,
            )?;
            let mut own_validation_transport = transport_for_output_path(
                "own-validation-bucket",
                "own-validation-bucket-s3-arn",
                "own-validation-bucket-gcp-sa-email",
                sub_matches,
            )?;
            let mut peer_validation_transport = transport_for_output_path(
                "peer-validation-bucket",
                "peer-validation-bucket-s3-arn",
                "peer-validation-bucket-gcp-sa-email",
                sub_matches,
            )?;
            let mut aggregation_transport = transport_for_output_path(
                "aggregation-bucket",
                "aggregation-bucket-s3-arn",
                "aggregation-bucket-gcp-sa-email",
                sub_matches,
            )?;

            let ingestor_pub_key = public_key_from_arg("ingestor-public-key", sub_matches);
            let peer_share_processor_pub_key =
                public_key_from_arg("peer-share-processor-public-key", sub_matches);
            let batch_signing_key = batch_signing_key_from_arg(sub_matches)?;
            let share_processor_ecies_key =
                PrivateKey::from_base64(sub_matches.value_of("ecies-private-key").unwrap())
                    .unwrap();

            let batch_ids: Vec<Uuid> = sub_matches
                .values_of("batch-id")
                .unwrap()
                .map(|v| Uuid::parse_str(v).unwrap())
                .collect();
            let batch_dates: Vec<NaiveDateTime> = sub_matches
                .values_of("batch-date")
                .unwrap()
                .map(|s| NaiveDateTime::parse_from_str(&s, DATE_FORMAT).unwrap())
                .collect();
            if batch_ids.len() != batch_dates.len() {
                return Err(anyhow!(
                    "must provide same number of batch-id and batch-date values"
                ));
            }

            let batch_info: Vec<_> = batch_ids.into_iter().zip(batch_dates).collect();
            BatchAggregator::new(
                &sub_matches.value_of("aggregation-id").unwrap(),
                &sub_matches.value_of("aggregation-start").map_or_else(
                    || Utc::now().naive_utc(),
                    |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
                ),
                &sub_matches.value_of("aggregation-end").map_or_else(
                    || Utc::now().naive_utc(),
                    |v| NaiveDateTime::parse_from_str(&v, DATE_FORMAT).unwrap(),
                ),
                sub_matches.is_present("is-first"),
                &mut *ingestion_transport,
                &mut *own_validation_transport,
                &mut *peer_validation_transport,
                &mut *aggregation_transport,
                &ingestor_pub_key,
                &batch_signing_key,
                &peer_share_processor_pub_key,
                &share_processor_ecies_key,
            )?
            .generate_sum_part(&batch_info)?;
            Ok(())
        }
        (_, _) => Ok(()),
    }
}

fn public_key_from_arg(arg: &str, matches: &ArgMatches) -> UnparsedPublicKey<Vec<u8>> {
    // UnparsedPublicKey::new doesn't return an error, so try parsing the
    // argument as a private key first.
    let key_bytes = base64::decode(matches.value_of(arg).unwrap()).unwrap();
    match EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_bytes) {
        Ok(priv_key) => UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_ASN1,
            Vec::from(priv_key.public_key().as_ref()),
        ),
        Err(_) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key_bytes),
    }
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

fn transport_for_output_path(
    path: &str,
    s3_arn_arg: &str,
    gcp_sa_email_arg: &str,
    matches: &ArgMatches,
) -> Result<Box<dyn Transport>> {
    let path = StoragePath::from_str(matches.value_of(path).unwrap())?;
    match path {
        StoragePath::S3Path(path) => Ok(Box::new(S3Transport::new(
            path,
            matches.is_present(s3_arn_arg),
        ))),
        StoragePath::GCSPath(path) => Ok(Box::new(GCSTransport::new(
            path,
            matches.value_of(gcp_sa_email_arg).map(String::from),
        ))),
        StoragePath::LocalPath(path) => Ok(Box::new(LocalFileTransport::new(path))),
    }
}
