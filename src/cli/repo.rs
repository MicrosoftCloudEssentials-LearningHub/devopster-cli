use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::config::{AppConfig, AuditConfig};
use crate::provider::{
    AuditFinding, AuditPolicy, BlueprintRequest, ProviderFactory, RepoSummary, RepoVisibility,
};
use crate::ui;

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
    /// Interactively fix missing description, topics, and license
    Fix(FixReposCommand),
    /// Sync files from a local directory or the blueprint repository
    Sync(SyncReposCommand),
    /// Create a new repository from a named blueprint
    Blueprint(BlueprintRepoCommand),
}

#[derive(Debug, Args)]
pub struct ListReposCommand {
    #[arg(long)]
    pub topic: Option<String>,
}

#[derive(Debug, Args)]
pub struct AuditReposCommand {}

#[derive(Debug, Args)]
pub struct FixReposCommand {}

#[derive(Debug, Args)]
pub struct SyncReposCommand {
    #[arg(long, default_value = ".github")]
    pub source: String,

    /// Compare blueprint workflow files and README markers instead of local files.
    #[arg(long, default_value_t = false)]
    pub from_blueprint: bool,

    /// Override the blueprint source repo (org/repo or GitHub URL).
    #[arg(long)]
    pub blueprint_repo: Option<String>,

    /// Override the blueprint source branch (default: main or config default_branch).
    #[arg(long)]
    pub blueprint_branch: Option<String>,

    /// Paths to sync from the blueprint repo (repeatable, defaults to .github).
    #[arg(long, value_name = "PATH")]
    pub blueprint_path: Vec<String>,

    #[arg(
        long,
        help = "Only sync to repositories matching this template (by topic overlap)"
    )]
    pub template: Option<String>,
}

#[derive(Debug, Args)]
pub struct BlueprintRepoCommand {
    #[arg(long)]
    pub name: String,

    #[arg(long)]
    pub template: String,

    #[arg(long)]
    pub description: Option<String>,

    #[arg(long, default_value_t = false)]
    pub private: bool,
}

const ORG_DEFAULT_LOCATION: &str = "Atlanta, USA";
const ORG_GITHUB_BADGE_LINE: &str = "[![GitHub](https://img.shields.io/badge/--181717?logo=github&logoColor=ffffff)](https://github.com/)";
const ORG_PROFILE_LINE: &str = "[brown9804](https://github.com/brown9804)";
const ORG_SEPARATOR_LINE: &str = "----------";
const ORG_BADGE_START_MARKER: &str = "<!-- START BADGE -->";
const ORG_BADGE_END_MARKER: &str = "<!-- END BADGE -->";
const ORG_TOTAL_VIEWS_BADGE_FALLBACK: &str =
    "https://img.shields.io/badge/Total%20views-0-limegreen";

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
                            require_description: config.audit.require_description,
                            require_topics: config.audit.require_topics,
                            min_topics: config.audit.min_topics,
                            require_license: config.audit.require_license,
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
            RepoAction::Fix(_) => {
                fix_repos(provider.as_ref(), &config).await?;
            }
            RepoAction::Sync(command) => {
                let use_blueprint = command.from_blueprint
                    || command.blueprint_repo.is_some()
                    || !command.blueprint_path.is_empty();

                let repos = provider.list_repositories(&config.organization).await?;
                let repos = scope_to_config(repos, &config.scoped_repos);
                let repos_to_sync: Vec<_> = if let Some(template_name) = &command.template {
                    let template = config
                        .templates
                        .iter()
                        .find(|t| t.name == *template_name)
                        .ok_or_else(|| {
                            anyhow::anyhow!("template '{}' not found in config", template_name)
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

                if repos_to_sync.is_empty() {
                    ui::warn("No repositories matched the configured sync scope.");
                    return Ok(());
                }

                if use_blueprint {
                    let blueprint = resolve_blueprint_source(&config, command)?;
                    sync_blueprint_requirements(
                        provider.as_ref(),
                        &config,
                        &repos_to_sync,
                        &blueprint,
                    )
                    .await?;
                } else {
                    sync_local_files(provider.as_ref(), &config, &repos_to_sync, command).await?;
                }
            }
            RepoAction::Blueprint(command) => {
                let template = config
                    .templates
                    .iter()
                    .find(|template| template.name == command.template)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "template '{}' was not found in the config",
                            command.template
                        )
                    })?;

                let request = BlueprintRequest {
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
                    .blueprint_repository(&config.organization, &request)
                    .await?;

                ui::success(&format!(
                    "Created repository '{}' via {}.",
                    result.name, result.provider
                ));
                if let Some(web_url) = result.web_url {
                    ui::key_value("URL", web_url);
                }
            }
        }

        Ok(())
    }
}

fn print_audit_findings(findings: Vec<AuditFinding>) {
    if findings.is_empty() {
        ui::success("Audit passed: all repositories meet the configured policy.");
        return;
    }

    // Group by repository.
    let mut by_repo: std::collections::BTreeMap<&str, Vec<&AuditFinding>> =
        std::collections::BTreeMap::new();
    for f in &findings {
        by_repo.entry(f.repository.as_str()).or_default().push(f);
    }

    ui::header("Audit Findings");
    ui::info(&format!(
        "{} issue(s) across {} repository(ies).",
        findings.len(),
        by_repo.len()
    ));
    for (repo, repo_findings) in &by_repo {
        ui::section(repo);
        for f in repo_findings {
            ui::item(&format!("[{}] {}", f.code, f.message));
        }
    }
    ui::info("Tip: run 'devopster stats --scope-missing' to scope to these repos.");
    ui::info("Then use 'devopster topics align' or 'devopster repo sync' to fix them.");
}

