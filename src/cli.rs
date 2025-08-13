use crate::core::{benchmark_url, build_client, download_file, TimeoutError};
use crate::log::{init_logger, log_info, log_error, log_debug, log_warn};
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::HumanBytes;
use std::{
    io::Write,
    path::PathBuf,
};

#[derive(Parser)]
#[command(name = "surf", version = "0.2.0", about = "A modern HTTP client with advanced features")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable logging to log.txt
    #[arg(long, global = true)]
    log: bool,
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

        /// Use HTTP/3 (experimental)
        #[arg(long)]
        http3: bool,
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

        /// Use HTTP/3 (experimental)
        #[arg(long)]
        http3: bool,
    },
}

pub async fn execute() -> Result<()> {
    let args = Cli::parse();

    // 根据命令类型确定日志目录
    let log_dir = if args.log {
        match &args.command {
            Commands::Download { output, .. } => {
                // 对于下载命令，使用输出文件的目录
                output.parent().map(|p| p.to_path_buf())
            }
            Commands::Get { output: Some(output), .. } => {
                // 对于带输出文件的GET命令，使用输出文件的目录
                output.parent().map(|p| p.to_path_buf())
            }
            _ => {
                // 对于其他命令，使用当前目录
                Some(PathBuf::from("."))
            }
        }
    } else {
        None
    };

    // Initialize logger
    init_logger(args.log, log_dir).await?;

    if args.log {
        log_info("Starting surf application");
    }

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
            log_info(&format!("GET request to: {}", url));
            log_debug(&format!("Parameters - include: {}, location: {}, timeout: {}s, verbose: {}, http3: {}",
                               include, location, connect_timeout, verbose, http3));

            if !headers.is_empty() {
                log_debug(&format!("Custom headers: {:?}", headers));
            }

            let client = match build_client(location, connect_timeout, http3, headers) {
                Ok(client) => {
                    log_debug("HTTP client built successfully");
                    client
                }
                Err(e) => {
                    log_error(&format!("Failed to build HTTP client: {}", e));
                    return Err(e);
                }
            };

            let response = match client.get(&url).send().await {
                Ok(response) => {
                    log_info(&format!("Received response with status: {}", response.status()));
                    response
                }
                Err(e) => {
                    log_error(&format!("Request failed: {}", e));
                    return Err(e.into());
                }
            };

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
            log_info(&format!("Response content size: {} bytes", content.len()));

            if include {
                println!("HTTP/{:?} {}", version, status);
                for (name, value) in &response_headers {
                    println!("{}: {}", name, value.to_str()?);
                }
                println!();
            }

            match output {
                Some(path) => {
                    log_info(&format!("Saving output to file: {}", path.display()));
                    match std::fs::File::create(&path) {
                        Ok(mut file) => {
                            if let Err(e) = file.write_all(content.as_bytes()) {
                                log_error(&format!("Failed to write to file {}: {}", path.display(), e));
                                return Err(e.into());
                            }
                            log_info("File saved successfully");
                        }
                        Err(e) => {
                            log_error(&format!("Failed to create file {}: {}", path.display(), e));
                            return Err(e.into());
                        }
                    }
                }
                None => {
                    println!("{}", content);
                    log_debug("Response content printed to stdout");
                }
            }

            if verbose {
                println!("\n< Response size: {}", HumanBytes(content.len() as u64));
            }

            log_info("GET request completed successfully");
            Ok(())
        }

        Commands::Download {
            url,
            output,
            parallel,
            continue_download,
            idle_timeout,
            http3,
        } => {
            log_info(&format!("Starting download from: {}", url));
            log_debug(&format!("Download parameters - output: {}, parallel: {}, continue: {}, timeout: {}s, http3: {}",
                               output.display(), parallel, continue_download, idle_timeout, http3));

            match download_file(&url, &output, parallel, continue_download, idle_timeout, http3).await
            {
                Ok(_) => {
                    log_info("Download completed successfully");
                    Ok(())
                }
                Err(e) => {
                    if let Some(timeout_err) = e.downcast_ref::<TimeoutError>() {
                        log_error(&format!("Download failed with timeout: {}", timeout_err));
                        eprintln!("Download failed: {}", timeout_err);
                    } else {
                        log_error(&format!("Download failed: {}", e));
                        eprintln!("Download failed: {}", e);
                    }
                    Err(e)
                }
            }
        }

        Commands::Bench {
            url,
            requests,
            concurrency,
            connect_timeout,
            http3,
        } => {
            log_info(&format!("Starting benchmark for: {}", url));
            log_debug(&format!("Benchmark parameters - requests: {}, concurrency: {}, timeout: {}s, http3: {}",
                               requests, concurrency, connect_timeout, http3));

            match benchmark_url(&url, requests, concurrency, connect_timeout, http3).await {
                Ok(_) => {
                    log_info("Benchmark completed successfully");
                    Ok(())
                }
                Err(e) => {
                    log_error(&format!("Benchmark failed: {}", e));
                    Err(e)
                }
            }
        }
    }
}