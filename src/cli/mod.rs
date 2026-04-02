pub mod catalog;
pub mod init;
pub mod login;
pub mod repo;
pub mod stats;
pub mod topics;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "devopster",
    version,
    about = "GitOps CLI for GitHub, Azure DevOps, and GitLab",
    long_about = None,
    help_template = "\
{before-help}devopster {version} — {about}

Usage: devopster [OPTIONS] <COMMAND>

Commands:
{tab}login github                  Sign in to GitHub via browser (gh CLI)
{tab}login azure-devops            Sign in to Azure DevOps via browser (az CLI)
{tab}login gitlab                  Sign in to GitLab via browser (glab CLI)
{tab}login all                     Sign in to all three providers sequentially
{tab}login status                  Show authentication status for all providers
{tab}login logout <provider>       Remove stored credentials for a provider
{tab}init                          Create devopster-config.yaml and sign in
{tab}init --no-login               Create devopster-config.yaml, skip sign-in
{tab}repo list                     List repositories in the configured organization
{tab}repo list --topic <topic>     Filter repositories by topic
{tab}repo audit                    Audit repos against the configured policy
{tab}repo fix                      Prompt to fix missing metadata
{tab}repo blueprint                Create a new repository from a blueprint
{tab}repo sync                     Push files from .github/ to all repositories
{tab}repo sync --from-blueprint    Sync files from the blueprint repo
{tab}catalog generate              Export a catalog.json of all repositories
{tab}topics align                  Add missing template topics to repositories
{tab}stats                         Print org-wide metadata coverage and compliance
{tab}stats --scope-missing         Also write non-compliant repos to scoped_repos

Options:
{options}
{after-help}"
)]
pub struct Cli {
    #[arg(
        long,
        short = 'c',
        global = true,
        env = "DEVOPSTER_CONFIG",
        default_value = "devopster-config.yaml"
    )]
    pub config: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Authenticate with a provider via browser sign-in
    Login(login::LoginCommand),
    /// Create devopster-config.yaml interactively and optionally sign in
    Init(init::InitCommand),
    /// List, audit, blueprint, and sync repositories
    Repo(repo::RepoCommand),
    /// Generate a machine-readable org catalog (catalog.json)
    Catalog(catalog::CatalogCommand),
    /// Add missing template topics to every matching repository
    Topics(topics::TopicsCommand),
    /// Print org-wide metadata coverage, compliance, and top topics
    Stats(stats::StatsCommand),
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login(command) => command.run().await,
        Commands::Init(command) => command.run(&cli.config).await,
        Commands::Repo(command) => command.run(&cli.config).await,
        Commands::Catalog(command) => command.run(&cli.config).await,
        Commands::Topics(command) => command.run(&cli.config).await,
        Commands::Stats(command) => command.run(&cli.config).await,
    }
}
