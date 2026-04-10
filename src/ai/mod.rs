//! GitHub Copilot AI integration.
//!
//! Uses the user's existing GitHub auth token (from `gh auth token` or the
//! devopster credential store) to obtain a short-lived Copilot session token,
//! then calls the Copilot chat completions endpoint.
//!
//! All public functions return `Option` — they degrade silently if the user
//! does not have a Copilot subscription or if the network call fails.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COMPLETIONS_URL: &str = "https://api.githubcopilot.com/chat/completions";

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

fn make_client() -> Result<Client> {
    Client::builder()
        .user_agent("devopster-cli/0.1")
        .build()
        .context("failed to build HTTP client")
}

/// Exchange a GitHub PAT / OAuth token for a short-lived Copilot session token.
/// Returns `None` if the user does not have Copilot access.
async fn copilot_token(github_token: &str) -> Option<String> {
    let client = make_client().ok()?;
    let resp = client
        .get(TOKEN_EXCHANGE_URL)
        .header("Authorization", format!("token {github_token}"))
        .header("Editor-Version", "devopster-cli/0.1")
        .header("Editor-Plugin-Version", "devopster-cli/0.1")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body: CopilotTokenResponse = resp.json().await.ok()?;
    Some(body.token)
}

/// Submit a chat completion request and return the assistant's text.
async fn complete(token: &str, system: &str, user: &str) -> Result<String> {
    let client = make_client()?;
    let resp = client
        .post(COMPLETIONS_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Copilot-Integration-Id", "devopster-cli")
        .json(&ChatRequest {
            model: "gpt-4o".into(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: system.into(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: user.into(),
                },
            ],
            max_tokens: 256,
            temperature: 0.2,
        })
        .send()
        .await
        .context("failed to call Copilot chat completions")?
        .error_for_status()
        .context("Copilot API returned an error")?;

    let body: ChatResponse = resp
        .json()
        .await
        .context("failed to decode Copilot response")?;
    Ok(body
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default())
}

/// Suggest up to 5 lowercase, hyphenated topics for a repository.
/// Returns `None` if Copilot is unavailable or the response cannot be parsed.
pub async fn suggest_topics(
    repo_name: &str,
    description: &str,
    github_token: &str,
) -> Option<Vec<String>> {
    let token = copilot_token(github_token).await?;
    let system = "You are a helpful assistant that suggests GitHub repository topics. \
                  Return ONLY a raw JSON array of lowercase, hyphenated topic strings. \
                  No explanation, no markdown fences. Maximum 5 items.";
    let prompt = format!(
        "Suggest topics for a repository named `{repo_name}` \
         with description: \"{description}\""
    );
    let raw = complete(&token, system, &prompt).await.ok()?;
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    serde_json::from_str::<Vec<String>>(&raw[start..=end]).ok()
}

/// Generate a concise one-sentence description for a repository.
/// Returns `None` if Copilot is unavailable.
pub async fn suggest_description(
    repo_name: &str,
    existing: &str,
    topics: &[String],
    github_token: &str,
) -> Option<String> {
    let token = copilot_token(github_token).await?;
    let system = "You are a helpful assistant that writes concise GitHub repository descriptions. \
                  One sentence, professional, no quotes, no markdown.";
    let topic_list = topics.join(", ");
    let prompt = format!(
        "Write a one-sentence description for a repository named `{repo_name}`. \
         Current description: \"{existing}\". Topics: [{topic_list}]"
    );
    let result = complete(&token, system, &prompt).await.ok()?;
    let cleaned = result
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Explain audit findings in plain language and suggest how to fix them.
/// Returns `None` if Copilot is unavailable.
pub async fn explain_audit_findings(findings_summary: &str, github_token: &str) -> Option<String> {
    let token = copilot_token(github_token).await?;
    let system = "You are a helpful assistant that explains repository audit findings \
                  and suggests concise, actionable fixes.";
    let prompt =
        format!("Explain these audit findings and suggest how to fix them:\n\n{findings_summary}");
    complete(&token, system, &prompt).await.ok()
}
