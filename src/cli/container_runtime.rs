use std::env;
use std::io::IsTerminal;
use std::process::Command;

use anyhow::{bail, Context, Result};

pub fn ensure_docker_ready() -> Result<()> {
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

    Ok(())
}

pub fn build_dev_image(image: &str) -> Result<()> {
    let mut command = Command::new("docker");
    command.args(["build", "--target", "dev", "-t", image, "."]);
    run_checked(command, "docker build failed")
}

pub fn run_in_dev_container(image: &str, command: &str, interactive: bool) -> Result<()> {
    let current_dir = env::current_dir().context("failed to read current directory")?;
    let current_dir = current_dir.to_string_lossy().to_string();
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .context("could not resolve HOME/USERPROFILE for config mount")?;
    let host_config_dir = format!("{home}/.config/devopster");

    let mut run = Command::new("docker");
    run.arg("run").arg("--rm");
    if interactive && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        run.arg("-it");
    }

    run.args([
        "-v",
        &format!("{host_config_dir}:/root/.config/devopster"),
        "-v",
        &format!("{current_dir}:/app"),
        "-w",
        "/app",
        image,
        "bash",
        "-lc",
        command,
    ]);

    run_checked(run, "docker run failed")
}

fn run_checked(mut command: Command, error_message: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| error_message.to_string())?;
    if !status.success() {
        bail!("{error_message}");
    }
    Ok(())
}
