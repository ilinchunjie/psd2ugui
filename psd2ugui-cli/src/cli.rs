use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use psd_export::{ExportOptions, export_psd_file};
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
    #[arg(long, visible_alias = "photoshop-exe")]
    pub photoshop_path: Option<PathBuf>,
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
    #[arg(long, visible_alias = "photoshop-exe")]
    pub photoshop_path: Option<PathBuf>,
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
    pub warnings: Vec<PipelineWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineWarning {
    pub code: String,
    pub message: String,
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
            photoshop_path: args.photoshop_path.clone(),
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
            photoshop_path: args.photoshop_path.clone(),
            photoshop_timeout_sec: args.photoshop_timeout_sec,
        },
    )?;

    let generated = generate_bundle(&bundle_dir, &out_dir)?;
    let warnings = generated
        .ui_plan
        .warnings
        .iter()
        .map(|warning| PipelineWarning {
            code: warning.code.clone(),
            message: warning.message.clone(),
        })
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

