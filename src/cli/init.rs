use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Args;

use crate::cli::login::{
    login_azure_devops, login_github, login_gitlab, refresh_cached_github_identity,
};
use crate::ui;

#[derive(Debug, Args)]
pub struct InitCommand {
    #[arg(long, default_value = "devopster-config.yaml")]
    pub output: String,

    /// Skip the interactive provider sign-in prompt
    #[arg(long)]
    pub no_login: bool,
}

impl InitCommand {
    pub async fn run(&self, config_path: &str) -> Result<()> {
        let destination = if self.output == "devopster-config.yaml" {
            config_path
        } else {
            &self.output
        };

        ui::header("devopster setup");
        ui::info("Let’s set up your configuration.");

        ui::section("Pick a provider");
        let provider_options = vec![
            "GitHub".to_string(),
            "Azure DevOps".to_string(),
            "GitLab".to_string(),
        ];
        let provider = match ui::select("Provider", &provider_options, 0)? {
            1 => "azure_devops",
            2 => "gitlab",
            _ => "github",
        };

        // ── Step 2: Sign in ───────────────────────────────────────────────────
        if !self.no_login {
            let already_ok = is_authenticated(provider).await;
            if !already_ok {
                ui::info(&format!("Signing in to {}...", provider_display(provider)));
                match provider {
                    "github" => login_github()?,
                    "azure_devops" => login_azure_devops()?,
                    "gitlab" => login_gitlab()?,
                    _ => {}
                }
            } else {
                ui::success(&format!(
                    "Already signed in to {}.",
                    provider_display(provider)
                ));
                if provider == "github" {
                    let _ = refresh_cached_github_identity();
                }
            }
        }

        // ── Step 3: Pick org / namespace ──────────────────────────────────────
        ui::section("Choose organization");
        let (org, project, api_url) = pick_org(provider).await?;
        ui::success(&format!("Using {}.", org));

        // ── Step 4: Pick repositories to target ──────────────────────────────
        ui::section("Choose repository scope");
        let scoped_repos = pick_repos(provider, &org, project.as_deref(), &api_url).await;

        // ── Step 5: Copilot (GitHub only) ─────────────────────────────────────
        let copilot_enabled = if provider == "github" {
            ask_copilot_enabled().await
        } else {
            false
        };

        // ── Step 6: Write config ──────────────────────────────────────────────
        let yaml = build_config_yaml(
            provider,
            &org,
            project.as_deref(),
            &api_url,
            &scoped_repos,
            copilot_enabled,
        );

        if Path::new(destination).exists() {
            let existing = std::fs::read_to_string(destination).unwrap_or_default();
            if existing == yaml {
                ui::success(&format!(
                    "Configuration is already up to date at {destination}."
                ));
                ui::info("Run `devopster repo list` to get started.");
                return Ok(());
            }

            ui::section(&format!("Review configuration for {destination}"));
            print_config_summary(
                provider,
                &org,
                project.as_deref(),
                &scoped_repos,
                copilot_enabled,
            );

            ui::info("Existing file preview:");
            for line in existing.lines().take(8) {
                ui::item(line);
            }
            if existing.lines().count() > 8 {
                ui::item(&format!(
                    "... ({} more lines)",
                    existing.lines().count().saturating_sub(8)
                ));
            }
            if !ui::prompt_confirm("Apply these values?", true)? {
                ui::warn("Keeping existing config.");
                return Ok(());
            }
        } else {
            ui::section(&format!("Review configuration for {destination}"));
            print_config_summary(
                provider,
                &org,
                project.as_deref(),
                &scoped_repos,
                copilot_enabled,
            );
            if !ui::prompt_confirm("Save this configuration?", true)? {
                ui::warn("Cancelled.");
                return Ok(());
            }
        }

        std::fs::write(destination, &yaml)
            .with_context(|| format!("failed to write config to {destination}"))?;

        ui::success(&format!("Configuration saved to {destination}."));
        ui::info("Run `devopster repo list` to get started.");

        Ok(())
    }
}

// ── Provider helpers ─────────────────────────────────────────────────────────

