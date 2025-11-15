use crate::core::{benchmark_url, build_client, download_file, TimeoutError};
use crate::log::{init_logger, log_info, log_error, log_debug, log_warn};
use crate::config::{Config, Profile};
use crate::history::{RequestHistory, HistoryEntry};
use crate::response::{ResponseFormatter, ResponseAnalyzer};
use crate::cache::CachedConfig;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    time::Instant,
};

#[derive(Parser)]
#[command(name = "surf", version = "0.3.8", about = "A modern HTTP client like curl with advanced features")]
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

    /// Use cached configuration from last run
    #[arg(short = 'x', long, global = true)]
    use_cache: bool,

    /// Do not save configuration to cache
    #[arg(long, global = true)]
    no_save: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Play a hidden snake game (Easter egg! 🎮)
    Play,

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

    /// Cache management
    Cache {
        #[command(subcommand)]
        action: CacheAction,
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

#[derive(Subcommand)]
enum CacheAction {
    /// Show cached configuration
    Show,
    /// Clear cached configuration
    Clear,
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
        Commands::Play => {
            // 隐藏的彩蛋游戏
            println!("\n Welcome to SURF Snake Game!");
            println!("Get ready to play...\n");
            std::thread::sleep(std::time::Duration::from_millis(500));
            crate::game::run_game().await
        }

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
            handle_get_request_with_cache(
                &url, include, output, location, headers, connect_timeout,
                verbose, http3, json, analyze, save_history, &config, args.no_color,
                args.use_cache, args.no_save, args.profile
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
            handle_download_with_cache(
                &url, output, parallel, continue_download, idle_timeout, http3,
                args.no_color, args.use_cache, args.no_save, args.profile
            ).await
        }

        Commands::Bench {
            url,
            requests,
            concurrency,
            connect_timeout,
            http3,
        } => {
            handle_benchmark_with_cache(
                &url, requests, concurrency, connect_timeout, http3,
                args.no_color, args.use_cache, args.no_save, args.profile
            ).await
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

        Commands::Cache { action } => {
            handle_cache_action(action).await
        }
    }
}

// ... 其余的函数保持不变 ...
async fn handle_get_request_with_cache(
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
    use_cache: bool,
    no_save: bool,
    profile: Option<String>,
) -> Result<()> {
    let cache_path = CachedConfig::get_cache_path();
    let mut cached_config = CachedConfig::load_from_file(&cache_path)?;

    if use_cache {
        if cached_config.is_empty() {
            eprintln!("Error: No cached configuration found. Please run a command without -x first to create a cache.");
            return Ok(());
        }

        // 检查是否有用户提供的参数与缓存冲突
        let provided_include = if include { Some(include) } else { None };
        let provided_location = if location { Some(location) } else { None };
        let provided_headers = if !headers.is_empty() { Some(headers.clone()) } else { None };
        let provided_connect_timeout = Some(connect_timeout).filter(|&t| t != 10); // 10是默认值
        let provided_verbose = if verbose { Some(verbose) } else { None };
        let provided_http3 = if http3 { Some(http3) } else { None };
        let provided_json = if json { Some(json) } else { None };
        let provided_analyze = if analyze { Some(analyze) } else { None };
        let provided_save_history = Some(save_history).filter(|&s| s != true); // true是默认值

        let conflicts = cached_config.detect_conflicts_get(
            provided_include,
            provided_location,
            &provided_headers,
            provided_connect_timeout,
            provided_verbose,
            provided_http3,
            provided_json,
            provided_analyze,
            provided_save_history,
        );

        if !conflicts.is_empty() {
            eprintln!("Error: Configuration conflicts detected when using cache:");
            for conflict in conflicts {
                eprintln!("  - {}", conflict);
            }
            eprintln!("Please resolve conflicts or run without -x to override cache.");
            return Ok(());
        }

        // 合并配置
        let (merged_include, merged_location, merged_headers, merged_connect_timeout,
            merged_verbose, merged_http3, merged_json, merged_analyze, merged_save_history) =
            cached_config.merge_get_config(
                provided_include,
                provided_location,
                provided_headers.clone(),
                provided_connect_timeout,
                provided_verbose,
                provided_http3,
                provided_json,
                provided_analyze,
                provided_save_history,
            );

        // 如果有新参数,更新并保存缓存
        let has_new_params = provided_include.is_some() || provided_location.is_some() ||
            provided_headers.is_some() || provided_connect_timeout.is_some() ||
            provided_verbose.is_some() || provided_http3.is_some() ||
            provided_json.is_some() || provided_analyze.is_some() ||
            provided_save_history.is_some();

        if has_new_params {
            cached_config.update_with_get(
                merged_include, merged_location, merged_headers.clone(), merged_connect_timeout,
                merged_verbose, merged_http3, merged_json, merged_analyze, merged_save_history,
                no_color, profile.clone()
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Updated cache with new parameters");
        }

        log_info("Using cached configuration for GET request");
        handle_get_request(
            url, merged_include, output, merged_location, merged_headers, merged_connect_timeout,
            merged_verbose, merged_http3, merged_json, merged_analyze, merged_save_history,
            config, no_color
        ).await
    } else {
        // 正常执行,不使用缓存
        let result = handle_get_request(
            url, include, output.clone(), location, headers.clone(), connect_timeout,
            verbose, http3, json, analyze, save_history, config, no_color
        ).await;

        // 保存配置到缓存(除非禁用保存)
        if !no_save && result.is_ok() {
            cached_config.update_with_get(
                include, location, headers, connect_timeout, verbose, http3,
                json, analyze, save_history, no_color, profile
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Configuration saved to cache");
        }

        result
    }
}

async fn handle_download_with_cache(
    url: &str,
    output: PathBuf,
    parallel: usize,
    continue_download: bool,
    idle_timeout: u64,
    http3: bool,
    no_color: bool,
    use_cache: bool,
    no_save: bool,
    profile: Option<String>,
) -> Result<()> {
    let cache_path = CachedConfig::get_cache_path();
    let mut cached_config = CachedConfig::load_from_file(&cache_path)?;

    if use_cache {
        if cached_config.is_empty() {
            eprintln!("Error: No cached configuration found. Please run a command without -x first to create a cache.");
            return Ok(());
        }

        // 检查冲突
        let provided_parallel = Some(parallel).filter(|&p| p != 4); // 4是默认值
        let provided_continue = if continue_download { Some(continue_download) } else { None };
        let provided_idle_timeout = Some(idle_timeout).filter(|&t| t != 30); // 30是默认值
        let provided_http3 = if http3 { Some(http3) } else { None };

        let conflicts = cached_config.detect_conflicts_download(
            provided_parallel,
            provided_continue,
            provided_idle_timeout,
            provided_http3,
        );

        if !conflicts.is_empty() {
            eprintln!("Error: Configuration conflicts detected when using cache:");
            for conflict in conflicts {
                eprintln!("  - {}", conflict);
            }
            eprintln!("Please resolve conflicts or run without -x to override cache.");
            return Ok(());
        }

        // 合并配置
        let (merged_parallel, merged_continue, merged_idle_timeout, merged_http3) =
            cached_config.merge_download_config(
                provided_parallel,
                provided_continue,
                provided_idle_timeout,
                provided_http3,
            );

        // 如果有新参数,更新并保存缓存
        let has_new_params = provided_parallel.is_some() || provided_continue.is_some() ||
            provided_idle_timeout.is_some() || provided_http3.is_some();

        if has_new_params {
            cached_config.update_with_download(
                merged_parallel, merged_continue, merged_idle_timeout, merged_http3,
                no_color, profile.clone()
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Updated cache with new parameters");
        }

        log_info("Using cached configuration for download");
        log_info(&format!("Starting download from: {}", url));
        log_debug(&format!("Download parameters - output: {}, parallel: {}, continue: {}, timeout: {}s, http3: {}",
                           output.display(), merged_parallel, merged_continue, merged_idle_timeout, merged_http3));

        match download_file(url, &output, merged_parallel, merged_continue, merged_idle_timeout, merged_http3).await {
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
    } else {
        // 正常执行
        log_info(&format!("Starting download from: {}", url));
        log_debug(&format!("Download parameters - output: {}, parallel: {}, continue: {}, timeout: {}s, http3: {}",
                           output.display(), parallel, continue_download, idle_timeout, http3));

        let result = match download_file(url, &output, parallel, continue_download, idle_timeout, http3).await {
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
        };

        // 保存配置到缓存
        if !no_save && result.is_ok() {
            cached_config.update_with_download(
                parallel, continue_download, idle_timeout, http3, no_color, profile
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Configuration saved to cache");
        }

        result
    }
}

async fn handle_benchmark_with_cache(
    url: &str,
    requests: usize,
    concurrency: usize,
    connect_timeout: u64,
    http3: bool,
    no_color: bool,
    use_cache: bool,
    no_save: bool,
    profile: Option<String>,
) -> Result<()> {
    let cache_path = CachedConfig::get_cache_path();
    let mut cached_config = CachedConfig::load_from_file(&cache_path)?;

    if use_cache {
        if cached_config.is_empty() {
            eprintln!("Error: No cached configuration found. Please run a command without -x first to create a cache.");
            return Ok(());
        }

        // 检查冲突
        let provided_requests = Some(requests).filter(|&r| r != 100); // 100是默认值
        let provided_concurrency = Some(concurrency).filter(|&c| c != 10); // 10是默认值
        let provided_connect_timeout = Some(connect_timeout).filter(|&t| t != 5); // 5是默认值
        let provided_http3 = if http3 { Some(http3) } else { None };

        let conflicts = cached_config.detect_conflicts_bench(
            provided_requests,
            provided_concurrency,
            provided_connect_timeout,
            provided_http3,
        );

        if !conflicts.is_empty() {
            eprintln!("Error: Configuration conflicts detected when using cache:");
            for conflict in conflicts {
                eprintln!("  - {}", conflict);
            }
            eprintln!("Please resolve conflicts or run without -x to override cache.");
            return Ok(());
        }

        // 合并配置
        let (merged_requests, merged_concurrency, merged_connect_timeout, merged_http3) =
            cached_config.merge_bench_config(
                provided_requests,
                provided_concurrency,
                provided_connect_timeout,
                provided_http3,
            );

        // 如果有新参数,更新并保存缓存
        let has_new_params = provided_requests.is_some() || provided_concurrency.is_some() ||
            provided_connect_timeout.is_some() || provided_http3.is_some();

        if has_new_params {
            cached_config.update_with_bench(
                merged_requests, merged_concurrency, merged_connect_timeout, merged_http3,
                no_color, profile.clone()
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Updated cache with new parameters");
        }

        log_info("Using cached configuration for benchmark");
        log_info(&format!("Starting benchmark for: {}", url));
        log_debug(&format!("Benchmark parameters - requests: {}, concurrency: {}, timeout: {}s, http3: {}",
                           merged_requests, merged_concurrency, merged_connect_timeout, merged_http3));

        match benchmark_url(url, merged_requests, merged_concurrency, merged_connect_timeout, merged_http3).await {
            Ok(_) => {
                log_info("Benchmark completed successfully");
                Ok(())
            }
            Err(e) => {
                log_error(&format!("Benchmark failed: {}", e));
                Err(e)
            }
        }
    } else {
        // 正常执行
        log_info(&format!("Starting benchmark for: {}", url));
        log_debug(&format!("Benchmark parameters - requests: {}, concurrency: {}, timeout: {}s, http3: {}",
                           requests, concurrency, connect_timeout, http3));

        let result = match benchmark_url(url, requests, concurrency, connect_timeout, http3).await {
            Ok(_) => {
                log_info("Benchmark completed successfully");
                Ok(())
            }
            Err(e) => {
                log_error(&format!("Benchmark failed: {}", e));
                Err(e)
            }
        };

        // 保存配置到缓存
        if !no_save && result.is_ok() {
            cached_config.update_with_bench(
                requests, concurrency, connect_timeout, http3, no_color, profile
            );
            cached_config.save_to_file(&cache_path)?;
            log_info("Configuration saved to cache");
        }

        result
    }
}

async fn handle_cache_action(action: CacheAction) -> Result<()> {
    let cache_path = CachedConfig::get_cache_path();

    match action {
        CacheAction::Show => {
            let cached_config = CachedConfig::load_from_file(&cache_path)?;
            println!("{}", cached_config.display_cached_config());
            Ok(())
        }
        CacheAction::Clear => {
            if cache_path.exists() {
                std::fs::remove_file(&cache_path)?;
                println!("Cached configuration cleared");
            } else {
                println!("No cached configuration found");
            }
            Ok(())
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