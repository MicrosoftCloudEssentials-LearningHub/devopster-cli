use std::env;

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use reqwest::{header, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::config::{AppConfig, AzureDevOpsConfig};

use super::{
    AuditFinding, AuditPolicy, BlueprintRequest, BlueprintResult, Provider, RepoSummary,
    RepoVisibility,
};

pub struct AzureDevOpsProvider {
    api_url: Url,
    client: Client,
    project: String,
}

impl AzureDevOpsProvider {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let azure = config
            .azure_devops
            .as_ref()
            .context("azure_devops provider selected but azure_devops config is missing")?;

        Ok(Self {
            api_url: azure.organization_url.clone(),
            client: build_client(azure)?,
            project: azure.project.clone(),
        })
    }
}

#[async_trait]
impl Provider for AzureDevOpsProvider {
    async fn list_repositories(&self, organization: &str) -> Result<Vec<RepoSummary>> {
        let endpoint = self.repositories_endpoint(organization)?;
        let response = self.client.get(endpoint).send().await.with_context(|| {
            format!(
                "failed to call Azure DevOps repository API for project {}",
                self.project
            )
        });

        let response = response.map_err(|e| auth::annotate_auth_error(e, "azure_devops"))?;

        if response.status() == StatusCode::NOT_FOUND {
            anyhow::bail!(
                "Azure DevOps project '{}' was not found or is not visible",
                self.project
            )
        }

        let response = response.error_for_status().with_context(|| {
            format!(
                "Azure DevOps repository API returned an error for project {}",
                self.project
            )
        })?;

        let payload: AzureDevOpsRepositoryList = response.json().await.with_context(|| {
            format!(
                "failed to decode Azure DevOps repository response for {}",
                self.project
            )
        })?;

        Ok(payload.value.into_iter().map(RepoSummary::from).collect())
    }

    async fn audit_repositories(
        &self,
        organization: &str,
        policy: &AuditPolicy,
    ) -> Result<Vec<AuditFinding>> {
        let repositories = self.list_repositories(organization).await?;
        let mut findings = Vec::new();

        for repository in repositories {
            if policy.require_description && repository.description.trim().is_empty() {
                findings.push(AuditFinding {
                    repository: repository.name.clone(),
                    code: "missing-description",
                    message: "Repository description is empty.".to_string(),
                });
            }

            if policy.require_default_branch
                && repository.default_branch.as_deref()
                    != Some(policy.required_default_branch.as_str())
            {
                let current = repository
                    .default_branch
                    .clone()
                    .unwrap_or_else(|| "<unset>".to_string());
                findings.push(AuditFinding {
                    repository: repository.name.clone(),
                    code: "default-branch-policy",
                    message: format!(
                        "Default branch '{current}' does not match required branch '{}'.",
                        policy.required_default_branch
                    ),
                });
            }
        }

        Ok(findings)
    }

    async fn blueprint_repository(
        &self,
        organization: &str,
        request: &BlueprintRequest,
    ) -> Result<BlueprintResult> {
        let endpoint = self.repositories_endpoint(organization)?;
        let payload = CreateAzureDevOpsRepositoryRequest {
            name: request.name.clone(),
            project: AzureDevOpsProjectReference {
                name: self.project.clone(),
            },
        };

        let response = self
            .client
            .post(endpoint)
            .json(&payload)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to create Azure DevOps repository '{}'",
                    request.name
                )
            })?
            .error_for_status()
            .with_context(|| {
                format!(
                    "Azure DevOps create repository API returned an error for '{}'",
                    request.name
                )
            })?;

        let repository: AzureDevOpsRepository = response.json().await.with_context(|| {
            format!(
                "failed to decode Azure DevOps create repository response for '{}'",
                request.name
            )
        })?;

        let _ = matches!(request.visibility, RepoVisibility::Private);

        Ok(BlueprintResult {
            name: repository.name,
            provider: "azure_devops",
            web_url: repository.web_url,
        })
    }
}

impl AzureDevOpsProvider {
    fn repositories_endpoint(&self, organization: &str) -> Result<Url> {
        let mut endpoint = self
            .api_url
            .join(&format!(
                "/{organization}/{}/_apis/git/repositories",
                self.project
            ))
            .with_context(|| {
                format!(
                    "failed to build Azure DevOps repositories URL for project {}",
                    self.project
                )
            })?;

        endpoint.query_pairs_mut().append_pair("api-version", "7.1");

        Ok(endpoint)
    }
}

fn build_client(config: &AzureDevOpsConfig) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );

    // Prefer env var (PAT → Basic auth); fall back to OAuth bearer token saved
    // by `devopster login azure-devops`.
    if let Ok(token) = env::var(&config.token_env) {
        // PAT-style: encode as Basic :{token}
        let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{token}"));
        let auth = format!("Basic {encoded}");
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&auth)
                .context("invalid Azure DevOps token header value")?,
        );
    } else if let Some(stored) = auth::load_token("azure_devops").ok().flatten() {
        // OAuth bearer token from browser login.
        let auth = format!("Bearer {}", stored.access_token);
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&auth)
                .context("invalid Azure DevOps OAuth token header value")?,
        );
    }

    Client::builder()
        .default_headers(headers)
        .user_agent("devopster-cli")
        .build()
        .context("failed to build Azure DevOps HTTP client")
}

#[derive(Debug, Deserialize)]
struct AzureDevOpsRepositoryList {
    value: Vec<AzureDevOpsRepository>,
}

#[derive(Debug, Deserialize)]
struct AzureDevOpsRepository {
    name: String,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
}

impl From<AzureDevOpsRepository> for RepoSummary {
    fn from(value: AzureDevOpsRepository) -> Self {
        Self {
            name: value.name,
            full_name: None,
            description: String::new(),
            topics: Vec::new(),
            license: None,
            default_branch: value
                .default_branch
                .as_deref()
                .map(|branch| branch.trim_start_matches("refs/heads/").to_string()),
            web_url: value.web_url,
            provider: "azure_devops",
            language: None,
            archived: false,
            is_private: false,
            stargazers_count: None,
            forks_count: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct CreateAzureDevOpsRepositoryRequest {
    name: String,
    project: AzureDevOpsProjectReference,
}

#[derive(Debug, Serialize)]
struct AzureDevOpsProjectReference {
    name: String,
}
