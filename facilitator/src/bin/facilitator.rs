use anyhow::{anyhow, Context, Result};
use chrono::{prelude::Utc, NaiveDateTime};
use clap::{App, Arg, ArgMatches, SubCommand};
use prio::encrypt::PrivateKey;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED,
    ECDSA_P256_SHA256_FIXED_SIGNING,
};
use rusoto_core::Region;
use std::{path::Path, str::FromStr};
use uuid::Uuid;

use facilitator::{
    aggregation::BatchAggregator,
    intake::BatchIntaker,
    sample::generate_ingestion_sample,
    test_utils::{
        DEFAULT_FACILITATOR_ECIES_PRIVATE_KEY, DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY,
        DEFAULT_INGESTOR_PRIVATE_KEY, DEFAULT_PHA_ECIES_PRIVATE_KEY,
        DEFAULT_PHA_SIGNING_PRIVATE_KEY,
    },
    transport::{LocalFileTransport, S3Transport, Transport},
    DATE_FORMAT,
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

enum StoragePath<'a> {
    S3Path { region: &'a str, bucket: &'a str },
    LocalPath(&'a str),
}

fn parse_path(s: &str) -> Result<StoragePath> {
    match s.strip_prefix("s3://") {
        Some(region_and_bucket) => {
            if !region_and_bucket.contains('/') {
                return Err(anyhow!(
                    "S3 storage must be like \"s3://{region}/{bucket name}\""
                ));
            }

            // All we require is that the string contain a region and a bucket name.
            // Further validation of bucket names is left to Amazon servers.
            let mut components = region_and_bucket.splitn(2, '/');
            let region = components.next().context("S3 URL missing region")?;
            let bucket = components.next().context("S3 URL missing bucket name")?;
            // splitn will only return 2 so it should never have more
            assert!(components.next().is_none());
            Ok(StoragePath::S3Path { region, bucket })
        }
        None => Ok(StoragePath::LocalPath(s)),
    }
}

fn path_validator(s: String) -> Result<(), String> {
    parse_path(s.as_ref())
        .map(|_| ())
        .map_err(|e| e.to_string())
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
                .arg(
                    Arg::with_name("pha-output")
                        .long("pha-output")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Storage to write sample data for the PHA \
                            (aka first server) into. May be either a local \
                            filesystem path or an S3 bucket, formatted as \
                            \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("facilitator-output")
                        .long("facilitator-output")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Storage to write sample data for the \
                            facilitator (aka second server) into. May be \
                            either a local filesystem path or an S3 bucket, \
                            formatted as \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("s3-use-ambient-credentials")
                        .long("s3-use-ambient-credentials")
                        .help(
                            "If present, authentication to S3 will try to use \
                            credentials found in environment variables or \
                            ~/.aws/. If omitted, OIDC credentials will be \
                            obtained from the GKE metadata service and the \
                            AWS_ROLE_ARN environment variable.",
                        ),
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
                .arg(
                    Arg::with_name("ingestor-private-key")
                        .long("ingestor-private-key")
                        .value_name("B64")
                        .help(
                            "Base64 encoded ECDSA P256 private key for the \
                            ingestor server",
                        )
                        .long_help(
                            "Base64 encoded ECDSA P256 private key for the \
                            ingestor server. If not specified, a fixed private \
                            key will be used.",
                        )
                        .default_value(DEFAULT_INGESTOR_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
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
                .arg(
                    Arg::with_name("share-processor-private-key")
                        .long("share-processor-private-key")
                        .value_name("B64")
                        .help("Base64 encoded share processor private key for the server")
                        .long_help(
                            "Base64 encoded ECDSA P256 share processor private \
                            key. If not specified, a fixed private key will be \
                            used.",
                        )
                        .default_value(DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(Arg::with_name("is-first").long("is-first").help(
                    "Whether this is the \"first\" server receiving a share, \
                    i.e., the PHA.",
                ))
                .arg(
                    Arg::with_name("ingestion-bucket")
                        .long("ingestion-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Directory containing ingestion data. May be \
                            either a local filesystem path or an S3 bucket, \
                            formatted as \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("validation-bucket")
                        .long("validation-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Peer validation bucket into which to write \
                            validation shares. May be either a local \
                            filesystem path or an S3 bucket, formatted as \
                            \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("s3-use-ambient-credentials")
                        .long("s3-use-ambient-credentials")
                        .help(
                            "If present, authentication to S3 will try to use \
                            credentials found in environment variables or \
                            ~/.aws/. If omitted, OIDC credentials will be \
                            obtained from the GKE metadata service and the \
                            AWS_ROLE_ARN environment variable.",
                        ),
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
                .arg(
                    Arg::with_name("ingestion-bucket")
                        .long("ingestion-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Directory containing ingestion data. May be \
                            either a local filesystem path or an S3 bucket, \
                            formatted as \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("own-validation-bucket")
                        .long("own-validation-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Bucket in which this share processor's validation \
                            shares were written. May be either a local \
                            filesystem path or an S3 bucket, formatted as \
                            \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("peer-validation-bucket")
                        .long("peer-validation-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Bucket in which the peer share processor's \
                            validation shares were written. May be either a \
                            local filesystem path or an S3 bucket, formatted \
                            as \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("aggregation-bucket")
                        .long("aggregation-bucket")
                        .value_name("DIR")
                        .default_value(".")
                        .validator(path_validator)
                        .help(
                            "Bucket into which sum parts are to be written. May be either a \
                            local filesystem path or an S3 bucket, formatted \
                            as \"s3://{region}/{bucket-name}\"",
                        ),
                )
                .arg(
                    Arg::with_name("s3-use-ambient-credentials")
                        .long("s3-use-ambient-credentials")
                        .help(
                            "If present, authentication to S3 will try to use \
                            credentials found in environment variables or \
                            ~/.aws/. If omitted, OIDC credentials will be \
                            obtained from the GKE metadata service and the \
                            AWS_ROLE_ARN environment variable.",
                        ),
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
                .arg(
                    Arg::with_name("share-processor-private-key")
                        .long("share-processor-private-key")
                        .value_name("B64")
                        .help("Base64 encoded share processor private key for the server")
                        .long_help(
                            "Base64 encoded ECDSA P256 share processor private \
                            key. If not specified, a fixed private key will be \
                            used.",
                        )
                        .default_value(DEFAULT_FACILITATOR_SIGNING_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
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
            let mut pha_transport = transport_for_output_path("pha-output", sub_matches)?;
            let mut facilitator_transport =
                transport_for_output_path("facilitator-output", sub_matches)?;

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
                &base64::decode(sub_matches.value_of("ingestor-private-key").unwrap()).unwrap(),
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
            let mut ingestion_transport =
                transport_for_output_path("ingestion-bucket", sub_matches)?;
            let mut validation_transport =
                transport_for_output_path("validation-bucket", sub_matches)?;

            let share_processor_ecies_key =
                PrivateKey::from_base64(sub_matches.value_of("ecies-private-key").unwrap())
                    .unwrap();

            let ingestor_pub_key = public_key_from_arg("ingestor-public-key", sub_matches);

            let share_processor_key_bytes =
                base64::decode(sub_matches.value_of("share-processor-private-key").unwrap())
                    .unwrap();
            let share_processor_key = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &share_processor_key_bytes,
            )
            .context("failed to parse value for share-processor-private-key")?;

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
                &share_processor_key,
                &ingestor_pub_key,
            )?;
            batch_intaker.generate_validation_share()?;
            Ok(())
        }
        ("aggregate", Some(sub_matches)) => {
            let mut ingestion_transport =
                transport_for_output_path("ingestion-bucket", sub_matches)?;
            let mut own_validation_transport =
                transport_for_output_path("own-validation-bucket", sub_matches)?;
            let mut peer_validation_transport =
                transport_for_output_path("peer-validation-bucket", sub_matches)?;
            let mut aggregation_transport =
                transport_for_output_path("aggregation-bucket", sub_matches)?;

            let ingestor_pub_key = public_key_from_arg("ingestor-public-key", sub_matches);
            let peer_share_processor_pub_key =
                public_key_from_arg("peer-share-processor-public-key", sub_matches);
            let share_processor_key_bytes =
                base64::decode(sub_matches.value_of("share-processor-private-key").unwrap())
                    .unwrap();
            let share_processor_key = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &share_processor_key_bytes,
            )
            .context("failed to parse value for share-processor-private-key")?;
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
                &share_processor_key,
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
    match EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &key_bytes) {
        Ok(priv_key) => UnparsedPublicKey::new(
            &ECDSA_P256_SHA256_FIXED,
            Vec::from(priv_key.public_key().as_ref()),
        ),
        Err(_) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key_bytes),
    }
}

fn transport_for_output_path(arg: &str, matches: &ArgMatches) -> Result<Box<dyn Transport>> {
    let path = parse_path(matches.value_of(arg).unwrap())?;
    match path {
        StoragePath::S3Path { region, bucket } => Ok(Box::new(S3Transport::new(
            Region::from_str(region)?,
            matches.is_present("s3-use-ambient-credentials"),
            bucket.to_string(),
        ))),
        StoragePath::LocalPath(path) => Ok(Box::new(LocalFileTransport::new(
            Path::new(path).to_path_buf(),
        ))),
    }
}
