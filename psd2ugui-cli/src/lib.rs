mod cli;

pub use cli::{
    BundleNaming, CliError, CliResult, ExportCommandArgs, PipelineCommandArgs, PipelineResult,
    PlanCommandArgs, RasterBackendArg, ValidateCommandArgs, generate_pipeline_bundle_dir,
    run_export, run_pipeline, run_plan, run_validate,
};

pub fn run() -> CliResult<()> {
    cli::run()
}
