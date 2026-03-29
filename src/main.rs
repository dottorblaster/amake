mod adapter;
mod config;
mod editor;
mod error;
mod runner;
mod sandbox;
mod template;

use clap::{Parser, Subcommand};
use config::Config;
use error::Error;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "amake", about = "A task runner for AI CLI tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run one or more tasks
    Run {
        /// Task names to execute (dependencies auto-included)
        #[arg(required = true)]
        tasks: Vec<String>,

        /// Show resolved commands without executing
        #[arg(long)]
        dry_run: bool,

        /// Continue on failure
        #[arg(short = 'k', long)]
        keep_going: bool,

        /// Set a variable (repeatable), e.g. --var key=value
        #[arg(long = "var", value_name = "KEY=VALUE")]
        vars: Vec<String>,

        /// Open $EDITOR to input a variable value (repeatable), e.g. --edit-var description
        #[arg(long = "edit-var", value_name = "NAME")]
        edit_vars: Vec<String>,

        /// Path to Amakefile (skip auto-discovery)
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,

        /// Force-enable clampdown sandbox for all tasks
        #[arg(long)]
        sandbox: bool,

        /// Disable sandbox for all tasks (overrides config)
        #[arg(long)]
        no_sandbox: bool,
    },

    /// List all tasks in the Amakefile
    List {
        /// Path to Amakefile (skip auto-discovery)
        #[arg(short = 'f', long = "file")]
        file: Option<PathBuf>,
    },

    /// List built-in adapters
    Adapters,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            tasks,
            dry_run,
            keep_going,
            vars,
            edit_vars,
            file,
            sandbox,
            no_sandbox,
        } => {
            let config = load_config(file)?;
            let mut vars = parse_vars(&vars)?;

            for name in &edit_vars {
                eprintln!("✎ opening editor for variable: {name}");
                let value = editor::edit_variable(name)?;
                vars.insert(name.clone(), value);
            }

            runner::run(
                &config,
                &tasks,
                &runner::RunOptions {
                    dry_run,
                    keep_going,
                    force_sandbox: sandbox,
                    no_sandbox,
                    vars,
                },
            )
        }

        Commands::List { file } => {
            let config = load_config(file)?;
            list_tasks(&config);
            Ok(())
        }

        Commands::Adapters => {
            let registry = adapter::AdapterRegistry::new();
            for name in registry.builtin_names() {
                println!("{name}");
            }
            Ok(())
        }
    }
}

fn load_config(file: Option<PathBuf>) -> Result<Config, Error> {
    let path = match file {
        Some(p) => p,
        None => config::find_amakefile()?,
    };
    Config::load(&path)
}

fn parse_vars(vars: &[String]) -> Result<BTreeMap<String, String>, Error> {
    vars.iter()
        .map(|v| {
            v.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| Error::UnresolvedVariable {
                    task: "<cli>".into(),
                    variable: v.clone(),
                    hint: "variables must be in KEY=VALUE format".into(),
                })
        })
        .collect()
}

fn list_tasks(config: &Config) {
    if config.tasks.is_empty() {
        println!("No tasks defined.");
        return;
    }

    let max_name = config
        .tasks
        .keys()
        .map(|n| n.len())
        .max()
        .unwrap_or(0);

    for (name, task) in &config.tasks {
        let tool = task
            .tool
            .as_deref()
            .or(config.defaults.tool.as_deref())
            .unwrap_or("(none)");

        let first_line = task
            .prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();

        let truncated = if first_line.len() > 60 {
            format!("{}...", &first_line[..57])
        } else {
            first_line.to_string()
        };

        println!("  {name:<max_name$}  [{tool}]  {truncated}");
    }
}