fn provider_display(provider: &str) -> &str {
    match provider {
        "github" => "GitHub",
        "azure_devops" => "Azure DevOps",
        "gitlab" => "GitLab",
        _ => provider,
    }
}

async fn is_authenticated(provider: &str) -> bool {
    match provider {
        "github" => cli_ok("gh", &["auth", "status"]).await,
        "azure_devops" => cli_ok("az", &["account", "show"]).await,
        "gitlab" => cli_ok("glab", &["auth", "status"]).await,
        _ => false,
    }
}

// ── Org / namespace picker ────────────────────────────────────────────────────

/// Returns `(org_name, optional_project, api_url)`.
async fn pick_org(provider: &str) -> Result<(String, Option<String>, String)> {
    match provider {
        "github" => pick_github_org().await,
        "azure_devops" => pick_azure_org().await,
        "gitlab" => pick_gitlab_group().await,
        _ => bail!("unknown provider: {provider}"),
    }
}

async fn pick_github_org() -> Result<(String, Option<String>, String)> {
    ui::info("Fetching your GitHub accounts and organizations...");

    let mut orgs: Vec<String> = Vec::new();

    // Personal account namespace
    if let Ok(login) = cli_capture("gh", &["api", "/user", "--jq", ".login"]).await {
        let login = login.trim().to_string();
        if !login.is_empty() {
            orgs.push(format!("{login} (personal)"));
        }
    }

    // Organizations the user belongs to
    if let Ok(out) = cli_capture(
        "gh",
        &["api", "/user/orgs", "--paginate", "--jq", ".[].login"],
    )
    .await
    {
        for line in out.lines() {
            let l = line.trim().to_string();
            if !l.is_empty() {
                orgs.push(l);
            }
        }
    }

    if orgs.is_empty() {
        return ask_org_url("github");
    }

    let mut options = orgs.clone();
    options.push("Paste a GitHub URL".to_string());
    let selected = ui::select("GitHub account or organization", &options, 0)?;
    if selected == options.len() - 1 {
        let input = ui::prompt_input("GitHub URL")?;
        let org = last_url_segment(input.trim())?;
        return Ok((org, None, "https://api.github.com".to_string()));
    }

    let selected = orgs.get(selected).unwrap_or(&orgs[0]).clone();
    let org = selected.trim_end_matches(" (personal)").to_string();
    Ok((org, None, "https://api.github.com".to_string()))
}

async fn pick_azure_org() -> Result<(String, Option<String>, String)> {
    ui::info("Enter your Azure DevOps organization URL, for example https://dev.azure.com/my-org.");
    let input = ui::prompt_input("Azure DevOps URL")?;
    let org_url = input.trim().trim_end_matches('/').to_string();

    let org_name =
        last_url_segment(&org_url).context("could not extract org name from Azure DevOps URL")?;

    // List projects under this org
    ui::info(&format!("Fetching projects for {}...", org_url));
    let projects: Vec<String> = cli_capture(
        "az",
        &[
            "devops", "project", "list", "--org", &org_url, "--output", "json", "--query",
            "[].name",
        ],
    )
    .await
    .ok()
    .and_then(|json| serde_json::from_str(&json).ok())
    .unwrap_or_default();

    let project = if projects.is_empty() {
        prompt("Azure DevOps project name")?
    } else if projects.len() == 1 {
        projects.into_iter().next().unwrap()
    } else {
        let idx = ui::select("Azure DevOps project", &projects, 0)?;
        projects.get(idx).unwrap_or(&projects[0]).clone()
    };

    Ok((org_name, Some(project), org_url))
}

