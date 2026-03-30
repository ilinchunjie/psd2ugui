use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};
use psd_export::{ExportOptions, RasterBackend, export_psd_file};
use serde::Serialize;
use thiserror::Error;
use ui_orchestrator::{generate_bundle, load_plan, validate_plan};

pub type CliResult<T> = std::result::Result<T, CliError>;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Export(#[from] psd_export::AppError),
    #[error(transparent)]
    Orchestrator(#[from] ui_orchestrator::OrchestratorError),
    #[error("failed to prepare pipeline output under {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Export(error) => error.exit_code(),
            Self::Orchestrator(_) | Self::Io { .. } => 1,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "psd2ugui",
    version,
    about = "Unified PSD export, orchestration, and Unity plan generation pipeline."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Export(ExportCommandArgs),
    Plan(PlanCommandArgs),
    Validate(ValidateCommandArgs),
    Pipeline(PipelineCommandArgs),
}

#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum RasterBackendArg {
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

#[derive(Debug, Clone, Args)]
pub struct ExportCommandArgs {
    pub input: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
    #[arg(long, action = clap::ArgAction::SetTrue, help = "Compatibility flag. Preview export is enabled by default.")]
    pub with_preview: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub include_hidden: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub strict: bool,
    #[arg(long, value_enum, default_value_t = RasterBackendArg::Rawpsd)]
    pub raster_backend: RasterBackendArg,
    #[arg(long)]
    pub photoshop_exe: Option<PathBuf>,
    #[arg(long, default_value_t = 120)]
    pub photoshop_timeout_sec: u64,
}

#[derive(Debug, Clone, Args)]
pub struct PlanCommandArgs {
    pub bundle_dir: PathBuf,
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ValidateCommandArgs {
    pub plan_path: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct PipelineCommandArgs {
    pub input: PathBuf,
    #[arg(long)]
    pub cache_dir: PathBuf,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub include_hidden: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub strict: bool,
    #[arg(long, value_enum, default_value_t = RasterBackendArg::Auto)]
    pub raster_backend: RasterBackendArg,
    #[arg(long)]
    pub photoshop_exe: Option<PathBuf>,
    #[arg(long, default_value_t = 120)]
    pub photoshop_timeout_sec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleNaming {
    Timestamped,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineResult {
    pub bundle_dir: String,
    pub plan_path: String,
    pub validation_report_path: String,
    pub document_id: String,
    pub warnings: Vec<String>,
}

pub fn run() -> CliResult<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Export(args) => {
            run_export(&args)?;
        }
        Command::Plan(args) => {
            run_plan(&args)?;
        }
        Command::Validate(args) => {
            run_validate(&args)?;
        }
        Command::Pipeline(args) => {
            let result = run_pipeline(&args)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .expect("pipeline result should always serialize")
            );
        }
    }

    Ok(())
}

pub fn run_export(args: &ExportCommandArgs) -> CliResult<()> {
    let manifest = export_psd_file(
        &args.input,
        ExportOptions {
            out_dir: args.out.clone(),
            include_hidden: args.include_hidden,
            with_preview: args.with_preview,
            strict: args.strict,
            raster_backend: args.raster_backend.clone().into(),
            photoshop_exe: args.photoshop_exe.clone(),
            photoshop_timeout_sec: args.photoshop_timeout_sec,
        },
    )?;

    println!(
        "Exported manifest with {} top-level layer(s) to {}",
        manifest.layers.len(),
        args.out.display()
    );

    Ok(())
}

pub fn run_plan(args: &PlanCommandArgs) -> CliResult<()> {
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| args.bundle_dir.join("plan"));
    let generated = generate_bundle(&args.bundle_dir, &out_dir)?;
    println!(
        "generated {}\n{}",
        generated.ui_plan_path.display(),
        generated.validation_report_path.display()
    );
    Ok(())
}

pub fn run_validate(args: &ValidateCommandArgs) -> CliResult<()> {
    let plan = load_plan(&args.plan_path)?;
    validate_plan(&plan)?;
    println!("plan is valid");
    Ok(())
}

pub fn run_pipeline(args: &PipelineCommandArgs) -> CliResult<PipelineResult> {
    let bundle_dir = generate_pipeline_bundle_dir(&args.cache_dir, &args.input, BundleNaming::Timestamped)?;
    let out_dir = bundle_dir.join("plan");

    let _manifest = export_psd_file(
        &args.input,
        ExportOptions {
            out_dir: bundle_dir.clone(),
            include_hidden: args.include_hidden,
            with_preview: true,
            strict: args.strict,
            raster_backend: args.raster_backend.clone().into(),
            photoshop_exe: args.photoshop_exe.clone(),
            photoshop_timeout_sec: args.photoshop_timeout_sec,
        },
    )?;

    let generated = generate_bundle(&bundle_dir, &out_dir)?;
    let warnings = generated
        .ui_plan
        .warnings
        .iter()
        .map(|warning| format!("{}: {}", warning.code, warning.message))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(PipelineResult {
        bundle_dir: to_forward_slashes(&bundle_dir),
        plan_path: to_forward_slashes(&generated.ui_plan_path),
        validation_report_path: to_forward_slashes(&generated.validation_report_path),
        document_id: generated.ui_plan.source_bundle.document_id.clone(),
        warnings,
    })
}

pub fn generate_pipeline_bundle_dir(
    cache_dir: &Path,
    input: &Path,
    naming: BundleNaming,
) -> CliResult<PathBuf> {
    fs::create_dir_all(cache_dir).map_err(|source| CliError::Io {
        path: cache_dir.to_path_buf(),
        source,
    })?;

    let stem = sanitize_bundle_segment(
        input
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("document"),
    );

    let bundle_name = match naming {
        BundleNaming::Timestamped => format!("{stem}-{}", timestamp_suffix()),
    };
    let bundle_dir = cache_dir.join(bundle_name);

    fs::create_dir_all(&bundle_dir).map_err(|source| CliError::Io {
        path: bundle_dir.clone(),
        source,
    })?;

    Ok(bundle_dir)
}

fn sanitize_bundle_segment(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            cleaned.push(character.to_ascii_lowercase());
        } else if (character == '-' || character == '_') && !cleaned.ends_with(character) {
            cleaned.push(character);
        } else if character.is_whitespace() && !cleaned.ends_with('_') {
            cleaned.push('_');
        }
    }

    let cleaned = cleaned.trim_matches(['-', '_']);
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned.to_string()
    }
}

