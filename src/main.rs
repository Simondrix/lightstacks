use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use parser::InfraFile;
use std::path::PathBuf;
use std::sync::Arc;
mod graph;
mod parser;
mod runtime;
use crate::runtime::Runtime;
use crate::terraform::{TerraformAction, TerraformRunner};
mod terraform;
/// tfstacks CLI
#[derive(Parser, Debug)]
#[command(name = "tfstacks")]
#[command(about = "Run Terraform modules with dependency management", long_about = None)]
struct Cli {
    /// Path to the infrastructure YAML file
    #[arg(
        long,
        env = "TFSTACKS_INFRA_FILE",
        default_value = "deployments/infra.yaml"
    )]
    infra_file: PathBuf,

    /// Target module ID (e.g., "account-1.tenant-a.webapp")
    #[arg(long)]
    module_id: String,

    #[arg(
        long,
        env = "TFSTACKS_CACHE_DIR",
        default_value = "/tmp/.tfstacks_cache"
    )]
    cache_dir: PathBuf,

    #[arg(long, env = "TFSTACKS_MODULES_DIR", default_value = "modules")]
    modules_dir: PathBuf,

    #[arg(long, env = "TFSTACKS_TF_BIN", default_value = "terraform")]
    bin_path: PathBuf,

    /// Terraform subcommand
    #[command(subcommand)]
    action: Actions,
}

#[derive(Subcommand, Debug)]
enum Actions {
    /// Plan the module
    Plan,
    /// Apply the module
    Apply,
    /// Destroy the module
    Destroy,
}

#[tokio::main]
async fn main() {
    if let Err(err) = main_wrapper().await {
        print_error(&err);
        std::process::exit(1);
    } else {
        println!(
            "{}",
            "✔ Success: module executed successfully".green().bold()
        );
    }
}

async fn main_wrapper() -> Result<()> {
    let cli = Cli::parse();

    // Load InfraFile from YAML
    let infra =
        InfraFile::from_path(&cli.infra_file).context("while parsing infrastructure YAML file")?;
    //dbg!(&infra);
    // Map CLI action to TerraformAction
    let action = match cli.action {
        Actions::Plan => TerraformAction::Plan,
        Actions::Apply => TerraformAction::Apply,
        Actions::Destroy => TerraformAction::Destroy,
    };

    // Create TerraformRunner (actual or mock)
    let runner = TerraformRunner::new(cli.bin_path, cli.cache_dir, cli.modules_dir);

    // Wrap in Arc to allow sharing across async tasks
    let runtime = Runtime::new(Arc::new(runner), &infra)?;
    // Run the target module by module ID
    runtime.run_module(&cli.module_id, action).await?;

    Ok(())
}

/// Prints an anyhow::Error with color and cause chain (Terraform-style)
fn print_error(context: &anyhow::Error) {
    eprintln!("{} {}:", "Error".red().bold(), context.to_string().bold());

    let err_chain = context.chain().skip(1);
    for cause in err_chain {
        print_error_cause(&cause.to_string());
    }
}

/// Prints a single cause in Terraform-style:
/// - first line: │ <first line of cause>
/// - remaining lines: indented 2 spaces
pub fn print_error_cause(msg: &str) {
    let mut lines = msg.lines();

    if let Some(first) = lines.next() {
        eprintln!("  | {}", first);
    }

    for line in lines {
        eprintln!("    {}", line);
    }
}
