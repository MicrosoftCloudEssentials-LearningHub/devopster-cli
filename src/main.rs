mod auth;
mod cli;
mod config;
mod provider;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
