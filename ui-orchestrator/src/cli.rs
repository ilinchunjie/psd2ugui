use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::error::Result;
use crate::planner::generate_bundle;
use crate::validation::{load_plan, validate_plan};

#[derive(Debug, Parser)]
#[command(author, version, about = "Stage-two offline UI planner")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Generate {
        bundle_dir: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Validate {
        plan_path: PathBuf,
    },
}

pub fn run() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Generate { bundle_dir, out } => {
            let out_dir = out.unwrap_or_else(|| bundle_dir.join("plan"));
            let generated = generate_bundle(&bundle_dir, &out_dir)?;
            println!(
                "generated {}\n{}",
                generated.ui_plan_path.display(),
                generated.validation_report_path.display()
            );
        }
        Command::Validate { plan_path } => {
            let plan = load_plan(&plan_path)?;
            validate_plan(&plan)?;
            println!("plan is valid");
        }
    }

    Ok(())
}
