use crate::core::{benchmark_url, build_client, download_file, TimeoutError};
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::HumanBytes;
use std::{
    io::Write,
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

        /// Connection timeout in seconds
        #[arg(short = 't', long, default_value = "10")]
        connect_timeout: u64,

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

        /// Idle timeout in seconds (time between two packets)
        #[arg(short = 't', long, default_value = "30")]
        idle_timeout: u64,
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

        /// Connection timeout in seconds
        #[arg(short = 't', long, default_value = "5")]
        connect_timeout: u64,
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
            connect_timeout,
            verbose,
            http3,
        } => {
            let client = build_client(location, connect_timeout, http3, headers)?;
            let response = client.get(&url).send().await?;

            // Save necessary info before moving response
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
                    file.write_all(content.as_bytes())?;
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
            idle_timeout,
        } => match download_file(&url, &output, parallel, continue_download, idle_timeout).await {
            Ok(_) => Ok(()),
            Err(e) => {
                if let Some(timeout_err) = e.downcast_ref::<TimeoutError>() {
                    eprintln!("Download failed: {}", timeout_err);
                } else {
                    eprintln!("Download failed: {}", e);
                }
                Err(e)
            }
        },

        Commands::Bench {
            url,
            requests,
            concurrency,
            connect_timeout,
        } => benchmark_url(&url, requests, concurrency, connect_timeout).await,
    }
}