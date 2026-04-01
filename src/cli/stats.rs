use anyhow::Result;
use clap::Args;

use crate::config::AppConfig;
use crate::provider::ProviderFactory;

#[derive(Debug, Args)]
pub struct StatsCommand {
    /// Write non-compliant repo names into scoped_repos in the config file.
    #[arg(long, default_value_t = false)]
    pub scope_missing: bool,
}

impl StatsCommand {
    pub async fn run(&self, config_path: &str) -> Result<()> {
        let config = AppConfig::load(config_path)?;
        let provider = ProviderFactory::from_config(&config)?;
        let repos = provider.list_repositories(&config.organization).await?;

        let total = repos.len();

        // --- metadata coverage ---
        let mut no_description: Vec<&str> = repos
            .iter()
            .filter(|r| r.description.trim().is_empty())
            .map(|r| r.name.as_str())
            .collect();
        no_description.sort_unstable();

        let mut no_topics: Vec<&str> = repos
            .iter()
            .filter(|r| r.topics.is_empty())
            .map(|r| r.name.as_str())
            .collect();
        no_topics.sort_unstable();

        let mut no_license: Vec<&str> = repos
            .iter()
            .filter(|r| r.license.is_none())
            .map(|r| r.name.as_str())
            .collect();
        no_license.sort_unstable();

        let required_branch = config.default_branch.as_str();
        let mut wrong_branch: Vec<(&str, &str)> = repos
            .iter()
            .filter(|r| r.default_branch.as_deref() != Some(required_branch))
            .map(|r| (r.name.as_str(), r.default_branch.as_deref().unwrap_or("<unset>")))
            .collect();
        wrong_branch.sort_unstable_by_key(|(n, _)| *n);

        // --- unique topic labels and frequency ---
        let mut topic_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for repo in &repos {
            for topic in &repo.topics {
                *topic_counts.entry(topic.as_str()).or_insert(0) += 1;
            }
        }
        let mut top_topics: Vec<(&str, usize)> = topic_counts.into_iter().collect();
        top_topics.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let unique_topic_count = top_topics.len();
        let repos_with_topics = total - no_topics.len();

        // --- default branch distribution ---
        let mut branch_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for repo in &repos {
            let branch = repo.default_branch.as_deref().unwrap_or("<unset>");
            *branch_counts.entry(branch).or_insert(0) += 1;
        }
        let mut branch_dist: Vec<(&str, usize)> = branch_counts.into_iter().collect();
        branch_dist.sort_by(|a, b| b.1.cmp(&a.1));

        // --- compliance: all four mandatory checks pass ---
        // Note: README.md presence requires a per-repo file API call; not checked here.
        let mut non_compliant: Vec<(&str, Vec<&str>)> = repos
            .iter()
            .filter_map(|r| {
                let mut issues: Vec<&str> = Vec::new();
                if r.description.trim().is_empty() { issues.push("no description"); }
                if r.topics.is_empty()             { issues.push("no topics"); }
                if r.license.is_none()             { issues.push("no license"); }
                if r.default_branch.as_deref() != Some(required_branch) {
                    issues.push("wrong branch");
                }
                if issues.is_empty() { None } else { Some((r.name.as_str(), issues)) }
            })
            .collect();
        non_compliant.sort_unstable_by_key(|(n, _)| *n);
        let compliant_count = total - non_compliant.len();

        // --- output ---
        let sep  = "=".repeat(56);
        let thin = "-".repeat(56);
        let w    = 32usize;

        println!("{sep}");
        println!("  Organization stats");
        println!("{sep}");
        println!("{:<w$} {}", "  Config:", config_path);
        println!("{:<w$} {}", "  Organization:", config.organization);
        println!("{:<w$} {}", "  Provider:", config.provider.as_str());
        println!("{:<w$} {}", "  Required default branch:", required_branch);
        if !config.templates.is_empty() {
            println!("{:<w$} {} defined", "  Templates:", config.templates.len());
        }
        if config.copilot_enabled {
            println!("{:<w$} enabled", "  Copilot:");
        }

        println!("{thin}");
        println!("{:<w$} {}", "  Total repositories:", total);
        if !config.scoped_repos.is_empty() {
            println!("{:<w$} {} of {total}", "  Scoped to:", config.scoped_repos.len());
        }

        println!("{thin}");
        println!("  Metadata coverage:");
        Self::print_coverage("description", total - no_description.len(), no_description.len(), &no_description, w);
        Self::print_coverage("topics",      total - no_topics.len(),      no_topics.len(),      &no_topics,      w);
        Self::print_coverage("license",     total - no_license.len(),     no_license.len(),     &no_license,     w);

        if !wrong_branch.is_empty() {
            println!(
                "{:<w$} {} / {} wrong (want: {required_branch})",
                "  Correct branch:",
                total - wrong_branch.len(),
                wrong_branch.len(),
            );
            for (name, branch) in &wrong_branch {
                println!("      - {name}  [{branch}]");
            }
        } else {
            println!("{:<w$} all {total} on '{required_branch}'", "  Correct branch:", "");
        }

        println!("{thin}");
        println!("  Compliance (description + topics + license + branch):");
        println!("{:<w$} {} / {total}", "  Compliant:", compliant_count);
        if !non_compliant.is_empty() {
            println!("  Non-compliant ({}):", non_compliant.len());
            for (name, issues) in &non_compliant {
                println!("      - {:<40} [{}]", name, issues.join(", "));
            }
        }

        if !top_topics.is_empty() {
            println!("{thin}");
            println!(
                "  Topic labels: {} unique across {} repos",
                unique_topic_count, repos_with_topics
            );
            for (topic, count) in top_topics.iter().take(10) {
                println!("      {:<36} x{}", topic, count);
            }
            if top_topics.len() > 10 {
                println!("      ... and {} more", top_topics.len() - 10);
            }
        }

        if !branch_dist.is_empty() {
            println!("{thin}");
            println!("  Default branch distribution:");
            for (branch, count) in &branch_dist {
                println!("      {:<36} x{}", branch, count);
            }
        }

        // Collect all repos that have any issue — unique, sorted
        if !non_compliant.is_empty() {
            println!("{thin}");
            if self.scope_missing {
                let names: Vec<&str> = non_compliant.iter().map(|(n, _)| *n).collect();
                Self::rewrite_scoped_repos(config_path, &names)?;
                println!("  scoped_repos updated in {config_path} ({} repos).", names.len());
                println!("  Run: devopster repo audit   -- to see per-repo issues");
                println!("       devopster topics align  -- to fix missing topics");
            } else {
                println!("  Tip: run with --scope-missing to auto-update scoped_repos");
                println!("  in {config_path} with the {n} repos above.",
                    n = non_compliant.len());
            }
        }

        println!("{sep}");
        Ok(())
    }

