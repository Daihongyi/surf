use crate::core::{benchmark_url, build_client, download_file, TimeoutError};
use crate::log::{init_logger, log_info, log_error, log_debug, log_warn};
use crate::config::{Config, Profile};
use crate::history::{RequestHistory, HistoryEntry};
use crate::response::{ResponseFormatter, ResponseAnalyzer};
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    time::Instant,
};

#[derive(Parser)]
#[command(name = "surf", version = "0.3.0", about = "A modern HTTP client with advanced features")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable logging to log.txt
    #[arg(long, global = true)]
    log: bool,

    /// Use configuration profile
    #[arg(long, global = true)]
    profile: Option<String>,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,
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

        /// Pretty print JSON responses
        #[arg(long)]
        json: bool,

        /// Analyze response headers
        #[arg(long)]
        analyze: bool,

        /// Save to history
        #[arg(long, default_value = "true")]
        save_history: bool,
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

    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// History management
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },

    /// Profile management
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Reset configuration to defaults
    Reset,
    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,
        /// Configuration value
        value: String,
    },
}

#[derive(Subcommand)]
enum HistoryAction {
    /// Show recent requests
    List {
        /// Number of entries to show
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },
    /// Search history
    Search {
        /// Search query
        query: String,
    },
    /// Show specific history entry
    Show {
        /// Entry ID
        id: String,
    },
    /// Clear all history
    Clear,
}

#[derive(Subcommand)]
enum ProfileAction {
    /// List all profiles
    List,
    /// Create or update a profile
    Create {
        /// Profile name
        name: String,
        /// Base URL
        #[arg(long)]
        base_url: Option<String>,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
        /// Follow redirects
        #[arg(long)]
        follow_redirects: bool,
    },
    /// Delete a profile
    Delete {
        /// Profile name
        name: String,
    },
    /// Show profile details
    Show {
        /// Profile name
        name: String,
    },
}

