use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::Result;
use crate::export::{self, ExportOptions};

#[derive(Debug, Parser)]
#[command(
    name = "psd-export",
    version,
    about = "Export PSD layer structure and assets for Unity prefab generation."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Export(ExportArgs),
}

#[derive(Debug, Args)]
struct ExportArgs {
    input: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, action = clap::ArgAction::SetTrue, help = "Compatibility flag. Preview export is enabled by default.")]
    with_preview: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    include_hidden: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    strict: bool,
    #[arg(long)]
    photoshop_exe: Option<PathBuf>,
    #[arg(long, default_value_t = 120)]
    photoshop_timeout_sec: u64,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Export(args) => {
            let output_dir = args.out.clone();
            let _ = args.with_preview;
            let manifest = export::export_psd_file(
                &args.input,
                ExportOptions {
                    out_dir: output_dir.clone(),
                    include_hidden: args.include_hidden,
                    with_preview: true,
                    strict: args.strict,
                    photoshop_exe: args.photoshop_exe,
                    photoshop_timeout_sec: args.photoshop_timeout_sec,
                },
            )?;

            println!(
                "Exported manifest with {} top-level layer(s) to {}",
                manifest.layers.len(),
                output_dir.display()
            );
        }
    }

    Ok(())
}

