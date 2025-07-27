use crate::core::{benchmark_url, build_client, download_file};
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::HumanBytes;
use std::{
    io::Write,  // 添加 Write trait 导入
    path::PathBuf,
};

#[derive(Parser)]
#[command(name = "surf", version = "0.1.0", about = "A modern HTTP client with advanced features")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch a URL and display the response
    Get {
        /// URL to fetch
        url: String,

        /// Include response headers in output
        #[arg(short = 'i', long)]
        include: bool,

        /// Save output to file
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,

        /// Follow redirects
        #[arg(short = 'L', long)]
        location: bool,

        /// Set custom headers (e.g., "Authorization: Bearer token")
        #[arg(short = 'H', long)]
        headers: Vec<String>,

        /// Timeout in seconds
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,

        /// Display verbose output
        #[arg(short = 'v', long)]
        verbose: bool,

        /// Use HTTP/3 (experimental)
        #[arg(long)]
        http3: bool,
    },

    /// Download a file with progress display and resumable transfers
    Download {
        /// URL to download
        url: String,

        /// Output file name
        output: PathBuf,

        /// Number of parallel connections
        #[arg(short = 'p', long, default_value = "4")]
        parallel: usize,

        /// Continue interrupted download
        #[arg(short = 'c', long)]
        continue_download: bool,

        /// Timeout in seconds
        #[arg(short = 't', long, default_value = "30")]
        timeout: u64,
    },

    /// Benchmark a URL by sending multiple requests
    Bench {
        /// URL to benchmark
        url: String,

        /// Number of requests to send
        #[arg(short = 'n', long, default_value = "100")]
        requests: usize,

        /// Number of concurrent connections
        #[arg(short = 'c', long, default_value = "10")]
        concurrency: usize,

        /// Timeout in seconds
        #[arg(short = 't', long, default_value = "5")]
        timeout: u64,
    },
}

pub async fn execute() -> Result<()> {
    let args = Cli::parse();

    match args.command {
        Commands::Get {
            url,
            include,
            output,
            location,
            headers,
            timeout,
            verbose,
            http3,
        } => {
            let client = build_client(location, timeout, http3, headers)?;
            let response = client.get(&url).send().await?;

            // 在移动 response 前保存必要信息
            let status = response.status();
            let version = response.version();
            let response_headers = response.headers().clone();

            if verbose {
                println!("> {:?} {}", version, status);
                for (name, value) in response.headers() {
                    println!("> {}: {}", name, value.to_str()?);
                }
                println!(">");
            }

            let content = response.text().await?;

            if include {
                println!("HTTP/{:?} {}", version, status);
                for (name, value) in &response_headers {
                    println!("{}: {}", name, value.to_str()?);
                }
                println!();
            }

            match output {
                Some(path) => {
                    let mut file = std::fs::File::create(path)?;
                    file.write_all(content.as_bytes())?; // 现在可以正常调用 write_all
                }
                None => println!("{}", content),
            }

            if verbose {
                println!("\n< Response size: {}", HumanBytes(content.len() as u64));
            }

            Ok(())
        }

        Commands::Download {
            url,
            output,
            parallel,
            continue_download,
            timeout,
        } => {
            download_file(&url, &output, parallel, continue_download, timeout).await
        }

        Commands::Bench {
            url,
            requests,
            concurrency,
            timeout,
        } => {
            benchmark_url(&url, requests, concurrency, timeout).await
        }
    }
}