struct ResolvedBlueprintSource {
    repo: String,
    branch: String,
    paths: Vec<String>,
}

fn resolve_blueprint_source(
    config: &AppConfig,
    command: &SyncReposCommand,
) -> Result<ResolvedBlueprintSource> {
    let repo = command
        .blueprint_repo
        .clone()
        .or_else(|| config.blueprint.as_ref().map(|b| b.repo.clone()))
        .context("blueprint.repo is not configured (set it in devopster-config.yaml)")?;

    let branch = command
        .blueprint_branch
        .clone()
        .or_else(|| config.blueprint.as_ref().map(|b| b.branch.clone()))
        .unwrap_or_else(|| config.default_branch.clone());

    let mut paths = if !command.blueprint_path.is_empty() {
        command.blueprint_path.clone()
    } else {
        config
            .blueprint
            .as_ref()
            .map(|b| b.paths.clone())
            .unwrap_or_default()
    };

    if paths.is_empty() {
        paths.push(".github".to_string());
    }

    Ok(ResolvedBlueprintSource {
        repo,
        branch,
        paths,
    })
}

fn parse_repo_slug(input: &str) -> Result<(String, String)> {
    let trimmed = input.trim().trim_end_matches('/');
    let slug = if let Some(pos) = trimmed.find("github.com/") {
        &trimmed[pos + "github.com/".len()..]
    } else {
        trimmed
    };

    let mut parts = slug.split('/').filter(|p| !p.is_empty());
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");

    if owner.is_empty() || repo.is_empty() {
        anyhow::bail!("blueprint repo must be in 'org/repo' or GitHub URL format")
    }

    let repo = repo.trim_end_matches(".git");
    Ok((owner.to_string(), repo.to_string()))
}

async fn sync_local_files(
    provider: &dyn crate::provider::Provider,
    config: &AppConfig,
    repos_to_sync: &[RepoSummary],
    command: &SyncReposCommand,
) -> Result<()> {
    let source_path = std::path::Path::new(&command.source);
    if !source_path.exists() {
        anyhow::bail!("sync source directory '{}' does not exist", command.source);
    }

    let files = collect_sync_files(source_path)?;
    if files.is_empty() {
        ui::warn(&format!("No files found in '{}'.", command.source));
        return Ok(());
    }

    ui::info(&format!(
        "Syncing {} file(s) to {} repositories...",
        files.len(),
        repos_to_sync.len()
    ));

    let commit_msg = format!(
        "chore: sync files from devopster (source: {})",
        command.source
    );
    let mut sync_count = 0usize;
    let mut error_count = 0usize;

    for repo in repos_to_sync {
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
                    ui::item(&format!("synced '{}' -> '{}'", relative_path, repo.name));
                }
                Err(err) => {
                    error_count += 1;
                    ui::error(&format!(
                        "error syncing '{}' to '{}': {err:#}",
                        relative_path, repo.name
                    ));
                }
            }
        }
    }

    ui::success(&format!(
        "Sync complete: {sync_count} file(s) synced, {error_count} error(s)."
    ));
    Ok(())
}