pub async fn execute() -> Result<()> {
    let args = Cli::parse();

    // Load configuration
    let config_path = Config::get_config_path();
    let mut config = Config::load_from_file(&config_path)?;

    // Apply profile if specified
    if let Some(profile_name) = &args.profile {
        if let Some(_profile) = config.get_profile(profile_name) {
            log_info(&format!("Using profile: {}", profile_name));
        } else {
            log_warn(&format!("Profile '{}' not found, using defaults", profile_name));
        }
    }

    // 根据命令类型确定日志目录
    let log_dir = if args.log {
        match &args.command {
            Commands::Download { output, .. } => {
                output.parent().map(|p| p.to_path_buf())
            }
            Commands::Get { output: Some(output), .. } => {
                output.parent().map(|p| p.to_path_buf())
            }
            _ => {
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
            json,
            analyze,
            save_history,
        } => {
            handle_get_request(
                &url, include, output, location, headers, connect_timeout,
                verbose, http3, json, analyze, save_history, &config, args.no_color
            ).await
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

            match download_file(&url, &output, parallel, continue_download, idle_timeout, http3).await {
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

        Commands::Config { action } => {
            handle_config_action(action, &mut config, &config_path).await
        }

        Commands::History { action } => {
            handle_history_action(action).await
        }

        Commands::Profile { action } => {
            handle_profile_action(action, &mut config, &config_path).await
        }
    }
}

async fn handle_get_request(
    url: &str,
    include: bool,
    output: Option<PathBuf>,
    location: bool,
    headers: Vec<String>,
    connect_timeout: u64,
    verbose: bool,
    http3: bool,
    json: bool,
    analyze: bool,
    save_history: bool,
    config: &Config,
    no_color: bool,
) -> Result<()> {
    log_info(&format!("GET request to: {}", url));
    log_debug(&format!("Parameters - include: {}, location: {}, timeout: {}s, verbose: {}, http3: {}",
                       include, location, connect_timeout, verbose, http3));

    let start_time = Instant::now();
    let mut request_headers = HashMap::new();

    // Merge config headers with command line headers
    let mut all_headers = config.default_headers.clone();
    for header in &headers {
        if let Some((key, value)) = header.split_once(':') {
            all_headers.insert(key.trim().to_string(), value.trim().to_string());
            request_headers.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    let header_vec: Vec<String> = all_headers
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect();

    if !headers.is_empty() {
        log_debug(&format!("Custom headers: {:?}", headers));
    }

    // Create history entry
    let mut history_entry = if save_history {
        Some(HistoryEntry::new("GET", url, request_headers))
    } else {
        None
    };

    let client = match build_client(location, connect_timeout, http3, header_vec) {
        Ok(client) => {
            log_debug("HTTP client built successfully");
            client
        }
        Err(e) => {
            log_error(&format!("Failed to build HTTP client: {}", e));
            if let Some(ref mut entry) = history_entry {
                *entry = entry.clone().with_error(e.to_string());
            }
            return Err(e);
        }
    };

    let response = match client.get(url).send().await {
        Ok(response) => {
            log_info(&format!("Received response with status: {}", response.status()));
            response
        }
        Err(e) => {
            log_error(&format!("Request failed: {}", e));
            if let Some(ref mut entry) = history_entry {
                *entry = entry.clone().with_error(e.to_string());
            }
            return Err(e.into());
        }
    };

    let response_time = start_time.elapsed().as_millis() as u64;
    let status = response.status();
    let version = response.version();
    let response_headers = response.headers().clone();

    // Response formatter
    let formatter = ResponseFormatter::new(!no_color, json, false);

    if verbose {
        println!("> {:?} {}", version, status);
        for (name, value) in response.headers() {
            println!("> {}: {}", name, value.to_str()?);
        }
        println!(">");
    }

    let content = response.text().await?;
    let content_size = content.len() as u64;

    log_info(&format!("Response content size: {} bytes", content.len()));

    // Update history entry
    if let Some(ref mut entry) = history_entry {
        *entry = entry.clone().with_response(status.as_u16(), response_time, content_size);

        // Save to history
        let history_path = RequestHistory::get_history_path();
        let mut history = RequestHistory::load_from_file(&history_path).unwrap_or_default();
        history.add_entry(entry.clone());
        let _ = history.save_to_file(&history_path);
    }

    if include {
        println!("{}", formatter.format_status_line(version, status));
        print!("{}", formatter.format_headers(&response_headers));
        println!();
    }

    // Analyze response if requested
    if analyze {
        let analysis = ResponseAnalyzer::analyze_headers(&response_headers);
        println!("=== Response Analysis ===");
        for (key, value) in analysis {
            println!("{}: {}", key, value);
        }
        println!("=== End Analysis ===\n");
    }

    let formatted_content = formatter.format_body(&content,
                                                  response_headers.get("content-type").and_then(|ct| ct.to_str().ok()));

    match output {
        Some(path) => {
            log_info(&format!("Saving output to file: {}", path.display()));
            match std::fs::File::create(&path) {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(formatted_content.as_bytes()) {
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
            println!("{}", formatted_content);
            log_debug("Response content printed to stdout");
        }
    }

    if verbose {
        println!("\n< {}", ResponseAnalyzer::get_response_summary(
            status, &response_headers, content.len(), response_time
        ));
    }

    log_info("GET request completed successfully");
    Ok(())
}

async fn handle_config_action(
    action: ConfigAction,
    config: &mut Config,
    config_path: &PathBuf,
) -> Result<()> {
    match action {
        ConfigAction::Show => {
            println!("Current configuration:");
            println!("Default timeout: {}s", config.default_timeout);
            println!("Default user agent: {}", config.default_user_agent);
            println!("Max redirects: {}", config.max_redirects);
            println!("Default headers:");
            for (key, value) in &config.default_headers {
                println!("  {}: {}", key, value);
            }
            println!("Profiles: {}", config.profiles.len());
            Ok(())
        }
        ConfigAction::Reset => {
            *config = Config::default();
            config.save_to_file(config_path)?;
            println!("Configuration reset to defaults");
            Ok(())
        }
        ConfigAction::Set { key, value } => {
            match key.as_str() {
                "timeout" => {
                    config.default_timeout = value.parse()?;
                    println!("Set default timeout to {}s", config.default_timeout);
                }
                "user_agent" => {
                    config.default_user_agent = value.clone();
                    config.default_headers.insert("User-Agent".to_string(), value);
                    println!("Set user agent to: {}", config.default_user_agent);
                }
                "max_redirects" => {
                    config.max_redirects = value.parse()?;
                    println!("Set max redirects to: {}", config.max_redirects);
                }
                _ => {
                    println!("Unknown configuration key: {}", key);
                    return Ok(());
                }
            }
            config.save_to_file(config_path)?;
            Ok(())
        }
    }
}

async fn handle_history_action(action: HistoryAction) -> Result<()> {
    let history_path = RequestHistory::get_history_path();
    let history = RequestHistory::load_from_file(&history_path).unwrap_or_default();

    match action {
        HistoryAction::List { limit } => {
            let entries = history.get_recent(limit);
            if entries.is_empty() {
                println!("No history entries found");
                return Ok(());
            }

            println!("Recent requests:");
            for entry in entries {
                let status_str = if let Some(status) = entry.status_code {
                    if entry.success {
                        format!("{} ✓", status)
                    } else {
                        format!("{} ✗", status)
                    }
                } else {
                    "Error".to_string()
                };

                let time_str = entry.response_time
                    .map(|t| format!("{}ms", t))
                    .unwrap_or_else(|| "N/A".to_string());

                println!("{} | {} {} | {} | {} | {}",
                         entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                         entry.method,
                         entry.url,
                         status_str,
                         time_str,
                         entry.id[..8].to_string()
                );
            }
            Ok(())
        }
        HistoryAction::Search { query } => {
            let results = history.search(&query);
            if results.is_empty() {
                println!("No matching history entries found");
                return Ok(());
            }

            println!("Search results for '{}':", query);
            for entry in results {
                println!("{} | {} {} | Status: {} | ID: {}",
                         entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
                         entry.method,
                         entry.url,
                         entry.status_code.map(|s| s.to_string()).unwrap_or_else(|| "Error".to_string()),
                         entry.id[..8].to_string()
                );
            }
            Ok(())
        }
        HistoryAction::Show { id } => {
            if let Some(entry) = history.get_by_id(&id) {
                println!("Request Details:");
                println!("ID: {}", entry.id);
                println!("Timestamp: {}", entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
                println!("Method: {}", entry.method);
                println!("URL: {}", entry.url);
                println!("Status: {}", entry.status_code.map(|s| s.to_string()).unwrap_or_else(|| "N/A".to_string()));
                println!("Response Time: {}ms", entry.response_time.unwrap_or(0));
                println!("Response Size: {} bytes", entry.response_size.unwrap_or(0));
                println!("Success: {}", entry.success);

                if !entry.headers.is_empty() {
                    println!("Headers:");
                    for (key, value) in &entry.headers {
                        println!("  {}: {}", key, value);
                    }
                }

                if let Some(ref error) = entry.error_message {
                    println!("Error: {}", error);
                }
            } else {
                println!("History entry not found: {}", id);
            }
            Ok(())
        }
        HistoryAction::Clear => {
            let history_path = RequestHistory::get_history_path();
            let mut history = RequestHistory::load_from_file(&history_path).unwrap_or_default();
            history.clear();
            history.save_to_file(&history_path)?;
            println!("History cleared");
            Ok(())
        }
    }
}

async fn handle_profile_action(
    action: ProfileAction,
    config: &mut Config,
    config_path: &PathBuf,
) -> Result<()> {
    match action {
        ProfileAction::List => {
            if config.profiles.is_empty() {
                println!("No profiles configured");
                return Ok(());
            }

            println!("Available profiles:");
            for (name, profile) in &config.profiles {
                println!("  {} - {}", name, profile.base_url.as_ref().unwrap_or(&"No base URL".to_string()));
            }
            Ok(())
        }
        ProfileAction::Create { name, base_url, timeout, follow_redirects } => {
            let profile = Profile {
                name: name.clone(),
                base_url,
                headers: HashMap::new(),
                timeout,
                follow_redirects,
            };

            config.add_profile(profile);
            config.save_to_file(config_path)?;
            println!("Profile '{}' created", name);
            Ok(())
        }
        ProfileAction::Delete { name } => {
            if config.remove_profile(&name) {
                config.save_to_file(config_path)?;
                println!("Profile '{}' deleted", name);
            } else {
                println!("Profile '{}' not found", name);
            }
            Ok(())
        }
        ProfileAction::Show { name } => {
            if let Some(profile) = config.get_profile(&name) {
                println!("Profile: {}", profile.name);
                println!("Base URL: {}", profile.base_url.as_ref().unwrap_or(&"None".to_string()));
                println!("Timeout: {}s", profile.timeout.unwrap_or(config.default_timeout));
                println!("Follow redirects: {}", profile.follow_redirects);

                if !profile.headers.is_empty() {
                    println!("Headers:");
                    for (key, value) in &profile.headers {
                        println!("  {}: {}", key, value);
                    }
                }
            } else {
                println!("Profile '{}' not found", name);
            }
            Ok(())
        }
    }
}