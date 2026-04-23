use std::env;
use std::io::IsTerminal;
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Args;

use crate::ui;

#[derive(Debug, Args)]
pub struct DevEnvCommand {
    /// Docker image tag used for the local developer container
    #[arg(long, default_value = "devopster-cli-dev")]
    pub image: String,

    /// Skip rebuilding the container image before launch
    #[arg(long)]
    pub no_build: bool,

    /// Skip running `devopster setup` after bootstrap inside the container
    #[arg(long)]
    pub no_onboarding: bool,
}

impl DevEnvCommand {
    pub async fn run(&self) -> Result<()> {
        ui::header("devopster local developer environment");

        ensure_docker_ready()?;

        if !self.no_build {
            ui::section("Build container image");
            run_docker(
                Command::new("docker").args(["build", "--target", "dev", "-t", &self.image, "."]),
                "docker build failed",
            )?;
        }

        ui::section("Start local container");
        let current_dir = env::current_dir().context("failed to read current directory")?;
        let current_dir = current_dir.to_string_lossy().to_string();
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .context("could not resolve HOME/USERPROFILE for config mount")?;
        let host_config_dir = format!("{home}/.config/devopster");

        let in_container_cmd = if self.no_onboarding {
            "make bootstrap"
        } else {
            "make bootstrap && devopster setup"
        };

        let mut run = Command::new("docker");
        run.arg("run").arg("--rm");
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            run.arg("-it");
        }
        run.args([
            "-v",
            &format!("{host_config_dir}:/root/.config/devopster"),
            "-v",
            &format!("{current_dir}:/app"),
            "-w",
            "/app",
            &self.image,
            "bash",
            "-lc",
            in_container_cmd,
        ]);

        run_docker(run, "docker run failed")?;
        ui::success("Local containerized developer environment completed.");

        Ok(())
    }
}

fn ensure_docker_ready() -> Result<()> {
    ui::section("Check Docker");

    let docker_ok = Command::new("docker")
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !docker_ok {
        bail!("Docker is required. Install and start Docker Desktop/Engine, then retry.");
    }

    let daemon_ok = Command::new("docker")
        .arg("info")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !daemon_ok {
        bail!("Docker is installed but daemon is not reachable. Start Docker and retry.");
    }

    ui::success("Docker is available and running.");
    Ok(())
}

fn run_docker(mut cmd: Command, message: &str) -> Result<()> {
    let status = cmd.status().with_context(|| message.to_string())?;
    if !status.success() {
        bail!("{message}");
    }
    Ok(())
}
