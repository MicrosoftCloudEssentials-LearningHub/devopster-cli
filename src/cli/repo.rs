use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::config::AppConfig;
use crate::provider::{
    AuditFinding, AuditPolicy, ProviderFactory, RepoSummary, RepoVisibility, ScaffoldRequest,
};

#[derive(Debug, Args)]
pub struct RepoCommand {
    #[command(subcommand)]
    pub action: RepoAction,
}

#[derive(Debug, Subcommand)]
pub enum RepoAction {
    /// List all repositories; use --topic to filter
    List(ListReposCommand),
    /// Audit repositories against the policy defined in devopster-config.yaml
    Audit(AuditReposCommand),
    /// Push files from a local directory to all matching repositories
    Sync(SyncReposCommand),
    /// Create a new repository from a named template
    Scaffold(ScaffoldRepoCommand),
}

#[derive(Debug, Args)]
pub struct ListReposCommand {
    #[arg(long)]
    pub topic: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuditReposCommand {}

#[derive(Debug, Args)]
pub struct SyncReposCommand {
    #[arg(long, default_value = ".github")]
    pub source: String,

    #[arg(long, help = "Only sync to repositories matching this template (by topic overlap)")]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct ScaffoldRepoCommand {
    #[arg(long)]
    pub name: String,

    #[arg(long)]
    pub template: String,

    #[arg(long)]
    pub description: Option<String>,

    #[arg(long, default_value_t = false)]
    pub private: bool,
}

impl RepoCommand {
    pub async fn run(&self, config_path: &str) -> Result<()> {
        let config = AppConfig::load(config_path)?;
        let provider = ProviderFactory::from_config(&config)?;

        match &self.action {
            RepoAction::List(command) => {
                let repos = provider.list_repositories(&config.organization).await?;
                let repos = scope_to_config(repos, &config.scoped_repos);
                print_repos(filter_repos(repos, command.topic.as_deref()));
            }
            RepoAction::Audit(_) => {
                let findings = provider
                    .audit_repositories(
                        &config.organization,
                        &AuditPolicy {
                            required_default_branch: config.default_branch.clone(),
                            require_description:    config.audit.require_description,
                            require_topics:         config.audit.require_topics,
                            min_topics:             config.audit.min_topics,
                            require_license:        config.audit.require_license,
                            require_default_branch: config.audit.require_default_branch,
                        },
                    )
                    .await?;
                // Filter findings to scoped repos if set.
                let findings = if config.scoped_repos.is_empty() {
                    findings
                } else {
                    findings
                        .into_iter()
                        .filter(|f| config.scoped_repos.contains(&f.repository))
                        .collect()
                };
                print_audit_findings(findings);
            }
            RepoAction::Sync(command) => {
                let source_path = std::path::Path::new(&command.source);
                if !source_path.exists() {
                    anyhow::bail!(
                        "sync source directory '{}' does not exist",
                        command.source
                    );
                }

                let files = collect_sync_files(source_path)?;
                if files.is_empty() {
                    println!("No files found in '{}'.", command.source);
                    return Ok(());
                }

                let repos = provider.list_repositories(&config.organization).await?;
                let repos = scope_to_config(repos, &config.scoped_repos);
                let repos_to_sync: Vec<_> = if let Some(template_name) = &command.template {
                    let template = config
                        .templates
                        .iter()
                        .find(|t| t.name == *template_name)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "template '{}' not found in config",
                                template_name
                            )
                        })?;
                    repos
                        .into_iter()
                        .filter(|repo| {
                            repo.topics
                                .iter()
                                .any(|topic| template.topics.contains(topic))
                        })
                        .collect()
                } else {
                    repos
                };

                println!(
                    "Syncing {} file(s) to {} repositories...",
                    files.len(),
                    repos_to_sync.len()
                );

                let commit_msg = format!(
                    "chore: sync files from devopster (source: {})",
                    command.source
                );
                let mut sync_count = 0usize;
                let mut error_count = 0usize;

                for repo in &repos_to_sync {
                    for (relative_path, file_content) in &files {
                        match provider
                            .push_file(
                                &config.organization,
                                &repo.name,
                                relative_path,
                                file_content,
                                &commit_msg,
                            )
                            .await
                        {
                            Ok(()) => {
                                sync_count += 1;
                                println!("  synced '{}' -> '{}'", relative_path, repo.name);
                            }
                            Err(err) => {
                                error_count += 1;
                                eprintln!(
                                    "  error syncing '{}' to '{}': {err:#}",
                                    relative_path, repo.name
                                );
                            }
                        }
                    }
                }

