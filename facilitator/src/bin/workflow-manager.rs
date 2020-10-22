use env_logger::Env;
use facilitator::WorkflowArgs;
use structopt::StructOpt;

fn main() -> Result<(), anyhow::Error> {
    env_logger::init_from_env(Env::new().default_filter_or("warn"));

    let args = WorkflowArgs::from_args();
    facilitator::workflow_main(args)
}
