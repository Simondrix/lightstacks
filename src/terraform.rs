use crate::parser::ModuleNode;
use anyhow::{Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::{fs, io};

/// Terraform actions
#[derive(Debug, Clone, Copy)]
pub enum TerraformAction {
    Plan,
    Apply,
    Destroy,
}

/// Terraform command outputs
pub struct TerraformOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
#[derive(Debug)]
pub struct MockRunner;

/// Trait for running Terraform commands
#[async_trait::async_trait]
pub trait RunTerraformCommand: std::fmt::Debug {
    async fn init(&self, module: &ModuleNode) -> Result<()>;
    async fn output(&self, module: &ModuleNode) -> Result<HashMap<String, Value>>;
    async fn apply(&self, module: &ModuleNode) -> Result<()>;
}

/// Mock runner for testing

#[async_trait::async_trait]
impl RunTerraformCommand for MockRunner {
    async fn init(&self, module: &ModuleNode) -> Result<()> {
        println!("[mock] terraform init '{}'", module.id);
        Ok(())
    }

    async fn output(&self, module: &ModuleNode) -> Result<HashMap<String, Value>> {
        Ok(module.mocked_outputs.clone().unwrap_or_default())
    }

    async fn apply(&self, module: &ModuleNode) -> Result<()> {
        println!("[mock] terraform apply '{}'", module.id);
        Ok(())
    }
}

/// Real Terraform runner
#[derive(Debug)]
pub struct TerraformRunner {
    pub bin_path: PathBuf,    // terraform binary
    pub cache_dir: PathBuf,   // per-module terraform state
    pub modules_dir: PathBuf, // terraform modules source
}

impl TerraformRunner {
    pub fn new(bin_path: PathBuf, cache_dir: PathBuf, modules_dir: PathBuf) -> Self {
        Self {
            bin_path,
            cache_dir,
            modules_dir,
        }
    }

    /// Get per-module terraform working directory
    fn module_dir(&self, module: &ModuleNode) -> PathBuf {
        self.cache_dir.join(&module.id)
    }

    /// Convert module variables to TF_VAR_* environment variables
    fn tf_var_env(vars: &HashMap<String, Value>) -> HashMap<String, String> {
        vars.iter()
            .map(|(k, v)| {
                let json_value = match v {
                    Value::String(s) => s.clone(),
                    _ => serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()),
                };
                (format!("TF_VAR_{}", k), json_value)
            })
            .collect()
    }

    /// Ensure terraform directory exists and copy module sources
    pub async fn ensure_module_dir(&self, module: &ModuleNode) -> Result<PathBuf> {
        let dir = self.module_dir(module);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("Failed to create terraform dir: {:?}", dir))?;

        let src_dir = self.modules_dir.join(&module.source);

        async fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
            let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
            while let Some((src_dir, dst_dir)) = stack.pop() {
                fs::create_dir_all(&dst_dir).await?;
                let mut entries = fs::read_dir(&src_dir).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let path = entry.path();
                    let dst_path = dst_dir.join(entry.file_name());
                    if path.is_dir() {
                        stack.push((path, dst_path));
                    } else {
                        fs::copy(&path, &dst_path).await?;
                    }
                }
            }
            Ok(())
        }

        copy_dir(&src_dir, &dir).await.with_context(|| {
            format!(
                "Failed to copy module files from {:?} to {:?}",
                src_dir, dir
            )
        })?;

        Ok(dir)
    }

    /// Run terraform CLI command asynchronously in a specific directory
    /// `args` is optional (default empty)
    pub async fn run_terraform_cmd(
        &self,
        dir: &Path,
        args: Option<&[&str]>,
        envs: Option<&HashMap<String, String>>,
    ) -> Result<TerraformOutput> {
        let args = args.unwrap_or(&[]);
        println!("Running {:?} with {:?} in {:?}", &self.bin_path, args, dir);
        let local_envs = HashMap::new();
        let envs = envs.unwrap_or(&local_envs);
        let output = Command::new(&self.bin_path)
            .args(args)
            .current_dir(dir)
            .envs(envs)
            .output()
            .await
            .with_context(|| format!("Failed to run terraform command {:?}", args))?;

        if !output.status.success() {
            anyhow::bail!(
                "Terraform command {:?} failed with status {:?}\nStderr: {}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(TerraformOutput {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Run terraform CLI command asynchronously interactively
    /// `args` and `envs` are optional (defaults: empty args, empty envs)
    pub async fn run_terraform_cmd_interactively(
        &self,
        dir: &Path,
        args: Option<&[&str]>,
        envs: Option<&HashMap<String, String>>,
    ) -> Result<()> {
        let args = args.unwrap_or(&[]);
        let local_envs = HashMap::new();
        let envs = envs.unwrap_or(&local_envs);
        println!("Running {:?} with {:?} in {:?}", &self.bin_path, args, dir);
        //dbg!(envs);

        let status = Command::new(&self.bin_path)
            .args(args)
            .current_dir(dir)
            .envs(envs)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("Failed to run terraform command {:?}", args))?;

        if !status.success() {
            anyhow::bail!(
                "Terraform command {:?} failed with status {:?}",
                args,
                status
            );
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl RunTerraformCommand for TerraformRunner {
    async fn init(&self, module: &ModuleNode) -> Result<()> {
        let dir = self.ensure_module_dir(module).await?;
        self.run_terraform_cmd(&dir, Some(&["init", "-input=false"]), None)
            .await?;
        Ok(())
    }

    async fn output(&self, module: &ModuleNode) -> Result<HashMap<String, Value>> {
        let dir = self.module_dir(module);
        let resp = self
            .run_terraform_cmd(&dir, Some(&["output", "-json"]), None)
            .await?;
        let value: HashMap<String, Value> =
            serde_json::from_slice(&resp.stdout).context("Failed to parse terraform output")?;
        Ok(value)
    }

    async fn apply(&self, module: &ModuleNode) -> Result<()> {
        let dir = self.module_dir(module);
        let envs = TerraformRunner::tf_var_env(&module.variables);
        self.run_terraform_cmd_interactively(&dir, Some(&["apply", "-auto-approve"]), Some(&envs))
            .await?;
        Ok(())
    }
}
