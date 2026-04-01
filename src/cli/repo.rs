use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::io::{self, Write};

use crate::config::AppConfig;
use crate::provider::{
    AuditFinding, AuditPolicy, BlueprintRequest, ProviderFactory, RepoSummary, RepoVisibility,
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

    /// Sync files from the blueprint source repository instead of local files.
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

    #[arg(long, help = "Only sync to repositories matching this template (by topic overlap)")]
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
            RepoAction::Fix(_) => {
                fix_repos(provider.as_ref(), &config).await?;
            }
            RepoAction::Sync(command) => {
                let use_blueprint = command.from_blueprint
                    || command.blueprint_repo.is_some()
                    || !command.blueprint_path.is_empty();

                let files = if use_blueprint {
                    let blueprint = resolve_blueprint_source(config, command)?;
                    let (owner, repo) = parse_repo_slug(&blueprint.repo)?;
                    let files = provider
                        .fetch_repository_files(
                            &owner,
                            &repo,
                            &blueprint.branch,
                            &blueprint.paths,
                        )
                        .await?;
                    if files.is_empty() {
                        println!("No files found in blueprint repo '{}'.", blueprint.repo);
                        return Ok(());
                    }
                    files
                } else {
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
                    files
                };

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

                let commit_msg = if use_blueprint {
                    let repo_name = command
                        .blueprint_repo
                        .as_deref()
                        .or_else(|| config.blueprint.as_ref().map(|b| b.repo.as_str()))
                        .unwrap_or("blueprint");
                    format!(
                        "chore: sync files from blueprint repo ({repo_name})"
                    )
                } else {
                    format!(
                        "chore: sync files from devopster (source: {})",
                        command.source
                    )
                };
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
            RepoAction::Blueprint(command) => {
                let template = config
                    .templates
                    .iter()
                    .find(|template| template.name == command.template)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("template '{}' was not found in the config", command.template)
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

    Ok(ResolvedBlueprintSource { repo, branch, paths })
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

async fn fix_repos(provider: &dyn crate::provider::Provider, config: &AppConfig) -> Result<()> {
    if config.scoped_repos.is_empty() {
        let proceed = prompt_confirm(
            "scoped_repos is empty; this will change ALL repositories. Continue?",
            false,
        )?;
        if !proceed {
            println!("Aborted.");
            return Ok(());
        }
    }

    let repos = provider.list_repositories(&config.organization).await?;
    let repos = scope_to_config(repos, &config.scoped_repos);

    if repos.is_empty() {
        println!("No repositories matched the configured scope.");
        return Ok(());
    }

    let supports_metadata = matches!(repos[0].provider, "github" | "gitlab");
    let supports_push = supports_metadata;

    println!("\nFixing {} repository(ies)...", repos.len());

    for repo in repos {
        let missing_description = config.audit.require_description
            && repo.description.trim().is_empty();
        let missing_topics = config.audit.require_topics
            && repo.topics.len() < config.audit.min_topics;
        let missing_license = config.audit.require_license
            && is_license_missing(repo.license.as_deref());

        if !(missing_description || missing_topics || missing_license) {
            continue;
        }

        println!("\nRepository: {}", repo.name);
        if missing_description {
            println!("  - missing description");
        }
        if missing_topics {
            println!("  - missing topics (min {})", config.audit.min_topics);
        }
        if missing_license {
            println!("  - missing license");
        }

        if !supports_metadata {
            println!("  This provider does not support metadata updates yet.");
            continue;
        }

        let mut chosen_template: Option<&crate::config::TemplateConfig> = None;

        if missing_topics {
            if config.templates.is_empty() {
                println!("  No templates configured; enter topics manually.");
            } else {
                println!("  Templates:");
                for (idx, template) in config.templates.iter().enumerate() {
                    println!(
                        "    {}. {} ({})",
                        idx + 1,
                        template.name,
                        template.topics.join(", ")
                    );
                }
            }

            let input = prompt_line(
                "  Topics (template #/name or comma list; blank to skip): ",
            )?;
            if !input.trim().is_empty() {
                let topics = if let Some((template, topics)) =
                    resolve_template_or_topics(&config.templates, &input)
                {
                    chosen_template = Some(template);
                    topics
                } else {
                    parse_topics_list(&input)
                };

                if !topics.is_empty() {
                    let merged = merge_topics(&repo.topics, &topics);
                    if merged.len() < config.audit.min_topics {
                        println!(
                            "  Skipping topics: only {} topic(s) provided (min {}).",
                            merged.len(),
                            config.audit.min_topics
                        );
                    } else if let Err(err) = provider
                        .align_topics(&config.organization, &repo.name, &merged)
                        .await
                    {
                        eprintln!("  Failed to update topics: {err:#}");
                    } else {
                        println!("  Topics updated: {}", merged.join(", "));
                    }
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
                    eprintln!("  Failed to update description: {err:#}");
                } else {
                    println!("  Description updated.");
                }
            }
        }

        if missing_license {
            if !supports_push {
                println!("  This provider does not support writing LICENSE files yet.");
                continue;
            }

            let input = prompt_line(
                "  License (mit/apache-2.0/bsd-3-clause/gpl-3.0 or file path; blank to skip): ",
            )?;
            if input.trim().is_empty() {
                continue;
            }

            let license_text = if let Some(text) = license_text_for(&input) {
                text.to_string()
            } else if std::path::Path::new(input.trim()).exists() {
                std::fs::read_to_string(input.trim()).with_context(|| {
                    format!("failed to read license file '{}'", input.trim())
                })?
            } else {
                println!("  Unknown license '{input}'. Skipping.");
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
                eprintln!("  Failed to write LICENSE: {err:#}");
            } else {
                println!("  LICENSE committed.");
            }
        }
    }

    Ok(())
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

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    Ok(input.trim().to_string())
}

fn prompt_confirm(prompt: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    let answer = prompt_line(&format!("{prompt} {hint}: "))?;
    if answer.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(answer.to_lowercase().as_str(), "y" | "yes"))
}

fn parse_topics_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

fn resolve_template_or_topics<'a>(
    templates: &'a [crate::config::TemplateConfig],
    input: &str,
) -> Option<(&'a crate::config::TemplateConfig, Vec<String>)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(index) = trimmed.parse::<usize>() {
        if index > 0 && index <= templates.len() {
            let template = &templates[index - 1];
            return Some((template, template.topics.clone()));
        }
    }

    if let Some(template) = templates.iter().find(|t| t.name == trimmed) {
        return Some((template, template.topics.clone()));
    }

    None
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