async fn sync_blueprint_requirements(
    provider: &dyn crate::provider::Provider,
    config: &AppConfig,
    repos_to_sync: &[RepoSummary],
    blueprint: &ResolvedBlueprintSource,
) -> Result<()> {
    let (owner, repo) = parse_repo_slug(&blueprint.repo)?;
    let blueprint_files = provider
        .fetch_repository_files(&owner, &repo, &blueprint.branch, &blueprint.paths)
        .await
        .with_context(|| {
            format!(
                "failed to read blueprint files from '{}' on branch '{}'",
                blueprint.repo, blueprint.branch
            )
        })?;

    let blueprint_workflows: std::collections::BTreeMap<String, Vec<u8>> = blueprint_files
        .into_iter()
        .filter(|(path, _)| path.starts_with(".github/workflows/"))
        .collect();

    if blueprint_workflows.is_empty() {
        ui::warn(&format!(
            "No workflow files found in blueprint repo '{}' under the configured paths.",
            blueprint.repo
        ));
        return Ok(());
    }

    ui::info(&format!(
        "Checking {} required workflow file(s) and org README markers across {} repositories...",
        blueprint_workflows.len(),
        repos_to_sync.len()
    ));

    let inspect_paths = vec![".github/workflows".to_string(), "README.md".to_string()];
    let mut sync_count = 0usize;
    let mut skipped_count = 0usize;
    let mut error_count = 0usize;

    for repo in repos_to_sync {
        let target_branch = repo
            .default_branch
            .as_deref()
            .unwrap_or(config.default_branch.as_str());
        let existing_files = provider
            .fetch_repository_files(
                &config.organization,
                &repo.name,
                target_branch,
                &inspect_paths,
            )
            .await
            .with_context(|| {
                format!(
                    "failed to inspect workflow files and README in '{}'",
                    repo.name
                )
            })?;
        let existing_files: std::collections::BTreeMap<String, Vec<u8>> =
            existing_files.into_iter().collect();
        let existing_workflows: std::collections::BTreeSet<String> = existing_files
            .keys()
            .filter(|path| path.starts_with(".github/workflows/"))
            .cloned()
            .collect();
        let missing_workflows: Vec<_> = blueprint_workflows
            .iter()
            .filter(|(path, _)| !existing_workflows.contains(*path))
            .collect();
        let current_readme = existing_files
            .get("README.md")
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        let missing_readme = detect_missing_readme_parts(current_readme.as_deref());

        ui::section(&format!("Repository: {}", repo.name));

        if missing_workflows.is_empty() && !missing_readme.any() {
            ui::success("Required blueprint workflows and README markers are present.");
            continue;
        }

        if !missing_workflows.is_empty() {
            let workflow_names = missing_workflows
                .iter()
                .map(|(path, _)| path.trim_start_matches(".github/workflows/"))
                .collect::<Vec<_>>()
                .join(", ");
            ui::warn(&format!("Missing workflow files: {workflow_names}"));

            if prompt_confirm(
                "  Add missing workflow file(s) from the blueprint repo?",
                true,
            )? {
                for (relative_path, file_content) in &missing_workflows {
                    match provider
                        .push_file(
                            &config.organization,
                            &repo.name,
                            relative_path,
                            file_content,
                            "chore: add missing workflow from org blueprint",
                        )
                        .await
                    {
                        Ok(()) => {
                            sync_count += 1;
                            ui::item(&format!("added '{}'", relative_path));
                        }
                        Err(err) => {
                            error_count += 1;
                            ui::error(&format!(
                                "error syncing '{}' to '{}': {err:#}",
                                relative_path, repo.name
                            ));
                        }
                    }
                }
            } else {
                skipped_count += missing_workflows.len();
                ui::warn("Skipped workflow updates.");
            }
        }

        if missing_readme.any() {
            ui::warn(&format!(
                "README missing: {}",
                missing_readme.labels().join(", ")
            ));

            if prompt_confirm(
                "  Update README with the org header and badge markers?",
                true,
            )? {
                let repair_values =
                    resolve_readme_repair_values(current_readme.as_deref(), missing_readme)?;
                let updated_readme = apply_org_readme_standard(
                    &repo.name,
                    current_readme.as_deref(),
                    missing_readme,
                    &repair_values,
                );
                match provider
                    .push_file(
                        &config.organization,
                        &repo.name,
                        "README.md",
                        updated_readme.as_bytes(),
                        "docs: add org README standard markers",
                    )
                    .await
                {
                    Ok(()) => {
                        sync_count += 1;
                        ui::item("updated 'README.md'");
                    }
                    Err(err) => {
                        error_count += 1;
                        ui::error(&format!(
                            "error updating 'README.md' in '{}': {err:#}",
                            repo.name
                        ));
                    }
                }
            } else {
                skipped_count += 1;
                ui::warn("Skipped README update.");
            }
        }
    }

    ui::success(&format!(
        "Blueprint sync complete: {sync_count} file(s) synced, {skipped_count} item(s) skipped, {error_count} error(s)."
    ));
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct MissingReadmeParts {
    location: bool,
    github_badge: bool,
    profile: bool,
    last_updated: bool,
    separator: bool,
    badge_block: bool,
}

impl MissingReadmeParts {
    fn any(self) -> bool {
        self.location
            || self.github_badge
            || self.profile
            || self.last_updated
            || self.separator
            || self.badge_block
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.location {
            labels.push("location line");
        }
        if self.github_badge {
            labels.push("GitHub badge line");
        }
        if self.profile {
            labels.push("brown9804 profile line");
        }
        if self.last_updated {
            labels.push("Last updated line");
        }
        if self.separator {
            labels.push("header separator");
        }
        if self.badge_block {
            labels.push("visitor badge block");
        }
        labels
    }
}

#[derive(Debug, Clone)]
struct ReadmeRepairValues {
    location_line: String,
    last_updated_line: String,
    badge_block: String,
}

fn detect_missing_readme_parts(readme: Option<&str>) -> MissingReadmeParts {
    let text = readme.unwrap_or("");

    MissingReadmeParts {
        location: !text.lines().any(is_org_location_line),
        github_badge: !text.contains(ORG_GITHUB_BADGE_LINE),
        profile: !text.contains(ORG_PROFILE_LINE),
        last_updated: !text.lines().any(is_last_updated_line),
        separator: !text.lines().any(|line| line.trim() == ORG_SEPARATOR_LINE),
        badge_block: !has_complete_badge_block(text),
    }
}

fn resolve_readme_repair_values(
    readme: Option<&str>,
    missing: MissingReadmeParts,
) -> Result<ReadmeRepairValues> {
    let existing_location =
        find_location_line(readme).unwrap_or_else(|| ORG_DEFAULT_LOCATION.to_string());
    let location_line = if missing.location {
        let input = prompt_line(&format!("  Location [default: {existing_location}]: "))?;
        if input.trim().is_empty() {
            existing_location
        } else {
            input.trim().to_string()
        }
    } else {
        existing_location
    };

    let last_updated_value = find_last_updated_value(readme).unwrap_or_else(today_iso_date);
    let total_views_badge = find_total_views_badge_url(readme)
        .unwrap_or_else(|| ORG_TOTAL_VIEWS_BADGE_FALLBACK.to_string());
    let refresh_date = find_refresh_date_value(readme).unwrap_or_else(today_iso_date);

    Ok(ReadmeRepairValues {
        location_line,
        last_updated_line: format!("Last updated: {last_updated_value}"),
        badge_block: build_badge_block(&total_views_badge, &refresh_date),
    })
}

fn apply_org_readme_standard(
    repo_name: &str,
    readme: Option<&str>,
    missing: MissingReadmeParts,
    values: &ReadmeRepairValues,
) -> String {
    let existing = readme.unwrap_or("").trim_end();
    let mut content = if existing.is_empty() {
        format!("# {repo_name}")
    } else {
        existing.to_string()
    };

    let header_additions = build_org_header_additions(missing, values);
    if !header_additions.is_empty() {
        content = insert_after_main_title(&content, &header_additions, repo_name);
    }

    if missing.badge_block {
        content = upsert_badge_block(&content, &values.badge_block);
    }

    if !content.ends_with('\n') {
        content.push('\n');
    }

    content
}

fn build_org_header_additions(missing: MissingReadmeParts, values: &ReadmeRepairValues) -> String {
    let mut sections = Vec::new();
    if missing.location {
        sections.push(values.location_line.clone());
    }
    if missing.github_badge {
        sections.push(ORG_GITHUB_BADGE_LINE.to_string());
    }
    if missing.profile {
        sections.push(ORG_PROFILE_LINE.to_string());
    }
    if missing.last_updated {
        sections.push(values.last_updated_line.clone());
    }
    if missing.separator {
        sections.push(ORG_SEPARATOR_LINE.to_string());
    }
    sections.join("\n\n")
}

fn upsert_badge_block(markdown: &str, badge_block: &str) -> String {
    let mut content = markdown.trim_end().to_string();
    let start = content.find(ORG_BADGE_START_MARKER);
    let end = content.find(ORG_BADGE_END_MARKER);

    match (start, end) {
        (Some(start_index), Some(end_index)) if end_index >= start_index => {
            let end_index = end_index + ORG_BADGE_END_MARKER.len();
            content.replace_range(start_index..end_index, badge_block);
            content
        }
        (Some(start_index), None) => {
            content.truncate(start_index);
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            if !content.ends_with("\n\n") && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(badge_block);
            content
        }
        _ => {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            if !content.ends_with("\n\n") && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(badge_block);
            content
        }
    }
}

fn build_badge_block(total_views_badge_url: &str, refresh_date: &str) -> String {
    format!(
        "{ORG_BADGE_START_MARKER}\n<div align=\"center\">\n  <img src=\"{total_views_badge_url}\" alt=\"Total views\">\n  <p>Refresh Date: {refresh_date}</p>\n</div>\n{ORG_BADGE_END_MARKER}"
    )
}

fn has_complete_badge_block(markdown: &str) -> bool {
    let Some(block) = extract_badge_block(markdown) else {
        return false;
    };

    block.contains("img.shields.io/badge/Total%20views-") && block.lines().any(is_refresh_date_line)
}

fn extract_badge_block(markdown: &str) -> Option<&str> {
    let start = markdown.find(ORG_BADGE_START_MARKER)?;
    let end = markdown.find(ORG_BADGE_END_MARKER)? + ORG_BADGE_END_MARKER.len();
    (end >= start).then_some(&markdown[start..end])
}

fn find_location_line(readme: Option<&str>) -> Option<String> {
    readme?
        .lines()
        .find(|line| is_org_location_line(line))
        .map(|line| line.trim().to_string())
}

fn find_last_updated_value(readme: Option<&str>) -> Option<String> {
    readme?.lines().find_map(|line| {
        let value = line.trim().strip_prefix("Last updated: ")?;
        is_iso_date(value).then(|| value.to_string())
    })
}

fn find_refresh_date_value(readme: Option<&str>) -> Option<String> {
    readme?.lines().find_map(|line| {
        let value = line
            .trim()
            .strip_prefix("<p>Refresh Date: ")?
            .strip_suffix("</p>")?;
        is_iso_date(value).then(|| value.to_string())
    })
}

fn find_total_views_badge_url(readme: Option<&str>) -> Option<String> {
    readme?.lines().find_map(|line| {
        let start = line.find("https://img.shields.io/badge/Total%20views-")?;
        let rest = &line[start..];
        let end = rest
            .find('"')
            .or_else(|| rest.find(')'))
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    })
}

fn today_iso_date() -> String {
    let unix_days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 86_400)
        .unwrap_or(0);

    iso_date_from_unix_days(unix_days as i64)
}

