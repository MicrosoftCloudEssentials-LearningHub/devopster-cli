pub mod catalog;
pub mod init;
pub mod login;
pub mod repo;
pub mod stats;
pub mod topics;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use std::io::{self, IsTerminal, Write};

use crate::ui;

#[derive(Debug, Parser)]
#[command(
    name = "devopster",
    version,
    about = "GitOps CLI for GitHub, Azure DevOps, and GitLab",
    long_about = None,
    help_template = "\
{before-help}devopster {version} — {about}

Usage: devopster [OPTIONS] [COMMAND]

Run without a command to open the interactive launcher.

Commands:
{tab}login                         Authentication commands
{tab}init                          Create devopster-config.yaml and sign in

Advanced actions:
{tab}repo                          Repository operations
{tab}catalog                       Catalog generation
{tab}topics                        Topic alignment
{tab}stats                         Organization statistics

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
    pub command: Option<Commands>,
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
        Some(command) => run_command(command, &cli.config).await,
        None if io::stdin().is_terminal() && io::stdout().is_terminal() => {
            run_interactive_launcher(&cli.config).await
        }
        None => {
            let mut command = Cli::command();
            command.print_long_help()?;
            println!();
            Ok(())
        }
    }
}

async fn run_command(command: Commands, config_path: &str) -> Result<()> {
    match command {
        Commands::Login(command) => command.run().await,
        Commands::Init(command) => command.run(config_path).await,
        Commands::Repo(command) => command.run(config_path).await,
        Commands::Catalog(command) => command.run(config_path).await,
        Commands::Topics(command) => command.run(config_path).await,
        Commands::Stats(command) => command.run(config_path).await,
    }
}

async fn run_interactive_launcher(config_path: &str) -> Result<()> {
    loop {
        ui::hero(
            "devopster launcher",
            "Choose a task with the keyboard and devopster will guide you through it.",
        );
        ui::key_value("Config", config_path);
        ui::note("Direct commands still work any time, for example: devopster repo audit");

        let options = vec![
            menu_item("Set up configuration", "Create or refresh devopster-config.yaml"),
            menu_item("Sign in", "Connect GitHub, Azure DevOps, or GitLab"),
            menu_item("Manage repositories", "List, audit, fix, blueprint, or sync"),
            menu_item("Generate catalog", "Export catalog.json for your organization"),
            menu_item("Align topics", "Apply missing template topics"),
            menu_item("View statistics", "Check metadata coverage and compliance"),
            menu_item("Show help", "See the direct CLI command reference"),
            menu_item("Exit", "Leave the launcher"),
        ];

        match ui::select("Choose an action", &options, 0)? {
            0 => launch_init(config_path).await?,
            1 => launch_login().await?,
            2 => launch_repo(config_path).await?,
            3 => run_command(
                Commands::Catalog(catalog::CatalogCommand {
                    action: catalog::CatalogAction::Generate(catalog::GenerateCatalogCommand {}),
                }),
                config_path,
            )
            .await?,
            4 => run_command(
                Commands::Topics(topics::TopicsCommand {
                    action: topics::TopicsAction::Align(topics::AlignTopicsCommand {}),
                }),
                config_path,
            )
            .await?,
            5 => launch_stats(config_path).await?,
            6 => print_help()?,
            _ => break,
        }

        if !ui::prompt_confirm("Return to the main menu?", true)? {
            break;
        }
    }

    Ok(())
}

async fn launch_init(config_path: &str) -> Result<()> {
    ui::section("Configuration");
    ui::note("Start here if this is your first time using devopster.");
    let options = vec![
        menu_item("Guided setup", "Create config and sign in during setup"),
        menu_item("Config only", "Create config without the sign-in step"),
        menu_item("Back", "Return to the main launcher"),
    ];
    match ui::select("Init", &options, 0)? {
        0 => run_command(
            Commands::Init(init::InitCommand {
                output: config_path.to_string(),
                no_login: false,
            }),
            config_path,
        )
        .await,
        1 => run_command(
            Commands::Init(init::InitCommand {
                output: config_path.to_string(),
                no_login: true,
            }),
            config_path,
        )
        .await,
        _ => Ok(()),
    }
}

