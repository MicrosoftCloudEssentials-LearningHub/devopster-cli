use std::env;

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use reqwest::{header, Client, Url};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::config::{AppConfig, GitLabConfig};

use super::{
    AuditFinding, AuditPolicy, Provider, RepoSummary, RepoVisibility, ScaffoldRequest,
    ScaffoldResult,
};

pub struct GitLabProvider {
    api_url: Url,
    client: Client,
}

impl GitLabProvider {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let gitlab = config
            .gitlab
            .as_ref()
            .context("gitlab provider selected but gitlab config is missing")?;

        Ok(Self {
            api_url: gitlab.api_url.clone(),
            client: build_client(gitlab)?,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_url.as_str().trim_end_matches('/'), path)
    }
}

#[async_trait]
impl Provider for GitLabProvider {
    async fn list_repositories(&self, organization: &str) -> Result<Vec<RepoSummary>> {
        self.fetch_projects(organization)
            .await
            .map_err(|e| auth::annotate_auth_error(e, "gitlab"))
    }

    async fn audit_repositories(
        &self,
        organization: &str,
        policy: &AuditPolicy,
    ) -> Result<Vec<AuditFinding>> {
        let repositories = self.fetch_projects(organization).await?;
        let mut findings = Vec::new();

        for repository in repositories {
            if policy.require_description && repository.description.trim().is_empty() {
                findings.push(AuditFinding {
                    repository: repository.name.clone(),
                    code: "missing-description",
                    message: "Repository description is empty.".to_string(),
                });
            }

            if policy.require_topics {
                let count = repository.topics.len();
                if count < policy.min_topics {
                    findings.push(AuditFinding {
                        repository: repository.name.clone(),
                        code: "missing-topics",
                        message: format!(
                            "Repository has {count} topic(s); at least {} required.",
                            policy.min_topics
                        ),
                    });
                }
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

    async fn scaffold_repository(
        &self,
        organization: &str,
        request: &ScaffoldRequest,
    ) -> Result<ScaffoldResult> {
        let endpoint = self.url("/projects");

        let payload = CreateGitLabProjectRequest {
            name: request.name.clone(),
            description: request.description.clone(),
            namespace_path: organization.to_string(),
            visibility: if matches!(request.visibility, RepoVisibility::Private) {
                "private"
            } else {
                "public"
            },
            initialize_with_readme: true,
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("failed to create GitLab project '{}'", request.name))?
            .error_for_status()
            .with_context(|| {
                format!(
                    "GitLab create project API returned an error for '{}'",
                    request.name
                )
            })?;

        let project: GitLabProject = response.json().await.with_context(|| {
            format!(
                "failed to decode GitLab create project response for '{}'",
                request.name
            )
        })?;

        if !request.template.topics.is_empty() {
            let update_url = self.url(&format!("/projects/{}", project.id));
            self.client
                .put(&update_url)
                .json(&UpdateGitLabTopics {
                    topics: request.template.topics.clone(),
                })
                .send()
                .await
                .with_context(|| {
                    format!("failed to set GitLab topics for '{}'", request.name)
                })?
                .error_for_status()
                .with_context(|| {
                    format!(
                        "GitLab topics API returned an error for '{}'",
                        request.name
                    )
                })?;
        }

        Ok(ScaffoldResult {
            name: project.name,
            provider: "gitlab",
            web_url: project.web_url,
        })
    }

    async fn update_description(
        &self,
        organization: &str,
        repository: &str,
        description: &str,
    ) -> Result<()> {
        let project_path = url_encode_project_path(organization, repository);
        let endpoint = self.url(&format!("/projects/{project_path}"));

        self.client
            .put(&endpoint)
            .json(&UpdateGitLabProjectRequest {
                description: description.to_string(),
            })
            .send()
            .await
            .with_context(|| {
                format!("failed to update description for '{organization}/{repository}'")
            })?
            .error_for_status()
            .with_context(|| {
                format!("GitLab update project API returned an error for '{repository}'")
            })?;

        Ok(())
    }

    async fn align_topics(
        &self,
        organization: &str,
        repository: &str,
        topics: &[String],
    ) -> Result<()> {
        let project_path = url_encode_project_path(organization, repository);
        let endpoint = self.url(&format!("/projects/{project_path}"));

        self.client
            .put(&endpoint)
            .json(&UpdateGitLabTopics {
                topics: topics.to_vec(),
            })
            .send()
            .await
            .with_context(|| format!("failed to align GitLab topics for '{repository}'"))?
            .error_for_status()
            .with_context(|| {
                format!("GitLab topics API returned an error for '{repository}'")
            })?;

        Ok(())
    }

    async fn push_file(
        &self,
        organization: &str,
        repository: &str,
        path: &str,
        content: &[u8],
        commit_message: &str,
    ) -> Result<()> {
        let project_path = url_encode_project_path(organization, repository);
        let file_path = url_encode_file_path(path);
        let check_url =
            self.url(&format!("/projects/{project_path}/repository/files/{file_path}?ref=main"));

        let exists = self
            .client
            .get(&check_url)
            .send()
            .await
            .with_context(|| format!("failed to check file '{path}' in '{repository}'"))?
            .status()
            .is_success();

        let file_url = self.url(&format!(
            "/projects/{project_path}/repository/files/{file_path}"
        ));

        let content_b64 = base64::engine::general_purpose::STANDARD.encode(content);
        let payload = GitLabFileRequest {
            branch: "main".to_string(),
            content: content_b64,
            commit_message: commit_message.to_string(),
        };

        if exists {
            self.client
                .put(&file_url)
                .json(&payload)
                .send()
                .await
                .with_context(|| {
                    format!("failed to update file '{path}' in GitLab repo '{repository}'")
                })?
                .error_for_status()
                .with_context(|| {
                    format!(
                        "GitLab update file API returned an error for '{path}' in '{repository}'"
                    )
                })?;
        } else {
            self.client
                .post(&file_url)
                .json(&payload)
                .send()
                .await
                .with_context(|| {
                    format!("failed to create file '{path}' in GitLab repo '{repository}'")
                })?
                .error_for_status()
                .with_context(|| {
                    format!(
                        "GitLab create file API returned an error for '{path}' in '{repository}'"
                    )
                })?;
        }

        Ok(())
    }
}

impl GitLabProvider {
    async fn fetch_projects(&self, organization: &str) -> Result<Vec<RepoSummary>> {
        let encoded_group = url_encode_path_segment(organization);
        let mut page = 1u32;
        let mut repositories = Vec::new();

        loop {
            let endpoint = self.url(&format!(
                "/groups/{encoded_group}/projects?per_page=100&page={page}&include_subgroups=false"
            ));

            let response = self
                .client
                .get(&endpoint)
                .send()
                .await
                .with_context(|| {
                    format!("failed to call GitLab projects API for group '{organization}'")
                })?;

            if response.status() == reqwest::StatusCode::NOT_FOUND {
                anyhow::bail!(
                    "GitLab group '{organization}' was not found or is not visible"
                );
            }

            let response = response.error_for_status().with_context(|| {
                format!("GitLab projects API returned an error for group '{organization}'")
            })?;

            let page_items: Vec<GitLabProject> = response.json().await.with_context(|| {
                format!("failed to decode GitLab projects response for group '{organization}'")
            })?;

            let fetched = page_items.len();
            repositories.extend(page_items.into_iter().map(RepoSummary::from));

            if fetched < 100 {
                break;
            }

            page += 1;
        }

        Ok(repositories)
    }
}

fn build_client(config: &GitLabConfig) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );

    // Prefer env var; fall back to OAuth bearer token saved by `devopster login gitlab`.
    if let Ok(token) = env::var(&config.token_env) {
        headers.insert(
            "PRIVATE-TOKEN",
            header::HeaderValue::from_str(&token)
                .context("invalid GitLab token header value")?,
        );
    } else if let Some(stored) = auth::load_token("gitlab").ok().flatten() {
        let auth = format!("Bearer {}", stored.access_token);
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&auth)
                .context("invalid GitLab OAuth token header value")?,
        );
    }

    Client::builder()
        .default_headers(headers)
        .user_agent("devopster-cli")
        .build()
        .context("failed to build GitLab HTTP client")
}

