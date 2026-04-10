use anyhow::Result;
use clap::{Args, Subcommand};
use std::collections::HashSet;

use crate::config::AppConfig;
use crate::provider::ProviderFactory;
use crate::ui;

#[derive(Debug, Args)]
pub struct TopicsCommand {
    #[command(subcommand)]
    pub action: TopicsAction,
}

#[derive(Debug, Subcommand)]
pub enum TopicsAction {
    /// Add any missing template topics to every matching repository
    Align(AlignTopicsCommand),
}

#[derive(Debug, Args)]
pub struct AlignTopicsCommand {}

impl TopicsCommand {
    pub async fn run(&self, config_path: &str) -> Result<()> {
        let config = AppConfig::load(config_path)?;
        let provider = ProviderFactory::from_config(&config)?;

        match &self.action {
            TopicsAction::Align(_) => {
                if config.templates.is_empty() {
                    ui::warn("No templates configured; nothing to align.");
                    return Ok(());
                }

                let all_repos = provider.list_repositories(&config.organization).await?;
                let mut updated = 0usize;
                let mut already_aligned = 0usize;

                for template in &config.templates {
                    let template_topics: HashSet<&str> =
                        template.topics.iter().map(String::as_str).collect();

                    for repo in &all_repos {
                        let repo_topics: HashSet<&str> =
                            repo.topics.iter().map(String::as_str).collect();

                        // Only process repos that already share at least one topic
                        // with this template (indicating they belong to this group)
                        if repo_topics.is_disjoint(&template_topics) {
                            continue;
                        }

                        let missing: Vec<String> = template
                            .topics
                            .iter()
                            .filter(|t| !repo_topics.contains(t.as_str()))
                            .cloned()
                            .collect();

                        if missing.is_empty() {
                            already_aligned += 1;
                            continue;
                        }

                        let mut merged = repo.topics.clone();
                        merged.extend(missing.iter().cloned());

                        provider
                            .align_topics(&config.organization, &repo.name, &merged)
                            .await?;

                        ui::item(&format!("Updated '{}': added {:?}", repo.name, missing));
                        updated += 1;
                    }
                }

                ui::success(&format!(
                    "Topic alignment complete: {updated} updated, {already_aligned} already aligned."
                ));
            }
        }

        Ok(())
    }
}