async fn launch_login() -> Result<()> {
    ui::section("Sign in");
    ui::note("Use this area to connect providers or review saved sign-in state.");
    let options = vec![
        menu_item("GitHub", "Sign in with the gh CLI"),
        menu_item("Azure DevOps", "Sign in with the az CLI"),
        menu_item("GitLab", "Sign in with the glab CLI"),
        menu_item("All providers", "Run all sign-in flows one after another"),
        menu_item("Login status", "Check whether providers are already signed in"),
        menu_item("Logout", "Remove saved credentials for one provider"),
        menu_item("Back", "Return to the main launcher"),
    ];

    let command = match ui::select("Login", &options, 0)? {
        0 => Some(login::LoginCommand {
            provider: login::LoginProvider::Github,
        }),
        1 => Some(login::LoginCommand {
            provider: login::LoginProvider::AzureDevops,
        }),
        2 => Some(login::LoginCommand {
            provider: login::LoginProvider::Gitlab,
        }),
        3 => Some(login::LoginCommand {
            provider: login::LoginProvider::All,
        }),
        4 => Some(login::LoginCommand {
            provider: login::LoginProvider::Status,
        }),
        5 => {
            let providers = vec![
                menu_item("github", "Remove GitHub credentials"),
                menu_item("azure_devops", "Remove Azure DevOps credentials"),
                menu_item("gitlab", "Remove GitLab credentials"),
            ];
            let selected = ui::select("Select provider to log out", &providers, 0)?;
            let provider = match selected {
                0 => "github",
                1 => "azure_devops",
                _ => "gitlab",
            };
            Some(login::LoginCommand {
                provider: login::LoginProvider::Logout(login::LogoutArgs {
                    provider: provider.to_string(),
                }),
            })
        }
        _ => None,
    };

    if let Some(command) = command {
        run_command(Commands::Login(command), "devopster-config.yaml").await?;
    }

    Ok(())
}

async fn launch_repo(config_path: &str) -> Result<()> {
    ui::section("Repository actions");
    ui::note("Pick the task you want to perform across repositories.");
    let options = vec![
        menu_item("List repositories", "Browse repositories, optionally by topic"),
        menu_item("Audit repositories", "Find missing metadata or branch drift"),
        menu_item("Fix repositories", "Interactively repair missing metadata"),
        menu_item("Create from blueprint", "Provision a new repository from a template"),
        menu_item("Sync shared files", "Push local or blueprint content to repositories"),
        menu_item("Back", "Return to the main launcher"),
    ];

    let command = match ui::select("Repository actions", &options, 0)? {
        0 => {
            let topic = ui::prompt_input("Topic filter (blank for all)")?;
            Some(repo::RepoCommand {
                action: repo::RepoAction::List(repo::ListReposCommand {
                    topic: if topic.trim().is_empty() {
                        None
                    } else {
                        Some(topic.trim().to_string())
                    },
                }),
            })
        }
        1 => Some(repo::RepoCommand {
            action: repo::RepoAction::Audit(repo::AuditReposCommand {}),
        }),
        2 => Some(repo::RepoCommand {
            action: repo::RepoAction::Fix(repo::FixReposCommand {}),
        }),
        3 => {
            ui::note("You will be asked for the new repository name and the template to use.");
            let name = prompt_required("Repository name")?;
            let template = prompt_required("Template name")?;
            let description = ui::prompt_input("Description (blank to use template)")?;
            let private = ui::prompt_confirm("Create as private repository?", false)?;
            Some(repo::RepoCommand {
                action: repo::RepoAction::Blueprint(repo::BlueprintRepoCommand {
                    name,
                    template,
                    description: if description.trim().is_empty() {
                        None
                    } else {
                        Some(description.trim().to_string())
                    },
                    private,
                }),
            })
        }
        4 => {
            let sync_options = vec![
                menu_item("Sync local files", "Use a local folder such as .github"),
                menu_item("Sync from blueprint", "Compare against the configured blueprint repo"),
            ];
            let sync_choice = ui::select("Sync mode", &sync_options, 0)?;
            let from_blueprint = sync_choice == 1;
            let source = if from_blueprint {
                ".github".to_string()
            } else {
                let source = ui::prompt_input("Source path [.github]")?;
                if source.trim().is_empty() {
                    ".github".to_string()
                } else {
                    source.trim().to_string()
                }
            };
            let template = ui::prompt_input("Template filter (blank for all repositories)")?;
            Some(repo::RepoCommand {
                action: repo::RepoAction::Sync(repo::SyncReposCommand {
                    source,
                    from_blueprint,
                    blueprint_repo: None,
                    blueprint_branch: None,
                    blueprint_path: Vec::new(),
                    template: if template.trim().is_empty() {
                        None
                    } else {
                        Some(template.trim().to_string())
                    },
                }),
            })
        }
        _ => None,
    };

    if let Some(command) = command {
        run_command(Commands::Repo(command), config_path).await?;
    }

    Ok(())
}

async fn launch_stats(config_path: &str) -> Result<()> {
    ui::section("Statistics");
    ui::note("View overall metadata health for the configured organization.");
    let scope_missing = ui::prompt_confirm(
        "Write non-compliant repositories into scoped_repos?",
        false,
    )?;
    run_command(
        Commands::Stats(stats::StatsCommand { scope_missing }),
        config_path,
    )
    .await
}

fn print_help() -> Result<()> {
    let mut command = Cli::command();
    command.print_long_help()?;
    println!();
    io::stdout().flush()?;
    Ok(())
}

fn prompt_required(label: &str) -> Result<String> {
    loop {
        let value = ui::prompt_input(label)?;
        if !value.trim().is_empty() {
            return Ok(value.trim().to_string());
        }
        ui::warn("This field is required.");
    }
}

fn menu_item(title: &str, description: &str) -> String {
    format!("{title}  {description}")
}
