#![allow(dead_code)]

use std::fs;

use anyhow::{Context, Result};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub provider: ProviderKind,
    pub organization: String,
    #[serde(default = "default_branch")]
    pub default_branch: String,
    pub github: Option<GitHubConfig>,
    pub azure_devops: Option<AzureDevOpsConfig>,
    pub gitlab: Option<GitLabConfig>,
    pub catalog: CatalogConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub templates: Vec<TemplateConfig>,
    /// When non-empty, devopster operations target only these repositories.
    /// An empty list means all repositories in the organization.
    #[serde(default)]
    pub scoped_repos: Vec<String>,
    /// Enable GitHub Copilot-assisted suggestions (requires a Copilot subscription).
    #[serde(default)]
    pub copilot_enabled: bool,
}

/// Policy rules applied by `devopster repo audit`.
/// All checks default to enabled. Override in devopster-config.yaml.
#[derive(Debug, Clone, Deserialize)]
pub struct AuditConfig {
    /// Fail if the repository description is empty.
    #[serde(default = "default_true")]
    pub require_description: bool,
    /// Fail if the repository has no topics.
    #[serde(default = "default_true")]
    pub require_topics: bool,
    /// Minimum number of topics required (only checked when require_topics is true).
    #[serde(default = "default_min_topics")]
    pub min_topics: usize,
    /// Fail if no license is detected.
    #[serde(default = "default_true")]
    pub require_license: bool,
    /// Fail if the default branch does not match the top-level `default_branch` setting.
    #[serde(default = "default_true")]
    pub require_default_branch: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            require_description: true,
            require_topics: true,
            min_topics: 1,
            require_license: true,
            require_default_branch: true,
        }
    }
}

fn default_true() -> bool { true }
fn default_min_topics() -> usize { 1 }

#[derive(Debug, Clone, Deserialize)]
pub enum ProviderKind {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "azure_devops")]
    AzureDevOps,
    #[serde(rename = "gitlab")]
    GitLab,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::AzureDevOps => "azure_devops",
            Self::GitLab => "gitlab",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubConfig {
    pub api_url: Url,
    pub token_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzureDevOpsConfig {
    pub organization_url: Url,
    pub project: String,
    pub token_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitLabConfig {
    pub api_url: Url,
    pub token_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogConfig {
    pub output_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TemplateConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub topics: Vec<String>,
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file at {path}"))?;

        serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse YAML config at {path}"))
    }
}

fn default_branch() -> String {
    "main".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_yaml(provider_block: &str) -> String {
        format!(
            "organization: test-org\ncatalog:\n  output_path: out/catalog.json\n{provider_block}"
        )
    }

    #[test]
    fn parses_github_provider() {
        let yaml = base_yaml(
            "provider: github\ngithub:\n  api_url: https://api.github.com\n  token_env: GITHUB_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(config.provider, ProviderKind::GitHub));
        assert_eq!(config.organization, "test-org");
    }

    #[test]
    fn parses_azure_devops_provider() {
        let yaml = base_yaml(
            "provider: azure_devops\nazure_devops:\n  organization_url: https://dev.azure.com/myorg\n  project: my-project\n  token_env: AZDO_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(config.provider, ProviderKind::AzureDevOps));
    }

    #[test]
    fn parses_gitlab_provider() {
        let yaml = base_yaml(
            "provider: gitlab\ngitlab:\n  api_url: https://gitlab.com/api/v4\n  token_env: GITLAB_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(config.provider, ProviderKind::GitLab));
    }

    #[test]
    fn default_branch_is_main() {
        let yaml = base_yaml(
            "provider: github\ngithub:\n  api_url: https://api.github.com\n  token_env: GITHUB_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.default_branch, "main");
    }

    #[test]
    fn custom_default_branch_is_respected() {
        let yaml = base_yaml(
            "provider: github\ndefault_branch: trunk\ngithub:\n  api_url: https://api.github.com\n  token_env: GITHUB_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.default_branch, "trunk");
    }

    #[test]
    fn parses_templates() {
        let yaml = base_yaml(
            "provider: github\ngithub:\n  api_url: https://api.github.com\n  token_env: GITHUB_TOKEN\ntemplates:\n  - name: my-template\n    description: A test template.\n    topics:\n      - rust\n      - gitops\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(config.templates.len(), 1);
        assert_eq!(config.templates[0].name, "my-template");
        assert_eq!(config.templates[0].topics, vec!["rust", "gitops"]);
    }

    #[test]
    fn empty_templates_when_omitted() {
        let yaml = base_yaml(
            "provider: github\ngithub:\n  api_url: https://api.github.com\n  token_env: GITHUB_TOKEN\n",
        );
        let config: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(config.templates.is_empty());
    }
}
