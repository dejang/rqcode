use clap::{Parser, Subcommand, ValueEnum};
use rqcode::Analyzer;
use rqcode::config::Config;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "rqcode",
    version,
    about = "Language-agnostic static analysis gate for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze a project directory or file and exit non-zero on violations.
    Check {
        /// Target codebase directory or file to analyze (defaults to current directory).
        #[arg(default_value = ".")]
        target_path: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,

        /// Optional path to a custom rqcode.toml config file.
        #[arg(long)]
        config: Option<PathBuf>,

        /// Treat warnings as gate failures.
        #[arg(long)]
        fail_on_warning: bool,
    },
    /// List registered analysis rules.
    Rules {
        /// Target codebase directory or file used to discover rqcode.toml (defaults to current directory).
        #[arg(default_value = ".")]
        target_path: PathBuf,

        /// Optional path to a custom rqcode.toml config file.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

/// Load `rqcode.toml` (from `explicit_config_file` if `--config` was passed, otherwise
/// auto-discovered from `target_path`) and build an [`Analyzer`] configured for `target_path`.
fn load_config_and_analyzer(
    target_path: &Path,
    explicit_config_file: Option<&Path>,
) -> Result<(Config, Analyzer), ExitCode> {
    let config = match explicit_config_file {
        Some(cfg_file) => Config::load_file(cfg_file),
        None => Config::load_from_dir(target_path),
    };
    let config = match config {
        Ok(c) => c,
        Err(err) => {
            eprintln!("rqcode config error: {err}");
            return Err(ExitCode::from(2));
        }
    };

    let analyzer = match Analyzer::from_config(&config, target_path) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("rqcode error: {err}");
            return Err(ExitCode::from(2));
        }
    };

    Ok((config, analyzer))
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Rules {
            target_path,
            config: explicit_config_file,
        } => {
            let Ok((_, analyzer)) =
                load_config_and_analyzer(&target_path, explicit_config_file.as_deref())
            else {
                return ExitCode::from(2);
            };

            let width = analyzer
                .rules()
                .rules()
                .iter()
                .map(|r| r.id().len())
                .max()
                .unwrap_or(14)
                .max(14);
            for rule in analyzer.rules().rules() {
                println!("{:<width$}  {}", rule.id(), rule.description());
            }
            ExitCode::SUCCESS
        }
        Command::Check {
            target_path,
            format,
            config: explicit_config_file,
            fail_on_warning,
        } => {
            let Ok((config, analyzer)) =
                load_config_and_analyzer(&target_path, explicit_config_file.as_deref())
            else {
                return ExitCode::from(2);
            };

            let report = match analyzer.check_path(&target_path, &config) {
                Ok(r) => r,
                Err(err) => {
                    eprintln!("rqcode error: {err}");
                    return ExitCode::from(2);
                }
            };

            match format {
                OutputFormat::Text => print!("{}", report.render_text()),
                OutputFormat::Json => match report.render_json() {
                    Ok(json) => println!("{json}"),
                    Err(err) => {
                        eprintln!("failed to serialize report: {err}");
                        return ExitCode::from(2);
                    }
                },
            }

            let failed =
                report.has_errors() || (fail_on_warning && report.stats().warning_count > 0);
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