async fn pick_gitlab_group() -> Result<(String, Option<String>, String)> {
    ui::info("Fetching your GitLab namespaces and groups...");

    let mut groups: Vec<String> = Vec::new();

    // Personal namespace
    if let Ok(username) = cli_capture("glab", &["api", "/user", "-q", ".username"]).await {
        let username = username.trim().to_string();
        if !username.is_empty() {
            groups.push(format!("{username} (personal)"));
        }
    }

    // Groups the user is a member of
    if let Ok(out) = cli_capture(
        "glab",
        &[
            "api",
            "/groups",
            "--field",
            "per_page=100",
            "-q",
            ".[].full_path",
        ],
    )
    .await
    {
        for line in out.lines() {
            let l = line.trim().to_string();
            if !l.is_empty() {
                groups.push(l);
            }
        }
    }

    if groups.is_empty() {
        return ask_org_url("gitlab");
    }

    let mut options = groups.clone();
    options.push("Paste a GitLab URL".to_string());
    let selected = ui::select("GitLab namespace or group", &options, 0)?;
    if selected == options.len() - 1 {
        let input = ui::prompt_input("GitLab URL")?;
        let group = last_url_segment(input.trim())?;
        return Ok((group, None, "https://gitlab.com/api/v4".to_string()));
    }

    let selected = groups.get(selected).unwrap_or(&groups[0]).clone();
    let group = selected.trim_end_matches(" (personal)").to_string();
    Ok((group, None, "https://gitlab.com/api/v4".to_string()))
}

fn ask_org_url(provider: &str) -> Result<(String, Option<String>, String)> {
    let example = match provider {
        "github" => "https://github.com/my-org",
        "gitlab" => "https://gitlab.com/my-group",
        _ => "https://dev.azure.com/my-org",
    };
    ui::warn("Could not fetch organizations automatically.");
    let input = ui::prompt_input(&format!("Organization URL (for example {example})"))?;
    let url_owned = input.trim().trim_end_matches('/').to_string();

    let org = last_url_segment(&url_owned)?;
    let api_url = match provider {
        "github" => "https://api.github.com".to_string(),
        "gitlab" => "https://gitlab.com/api/v4".to_string(),
        _ => {
            let parts: Vec<&str> = url_owned.splitn(4, '/').collect();
            parts[..parts.len().min(3)].join("/")
        }
    };
    Ok((org, None, api_url))
}

// ── Repo scope picker ─────────────────────────────────────────────────────────

async fn pick_repos(
    provider: &str,
    org: &str,
    project: Option<&str>,
    org_url: &str,
) -> Vec<String> {
    let repos = fetch_repo_names(provider, org, project, org_url).await;

    let repos = match repos {
        Ok(r) if !r.is_empty() => r,
        _ => {
            ui::warn("Could not fetch the repository list. All repositories will be targeted.");
            return Vec::new();
        }
    };

    ui::info(&format!("Found {} repositories in {}.", repos.len(), org));
    let scope_options = vec![
        format!("Target all {} repositories", repos.len()),
        "Select specific repositories".to_string(),
    ];
    let scope = match ui::select("Repository scope", &scope_options, 0) {
        Ok(choice) => choice,
        Err(_) => return Vec::new(),
    };

    if scope == 0 {
        return Vec::new();
    }

    let selected = match ui::multi_select("Select repositories", &repos, None) {
        Ok(selected) => selected,
        Err(_) => return Vec::new(),
    };

    if selected.is_empty() {
        return Vec::new();
    }

    let selected: Vec<String> = selected
        .into_iter()
        .filter_map(|idx| repos.get(idx).cloned())
        .collect();

    if selected.is_empty() {
        return Vec::new();
    }

    ui::success(&format!("{} repositories selected.", selected.len()));
    selected
}

async fn fetch_repo_names(
    provider: &str,
    org: &str,
    project: Option<&str>,
    org_url: &str,
) -> Result<Vec<String>> {
    match provider {
        "github" => {
            let out = cli_capture(
                "gh",
                &[
                    "repo", "list", org, "--limit", "200", "--json", "name", "--jq", ".[].name",
                ],
            )
            .await?;
            Ok(non_empty_lines(&out))
        }
        "azure_devops" => {
            let project = project.context("Azure DevOps project is required")?;
            let json = cli_capture(
                "az",
                &[
                    "repos",
                    "list",
                    "--org",
                    org_url,
                    "--project",
                    project,
                    "--output",
                    "json",
                    "--query",
                    "[].name",
                ],
            )
            .await?;
            Ok(serde_json::from_str::<Vec<String>>(&json)?)
        }
        "gitlab" => {
            let encoded = org.replace('/', "%2F");
            let endpoint = format!("/groups/{encoded}/projects?per_page=100&simple=true");
            let out = cli_capture("glab", &["api", &endpoint, "-q", ".[].path"]).await?;
            Ok(non_empty_lines(&out))
        }
        _ => bail!("unknown provider: {provider}"),
    }
}

