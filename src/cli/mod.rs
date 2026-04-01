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
{before-help}devopster {version}
{about}

Usage: devopster [OPTIONS] <COMMAND>

Commands:
{tab}+-------------------+---------------------------------------------------+
{tab}| login             | Authenticate with a provider via browser sign-in  |
{tab}+-------------------+---------------------------------------------------+
{tab}| init              | Create devopster-config.yaml and sign in          |
{tab}+-------------------+---------------------------------------------------+
{tab}| repo list         | List repositories (optionally filter by topic)    |
{tab}| repo audit        | Audit repos against the configured policy         |
{tab}| repo scaffold     | Create a new repository from a template           |
{tab}| repo sync         | Push files from .github/ to all repositories      |
{tab}+-------------------+---------------------------------------------------+
{tab}| catalog generate  | Export a catalog.json of all repositories         |
{tab}+-------------------+---------------------------------------------------+
{tab}| topics align      | Add missing template topics to repositories       |
{tab}+-------------------+---------------------------------------------------+
{tab}| stats             | Print org-wide metadata coverage and compliance   |
{tab}+-------------------+---------------------------------------------------+

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
    /// List, audit, scaffold, and sync repositories
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
