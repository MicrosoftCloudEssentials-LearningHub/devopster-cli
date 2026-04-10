use std::env;

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use reqwest::{header, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::config::{AppConfig, GitHubConfig};

use super::{
    AuditFinding, AuditPolicy, BlueprintRequest, BlueprintResult, Provider, RepoSummary,
    RepoVisibility,
};

pub struct GitHubProvider {
    api_url: Url,
    client: Client,
}

impl GitHubProvider {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let github = config
            .github
            .as_ref()
            .context("github provider selected but github config is missing")?;

        let client = build_client(github)?;

        Ok(Self {
            api_url: github.api_url.clone(),
            client,
        })
    }
}

#[async_trait]
impl Provider for GitHubProvider {
    async fn list_repositories(&self, organization: &str) -> Result<Vec<RepoSummary>> {
        self.fetch_repositories(organization)
            .await
            .map_err(|e| auth::annotate_auth_error(e, "github"))
    }

    async fn audit_repositories(
        &self,
        organization: &str,
        policy: &AuditPolicy,
    ) -> Result<Vec<AuditFinding>> {
        let repositories = self.fetch_repositories(organization).await?;
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

            if policy.require_license {
                let missing = repository
                    .license
                    .as_deref()
                    .map(|l| l.trim().is_empty() || l == "NOASSERTION")
                    .unwrap_or(true);
                if missing {
                    findings.push(AuditFinding {
                        repository: repository.name.clone(),
                        code: "missing-license",
                        message:
                            "Repository license is missing or not detectable from GitHub metadata."
                                .to_string(),
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

    async fn blueprint_repository(
        &self,
        organization: &str,
        request: &BlueprintRequest,
    ) -> Result<BlueprintResult> {
        let endpoint = self
            .api_url
            .join(&format!("/orgs/{organization}/repos"))
            .with_context(|| format!("failed to build GitHub URL for org {organization}"))?;

        let payload = CreateGitHubRepositoryRequest {
            name: request.name.clone(),
            description: request.description.clone(),
            private: matches!(request.visibility, RepoVisibility::Private),
            auto_init: true,
        };

        let response = self
            .client
            .post(endpoint)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("failed to create GitHub repository '{}'", request.name))?;

        let response = response.error_for_status().with_context(|| {
            format!(
                "GitHub create repository API returned an error for '{}'",
                request.name
            )
        })?;

        let repository: GitHubRepository = response.json().await.with_context(|| {
            format!(
                "failed to decode GitHub create repository response for '{}'",
                request.name
            )
        })?;

        if !request.template.topics.is_empty() {
            let topics_endpoint = self
                .api_url
                .join(&format!("/repos/{organization}/{}/topics", request.name))
                .with_context(|| {
                    format!("failed to build GitHub topics URL for '{}'", request.name)
                })?;

            let topics_payload = UpdateGitHubTopicsRequest {
                names: request.template.topics.clone(),
            };

            self.client
                .put(topics_endpoint)
                .json(&topics_payload)
                .send()
                .await
                .with_context(|| format!("failed to set GitHub topics for '{}'", request.name))?
                .error_for_status()
                .with_context(|| {
                    format!("GitHub topics API returned an error for '{}'", request.name)
                })?;
        }

        // Push README with template-specific content
        let readme_content = format!(
            "# {}\n\n{}\n\n<!-- Generated by devopster via template '{}' -->\n",
            request.name, request.description, request.template.name
        );
        self.push_file(
            organization,
            &request.name,
            "README.md",
            readme_content.as_bytes(),
            "chore: initialize README from devopster template",
        )
        .await?;

        // Push default CODEOWNERS
        let codeowners_content = format!("# Code owners for this repository.\n* @{organization}\n");
        self.push_file(
            organization,
            &request.name,
            ".github/CODEOWNERS",
            codeowners_content.as_bytes(),
            "chore: add CODEOWNERS from devopster template",
        )
        .await?;

        Ok(BlueprintResult {
            name: repository.name,
            provider: "github",
            web_url: repository.web_url,
        })
    }

    async fn update_description(
        &self,
        organization: &str,
        repository: &str,
        description: &str,
    ) -> Result<()> {
        let endpoint = self
            .api_url
            .join(&format!("/repos/{organization}/{repository}"))
            .with_context(|| {
                format!("failed to build GitHub URL for '{organization}/{repository}'")
            })?;

        self.client
            .patch(endpoint)
            .json(&UpdateGitHubRepositoryRequest {
                description: description.to_string(),
            })
            .send()
            .await
            .with_context(|| {
                format!("failed to update description for '{organization}/{repository}'")
            })?
            .error_for_status()
            .with_context(|| {
                format!("GitHub update repo API returned an error for '{repository}'")
            })?;

        Ok(())
    }

    async fn align_topics(
        &self,
        organization: &str,
        repository: &str,
        topics: &[String],
    ) -> Result<()> {
        let endpoint = self
            .api_url
            .join(&format!("/repos/{organization}/{repository}/topics"))
            .with_context(|| format!("failed to build GitHub topics URL for '{repository}'"))?;

        self.client
            .put(endpoint)
            .json(&UpdateGitHubTopicsRequest {
                names: topics.to_vec(),
            })
            .send()
            .await
            .with_context(|| format!("failed to align topics for '{repository}'"))?
            .error_for_status()
            .with_context(|| format!("GitHub topics API returned an error for '{repository}'"))?;

        Ok(())
    }

    async fn readme_first_line(
        &self,
        organization: &str,
        repository: &str,
    ) -> Result<Option<String>> {
        let endpoint = self
            .api_url
            .join(&format!("/repos/{organization}/{repository}/readme"))
            .with_context(|| {
                format!("failed to build GitHub README URL for '{organization}/{repository}'")
            })?;

        let response =
            self.client.get(endpoint).send().await.with_context(|| {
                format!("failed to fetch README for '{organization}/{repository}'")
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = response
            .error_for_status()
            .with_context(|| format!("GitHub README API returned an error for '{repository}'"))?;

        let readme: GitHubReadmeContent = response
            .json()
            .await
            .with_context(|| format!("failed to decode README response for '{repository}'"))?;

        if readme.content.trim().is_empty() {
            return Ok(None);
        }

        let cleaned = readme.content.replace('\n', "");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cleaned)
            .with_context(|| format!("failed to decode README content for '{repository}'"))?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(first_readme_line(&text))
    }

    async fn fetch_repository_files(
        &self,
        owner: &str,
        repository: &str,
        branch: &str,
        paths: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let roots = normalize_paths(paths);

        let mut endpoint = self
            .api_url
            .join(&format!("/repos/{owner}/{repository}/git/trees/{branch}"))
            .with_context(|| {
                format!("failed to build GitHub tree URL for '{owner}/{repository}'")
            })?;
        endpoint.query_pairs_mut().append_pair("recursive", "1");

        let response = self.client.get(endpoint).send().await.with_context(|| {
            format!("failed to fetch repository tree for '{owner}/{repository}'")
        })?;
        let response = response.error_for_status().with_context(|| {
            format!("GitHub tree API returned an error for '{owner}/{repository}'")
        })?;

        let tree: GitHubTree = response.json().await.with_context(|| {
            format!("failed to decode tree response for '{owner}/{repository}'")
        })?;

        let mut files = Vec::new();
        for entry in tree.tree {
            if entry.kind != "blob" {
                continue;
            }
            if !matches_any_path(&entry.path, &roots) {
                continue;
            }

            if let Some(bytes) = self
                .fetch_file_bytes(owner, repository, branch, &entry.path)
                .await?
            {
                files.push((entry.path, bytes));
            }
        }

        Ok(files)
    }

    async fn push_file(
        &self,
        organization: &str,
        repository: &str,
        path: &str,
        content: &[u8],
        commit_message: &str,
    ) -> Result<()> {
        let endpoint = self
            .api_url
            .join(&format!(
                "/repos/{organization}/{repository}/contents/{path}"
            ))
            .with_context(|| format!("failed to build GitHub contents URL for '{path}'"))?;

        // Fetch current SHA if the file exists (required for updates)
        let existing_sha: Option<String> = {
            let response = self
                .client
                .get(endpoint.clone())
                .send()
                .await
                .with_context(|| format!("failed to check file '{path}' in '{repository}'"))?;
            if response.status().is_success() {
                let meta: GitHubFileContent = response.json().await.with_context(|| {
                    format!("failed to decode file metadata for '{path}' in '{repository}'")
                })?;
                meta.sha
            } else {
                None
            }
        };

        let content_b64 = base64::engine::general_purpose::STANDARD.encode(content);
        let identity = auth::load_github_identity().ok().flatten();
        self.client
            .put(endpoint)
            .json(&PushGitHubFileRequest {
                message: commit_message.to_string(),
                content: content_b64,
                sha: existing_sha,
                author: identity.as_ref().map(GitHubCommitIdentity::from),
                committer: identity.as_ref().map(GitHubCommitIdentity::from),
            })
            .send()
            .await
            .with_context(|| format!("failed to push file '{path}' to '{repository}'"))?
            .error_for_status()
            .with_context(|| {
                format!("GitHub contents API returned an error for '{path}' in '{repository}'")
            })?;

        Ok(())
    }
}

impl GitHubProvider {
    async fn fetch_repositories(&self, organization: &str) -> Result<Vec<RepoSummary>> {
        let mut page = 1;
        let mut repositories = Vec::new();

        loop {
            let mut endpoint = self
                .api_url
                .join(&format!("/orgs/{organization}/repos"))
                .with_context(|| format!("failed to build GitHub URL for org {organization}"))?;

            endpoint
                .query_pairs_mut()
                .append_pair("type", "all")
                .append_pair("sort", "updated")
                .append_pair("per_page", "100")
                .append_pair("page", &page.to_string());

            let response = self.client.get(endpoint).send().await.with_context(|| {
                format!("failed to call GitHub repository API for org {organization}")
            })?;

            if response.status() == StatusCode::NOT_FOUND {
                anyhow::bail!(
                    "GitHub organization '{organization}' was not found or is not visible"
                )
            }

            let response = response.error_for_status().with_context(|| {
                format!("GitHub repository API returned an error for org {organization}")
            })?;

            let page_items: Vec<GitHubRepository> = response.json().await.with_context(|| {
                format!("failed to decode GitHub repository response for org {organization}")
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

    async fn fetch_file_bytes(
        &self,
        owner: &str,
        repository: &str,
        branch: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>> {
        let mut endpoint = self
            .api_url
            .join(&format!("/repos/{owner}/{repository}/contents/{path}"))
            .with_context(|| {
                format!("failed to build GitHub contents URL for '{owner}/{repository}'")
            })?;
        endpoint.query_pairs_mut().append_pair("ref", branch);

        let response =
            self.client.get(endpoint).send().await.with_context(|| {
                format!("failed to fetch file '{path}' in '{owner}/{repository}'")
            })?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = response.error_for_status().with_context(|| {
            format!("GitHub contents API returned an error for '{owner}/{repository}'")
        })?;

        let content: GitHubFileContent = response.json().await.with_context(|| {
            format!("failed to decode contents response for '{owner}/{repository}'")
        })?;

        let encoding = content.encoding.as_deref().unwrap_or("base64");
        let raw = content.content.unwrap_or_default().replace('\n', "");
        if raw.is_empty() {
            return Ok(None);
        }

        if encoding != "base64" {
            return Ok(None);
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .with_context(|| format!("failed to decode base64 for '{path}'"))?;
        Ok(Some(bytes))
    }
}

fn build_client(config: &GitHubConfig) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/vnd.github+json"),
    );

    // Prefer env var; fall back to token saved by `devopster login github`.
    let token = env::var(&config.token_env).ok().or_else(|| {
        auth::load_token("github")
            .ok()
            .flatten()
            .map(|t| t.access_token)
    });

    if let Some(token) = token {
        let auth = format!("Bearer {token}");
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&auth).context("invalid GitHub token header value")?,
        );
    }

    Client::builder()
        .default_headers(headers)
        .user_agent("devopster-cli")
        .build()
        .context("failed to build GitHub HTTP client")
}

#[derive(Debug, Deserialize)]
struct GitHubRepository {
    name: String,
    #[serde(default)]
    full_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    license: Option<GitHubLicense>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(rename = "private", default)]
    is_private: bool,
    #[serde(default)]
    stargazers_count: Option<u64>,
    #[serde(default)]
    forks_count: Option<u64>,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

impl From<GitHubRepository> for RepoSummary {
    fn from(value: GitHubRepository) -> Self {
        Self {
            name: value.name,
            full_name: value.full_name,
            description: value.description.unwrap_or_default(),
            topics: value.topics,
            license: value.license.and_then(|license| license.spdx_id),
            default_branch: value.default_branch,
            web_url: value.web_url.or(value.html_url),
            provider: "github",
            language: value.language,
            archived: value.archived,
            is_private: value.is_private,
            stargazers_count: value.stargazers_count,
            forks_count: value.forks_count,
            updated_at: value.pushed_at.or(value.updated_at),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubLicense {
    #[serde(default)]
    spdx_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateGitHubRepositoryRequest {
    name: String,
    description: String,
    private: bool,
    auto_init: bool,
}

#[derive(Debug, Serialize)]
struct UpdateGitHubTopicsRequest {
    names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubFileContent {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
    sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubTree {
    tree: Vec<GitHubTreeEntry>,
}

#[derive(Debug, Deserialize)]
struct GitHubTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Serialize)]
struct PushGitHubFileRequest {
    message: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<GitHubCommitIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    committer: Option<GitHubCommitIdentity>,
}

#[derive(Debug, Serialize)]
struct GitHubCommitIdentity {
    name: String,
    email: String,
}

impl From<&auth::GitHubIdentity> for GitHubCommitIdentity {
    fn from(value: &auth::GitHubIdentity) -> Self {
        Self {
            name: value.name.clone(),
            email: value.email.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct UpdateGitHubRepositoryRequest {
    description: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReadmeContent {
    #[serde(default)]
    content: String,
}

fn first_readme_line(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let mut s = line.trim();
        if s.is_empty() {
            continue;
        }
        loop {
            let trimmed = s.trim_start_matches(|c: char| {
                c == '#' || c == '>' || c == '-' || c == '*' || c == ' '
            });
            if trimmed == s {
                break;
            }
            s = trimmed;
        }
        let cleaned = s.trim().trim_matches('`');
        if !cleaned.is_empty() {
            return Some(cleaned.to_string());
        }
    }
    None
}

fn normalize_paths(paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return vec![".github".to_string()];
    }

    paths
        .iter()
        .map(|p| p.trim().trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn matches_any_path(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|root| path == root || path.starts_with(&format!("{root}/")))
}
