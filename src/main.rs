mod cli;
mod core;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::execute().await
}