                println!(
                    "Sync complete: {sync_count} file(s) synced, {error_count} error(s)."
                );
            }
            RepoAction::Scaffold(command) => {
                let template = config
                    .templates
                    .iter()
                    .find(|template| template.name == command.template)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("template '{}' was not found in the config", command.template)
                    })?;

                let request = ScaffoldRequest {
                    name: command.name.clone(),
                    description: command
                        .description
                        .clone()
                        .unwrap_or_else(|| template.description.clone()),
                    visibility: if command.private {
                        RepoVisibility::Private
                    } else {
                        RepoVisibility::Public
                    },
                    template,
                };

                let result = provider
                    .scaffold_repository(&config.organization, &request)
                    .await?;

                println!("Created repository '{}' via {}.", result.name, result.provider);
                if let Some(web_url) = result.web_url {
                    println!("URL: {web_url}");
                }
            }
        }

        Ok(())
    }
}

fn print_audit_findings(findings: Vec<AuditFinding>) {
    if findings.is_empty() {
        println!("Audit passed: all repositories meet the configured policy.");
        return;
    }

    // Group by repository.
    let mut by_repo: std::collections::BTreeMap<&str, Vec<&AuditFinding>> =
        std::collections::BTreeMap::new();
    for f in &findings {
        by_repo.entry(f.repository.as_str()).or_default().push(f);
    }

    let sep = "-".repeat(64);
    println!("{sep}");
    println!(
        "  Audit findings: {} issue(s) in {} repository(ies)",
        findings.len(),
        by_repo.len()
    );
    println!("{sep}");
    for (repo, repo_findings) in &by_repo {
        println!("  {repo}");
        for f in repo_findings {
            println!("    [{}] {}", f.code, f.message);
        }
    }
    println!("{sep}");
    println!(
        "  Tip: run 'devopster stats --scope-missing' to scope to these repos,"
    );
    println!("  then use 'devopster topics align' or 'devopster repo sync' to fix.");
}

fn filter_repos(repos: Vec<RepoSummary>, topic: Option<&str>) -> Vec<RepoSummary> {
    match topic {
        Some(topic_filter) => repos
            .into_iter()
            .filter(|repo| repo.topics.iter().any(|topic| topic == topic_filter))
            .collect(),
        None => repos,
    }
}

/// Keep only repos listed in `config.scoped_repos` (if non-empty).
fn scope_to_config(repos: Vec<RepoSummary>, scoped: &[String]) -> Vec<RepoSummary> {
    if scoped.is_empty() {
        repos
    } else {
        repos.into_iter().filter(|r| scoped.contains(&r.name)).collect()
    }
}

fn print_repos(repos: Vec<RepoSummary>) {
    if repos.is_empty() {
        println!("No repositories matched the requested filter.");
        return;
    }

    let total = repos.len();
    let separator = "-".repeat(72);

    println!("{separator}");
    for repo in repos {
        // Name + provider badge on the same line
        println!("{:<55} [{}]", repo.name, repo.provider);

        // Description (wrapped to keep the block readable)
        if !repo.description.is_empty() {
            // Truncate long descriptions to keep output scannable
            let desc = if repo.description.chars().count() > 120 {
                format!("{}...", repo.description.chars().take(120).collect::<String>())
            } else {
                repo.description.clone()
            };
            println!("  {desc}");
        }

        // Topics
        if !repo.topics.is_empty() {
            println!("  topics: {}", repo.topics.join(", "));
        }

        // URL
        if let Some(url) = repo.web_url {
            println!("  {url}");
        }
        println!("{separator}");
    }
    println!("{total} repositories");
}

fn collect_sync_files(source: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    collect_sync_files_recursive(source, source, &mut files)?;
    Ok(files)
}

fn collect_sync_files_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("failed to read directory '{}'", dir.display()))?
    {
        let entry = entry
            .with_context(|| format!("failed to read entry in '{}'", dir.display()))?;
        let path = entry.path();

        if path.is_dir() {
            collect_sync_files_recursive(base, &path, files)?;
        } else {
            let relative = path
                .strip_prefix(base)
                .with_context(|| {
                    format!("failed to compute relative path for '{}'", path.display())
                })?
                .to_string_lossy()
                .to_string();
            let content = std::fs::read(&path)
                .with_context(|| format!("failed to read file '{}'", path.display()))?;
            files.push((relative, content));
        }
    }
    Ok(())
}
