use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;

use crate::config::{AppConfig, ProviderKind, TemplateConfig};

pub mod azure_devops;
pub mod github;
pub mod gitlab;

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    pub name: String,
    pub full_name: Option<String>,
    pub description: String,
    pub topics: Vec<String>,
    pub license: Option<String>,
    pub default_branch: Option<String>,
    pub web_url: Option<String>,
    pub provider: &'static str,
    pub language: Option<String>,
    pub archived: bool,
    pub is_private: bool,
    pub stargazers_count: Option<u64>,
    pub forks_count: Option<u64>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuditPolicy {
    pub required_default_branch: String,
    pub require_description: bool,
    pub require_topics: bool,
    pub min_topics: usize,
    pub require_license: bool,
    pub require_default_branch: bool,
}

#[derive(Debug, Clone)]
pub struct AuditFinding {
    pub repository: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum RepoVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone)]
pub struct BlueprintRequest {
    pub name: String,
    pub description: String,
    pub visibility: RepoVisibility,
    pub template: TemplateConfig,
}

#[derive(Debug, Clone)]
pub struct BlueprintResult {
    pub name: String,
    pub provider: &'static str,
    pub web_url: Option<String>,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn list_repositories(&self, organization: &str) -> Result<Vec<RepoSummary>>;

    async fn audit_repositories(
        &self,
        organization: &str,
        policy: &AuditPolicy,
    ) -> Result<Vec<AuditFinding>>;

    async fn blueprint_repository(
        &self,
        organization: &str,
        request: &BlueprintRequest,
    ) -> Result<BlueprintResult>;

    /// Overwrite the full topic list for a single repository.
    /// Providers that do not support topics may use the default no-op.
    async fn align_topics(
        &self,
        _organization: &str,
        _repository: &str,
        _topics: &[String],
    ) -> Result<()> {
        Ok(())
    }

    /// Update a repository description.
    /// Providers that do not support metadata updates will return an error.
    async fn update_description(
        &self,
        _organization: &str,
        _repository: &str,
        _description: &str,
    ) -> Result<()> {
        anyhow::bail!("update_description is not supported by this provider")
    }

    /// Best-effort fetch of the first non-empty README line for suggestions.
    /// Providers that do not support README access may return Ok(None).
    async fn readme_first_line(
        &self,
        _organization: &str,
        _repository: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Fetch repository files from a source repo/branch for blueprint sync.
    /// Providers that do not support repo reads will return an error.
    async fn fetch_repository_files(
        &self,
        _owner: &str,
        _repository: &str,
        _branch: &str,
        _paths: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>> {
        anyhow::bail!("fetch_repository_files is not supported by this provider")
    }

    /// Create or update a single file in a repository.
    /// Providers that do not support file push will return an error.
    async fn push_file(
        &self,
        _organization: &str,
        _repository: &str,
        _path: &str,
        _content: &[u8],
        _commit_message: &str,
    ) -> Result<()> {
        anyhow::bail!("push_file is not supported by this provider")
    }
}

pub struct ProviderFactory;

impl ProviderFactory {
    pub fn from_config(config: &AppConfig) -> Result<Box<dyn Provider>> {
        match &config.provider {
            ProviderKind::GitHub => Ok(Box::new(github::GitHubProvider::from_config(config)?)),
            ProviderKind::AzureDevOps => {
                Ok(Box::new(azure_devops::AzureDevOpsProvider::from_config(config)?))
            }
            ProviderKind::GitLab => Ok(Box::new(gitlab::GitLabProvider::from_config(config)?)),
        }
    }
}
