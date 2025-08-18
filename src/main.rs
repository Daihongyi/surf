mod cli;
mod core;
mod log;
mod config;
mod history;
mod response;
mod cache;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::execute().await
}