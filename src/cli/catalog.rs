use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config::AppConfig;
use crate::provider::{ProviderFactory, RepoSummary};
use crate::ui;

#[derive(Debug, Args)]
pub struct CatalogCommand {
    #[command(subcommand)]
    pub action: CatalogAction,
}

#[derive(Debug, Subcommand)]
pub enum CatalogAction {
    /// Export a catalog.json of all repositories (compatible with org-catalog GitHub Pages)
    Generate(GenerateCatalogCommand),
}

#[derive(Debug, Args)]
pub struct GenerateCatalogCommand {}

/// Shape of each repo entry — compatible with the org-catalog catalog.json schema.
/// See: https://github.com/MicrosoftCloudEssentials-LearningHub/org-catalog
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    description: String,
    topics: Vec<String>,
    /// Top topics used as lightweight category hints (org-catalog computes this
    /// from TF-IDF over READMEs; we use topics as a reasonable stand-in).
    categories: Vec<String>,
    /// TF-IDF keywords from README content — requires per-repo API calls,
    /// so left empty here. The org-catalog GitHub Action fills these in.
    keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    archived: bool,
    private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stargazers_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    forks_count: Option<u64>,
    /// README hero image — not fetched here (requires per-repo API call).
    image_url: Option<String>,
}

/// Top-level catalog.json — matches the format consumed by org-catalog GitHub Pages site.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    generated_at: String,
    org: String,
    repos: Vec<CatalogEntry>,
}

impl From<RepoSummary> for CatalogEntry {
    fn from(r: RepoSummary) -> Self {
        let categories = r.topics.iter().take(6).cloned().collect();
        Self {
            name: r.name,
            full_name: r.full_name,
            url: r.web_url,
            description: r.description,
            topics: r.topics,
            categories,
            keywords: Vec::new(),
            language: r.language,
            updated_at: r.updated_at,
            archived: r.archived,
            private: r.is_private,
            stargazers_count: r.stargazers_count,
            forks_count: r.forks_count,
            image_url: None,
        }
    }
}

impl CatalogCommand {
    pub async fn run(&self, config_path: &str) -> Result<()> {
        let config = AppConfig::load(config_path)?;
        let provider = ProviderFactory::from_config(&config)?;

        match &self.action {
            CatalogAction::Generate(_) => {
                let repositories = provider.list_repositories(&config.organization).await?;
                let count = repositories.len();

                let catalog = Catalog {
                    generated_at: now_iso8601(),
                    org: config.organization.clone(),
                    repos: repositories.into_iter().map(CatalogEntry::from).collect(),
                };

                let json = serde_json::to_string_pretty(&catalog)
                    .map_err(|e| anyhow::anyhow!("failed to serialize catalog: {e}"))?;

                if let Some(parent) = std::path::Path::new(&config.catalog.output_path).parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            anyhow::anyhow!("failed to create output directory: {e}")
                        })?;
                    }
                }

                std::fs::write(&config.catalog.output_path, json).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to write catalog to '{}': {e}",
                        config.catalog.output_path
                    )
                })?;

                ui::success(&format!(
                    "Catalog written to '{}': {count} repositories.",
                    config.catalog.output_path
                ));
            }
        }

        Ok(())
    }
}

/// Format the current UTC time as ISO 8601 (e.g. "2026-04-01T12:34:56Z").
/// Uses the civil_from_days algorithm: https://www.howardhinnant.com/date_algorithms.html
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tod = secs % 86400;
    let h = tod / 3600;
    let m = (tod % 3600) / 60;
    let s = tod % 60;

    let days = secs / 86400;
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = y + if mon <= 2 { 1 } else { 0 };

    format!("{yr:04}-{mon:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}
