use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Allow-list of devopster subcommand groups the desktop UI is permitted to invoke.
const ALLOWED_COMMANDS: &[&str] = &[
    "diagnostics",
    "inventory",
    "setup",
    "config",
    "stats",
    "topics",
    "catalog",
    "repo",
    "dev",
    "dev-env",
    "init",
    "login",
    "--version",
    "--help",
];

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Resolve the path to the bundled `devopster` sidecar, or fall back to PATH.
fn resolve_sidecar(app: &tauri::AppHandle) -> String {
    use tauri::Manager;

    if let Ok(resource_dir) = app.path().resource_dir() {
        let mut candidate: PathBuf = resource_dir.clone();
        candidate.push("binaries");
        candidate.push(if cfg!(windows) {
            "devopster.exe"
        } else {
            "devopster"
        });
        if candidate.exists() {
            return candidate.to_string_lossy().to_string();
        }
    }

    "devopster".to_string()
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let Some(first) = args.first() else {
        return Err("missing devopster subcommand".to_string());
    };
    if !ALLOWED_COMMANDS.contains(&first.as_str()) {
        return Err(format!("subcommand '{first}' is not allowed from the GUI"));
    }
    for a in args {
        if a.contains('\n') || a.contains('\r') || a.contains('\0') {
            return Err("arguments cannot contain control characters".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
async fn run_devopster(
    app: tauri::AppHandle,
    args: Vec<String>,
) -> Result<CommandResult, String> {
    validate_args(&args)?;
    let bin = resolve_sidecar(&app);

    let output = Command::new(&bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("failed to start {bin}: {e}"))?;

    Ok(CommandResult {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[tauri::command]
async fn stream_devopster(
    app: tauri::AppHandle,
    window: tauri::Window,
    args: Vec<String>,
) -> Result<i32, String> {
    use tauri::Emitter;

    validate_args(&args)?;
    let bin = resolve_sidecar(&app);

    let mut child = Command::new(&bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start {bin}: {e}"))?;

    if let Some(out) = child.stdout.take() {
        let win = window.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = win.emit("devopster:stdout", line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let win = window.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = win.emit("devopster:stderr", line);
            }
        });
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait failed: {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// Run a JSON-emitting devopster subcommand and return the parsed JSON value.
#[tauri::command]
async fn run_devopster_json(
    app: tauri::AppHandle,
    args: Vec<String>,
) -> Result<serde_json::Value, String> {
    validate_args(&args)?;
    let bin = resolve_sidecar(&app);

    let output = Command::new(&bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("failed to start {bin}: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "devopster exited {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("invalid JSON from devopster: {e}"))
}

#[derive(Debug, Serialize)]
pub struct EnvInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub sidecar_path: String,
    pub sidecar_exists: bool,
    pub config_path: String,
    pub config_exists: bool,
}

#[tauri::command]
fn env_info(app: tauri::AppHandle) -> EnvInfo {
    let sidecar = resolve_sidecar(&app);
    let sidecar_exists = std::path::Path::new(&sidecar).exists();
    let config_path = std::env::var("DEVOPSTER_CONFIG").unwrap_or_else(|_| {
        let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        p.push("devopster-config.yaml");
        p.to_string_lossy().to_string()
    });
    let config_exists = std::path::Path::new(&config_path).exists();

    EnvInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        sidecar_path: sidecar,
        sidecar_exists,
        config_path,
        config_exists,
    }
}

/// Read the user's devopster-config.yaml as text.
#[tauri::command]
fn read_config(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))
}

/// Write the user's devopster-config.yaml as text. Only allows yaml/yml extensions.
#[tauri::command]
fn write_config(path: String, contents: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext != "yaml" && ext != "yml" {
        return Err("refusing to write non-yaml config path".to_string());
    }
    std::fs::write(&path, contents).map_err(|e| format!("write {path}: {e}"))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            run_devopster,
            stream_devopster,
            run_devopster_json,
            env_info,
            read_config,
            write_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DevOpster desktop");
}
