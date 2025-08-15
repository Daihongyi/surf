mod cli;
mod core;
mod log;
mod config;
mod history;
mod response;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::execute().await
}