// ── Copilot prompt ────────────────────────────────────────────────────────────

async fn ask_copilot_enabled() -> bool {
    ui::prompt_confirm(
        "Enable Copilot-assisted suggestions for repos missing topics or descriptions?",
        true,
    )
    .unwrap_or(true)
}

// ── Config summary display ────────────────────────────────────────────────────

fn print_config_summary(
    provider: &str,
    org: &str,
    project: Option<&str>,
    scoped_repos: &[String],
    copilot_enabled: bool,
) {
    ui::key_value("Provider", provider_display(provider));
    ui::key_value("Organization", org);
    if let Some(p) = project {
        ui::key_value("Project", p);
    }
    if scoped_repos.is_empty() {
        ui::key_value("Scope", "all repositories");
    } else {
        ui::key_value(
            "Scope",
            format!(
                "{} selected: {}",
                scoped_repos.len(),
                scoped_repos.join(", ")
            ),
        );
    }
    ui::key_value(
        "Copilot",
        if copilot_enabled {
            "enabled"
        } else {
            "disabled"
        },
    );
}

// ── Config YAML builder ───────────────────────────────────────────────────────

fn build_config_yaml(
    provider: &str,
    org: &str,
    project: Option<&str>,
    org_url: &str,
    scoped_repos: &[String],
    copilot_enabled: bool,
) -> String {
    let mut y = String::new();

    y.push_str(&format!("provider: {provider}\n"));
    y.push_str(&format!("organization: {org}\n"));

    if !scoped_repos.is_empty() {
        y.push_str("scoped_repos:\n");
        for r in scoped_repos {
            y.push_str(&format!("  - {r}\n"));
        }
    }

    if copilot_enabled {
        y.push_str("copilot_enabled: true\n");
    }

    y.push('\n');

    match provider {
        "github" => {
            y.push_str("github:\n");
            y.push_str("  api_url: https://api.github.com\n");
            y.push_str("  token_env: GITHUB_TOKEN\n");
        }
        "azure_devops" => {
            let proj = project.unwrap_or("your-project");
            y.push_str("azure_devops:\n");
            y.push_str(&format!("  organization_url: {org_url}\n"));
            y.push_str(&format!("  project: {proj}\n"));
            y.push_str("  token_env: AZDO_TOKEN\n");
        }
        "gitlab" => {
            y.push_str("gitlab:\n");
            y.push_str("  api_url: https://gitlab.com/api/v4\n");
            y.push_str("  token_env: GITLAB_TOKEN\n");
        }
        _ => {}
    }

    y.push('\n');
    y.push_str("default_branch: main\n");
    y.push_str("catalog:\n");
    y.push_str("  output_path: generated/catalog.json\n");
    y.push('\n');
    y.push_str("templates:\n");
    y.push_str("  - name: default\n");
    y.push_str("    description: Default repository template.\n");
    y.push_str("    topics: []\n");

    y
}

// ── Shared CLI utilities ──────────────────────────────────────────────────────

/// Run a command and return true if it exits successfully.
async fn cli_ok(bin: &str, args: &[&str]) -> bool {
    tokio::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a command and return its trimmed stdout, or an error.
async fn cli_capture(bin: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new(bin)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to run `{bin}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`{bin}` exited with an error: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn non_empty_lines(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn last_url_segment(url: &str) -> Result<String> {
    url.trim_end_matches('/')
        .split('/')
        .next_back()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .context("could not extract name from URL")
}

fn prompt(label: &str) -> Result<String> {
    Ok(ui::prompt_input(label)?.trim().to_string())
}
