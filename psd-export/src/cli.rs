use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::Result;
use crate::export::{self, ExportOptions, RasterBackend};

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

#[derive(Clone, Debug, ValueEnum)]
enum RasterBackendArg {
    Rawpsd,
    Photoshop,
    Auto,
}

impl From<RasterBackendArg> for RasterBackend {
    fn from(value: RasterBackendArg) -> Self {
        match value {
            RasterBackendArg::Rawpsd => RasterBackend::RawPsd,
            RasterBackendArg::Photoshop => RasterBackend::Photoshop,
            RasterBackendArg::Auto => RasterBackend::Auto,
        }
    }
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
    #[arg(long, value_enum, default_value_t = RasterBackendArg::Rawpsd)]
    raster_backend: RasterBackendArg,
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
                    raster_backend: args.raster_backend.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_rawpsd_backend_by_default() {
        let cli =
            Cli::try_parse_from(["psd-export", "export", "demo.psd", "--out", "out"]).unwrap();
        let Commands::Export(args) = cli.command;
        assert!(matches!(args.raster_backend, RasterBackendArg::Rawpsd));
        assert_eq!(args.photoshop_timeout_sec, 120);
        assert!(args.photoshop_exe.is_none());
    }

    #[test]
    fn parses_photoshop_backend_options() {
        let cli = Cli::try_parse_from([
            "psd-export",
            "export",
            "demo.psd",
            "--out",
            "out",
            "--raster-backend",
            "photoshop",
            "--photoshop-exe",
            "C:\\Program Files\\Adobe\\Adobe Photoshop 2025\\Photoshop.exe",
            "--photoshop-timeout-sec",
            "45",
        ])
        .unwrap();
        let Commands::Export(args) = cli.command;
        assert!(matches!(args.raster_backend, RasterBackendArg::Photoshop));
        assert_eq!(args.photoshop_timeout_sec, 45);
        assert_eq!(
            args.photoshop_exe,
            Some(PathBuf::from(
                "C:\\Program Files\\Adobe\\Adobe Photoshop 2025\\Photoshop.exe"
            ))
        );
    }
}
