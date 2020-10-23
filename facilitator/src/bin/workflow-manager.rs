use facilitator::WorkflowArgs;
use structopt::StructOpt;

fn main() -> Result<(), anyhow::Error> {
    let args = WorkflowArgs::from_args();
    facilitator::workflow_main(args)
}