/// Percent-encode a single URL path segment (e.g. a group or project name).
fn url_encode_path_segment(segment: &str) -> String {
    let mut result = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => result.push_str(&format!("%{byte:02X}")),
        }
    }
    result
}

/// Build the GitLab `namespace%2Frepo` identifier used in project-scoped API paths.
fn url_encode_project_path(organization: &str, repository: &str) -> String {
    format!(
        "{}%2F{}",
        url_encode_path_segment(organization),
        url_encode_path_segment(repository)
    )
}

/// Encode a file path for use in the GitLab repository files API (slashes → %2F).
fn url_encode_file_path(path: &str) -> String {
    path.replace('/', "%2F")
}

#[derive(Debug, Deserialize)]
struct GitLabProject {
    id: u64,
    name: String,
    #[serde(default)]
    path_with_namespace: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    star_count: Option<u64>,
    #[serde(default)]
    forks_count: Option<u64>,
    #[serde(default)]
    last_activity_at: Option<String>,
}

impl From<GitLabProject> for RepoSummary {
    fn from(value: GitLabProject) -> Self {
        let is_private = value.visibility.as_deref() == Some("private");
        Self {
            name: value.name,
            full_name: value.path_with_namespace,
            description: value.description.unwrap_or_default(),
            topics: value.topics,
            license: None,
            default_branch: value.default_branch,
            web_url: value.web_url,
            provider: "gitlab",
            language: None,
            archived: value.archived,
            is_private,
            stargazers_count: value.star_count,
            forks_count: value.forks_count,
            updated_at: value.last_activity_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct CreateGitLabProjectRequest {
    name: String,
    description: String,
    namespace_path: String,
    visibility: &'static str,
    initialize_with_readme: bool,
}

#[derive(Debug, Serialize)]
struct UpdateGitLabTopics {
    topics: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UpdateGitLabProjectRequest {
    description: String,
}

#[derive(Debug, Serialize)]
struct GitLabFileRequest {
    branch: String,
    content: String,
    commit_message: String,
}
