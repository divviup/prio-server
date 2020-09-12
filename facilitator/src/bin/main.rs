use chrono::prelude::*;
use chrono::DateTime;
use clap::{App, Arg, SubCommand};
use facilitator::sample::generate_ingestion_sample;
use facilitator::transport::FileTransport;
use facilitator::Error;
use libprio_rs::encrypt::PrivateKey;
use std::path::Path;
use uuid::Uuid;

const DATE_FORMAT: &str = "%Y/%m/%d/%H/%M";
const DEFAULT_PHA_PRIVATE_KEY: &str =
    "BIl6j+J6dYttxALdjISDv6ZI4/VWVEhUzaS05LgrsfswmbLOgNt9HUC2E0w+9Rq\
    Zx3XMkdEHBHfNuCSMpOwofVSq3TfyKwn0NrftKisKKVSaTOt5seJ67P5QL4hxgPWvxw==";
const DEFAULT_FACILITATOR_PRIVATE_KEY: &str =
    "BNNOqoU54GPo+1gTPv+hCgA9U2ZCKd76yOMrWa1xTWgeb4LhFLMQIQoRwDVaW64g\
    /WTdcxT4rDULoycUNFB60LER6hPEHg/ObBnRPV1rwS3nj9Bj0tbjVPPyL9p8QW8B+w==";

fn usize_validator(s: String) -> Result<(), String> {
    s.parse::<usize>().map(|_| ()).map_err(|e| e.to_string())
}

fn f64_validator(s: String) -> Result<(), String> {
    s.parse::<f64>().map(|_| ()).map_err(|e| e.to_string())
}

fn date_validator(s: String) -> Result<(), String> {
    DateTime::parse_from_str(&s, DATE_FORMAT)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn b64_validator(s: String) -> Result<(), String> {
    base64::decode(s).map(|_| ()).map_err(|e| e.to_string())
}

fn uuid_validator(s: String) -> Result<(), String> {
    Uuid::parse_str(&s).map(|_| ()).map_err(|e| e.to_string())
}

fn main() -> Result<(), Error> {
    let matches = App::new("facilitator")
        .about("Prio facilitator server")
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
                    Arg::with_name("output")
                        .long("output")
                        .short("o")
                        .value_name("DIR")
                        .default_value(".")
                        .help("Directory to write sample data into"),
                )
                .arg(
                    Arg::with_name("aggregation-id")
                        .long("aggregation-id")
                        .value_name("ID")
                        .default_value("fake-aggregation")
                        .help("Aggregation ID to use when constructing object keys"),
                )
                .arg(
                    Arg::with_name("batch-id")
                        .long("batch-id")
                        .value_name("UUID")
                        .help("Batch ID to use when constructing object keys")
                        .long_help(
                            "Batch ID to use when constructing object keys. If omitted, a \
                            UUID is generated.",
                        )
                        .validator(uuid_validator),
                )
                .arg(
                    Arg::with_name("date")
                        .long("date")
                        .value_name("DATE")
                        .help("Date to use when constructing object keys in YYYYmmddHHMM format")
                        .long_help(
                            "Date to use when constructing object keys. If omitted, the current \
                        time is used.",
                        )
                        .validator(date_validator),
                )
                .arg(
                    Arg::with_name("dimension")
                        .long("dimension")
                        .short("d")
                        .value_name("INT")
                        .default_value("123")
                        .validator(usize_validator)
                        .help("Length in bits of the data packets to generate"),
                )
                .arg(
                    Arg::with_name("packet-count")
                        .long("packet-count")
                        .short("p")
                        .value_name("INT")
                        .default_value("10")
                        .validator(usize_validator)
                        .help("Number of data packets to generate"),
                )
                .arg(
                    Arg::with_name("pha-private-key")
                        .long("pha-private-key")
                        .value_name("B64")
                        .help("Base64 encoded private key for the PHA server")
                        .long_help("If not specified, a fixed private key will be used.")
                        .default_value(DEFAULT_PHA_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(
                    Arg::with_name("facilitator-private-key")
                        .long("facilitator-private-key")
                        .value_name("B64")
                        .help("Base64 encoded private key for the facilitator server")
                        .long_help("If not specified, a fixed private key will be used.")
                        .default_value(DEFAULT_FACILITATOR_PRIVATE_KEY)
                        .hide_default_value(true)
                        .validator(b64_validator),
                )
                .arg(
                    Arg::with_name("bins")
                        .long("bins")
                        .value_name("INT")
                        .help("Number of bins for the aggregation")
                        .default_value("100")
                        .validator(usize_validator),
                )
                .arg(
                    Arg::with_name("epsilon")
                        .long("epsilon")
                        .value_name("DOUBLE")
                        .help(
                            "Differential privacy parameter for local randomization before \
                            aggregation",
                        )
                        .default_value("0.23")
                        .validator(f64_validator),
                )
                .arg(
                    Arg::with_name("batch-start-time")
                        .long("batch-start-time")
                        .value_name("MILLIS")
                        .help("Start of timespan covered by the batch, in milliseconds since epoch")
                        .default_value("1000000000")
                        .validator(usize_validator),
                )
                .arg(
                    Arg::with_name("batch-end-time")
                        .long("batch-end-time")
                        .value_name("MILLIS")
                        .help("End of timespan covered by the batch, in milliseconds since epoch")
                        .default_value("1000000100")
                        .validator(usize_validator),
                ),
        )
        .get_matches();

    let _verbose = matches.is_present("verbose");

    match matches.subcommand() {
        ("generate-ingestion-sample", Some(sub_matches)) => {
            // The configuration of the Args above should guarantee that the
            // various parameters are present and valid, so it is safe to use
            // unwrap() here.
            generate_ingestion_sample(
                Box::new(FileTransport::new(
                    Path::new(sub_matches.value_of("output").unwrap()).to_path_buf(),
                )),
                sub_matches
                    .value_of("batch-uuid")
                    .map_or_else(|| Uuid::new_v4(), |v| Uuid::parse_str(v).unwrap()),
                sub_matches.value_of("aggregation-id").unwrap().to_owned(),
                sub_matches.value_of("date").map_or_else(
                    || Utc::now().format(DATE_FORMAT).to_string(),
                    |v| v.to_string(),
                ),
                &PrivateKey::from_base64(sub_matches.value_of("pha-private-key").unwrap()).unwrap(),
                &PrivateKey::from_base64(sub_matches.value_of("facilitator-private-key").unwrap())
                    .unwrap(),
                sub_matches
                    .value_of("dimension")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
                sub_matches
                    .value_of("packet-count")
                    .unwrap()
                    .parse::<usize>()
                    .unwrap(),
                sub_matches
                    .value_of("bins")
                    .unwrap()
                    .parse::<i32>()
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
            )
        }
        (_, _) => Ok(()),
    }
}
