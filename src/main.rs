mod cli;
mod core;
mod log;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::execute().await
}