fn timestamp_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn to_forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn parses_export_command_with_photoshop_options() {
        let cli = Cli::try_parse_from([
            "psd2ugui",
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

        let Command::Export(args) = cli.command else {
            panic!("expected export command");
        };

        assert_eq!(args.raster_backend, RasterBackendArg::Photoshop);
        assert_eq!(
            args.photoshop_exe,
            Some(PathBuf::from(
                "C:\\Program Files\\Adobe\\Adobe Photoshop 2025\\Photoshop.exe"
            ))
        );
        assert_eq!(args.photoshop_timeout_sec, 45);
    }

    #[test]
    fn parses_pipeline_command_defaults_to_auto_backend() {
        let cli = Cli::try_parse_from([
            "psd2ugui",
            "pipeline",
            "demo.psd",
            "--cache-dir",
            "cache",
        ])
        .unwrap();

        let Command::Pipeline(args) = cli.command else {
            panic!("expected pipeline command");
        };

        assert_eq!(args.raster_backend, RasterBackendArg::Auto);
        assert_eq!(args.cache_dir, PathBuf::from("cache"));
        assert!(args.photoshop_exe.is_none());
        assert_eq!(args.photoshop_timeout_sec, 120);
    }

    #[test]
    fn generates_timestamped_bundle_directory_under_cache_dir() {
        let cache_dir = tempdir().unwrap();
        let bundle_dir = generate_pipeline_bundle_dir(
            cache_dir.path(),
            Path::new("sample ui.psd"),
            BundleNaming::Timestamped,
        )
        .unwrap();

        assert!(bundle_dir.starts_with(cache_dir.path()));
        assert!(bundle_dir.exists());
        let name = bundle_dir.file_name().and_then(|value| value.to_str()).unwrap();
        assert!(name.starts_with("sample_ui-"));
    }

    #[test]
    fn pipeline_generates_bundle_and_reports_json_contract() {
        let fixture = find_rawpsd_fixture("test2.psd").expect("rawpsd fixture not found");
        let cache_dir = tempdir().unwrap();

        let result = run_pipeline(&PipelineCommandArgs {
            input: fixture,
            cache_dir: cache_dir.path().to_path_buf(),
            include_hidden: false,
            strict: false,
            raster_backend: RasterBackendArg::Rawpsd,
            photoshop_exe: None,
            photoshop_timeout_sec: 120,
        })
        .unwrap();

        assert!(Path::new(&result.bundle_dir).exists());
        assert!(Path::new(&result.plan_path).exists());
        assert!(Path::new(&result.validation_report_path).exists());
        assert!(!result.document_id.is_empty());
        assert_eq!(result.bundle_dir, result.bundle_dir.replace('\\', "/"));
        assert_eq!(result.plan_path, result.plan_path.replace('\\', "/"));
        assert_eq!(
            result.validation_report_path,
            result.validation_report_path.replace('\\', "/")
        );
    }

    fn find_rawpsd_fixture(file_name: &str) -> Option<PathBuf> {
        let cargo_home = std::env::var("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap()
                    .join(".cargo")
            });
        let registry_src = cargo_home.join("registry").join("src");
        find_fixture_recursive(&registry_src, file_name)
    }

    fn find_fixture_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
        let entries = fs::read_dir(root).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .map(|name| name.to_string_lossy().contains("rawpsd-0.2.2"))
                    .unwrap_or(false)
                {
                    let candidate = path.join("data").join(file_name);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }

                if let Some(found) = find_fixture_recursive(&path, file_name) {
                    return Some(found);
                }
            }
        }

        None
    }
}
