/// Browser-based login by delegating to each platform's own CLI tool.
///
/// - GitHub       -> `gh auth login --web --hostname github.com`
///   then reads the token via `gh auth token`
/// - Azure DevOps -> `az login --allow-no-subscriptions`
///   then reads the token via `az account get-access-token`
/// - GitLab       -> `glab auth login`
///   then reads the token via `glab auth token`
///
/// No OAuth app registration required. Each platform CLI owns the browser
/// redirect; devopster just reads the resulting token and caches it so the
/// providers can use it without an environment variable being set.
use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};

use crate::auth::{
    load_token, remove_token, save_github_identity, save_token, GitHubIdentity, StoredToken,
};
use crate::ui;

const AZURE_DEVOPS_RESOURCE_ID: &str = "499b84ac-1321-427f-aa17-267ca6975798";

// --- CLI types ---

#[derive(Debug, Args)]
pub struct LoginCommand {
    #[command(subcommand)]
    pub provider: LoginProvider,
}

#[derive(Debug, Subcommand)]
pub enum LoginProvider {
    /// Sign in to GitHub via browser (requires the `gh` CLI)
    Github,
    /// Sign in to Azure DevOps via Microsoft browser sign-in (requires the `az` CLI)
    AzureDevops,
    /// Sign in to GitLab via browser (requires the `glab` CLI)
    Gitlab,
    /// Sign in to all three providers sequentially
    All,
    /// Remove stored credentials for a provider
    Logout(LogoutArgs),
    /// Show authentication status for all providers
    Status,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Provider to sign out from: github, azure_devops, gitlab
    pub provider: String,
}

// --- Entry point ---

impl LoginCommand {
    pub async fn run(&self) -> Result<()> {
        match &self.provider {
            LoginProvider::Github => login_github(),
            LoginProvider::AzureDevops => login_azure_devops(),
            LoginProvider::Gitlab => login_gitlab(),
            LoginProvider::All => {
                login_github()?;
                login_azure_devops()?;
                login_gitlab()
            }
            LoginProvider::Logout(args) => {
                remove_token(&args.provider)?;
                ui::success(&format!("Signed out from {}.", args.provider));
                Ok(())
            }
            LoginProvider::Status => auth_status(),
        }
    }
}

// --- Auth status ---

fn auth_status() -> Result<()> {
    let providers = [
        ("github", "GITHUB_TOKEN", "devopster login github"),
        (
            "azure_devops",
            "AZURE_DEVOPS_PAT",
            "devopster login azure-devops",
        ),
        ("gitlab", "GITLAB_TOKEN", "devopster login gitlab"),
    ];

    ui::header("Authentication Status");

    for (name, env_var, login_cmd) in &providers {
        let (status, source) = if env::var(env_var).is_ok() {
            ("logged in", format!("env var ${env_var}"))
        } else if load_token(name)?.is_some() {
            ("logged in", "saved token".to_string())
        } else {
            ("not logged in", format!("run `{login_cmd}`"))
        };
        ui::item(&format!("{:<15} {:<12} {}", name, status, source));
    }

    Ok(())
}

// --- GitHub ---