fn iso_date_from_unix_days(unix_days: i64) -> String {
    let z = unix_days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_param = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_param + 2) / 5 + 1;
    let month = month_param + if month_param < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }

    format!("{year:04}-{month:02}-{day:02}")
}

fn insert_after_main_title(markdown: &str, additions: &str, repo_name: &str) -> String {
    let content = markdown.trim_end();
    if content.is_empty() {
        return format!("# {repo_name}\n\n{}", additions.trim());
    }

    let lines: Vec<&str> = content.lines().collect();
    if let Some(index) = lines
        .iter()
        .position(|line| line.trim_start().starts_with("# "))
    {
        let before = lines[..=index].join("\n");
        let after = lines[index + 1..].join("\n");
        if after.trim().is_empty() {
            format!("{}\n\n{}", before.trim_end(), additions.trim())
        } else {
            format!(
                "{}\n\n{}\n\n{}",
                before.trim_end(),
                additions.trim(),
                after.trim_start()
            )
        }
    } else {
        format!("# {repo_name}\n\n{}\n\n{}", additions.trim(), content)
    }
}

fn is_org_location_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with('>')
        || trimmed.starts_with('[')
        || trimmed.starts_with('<')
        || trimmed.starts_with("Last updated:")
        || trimmed == ORG_SEPARATOR_LINE
    {
        return false;
    }

    let parts: Vec<_> = trimmed.split(',').map(|part| part.trim()).collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '.' | '\'' | '&'))
        })
}

fn is_last_updated_line(line: &str) -> bool {
    let Some(value) = line.trim().strip_prefix("Last updated: ") else {
        return false;
    };

    is_iso_date(value)
}

