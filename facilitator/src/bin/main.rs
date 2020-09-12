use clap::{App, Arg, SubCommand};
use std::process;

fn integer_validator(s: String) -> Result<(), String> {
    s.parse::<usize>().map(|_| ()).map_err(|e| e.to_string())
}

fn main() {
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
                    Arg::with_name("dimension")
                        .long("dimension")
                        .short("d")
                        .value_name("DIM")
                        .default_value("123")
                        .validator(integer_validator)
                        .help("Dimension parameter. Must be an integer."),
                ),
        )
        .get_matches();

    let verbose = matches.is_present("verbose");

    if verbose {
        println!("verbose output is on");
    }

    match matches.subcommand() {
        ("generate-ingestion-sample", Some(sub_matches)) => {
            println!("subcommand generate sample {:?}", sub_matches)
        }
        _ => println!("no subcommand provided"),
    }
}
