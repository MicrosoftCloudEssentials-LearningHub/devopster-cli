use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIdentity {
    pub login: String,
    pub name: String,
    pub email: String,
}

/// A persisted access token for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    /// Only present when the provider issued a refresh token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_identity: Option<GitHubIdentity>,
}

/// The on-disk token store.  Keys are provider names ("github", "azure_devops",
/// "gitlab").
#[derive(Debug, Default, Serialize, Deserialize)]
struct TokenStore {
    tokens: HashMap<String, StoredToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    github_identity: Option<GitHubIdentity>,
}

fn token_store_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine OS config directory")?;
    Ok(base.join("devopster").join("tokens.json"))
}

fn load_store() -> Result<TokenStore> {
    let path = token_store_path()?;
    if !path.exists() {
        return Ok(TokenStore::default());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read token store at {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse token store at {}", path.display()))
}

fn save_store(store: &TokenStore) -> Result<()> {
    let path = token_store_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir at {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(store).context("failed to serialize token store")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write token store to {}", path.display()))
}

/// Persist a token for the given provider.
pub fn save_token(provider: &str, token: StoredToken) -> Result<()> {
    let mut store = load_store()?;
    if provider == "github" {
        store.github_identity = token.github_identity.clone();
    }
    store.tokens.insert(provider.to_string(), token);
    save_store(&store)
}

/// Retrieve a previously saved token for the given provider, if any.
pub fn load_token(provider: &str) -> Result<Option<StoredToken>> {
    let store = load_store()?;
    Ok(store.tokens.get(provider).cloned())
}

/// Remove the stored token for the given provider.
pub fn remove_token(provider: &str) -> Result<()> {
    let mut store = load_store()?;
    store.tokens.remove(provider);
    if provider == "github" {
        store.github_identity = None;
    }
    save_store(&store)
}

pub fn load_github_identity() -> Result<Option<GitHubIdentity>> {
    let store = load_store()?;
    Ok(store
        .github_identity
        .or_else(|| store.tokens.get("github").and_then(|token| token.github_identity.clone())))
}

pub fn save_github_identity(identity: GitHubIdentity) -> Result<()> {
    let mut store = load_store()?;
    store.github_identity = Some(identity.clone());
    if let Some(token) = store.tokens.get_mut("github") {
        token.github_identity = Some(identity);
    }
    save_store(&store)
}

/// Wrap an API error with a re-login hint when it looks like an auth failure.
pub fn annotate_auth_error(err: anyhow::Error, provider: &str) -> anyhow::Error {
    let msg = err.to_string().to_lowercase();
    if msg.contains("401")
        || msg.contains("403")
        || msg.contains("unauthorized")
        || msg.contains("forbidden")
    {
        let login_cmd = match provider {
            "azure_devops" => "devopster login azure-devops",
            "gitlab" => "devopster login gitlab",
            _ => "devopster login github",
        };
        err.context(format!(
            "Authentication failed for {provider}. \
             Run `{login_cmd}` to sign in, or set the corresponding token environment variable."
        ))
    } else {
        err
    }
}