    fn print_coverage(label: &str, have: usize, missing: usize, missing_names: &[&str], w: usize) {
        let padded = format!("  With {label}:");
        if missing == 0 {
            println!("{:<w$} {have} / 0 missing", padded);
        } else {
            println!("{:<w$} {have} / {missing} missing", padded);
            for name in missing_names {
                println!("      - {name}");
            }
        }
    }

    /// Rewrite the `scoped_repos:` block in the YAML config file in-place.
    /// Preserves all other keys, comments, and ordering.
    fn rewrite_scoped_repos(config_path: &str, names: &[&str]) -> Result<()> {
        let raw = std::fs::read_to_string(config_path)?;
        let ends_with_newline = raw.ends_with('\n');

        let mut output: Vec<String> = Vec::new();
        let mut found = false;
        let mut iter = raw.lines().peekable();

        while let Some(line) = iter.next() {
            if line == "scoped_repos:" || line.starts_with("scoped_repos:") {
                found = true;
                if names.is_empty() {
                    output.push("scoped_repos: []".to_string());
                } else {
                    output.push("scoped_repos:".to_string());
                    for name in names {
                        output.push(format!("  - {name}"));
                    }
                }
                // consume existing list items
                while iter.peek().map(|l| l.starts_with("  -")).unwrap_or(false) {
                    iter.next();
                }
            } else {
                output.push(line.to_string());
            }
        }

        if !found {
            // insert after the organization: line
            let mut patched: Vec<String> = Vec::new();
            for line in output {
                let is_org = line.starts_with("organization:");
                patched.push(line);
                if is_org {
                    if names.is_empty() {
                        patched.push("scoped_repos: []".to_string());
                    } else {
                        patched.push("scoped_repos:".to_string());
                        for name in names {
                            patched.push(format!("  - {name}"));
                        }
                    }
                }
            }
            output = patched;
        }

        let mut result = output.join("\n");
        if ends_with_newline {
            result.push('\n');
        }
        std::fs::write(config_path, result)?;
        Ok(())
    }
}