pub fn login_github() -> Result<()> {
    require_tool("gh", "GitHub CLI", "https://cli.github.com")?;
    bail_if_ci("github", "GITHUB_TOKEN")?;

    // The device-code entry page is always this URL — open it before gh asks
    // anything so the user only needs to type in the one-time code shown below.
    const URL: &str = "https://github.com/login/device";
    ui::info(&format!(
        "Opening browser for GitHub sign-in: {}",
        term_link(URL, URL)
    ));
    open_browser(URL);

    // Use the device code flow (no --web flag) so it works inside containers
    // and any environment without a local browser.
    //
    // gh asks two questions when stdin is not a TTY:
    //   1. "How would you like to authenticate?" → \n selects the default
    //      "Login with a web browser" (= device code flow)
    //   2. "Press Enter to open github.com in your browser..." → \n continues
    //
    // We pipe those two Enter presses with a delay so the one-time code is
    // printed to the terminal before we advance past the second prompt.
    let mut child = Command::new("gh")
        .args([
            "auth",
            "login",
            "--hostname",
            "github.com",
            "--git-protocol",
            "https",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to launch `gh auth login`")?;

    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            // Select "Login with a web browser" (default first option).
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = stdin.write_all(b"\n");
            // Wait for gh to print the one-time code, then confirm "Press Enter".
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = stdin.write_all(b"\n");
        });
    }

    let status = child.wait().context("failed to wait for `gh auth login`")?;
    if !status.success() {
        bail!("GitHub sign-in failed");
    }

    let token = run_capture("gh", &["auth", "token"])
        .context("failed to retrieve GitHub token from `gh auth token`")?;

    let identity = resolve_github_identity().ok();
    save_token(
        "github",
        StoredToken {
            access_token: token,
            refresh_token: None,
            github_identity: identity.clone(),
        },
    )?;
    if let Some(identity) = identity {
        save_github_identity(identity)?;
    }
    ui::success("Successfully signed in to GitHub. Credentials saved.");
    Ok(())
}

// --- Azure DevOps ---