fn is_refresh_date_line(line: &str) -> bool {
    let Some(value) = line.trim().strip_prefix("<p>Refresh Date: ") else {
        return false;
    };
    let Some(value) = value.strip_suffix("</p>") else {
        return false;
    };

    is_iso_date(value)
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

async fn fix_repos(provider: &dyn crate::provider::Provider, config: &AppConfig) -> Result<()> {
    if config.scoped_repos.is_empty() {
        let proceed = prompt_confirm(
            "scoped_repos is empty; this will change ALL repositories. Continue?",
            false,
        )?;
        if !proceed {
            ui::warn("Aborted.");
            return Ok(());
        }
    }

    let repos = provider.list_repositories(&config.organization).await?;
    let repos = scope_to_config(repos, &config.scoped_repos);

    if repos.is_empty() {
        ui::warn("No repositories matched the configured scope.");
        return Ok(());
    }

    let total_repos = repos.len();
    let repos: Vec<_> = repos
        .into_iter()
        .map(|repo| {
            let missing = missing_metadata(&repo, &config.audit);
            (repo, missing)
        })
        .filter(|(_, missing)| missing.any())
        .collect();

    if repos.is_empty() {
        ui::success(
            "No repositories in scope are missing description, topics, or license metadata.",
        );
        ui::info("Nothing to fix.");
        return Ok(());
    }

    let supports_metadata = matches!(repos[0].0.provider, "github" | "gitlab");
    let supports_push = supports_metadata;

    ui::info(&format!(
        "Fixing {} of {} repository(ies)...",
        repos.len(),
        total_repos
    ));

    for (repo, missing) in repos {
        let missing_description = missing.description;
        let missing_topics = missing.topics;
        let missing_license = missing.license;

        ui::section(&format!("Repository: {}", repo.name));
        if missing_description {
            ui::item("missing description");
        }
        if missing_topics {
            ui::item(&format!("missing topics (min {})", config.audit.min_topics));
        }
        if missing_license {
            ui::item("missing license");
        }

        if !supports_metadata {
            ui::warn("This provider does not support metadata updates yet.");
            continue;
        }

        let mut chosen_template: Option<&crate::config::TemplateConfig> = None;

        if missing_topics {
            if config.templates.is_empty() {
                ui::warn("No templates configured; enter topics manually.");
            }

            let topics = select_topics_input(config, &mut chosen_template)?;
            if !topics.is_empty() {
                let merged = merge_topics(&repo.topics, &topics);
                if merged.len() < config.audit.min_topics {
                    ui::warn(&format!(
                        "Skipping topics: only {} topic(s) provided (min {}).",
                        merged.len(),
                        config.audit.min_topics
                    ));
                } else if let Err(err) = provider
                    .align_topics(&config.organization, &repo.name, &merged)
                    .await
                {
                    ui::error(&format!("Failed to update topics: {err:#}"));
                } else {
                    ui::success(&format!("Topics updated: {}", merged.join(", ")));
                }
            }
        }

        if missing_description {
            let mut default_desc = chosen_template.map(|t| t.description.clone());
            if default_desc.is_none() {
                if let Ok(Some(line)) = provider
                    .readme_first_line(&config.organization, &repo.name)
                    .await
                {
                    default_desc = Some(line);
                }
            }

            let prompt = if let Some(default) = &default_desc {
                format!(
                    "  Description [default: {}] (blank to keep missing): ",
                    default
                )
            } else {
                "  Description (blank to skip): ".to_string()
            };

            let input = prompt_line(&prompt)?;
            let new_desc = if input.trim().is_empty() {
                default_desc
            } else {
                Some(input)
            };

            if let Some(desc) = new_desc {
                if let Err(err) = provider
                    .update_description(&config.organization, &repo.name, &desc)
                    .await
                {
                    ui::error(&format!("Failed to update description: {err:#}"));
                } else {
                    ui::success("Description updated.");
                }
            }
        }

        if missing_license {
            if !supports_push {
                ui::warn("This provider does not support writing LICENSE files yet.");
                continue;
            }

            let input = select_license_input()?;
            if input.trim().is_empty() {
                continue;
            }

            let license_text = if let Some(text) = license_text_for(&input) {
                text.to_string()
            } else if std::path::Path::new(input.trim()).exists() {
                std::fs::read_to_string(input.trim())
                    .with_context(|| format!("failed to read license file '{}'", input.trim()))?
            } else {
                ui::warn(&format!("Unknown license '{input}'. Skipping."));
                continue;
            };

            if let Err(err) = provider
                .push_file(
                    &config.organization,
                    &repo.name,
                    "LICENSE",
                    license_text.as_bytes(),
                    "chore: add LICENSE",
                )
                .await
            {
                ui::error(&format!("Failed to write LICENSE: {err:#}"));
            } else {
                ui::success("LICENSE committed.");
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct MissingMetadata {
    description: bool,
    topics: bool,
    license: bool,
}

impl MissingMetadata {
    fn any(self) -> bool {
        self.description || self.topics || self.license
    }
}

fn missing_metadata(repo: &RepoSummary, audit: &AuditConfig) -> MissingMetadata {
    MissingMetadata {
        description: audit.require_description && repo.description.trim().is_empty(),
        topics: audit.require_topics && repo.topics.len() < audit.min_topics,
        license: audit.require_license && is_license_missing(repo.license.as_deref()),
    }
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
        repos
            .into_iter()
            .filter(|r| scoped.contains(&r.name))
            .collect()
    }
}

fn print_repos(repos: Vec<RepoSummary>) {
    if repos.is_empty() {
        ui::warn("No repositories matched the requested filter.");
        return;
    }

    let total = repos.len();
    let separator = "-".repeat(72);

    ui::header("Repositories");
    for repo in repos {
        println!("{separator}");
        println!("{:<55} [{}]", repo.name, repo.provider);

        // Description (wrapped to keep the block readable)
        if !repo.description.is_empty() {
            // Truncate long descriptions to keep output scannable
            let desc = if repo.description.chars().count() > 120 {
                format!(
                    "{}...",
                    repo.description.chars().take(120).collect::<String>()
                )
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
    }
    println!("{separator}");
    ui::info(&format!("{total} repositories"));
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
        let entry =
            entry.with_context(|| format!("failed to read entry in '{}'", dir.display()))?;
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

fn prompt_line(prompt: &str) -> Result<String> {
    Ok(ui::prompt_input(prompt)?.trim().to_string())
}

fn prompt_confirm(prompt: &str, default_yes: bool) -> Result<bool> {
    ui::prompt_confirm(prompt, default_yes)
}

fn parse_topics_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn select_topics_input<'a>(
    config: &'a AppConfig,
    chosen_template: &mut Option<&'a crate::config::TemplateConfig>,
) -> Result<Vec<String>> {
    if config.templates.is_empty() {
        return Ok(parse_topics_list(&prompt_line(
            "Topics (comma list; blank to skip)",
        )?));
    }

    let mut options = config
        .templates
        .iter()
        .map(|template| {
            format!(
                "Use template: {} ({})",
                template.name,
                template.topics.join(", ")
            )
        })
        .collect::<Vec<_>>();
    options.push("Enter topics manually".to_string());
    options.push("Skip topics update".to_string());

    let choice = ui::select("How would you like to fill topics?", &options, 0)?;
    if choice < config.templates.len() {
        let template = &config.templates[choice];
        *chosen_template = Some(template);
        return Ok(template.topics.clone());
    }
    if choice == config.templates.len() {
        return Ok(parse_topics_list(&prompt_line(
            "Topics (comma list; blank to skip)",
        )?));
    }

    Ok(Vec::new())
}

fn select_license_input() -> Result<String> {
    let options = vec![
        "MIT".to_string(),
        "Apache-2.0".to_string(),
        "BSD-3-Clause".to_string(),
        "GPL-3.0".to_string(),
        "Use a custom license file path".to_string(),
        "Skip license update".to_string(),
    ];
    match ui::select("Choose a license", &options, 0)? {
        0 => Ok("mit".to_string()),
        1 => Ok("apache-2.0".to_string()),
        2 => Ok("bsd-3-clause".to_string()),
        3 => Ok("gpl-3.0".to_string()),
        4 => prompt_line("License file path"),
        _ => Ok(String::new()),
    }
}

fn merge_topics(existing: &[String], extra: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for topic in existing.iter().chain(extra.iter()) {
        let key = topic.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            merged.push(topic.trim().to_string());
        }
    }

    merged
}

fn is_license_missing(license: Option<&str>) -> bool {
    match license {
        None => true,
        Some(value) => {
            let trimmed = value.trim();
            trimmed.is_empty() || trimmed == "NOASSERTION"
        }
    }
}

fn license_text_for(input: &str) -> Option<&'static str> {
    match input.trim().to_ascii_lowercase().as_str() {
        "mit" => Some(MIT_LICENSE),
        "apache-2.0" | "apache" => Some(APACHE_2_0_LICENSE),
        "bsd-3-clause" | "bsd-3" => Some(BSD_3_CLAUSE_LICENSE),
        "gpl-3.0" | "gpl-3.0-only" | "gplv3" => Some(GPL_3_0_LICENSE),
        _ => None,
    }
}

const MIT_LICENSE: &str = "MIT License\n\nCopyright (c) YEAR\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"), to deal\nin the Software without restriction, including without limitation the rights\nto use, copy, modify, merge, publish, distribute, sublicense, and/or sell\ncopies of the Software, and to permit persons to whom the Software is\nfurnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\nOUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\nSOFTWARE.\n";

const APACHE_2_0_LICENSE: &str = "Apache License\nVersion 2.0, January 2004\nhttp://www.apache.org/licenses/\n\nTERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n\n1. Definitions.\n\n\"License\" shall mean the terms and conditions for use, reproduction, and\ndistribution as defined by Sections 1 through 9 of this document.\n\n\"Licensor\" shall mean the copyright owner or entity authorized by\nthe copyright owner that is granting the License.\n\n\"Legal Entity\" shall mean the union of the acting entity and all\nother entities that control, are controlled by, or are under common\ncontrol with that entity. For the purposes of this definition,\n\"control\" means (i) the power, direct or indirect, to cause the\ndirection or management of such entity, whether by contract or\notherwise, or (ii) ownership of fifty percent (50%) or more of the\noutstanding shares, or (iii) beneficial ownership of such entity.\n\n\"You\" (or \"Your\") shall mean an individual or Legal Entity\nexercising permissions granted by this License.\n\n\"Source\" form shall mean the preferred form for making modifications,\nincluding but not limited to software source code, documentation\nsource, and configuration files.\n\n\"Object\" form shall mean any form resulting from mechanical\ntransformation or translation of a Source form, including but\nnot limited to compiled object code, generated documentation,\nand conversions to other media types.\n\n\"Work\" shall mean the work of authorship, whether in Source or\nObject form, made available under the License, as indicated by a\ncopyright notice that is included in or attached to the work\n(an example is provided in the Appendix below).\n\n\"Derivative Works\" shall mean any work, whether in Source or Object\nform, that is based on (or derived from) the Work and for which the\neditorial revisions, annotations, elaborations, or other modifications\nrepresent, as a whole, an original work of authorship. For the purposes\nof this License, Derivative Works shall not include works that remain\nseparable from, or merely link (or bind by name) to the interfaces of,\nthe Work and Derivative Works thereof.\n\n\"Contribution\" shall mean any work of authorship, including\nthe original version of the Work and any modifications or additions\nto that Work or Derivative Works thereof, that is intentionally\nsubmitted to Licensor for inclusion in the Work by the copyright owner\nor by an individual or Legal Entity authorized to submit on behalf of\nthe copyright owner. For the purposes of this definition, \"submitted\"\nmeans any form of electronic, verbal, or written communication sent\nto the Licensor or its representatives, including but not limited to\ncommunication on electronic mailing lists, source code control systems,\nand issue tracking systems that are managed by, or on behalf of, the\nLicensor for the purpose of discussing and improving the Work, but\nexcluding communication that is conspicuously marked or otherwise\ndesignated in writing by the copyright owner as \"Not a Contribution.\"\n\n\"Contributor\" shall mean Licensor and any individual or Legal Entity\non behalf of whom a Contribution has been received by Licensor and\nsubsequently incorporated within the Work.\n\n2. Grant of Copyright License. Subject to the terms and conditions of\nthis License, each Contributor hereby grants to You a perpetual,\nworldwide, non-exclusive, no-charge, royalty-free, irrevocable\ncopyright license to reproduce, prepare Derivative Works of,\npublicly display, publicly perform, sublicense, and distribute the\nWork and such Derivative Works in Source or Object form.\n\n3. Grant of Patent License. Subject to the terms and conditions of\nthis License, each Contributor hereby grants to You a perpetual,\nworldwide, non-exclusive, no-charge, royalty-free, irrevocable\n(except as stated in this section) patent license to make, have made,\nuse, offer to sell, sell, import, and otherwise transfer the Work,\nwhere such license applies only to those patent claims licensable\nby such Contributor that are necessarily infringed by their\nContribution(s) alone or by combination of their Contribution(s)\nwith the Work to which such Contribution(s) was submitted. If You\ninstitute patent litigation against any entity (including a\ncross-claim or counterclaim in a lawsuit) alleging that the Work\nor a Contribution incorporated within the Work constitutes direct\nor contributory patent infringement, then any patent licenses\ngranted to You under this License for that Work shall terminate\nas of the date such litigation is filed.\n\n4. Redistribution. You may reproduce and distribute copies of the\nWork or Derivative Works thereof in any medium, with or without\nmodifications, and in Source or Object form, provided that You\nmeet the following conditions:\n\n(a) You must give any other recipients of the Work or\n    Derivative Works a copy of this License; and\n\n(b) You must cause any modified files to carry prominent notices\n    stating that You changed the files; and\n\n(c) You must retain, in the Source form of any Derivative Works\n    that You distribute, all copyright, patent, trademark, and\n    attribution notices from the Source form of the Work,\n    excluding those notices that do not pertain to any part of\n    the Derivative Works; and\n\n(d) If the Work includes a \"NOTICE\" text file as part of its\n    distribution, then any Derivative Works that You distribute must\n    include a readable copy of the attribution notices contained\n    within such NOTICE file, excluding those notices that do not\n    pertain to any part of the Derivative Works, in at least one\n    of the following places: within a NOTICE text file distributed\n    as part of the Derivative Works; within the Source form or\n    documentation, if provided along with the Derivative Works; or,\n    within a display generated by the Derivative Works, if and\n    wherever such third-party notices normally appear. The contents\n    of the NOTICE file are for informational purposes only and\n    do not modify the License. You may add Your own attribution\n    notices within Derivative Works that You distribute, alongside\n    or as an addendum to the NOTICE text from the Work, provided\n    that such additional attribution notices cannot be construed\n    as modifying the License.\n\nYou may add Your own copyright statement to Your modifications and\nmay provide additional or different license terms and conditions\nfor use, reproduction, or distribution of Your modifications, or\nfor any such Derivative Works as a whole, provided Your use,\nreproduction, and distribution of the Work otherwise complies with\nthe conditions stated in this License.\n\n5. Submission of Contributions. Unless You explicitly state otherwise,\nany Contribution intentionally submitted for inclusion in the Work\nby You to the Licensor shall be under the terms and conditions of\nthis License, without any additional terms or conditions. Notwithstanding\nthe above, nothing herein shall supersede or modify the terms of any\nseparate license agreement you may have executed with Licensor regarding\nsuch Contributions.\n\n6. Trademarks. This License does not grant permission to use the trade\nnames, trademarks, service marks, or product names of the Licensor,\nexcept as required for reasonable and customary use in describing the\norigin of the Work and reproducing the content of the NOTICE file.\n\n7. Disclaimer of Warranty. Unless required by applicable law or\nagreed to in writing, Licensor provides the Work (and each\nContributor provides its Contributions) on an \"AS IS\" BASIS,\nWITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or\nimplied, including, without limitation, any warranties or conditions\nof TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A\nPARTICULAR PURPOSE. You are solely responsible for determining the\nappropriateness of using or redistributing the Work and assume any\nrisks associated with Your exercise of permissions under this License.\n\n8. Limitation of Liability. In no event and under no legal theory,\nwhether in tort (including negligence), contract, or otherwise,\nunless required by applicable law (such as deliberate and grossly\nnegligent acts) or agreed to in writing, shall any Contributor be\nliable to You for damages, including any direct, indirect, special,\nincidental, or consequential damages of any character arising as a\nresult of this License or out of the use or inability to use the\nWork (including but not limited to damages for loss of goodwill,\nwork stoppage, computer failure or malfunction, or any and all\nother commercial damages or losses), even if such Contributor\nhas been advised of the possibility of such damages.\n\n9. Accepting Warranty or Additional Liability. While redistributing\nthe Work or Derivative Works thereof, You may choose to offer,\nand charge a fee for, acceptance of support, warranty, indemnity,\nor other liability obligations and/or rights consistent with this\nLicense. However, in accepting such obligations, You may act only\non Your own behalf and on Your sole responsibility, not on behalf\nof any other Contributor, and only if You agree to indemnify,\ndefend, and hold each Contributor harmless for any liability\nincurred by, or claims asserted against, such Contributor by reason\nof your accepting any such warranty or additional liability.\n\nEND OF TERMS AND CONDITIONS\n";

const BSD_3_CLAUSE_LICENSE: &str = "BSD 3-Clause License\n\nCopyright (c) YEAR\nAll rights reserved.\n\nRedistribution and use in source and binary forms, with or without\nmodification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice, this\n   list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must reproduce the above copyright notice,\n   this list of conditions and the following disclaimer in the documentation\n   and/or other materials provided with the distribution.\n\n3. Neither the name of the copyright holder nor the names of its\n   contributors may be used to endorse or promote products derived from\n   this software without specific prior written permission.\n\nTHIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\"\nAND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE\nIMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE\nDISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE\nFOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL\nDAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR\nSERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER\nCAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,\nOR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE\nOF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.\n";

const GPL_3_0_LICENSE: &str = "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n\nThis program is free software: you can redistribute it and/or modify\nit under the terms of the GNU General Public License as published by\nthe Free Software Foundation, either version 3 of the License, or\n(at your option) any later version.\n\nThis program is distributed in the hope that it will be useful,\nbut WITHOUT ANY WARRANTY; without even the implied warranty of\nMERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\nGNU General Public License for more details.\n\nYou should have received a copy of the GNU General Public License\nalong with this program.  If not, see <https://www.gnu.org/licenses/>.\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo() -> RepoSummary {
        RepoSummary {
            name: "sample".to_string(),
            full_name: None,
            description: String::new(),
            topics: Vec::new(),
            license: None,
            default_branch: Some("main".to_string()),
            web_url: None,
            provider: "github",
            language: None,
            archived: false,
            is_private: false,
            stargazers_count: None,
            forks_count: None,
            updated_at: None,
        }
    }

    #[test]
    fn missing_metadata_respects_enabled_audit_checks() {
        let audit = AuditConfig {
            require_description: true,
            require_topics: true,
            min_topics: 2,
            require_license: true,
            require_default_branch: true,
        };

        let missing = missing_metadata(&sample_repo(), &audit);

        assert!(missing.description);
        assert!(missing.topics);
        assert!(missing.license);
        assert!(missing.any());
    }

    #[test]
    fn missing_metadata_ignores_disabled_checks() {
        let audit = AuditConfig {
            require_description: false,
            require_topics: false,
            min_topics: 3,
            require_license: false,
            require_default_branch: true,
        };

        let missing = missing_metadata(&sample_repo(), &audit);

        assert!(!missing.description);
        assert!(!missing.topics);
        assert!(!missing.license);
        assert!(!missing.any());
    }

    #[test]
    fn detect_missing_readme_parts_accepts_org_standard_lines() {
        let readme = format!(
            "# demo\n\nAtlanta, USA\n\n{ORG_GITHUB_BADGE_LINE}\n{ORG_PROFILE_LINE}\n\nLast updated: 2026-04-02\n\n{ORG_SEPARATOR_LINE}\n\nBody\n\n{}\n",
            build_badge_block(
                "https://img.shields.io/badge/Total%20views-1580-limegreen",
                "2026-04-02"
            )
        );

        let missing = detect_missing_readme_parts(Some(&readme));

        assert!(!missing.any());
    }

    #[test]
    fn apply_org_readme_standard_inserts_missing_markers() {
        let original = "# demo\n\n## About\nHello\n";
        let values = ReadmeRepairValues {
            location_line: "Atlanta, USA".to_string(),
            last_updated_line: "Last updated: 2026-04-02".to_string(),
            badge_block: build_badge_block(
                "https://img.shields.io/badge/Total%20views-42-limegreen",
                "2026-04-02",
            ),
        };
        let updated = apply_org_readme_standard(
            "demo",
            Some(original),
            detect_missing_readme_parts(Some(original)),
            &values,
        );

        assert!(updated.contains("Atlanta, USA"));
        assert!(updated.contains(ORG_GITHUB_BADGE_LINE));
        assert!(updated.contains(ORG_PROFILE_LINE));
        assert!(updated.contains("Last updated: 2026-04-02"));
        assert!(updated.contains(ORG_SEPARATOR_LINE));
        assert!(updated.contains(ORG_BADGE_START_MARKER));
        assert!(updated.contains("## About\nHello"));
    }

    #[test]
    fn detect_missing_readme_parts_flags_incomplete_badge_block() {
        let readme = format!(
            "# demo\n\nAtlanta, USA\n\n{ORG_GITHUB_BADGE_LINE}\n{ORG_PROFILE_LINE}\n\nLast updated: 2026-04-02\n\n{ORG_SEPARATOR_LINE}\n\n{ORG_BADGE_START_MARKER}\n<div align=\"center\">\n</div>\n{ORG_BADGE_END_MARKER}\n"
        );

        let missing = detect_missing_readme_parts(Some(&readme));

        assert!(missing.badge_block);
    }

    #[test]
    fn upsert_badge_block_replaces_existing_fragment() {
        let original =
            format!("# demo\n\nBody\n\n{ORG_BADGE_START_MARKER}\nold\n{ORG_BADGE_END_MARKER}\n");
        let updated = upsert_badge_block(
            &original,
            &build_badge_block(
                "https://img.shields.io/badge/Total%20views-42-limegreen",
                "2026-04-02",
            ),
        );

        assert!(!updated.contains("\nold\n"));
        assert!(updated.contains("Total%20views-42-limegreen"));
        assert!(updated.contains("Refresh Date: 2026-04-02"));
    }

    #[test]
    fn iso_date_from_unix_days_formats_epoch() {
        assert_eq!(iso_date_from_unix_days(0), "1970-01-01");
        assert_eq!(iso_date_from_unix_days(19_815), "2024-04-02");
    }
}