pub fn login_azure_devops() -> Result<()> {
    require_tool(
        "az",
        "Azure CLI",
        "https://learn.microsoft.com/cli/azure/install-azure-cli",
    )?;
    bail_if_ci("azure_devops", "AZURE_DEVOPS_PAT")?;

    // Try to get a token silently first (user may already be signed in).
    if let Ok(token) = az_devops_access_token() {
        save_token(
            "azure_devops",
            StoredToken {
                access_token: token,
                refresh_token: None,
                github_identity: None,
            },
        )?;
        ui::success("Already signed in. Azure DevOps credentials saved.");
        return Ok(());
    }

    // Use device code flow so it works consistently inside containers and on
    // any host.  Pre-open the device login page so the user only needs to
    // type in the code that `az` prints -- no manual browser step needed.
    const DEVICE_URL: &str = "https://microsoft.com/devicelogin";
    ui::info(&format!(
        "Opening browser for Microsoft account sign-in: {}",
        term_link(DEVICE_URL, DEVICE_URL)
    ));
    open_browser(DEVICE_URL);

    let status = Command::new("az")
        .args(["login", "--use-device-code", "--allow-no-subscriptions"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to launch `az login`")?;

    if !status.success() {
        bail!("Azure sign-in failed");
    }

    let token = az_devops_access_token()
        .context("signed in but could not retrieve an Azure DevOps access token")?;

    save_token(
        "azure_devops",
        StoredToken {
            access_token: token,
            refresh_token: None,
            github_identity: None,
        },
    )?;
    ui::success("Successfully signed in to Azure DevOps. Credentials saved.");
    Ok(())
}

fn az_devops_access_token() -> Result<String> {
    run_capture(
        "az",
        &[
            "account",
            "get-access-token",
            "--resource",
            AZURE_DEVOPS_RESOURCE_ID,
            "--query",
            "accessToken",
            "--output",
            "tsv",
        ],
    )
}

// --- GitLab ---

pub fn login_gitlab() -> Result<()> {
    require_tool(
        "glab",
        "GitLab CLI",
        "https://gitlab.com/gitlab-org/cli#installation",
    )?;
    bail_if_ci("gitlab", "GITLAB_TOKEN")?;

    // `glab auth login --web` opens the browser for the OAuth flow.
    // Show a clickable fallback link in case the launch fails.
    const GITLAB_URL: &str = "https://gitlab.com/-/profile/personal_access_tokens";
    ui::info(&format!(
        "Opening browser for GitLab sign-in. Fallback: {}",
        term_link(GITLAB_URL, GITLAB_URL)
    ));

    let status = Command::new("glab")
        .args(["auth", "login", "--web", "--hostname", "gitlab.com"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to launch `glab auth login`")?;

    if !status.success() {
        bail!("GitLab sign-in failed");
    }

    let token = run_capture("glab", &["auth", "token"])
        .context("failed to retrieve GitLab token from `glab auth token`")?;

    save_token(
        "gitlab",
        StoredToken {
            access_token: token,
            refresh_token: None,
            github_identity: None,
        },
    )?;
    ui::success("Successfully signed in to GitLab. Credentials saved.");
    Ok(())
}

pub fn refresh_cached_github_identity() -> Result<Option<GitHubIdentity>> {
    let identity = resolve_github_identity()?;
    save_github_identity(identity.clone())?;
    Ok(Some(identity))
}

// --- Helpers ---

/// In a CI environment (no TTY / browser) browser login cannot work.
/// Tell the user to supply the token via an environment variable instead.
fn bail_if_ci(provider: &str, env_var: &str) -> Result<()> {
    let is_ci = env::var("CI").is_ok()
        || env::var("GITHUB_ACTIONS").is_ok()
        || env::var("TF_BUILD").is_ok()        // Azure Pipelines
        || env::var("GITLAB_CI").is_ok();

    if is_ci {
        bail!(
            "Browser sign-in is not available in CI environments.\n\
             Set the {env_var} environment variable to authenticate with {provider} in CI.\n\
             Alternatively mount your host credentials:\n\
             docker run -v ~/.config/devopster:/root/.config/devopster ..."
        );
    }
    Ok(())
}

/// Verify a required CLI tool is on PATH, with a helpful install hint if not.
fn require_tool(bin: &str, name: &str, install_url: &str) -> Result<()> {
    which(bin).with_context(|| {
        format!(
            "`{bin}` not found in PATH.\n\
             The {name} is required for browser sign-in.\n\
             Install it from: {install_url}"
        )
    })
}

fn which(bin: &str) -> Result<()> {
    let status = Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("could not check for `{bin}`"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("`{bin}` not found")
    }
}

/// Open `url` in the system default browser.
///
/// On the host (macOS/Windows/Linux with a display) the OS command is used
/// directly.  Inside a Docker container there is no display — instead the URL
/// is written to `/app/.devopster_open_url` (the mounted project directory).
/// `setup.sh` runs a lightweight watcher on the host that reads that file and
/// calls `open` so the browser pops up automatically on the host machine.
fn open_browser(url: &str) {
    // Inside a Docker container /.dockerenv always exists.
    if std::path::Path::new("/.dockerenv").exists() {
        // Signal the host-side watcher in setup.sh.
        let _ = std::fs::write("/app/.devopster_open_url", url);
        return;
    }

    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/c", "start", "", url]).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    let _ = result;
}

/// Return an OSC 8 terminal hyperlink.  Terminals that do not support OSC 8
/// will show `label` as plain text; the escape sequences are silently ignored.
fn term_link(url: &str, label: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
}

/// Run a command and return its trimmed stdout as a String.
fn run_capture(bin: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("failed to run `{bin}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`{bin}` exited with an error: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        bail!("`{bin}` returned an empty token");
    }
    Ok(stdout)
}

fn resolve_github_identity() -> Result<GitHubIdentity> {
    let user_json =
        run_capture("gh", &["api", "/user"]).context("failed to load signed-in GitHub user")?;
    let user: GitHubUser =
        serde_json::from_str(&user_json).context("failed to decode signed-in GitHub user")?;

    let display_name = user
        .name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| user.login.clone());

    let email = user
        .email
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| github_primary_email().ok())
        .unwrap_or_else(|| github_noreply_email(user.id, &user.login));

    Ok(GitHubIdentity {
        login: user.login,
        name: display_name,
        email,
    })
}

fn github_primary_email() -> Result<String> {
    let emails_json = run_capture("gh", &["api", "/user/emails"])
        .context("failed to load GitHub email addresses")?;
    let emails: Vec<GitHubEmail> =
        serde_json::from_str(&emails_json).context("failed to decode GitHub email addresses")?;

    emails
        .into_iter()
        .find(|email| email.primary && email.verified && !email.email.trim().is_empty())
        .map(|email| email.email)
        .context("no primary verified GitHub email address is available")
}

fn github_noreply_email(id: u64, login: &str) -> String {
    format!("{id}+{login}@users.noreply.github.com")
}

#[derive(Debug, serde::Deserialize)]
struct GitHubUser {
    login: String,
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}
