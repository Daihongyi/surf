use crate::core::{benchmark_url, build_client, download_file, TimeoutError, ClientType};
use crate::log::{init_logger, log_info, log_error, log_debug, log_warn};
use crate::config::{Config, Profile};
use crate::history::{RequestHistory, HistoryEntry};
use crate::response::{ResponseFormatter, ResponseAnalyzer};
use crate::cache::CachedConfig;
use crate::traits::{CacheableAction, ConfigActionHandler, SimpleActionHandler, GlobalContext};
use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use indicatif::HumanBytes;
use std::{collections::HashMap, io::Write, path::PathBuf, time::Instant};
use async_trait::async_trait;

#[derive(Parser)]
#[command(name = "surf", version = "0.5.1-A", about = "A modern HTTP client like curl with advanced features,build with rust")]
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
        url: String,
        #[arg(short = 'i', long)]
        include: bool,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        #[arg(short = 'L', long)]
        location: bool,
        #[arg(short = 'H', long)]
        headers: Vec<String>,
        #[arg(short = 't', long, default_value = "10")]
        connect_timeout: u64,
        #[arg(short = 'v', long)]
        verbose: bool,
        #[arg(long)]
        http3: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        analyze: bool,
        #[arg(long, default_value = "true")]
        save_history: bool,
    },
    /// Download a file with progress display and resumable transfers
    Download {
        url: String,
        output: PathBuf,
        #[arg(short = 'p', long, default_value = "4")]
        parallel: usize,
        #[arg(short = 'c', long)]
        continue_download: bool,
        #[arg(short = 't', long, default_value = "30")]
        idle_timeout: u64,
        #[arg(long)]
        http3: bool,
        /// Verify the downloaded file against a SHA-256 hash
        #[arg(long, value_name = "SHA256")]
        hash_check: Option<String>,
    },
    /// Benchmark a URL by sending multiple requests
    Bench {
        url: String,
        #[arg(short = 'n', long, default_value = "100")]
        requests: usize,
        #[arg(short = 'c', long, default_value = "10")]
        concurrency: usize,
        #[arg(short = 't', long, default_value = "5")]
        connect_timeout: u64,
        #[arg(long)]
        http3: bool,
    },
    /// Configuration management
    Config { #[command(subcommand)] action: ConfigAction },
    /// History management
    History { #[command(subcommand)] action: HistoryAction },
    /// Profile management
    Profile { #[command(subcommand)] action: ProfileAction },
    /// Cache management
    Cache { #[command(subcommand)] action: CacheAction },
    /// Resume/download management
    Resume { #[command(subcommand)] action: ResumeAction },
}

#[derive(Subcommand, Clone)]
enum ConfigAction { Show, Reset, Set { key: String, value: String } }
#[derive(Subcommand, Clone)]
enum HistoryAction { List { #[arg(short = 'n', long, default_value = "10")] limit: usize }, Search { query: String }, Show { id: String }, Clear }
#[derive(Subcommand, Clone)]
enum ProfileAction { List, Create { name: String, #[arg(long)] base_url: Option<String>, #[arg(long)] timeout: Option<u64>, #[arg(long)] follow_redirects: bool }, Delete { name: String }, Show { name: String } }
#[derive(Subcommand, Clone)]
enum CacheAction { Show, Clear }
#[derive(Subcommand, Clone)]
enum ResumeAction { List, Show { url_or_hash: String }, Resume { url: String, #[arg(short = 'o', long)] output: Option<PathBuf>, #[arg(short = 't', long, default_value = "30")] idle_timeout: u64, #[arg(long)] http3: bool }, Cleanup { #[arg(short = 'd', long, default_value = "7")] days: u64 }, Delete { url_or_hash: String } }

/// 安全截断字符串，确保不切断 UTF-8 字符
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

pub async fn execute() -> Result<()> {
    let args = Cli::parse();
    let config_path = Config::get_config_path();
    let mut config = Config::load_from_file(&config_path)?;

    if let Some(profile_name) = &args.profile {
        if let Some(_profile) = config.get_profile(profile_name) {
            log_info(&format!("Using profile: {}", profile_name));
        } else {
            log_warn(&format!("Profile '{}' not found, using defaults", profile_name));
        }
    }

    let log_dir = if args.log {
        match &args.command {
            Commands::Download { output, .. } => output.parent().map(|p| p.to_path_buf()),
            Commands::Get { output: Some(output), .. } => output.parent().map(|p| p.to_path_buf()),
            _ => Some(PathBuf::from(".")),
        }
    } else { None };

    init_logger(args.log, log_dir).await?;
    if args.log { log_info("Starting surf application"); }

    let ctx = GlobalContext {
        no_color: args.no_color,
        use_cache: args.use_cache,
        no_save: args.no_save,
        profile: args.profile.clone(),
        config: config.clone(),
    };

    match args.command {
        Commands::Play => {
            println!("\n🎮 Welcome to SURF Snake Game!");
            println!("Get ready to play...\n");
            std::thread::sleep(std::time::Duration::from_millis(500));
            crate::game::run_game().await
        }
        Commands::Get { url, include, output, location, headers, connect_timeout, verbose, http3, json, analyze, save_history } => {
            let cmd = GetCommand {
                url,
                output,
                args: GetArgs {
                    include: if include { Some(include) } else { None },
                    location: if location { Some(location) } else { None },
                    headers: if !headers.is_empty() { Some(headers) } else { None },
                    connect_timeout: Some(connect_timeout).filter(|&t| t != 10),
                    verbose: if verbose { Some(verbose) } else { None },
                    http3: if http3 { Some(http3) } else { None },
                    json: if json { Some(json) } else { None },
                    analyze: if analyze { Some(analyze) } else { None },
                    save_history: Some(save_history).filter(|&s| s != true),
                },
            };
            cmd.execute(&ctx).await
        }
        Commands::Download { url, output, parallel, continue_download, idle_timeout, http3, hash_check } => {
            let cmd = DownloadCommand {
                url, output,
                args: DownloadArgs {
                    parallel: Some(parallel).filter(|&p| p != 4),
                    continue_download: if continue_download { Some(continue_download) } else { None },
                    idle_timeout: Some(idle_timeout).filter(|&t| t != 30),
                    http3: if http3 { Some(http3) } else { None },
                },
                hash_check,
            };
            cmd.execute(&ctx).await
        }
        Commands::Bench { url, requests, concurrency, connect_timeout, http3 } => {
            let cmd = BenchCommand {
                url,
                args: BenchArgs {
                    requests: Some(requests).filter(|&r| r != 100),
                    concurrency: Some(concurrency).filter(|&c| c != 10),
                    connect_timeout: Some(connect_timeout).filter(|&t| t != 5),
                    http3: if http3 { Some(http3) } else { None },
                },
            };
            cmd.execute(&ctx).await
        }
        Commands::Config { action } => action.handle(&mut config, &config_path).await,
        Commands::History { action } => action.handle().await,
        Commands::Profile { action } => action.handle(&mut config, &config_path).await,
        Commands::Cache { action } => action.handle().await,
        Commands::Resume { action } => action.handle().await,
    }
}

// ================= Traits Impls for Cacheable Commands =================

#[derive(Clone)]
struct GetArgs {
    include: Option<bool>, location: Option<bool>, headers: Option<Vec<String>>,
    connect_timeout: Option<u64>, verbose: Option<bool>, http3: Option<bool>,
    json: Option<bool>, analyze: Option<bool>, save_history: Option<bool>,
}

struct GetCommand { url: String, output: Option<PathBuf>, args: GetArgs }

#[async_trait]
impl CacheableAction for GetCommand {
    type Args = GetArgs;
    type Merged = (bool, bool, Vec<String>, u64, bool, bool, bool, bool, bool);

    fn get_provided_args(&self) -> Self::Args { self.args.clone() }

    fn detect_conflicts(&self, cache: &CachedConfig, args: &Self::Args) -> Vec<String> {
        cache.detect_conflicts_get(args.include, args.location, &args.headers, args.connect_timeout, args.verbose, args.http3, args.json, args.analyze, args.save_history)
    }

    fn merge_config(&self, cache: &CachedConfig, args: &Self::Args) -> Self::Merged {
        cache.merge_get_config(args.include, args.location, args.headers.clone(), args.connect_timeout, args.verbose, args.http3, args.json, args.analyze, args.save_history)
    }

    fn has_new_params(&self, args: &Self::Args) -> bool {
        args.include.is_some() || args.location.is_some() || args.headers.is_some() || args.connect_timeout.is_some() ||
            args.verbose.is_some() || args.http3.is_some() || args.json.is_some() || args.analyze.is_some() || args.save_history.is_some()
    }

    fn update_cache(&self, cache: &mut CachedConfig, args: &Self::Args, ctx: &GlobalContext) {
        cache.update_with_get(args.include.unwrap_or(false), args.location.unwrap_or(false), args.headers.clone().unwrap_or_default(), args.connect_timeout.unwrap_or(10), args.verbose.unwrap_or(false), args.http3.unwrap_or(false), args.json.unwrap_or(false), args.analyze.unwrap_or(false), args.save_history.unwrap_or(true), ctx.no_color, ctx.profile.clone());
    }

    async fn run(&self, merged: Self::Merged, ctx: &GlobalContext) -> Result<()> {
        let (include, location, headers, connect_timeout, verbose, http3, json, analyze, save_history) = merged;
        handle_get_request(&self.url, include, self.output.clone(), location, headers, connect_timeout, verbose, http3, json, analyze, save_history, &ctx.config, ctx.no_color).await
    }
}

#[derive(Clone)]
struct DownloadArgs { parallel: Option<usize>, continue_download: Option<bool>, idle_timeout: Option<u64>, http3: Option<bool> }
struct DownloadCommand { url: String, output: PathBuf, args: DownloadArgs, hash_check: Option<String> }

#[async_trait]
impl CacheableAction for DownloadCommand {
    type Args = DownloadArgs;
    type Merged = (usize, bool, u64, bool);

    fn get_provided_args(&self) -> Self::Args { self.args.clone() }
    fn detect_conflicts(&self, cache: &CachedConfig, args: &Self::Args) -> Vec<String> {
        cache.detect_conflicts_download(args.parallel, args.continue_download, args.idle_timeout, args.http3)
    }
    fn merge_config(&self, cache: &CachedConfig, args: &Self::Args) -> Self::Merged {
        cache.merge_download_config(args.parallel, args.continue_download, args.idle_timeout, args.http3)
    }
    fn has_new_params(&self, args: &Self::Args) -> bool {
        args.parallel.is_some() || args.continue_download.is_some() || args.idle_timeout.is_some() || args.http3.is_some()
    }
    fn update_cache(&self, cache: &mut CachedConfig, args: &Self::Args, ctx: &GlobalContext) {
        cache.update_with_download(args.parallel.unwrap_or(4), args.continue_download.unwrap_or(false), args.idle_timeout.unwrap_or(30), args.http3.unwrap_or(false), ctx.no_color, ctx.profile.clone());
    }
    async fn run(&self, merged: Self::Merged, _ctx: &GlobalContext) -> Result<()> {
        let (parallel, continue_download, idle_timeout, http3) = merged;
        log_info(&format!("Starting download from: {}", self.url));
        log_debug(&format!(
            "Download parameters - output: {}, parallel: {}, continue: {}, timeout: {}s, http3: {}",
            self.output.display(), parallel, continue_download, idle_timeout, http3
        ));
        match download_file(&self.url, &self.output, parallel, continue_download, idle_timeout, http3, self.hash_check.as_deref()).await {
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
}

#[derive(Clone)]
struct BenchArgs { requests: Option<usize>, concurrency: Option<usize>, connect_timeout: Option<u64>, http3: Option<bool> }
struct BenchCommand { url: String, args: BenchArgs }

#[async_trait]
impl CacheableAction for BenchCommand {
    type Args = BenchArgs;
    type Merged = (usize, usize, u64, bool);

    fn get_provided_args(&self) -> Self::Args { self.args.clone() }
    fn detect_conflicts(&self, cache: &CachedConfig, args: &Self::Args) -> Vec<String> {
        cache.detect_conflicts_bench(args.requests, args.concurrency, args.connect_timeout, args.http3)
    }
    fn merge_config(&self, cache: &CachedConfig, args: &Self::Args) -> Self::Merged {
        cache.merge_bench_config(args.requests, args.concurrency, args.connect_timeout, args.http3)
    }
    fn has_new_params(&self, args: &Self::Args) -> bool {
        args.requests.is_some() || args.concurrency.is_some() || args.connect_timeout.is_some() || args.http3.is_some()
    }
    fn update_cache(&self, cache: &mut CachedConfig, args: &Self::Args, ctx: &GlobalContext) {
        cache.update_with_bench(args.requests.unwrap_or(100), args.concurrency.unwrap_or(10), args.connect_timeout.unwrap_or(5), args.http3.unwrap_or(false), ctx.no_color, ctx.profile.clone());
    }
    async fn run(&self, merged: Self::Merged, _ctx: &GlobalContext) -> Result<()> {
        let (requests, concurrency, connect_timeout, http3) = merged;
        log_info(&format!("Starting benchmark for: {}", self.url));
        log_debug(&format!(
            "Benchmark parameters - requests: {}, concurrency: {}, timeout: {}s, http3: {}",
            requests, concurrency, connect_timeout, http3
        ));
        match benchmark_url(&self.url, requests, concurrency, connect_timeout, http3).await {
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

// ================= Traits Impls for Action Commands =================

#[async_trait]
impl ConfigActionHandler for ConfigAction {
    async fn handle(&self, config: &mut Config, config_path: &PathBuf) -> Result<()> { handle_config_action(self.clone(), config, config_path).await }
}
#[async_trait]
impl ConfigActionHandler for ProfileAction {
    async fn handle(&self, config: &mut Config, config_path: &PathBuf) -> Result<()> { handle_profile_action(self.clone(), config, config_path).await }
}
#[async_trait]
impl SimpleActionHandler for HistoryAction {
    async fn handle(&self) -> Result<()> { handle_history_action(self.clone()).await }
}
#[async_trait]
impl SimpleActionHandler for CacheAction {
    async fn handle(&self) -> Result<()> { handle_cache_action(self.clone()).await }
}
#[async_trait]
impl SimpleActionHandler for ResumeAction {
    async fn handle(&self) -> Result<()> { handle_resume_action(self.clone()).await }
}

// ================= Original Helper Functions =================

async fn handle_get_request(
    url: &str, include: bool, output: Option<PathBuf>, location: bool, headers: Vec<String>,
    connect_timeout: u64, verbose: bool, http3: bool, json: bool, analyze: bool,
    save_history: bool, config: &Config, no_color: bool,
) -> Result<()> {
    log_info(&format!("GET request to: {}", url));
    log_debug(&format!(
        "Parameters - include: {}, location: {}, timeout: {}s, verbose: {}, http3: {}",
        include, location, connect_timeout, verbose, http3
    ));
    let start_time = Instant::now();
    let mut request_headers = HashMap::new();
    let mut all_headers = config.default_headers.clone();
    for header in &headers {
        if let Some((key, value)) = header.split_once(':') {
            all_headers.insert(key.trim().to_string(), value.trim().to_string());
            request_headers.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let header_vec: Vec<String> = all_headers.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
    if !headers.is_empty() { log_debug(&format!("Custom headers: {:?}", headers)); }

    let mut history_entry = if save_history { Some(HistoryEntry::new("GET", url, request_headers)) } else { None };

    let client = match build_client(location, connect_timeout, http3, header_vec, ClientType::Get) {
        Ok(client) => { log_debug("HTTP client built successfully for GET request (300s total timeout)"); client }
        Err(e) => {
            log_error(&format!("Failed to build HTTP client: {}", e));
            if let Some(ref mut entry) = history_entry { *entry = entry.clone().with_error(e.to_string()); }
            return Err(e);
        }
    };

    let response = match client.get(url).send().await {
        Ok(response) => { log_info(&format!("Received response with status: {}", response.status())); response }
        Err(e) => {
            log_error(&format!("Request failed: {}", e));
            if let Some(ref mut entry) = history_entry { *entry = entry.clone().with_error(e.to_string()); }
            return Err(e.into());
        }
    };

    let response_time = start_time.elapsed().as_millis() as u64;
    let status = response.status();
    let version = response.version();
    let response_headers = response.headers().clone();
    let formatter = ResponseFormatter::new(!no_color, json, false);

    if verbose {
        println!("> {:?} {}", version, status);
        for (name, value) in response.headers() { println!("> {}: {}", name, value.to_str()?); }
        println!(">");
    }

    let content = response.text().await?;
    let content_size = content.len() as u64;
    log_info(&format!("Response content size: {} bytes", content.len()));

    if let Some(ref mut entry) = history_entry {
        *entry = entry.clone().with_response(status.as_u16(), response_time, content_size);
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

    if analyze {
        let analysis = ResponseAnalyzer::analyze_headers(&response_headers);
        println!("=== Response Analysis ===");
        for (key, value) in analysis { println!("{}: {}", key, value); }
        println!("=== End Analysis ===\n");
    }

    let formatted_content = formatter.format_body(&content, response_headers.get("content-type").and_then(|ct| ct.to_str().ok()));

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
        None => { println!("{}", formatted_content); log_debug("Response content printed to stdout"); }
    }

    if verbose {
        println!("\n< {}", ResponseAnalyzer::get_response_summary(status, &response_headers, content.len(), response_time));
    }
    log_info("GET request completed successfully");
    Ok(())
}

async fn handle_config_action(action: ConfigAction, config: &mut Config, config_path: &PathBuf) -> Result<()> {
    match action {
        ConfigAction::Show => {
            println!("Current configuration:");
            println!("Default timeout: {}s", config.default_timeout);
            println!("Default user agent: {}", config.default_user_agent);
            println!("Max redirects: {}", config.max_redirects);
            println!("Default headers:");
            for (key, value) in &config.default_headers { println!(" {}: {}", key, value); }
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
                "timeout" => { config.default_timeout = value.parse()?; println!("Set default timeout to {}s", config.default_timeout); }
                "user_agent" => { config.default_user_agent = value.clone(); config.default_headers.insert("User-Agent".to_string(), value); println!("Set user agent to: {}", config.default_user_agent); }
                "max_redirects" => { config.max_redirects = value.parse()?; println!("Set max redirects to {}", config.max_redirects); }
                _ => { println!("Unknown configuration key: {}", key); return Ok(()); }
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
            if entries.is_empty() { println!("No history entries found"); return Ok(()); }
            println!("Recent requests:");
            for entry in entries {
                let status_str = if let Some(status) = entry.status_code {
                    if entry.success { format!("{} ✓", status) } else { format!("{} ✗", status) }
                } else { "Error".to_string() };
                let time_str = entry.response_time.map(|t| format!("{}ms", t)).unwrap_or_else(|| "N/A".to_string());
                let id_short: String = entry.id.chars().take(8).collect();
                println!("{} | {} {} | {} | {} | {}", entry.timestamp.format("%Y-%m-%d %H:%M:%S"), entry.method, entry.url, status_str, time_str, id_short);
            }
            Ok(())
        }
        HistoryAction::Search { query } => {
            let results = history.search(&query);
            if results.is_empty() { println!("No matching history entries found"); return Ok(()); }
            println!("Search results for '{}':", query);
            for entry in results {
                let id_short: String = entry.id.chars().take(8).collect();
                println!("{} | {} {} | Status: {} | ID: {}", entry.timestamp.format("%Y-%m-%d %H:%M:%S"), entry.method, entry.url, entry.status_code.map(|s| s.to_string()).unwrap_or_else(|| "Error".to_string()), id_short);
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
                    for (key, value) in &entry.headers { println!(" {}: {}", key, value); }
                }
                if let Some(ref error) = entry.error_message { println!("Error: {}", error); }
            } else { println!("History entry not found: {}", id); }
            Ok(())
        }
        HistoryAction::Clear => {
            let mut history = RequestHistory::load_from_file(&history_path).unwrap_or_default();
            history.clear();
            history.save_to_file(&history_path)?;
            println!("History cleared");
            Ok(())
        }
    }
}

async fn handle_profile_action(action: ProfileAction, config: &mut Config, config_path: &PathBuf) -> Result<()> {
    match action {
        ProfileAction::List => {
            if config.profiles.is_empty() { println!("No profiles configured"); return Ok(()); }
            println!("Available profiles:");
            for (name, profile) in &config.profiles {
                println!(" {} - {}", name, profile.base_url.as_ref().unwrap_or(&"No base URL".to_string()));
            }
            Ok(())
        }
        ProfileAction::Create { name, base_url, timeout, follow_redirects } => {
            let profile = Profile { name: name.clone(), base_url, headers: HashMap::new(), timeout, follow_redirects };
            config.add_profile(profile);
            config.save_to_file(config_path)?;
            println!("Profile '{}' created", name);
            Ok(())
        }
        ProfileAction::Delete { name } => {
            if config.remove_profile(&name) { config.save_to_file(config_path)?; println!("Profile '{}' deleted", name); }
            else { println!("Profile '{}' not found", name); }
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
                    for (key, value) in &profile.headers { println!(" {}: {}", key, value); }
                }
            } else { println!("Profile '{}' not found", name); }
            Ok(())
        }
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
            if cache_path.exists() { std::fs::remove_file(&cache_path)?; println!("Cached configuration cleared"); }
            else { println!("No cached configuration found"); }
            Ok(())
        }
    }
}

async fn handle_resume_action(action: ResumeAction) -> Result<()> {
    use crate::resume::{ResumeManager, DownloadStatus};
    use chrono::{DateTime, Local, Utc};
    let resume_manager: ResumeManager = ResumeManager::new()?;
    match action {
        ResumeAction::List => {
            let downloads = resume_manager.list_all_downloads()?;
            if downloads.is_empty() { println!("No resumable downloads found"); return Ok(()); }
            println!("\nResumable Downloads:");
            println!("{:-<120}", "");
            println!("{:<40} {:<15} {:<12} {:<12} {:<20} {:<15}", "URL", "Status", "Progress", "Size", "Last Updated", "Hash");
            println!("{:-<120}", "");
            for download in &downloads {
                let status_str = match download.status {
                    DownloadStatus::InProgress => "In Progress".to_string(),
                    DownloadStatus::Paused => "Paused".to_string(),
                    DownloadStatus::Completed => "Completed".to_string(),
                    DownloadStatus::Failed => "Failed".to_string(),
                };
                let progress = if download.total_size > 0 { format!("{:.1}%", download.get_progress_percentage()) } else { "Unknown".to_string() };
                let size_str = if download.total_size > 0 { format!("{}", HumanBytes(download.total_size)) } else { "Unknown".to_string() };
                let url_display = truncate_string(&download.url, 37);
                let last_update = DateTime::<Utc>::from_timestamp(download.last_update_time as i64, 0).unwrap_or_else(|| Utc::now());
                let local_time: DateTime<Local> = last_update.into();
                let hash_short: String = download.url_hash.chars().take(12).collect();
                println!("{:<40} {:<15} {:<12} {:<12} {:<20} {:<15}", url_display, status_str, progress, size_str, local_time.format("%Y-%m-%d %H:%M").to_string(), hash_short);
            }
            println!("{:-<120}", "");
            println!("\nTotal: {} downloads", downloads.len());
            Ok(())
        }
        ResumeAction::Show { url_or_hash } => {
            let metadata = if let Ok(Some(meta)) = resume_manager.load_metadata(&url_or_hash) { meta } else {
                let downloads = resume_manager.list_all_downloads()?;
                downloads.into_iter().find(|d| d.url_hash.starts_with(&url_or_hash)).ok_or_else(|| anyhow!("Download not found: {}", url_or_hash))?
            };
            println!("\nDownload Details:");
            println!("{:-<80}", "");
            println!("URL: {}", metadata.url);
            println!("Hash: {}", metadata.url_hash);
            println!("Output: {}", metadata.output_path.display());
            println!("Status: {:?}", metadata.status);
            println!("Total Size: {}", HumanBytes(metadata.total_size));
            println!("Downloaded: {} ({:.1}%)", HumanBytes(metadata.downloaded), metadata.get_progress_percentage());
            println!("Supports Range: {}", metadata.supports_range);
            if let Some(ref etag) = metadata.etag { println!("ETag: {}", etag); }
            if let Some(ref last_modified) = metadata.last_modified { println!("Last-Modified: {}", last_modified); }
            let start_time = DateTime::<Utc>::from_timestamp(metadata.start_time as i64, 0).unwrap_or_else(|| Utc::now());
            let last_update = DateTime::<Utc>::from_timestamp(metadata.last_update_time as i64, 0).unwrap_or_else(|| Utc::now());
            println!("Started: {}", start_time.format("%Y-%m-%d %H:%M:%S UTC"));
            println!("Last Update: {}", last_update.format("%Y-%m-%d %H:%M:%S UTC"));
            if metadata.chunks.len() > 1 {
                println!("\nChunks ({}):", metadata.chunks.len());
                let completed = metadata.chunks.iter().filter(|c| c.status == crate::resume::ChunkStatus::Completed).count();
                let in_progress = metadata.chunks.iter().filter(|c| c.status == crate::resume::ChunkStatus::Downloading).count();
                let pending = metadata.chunks.iter().filter(|c| c.status == crate::resume::ChunkStatus::Pending).count();
                let failed = metadata.chunks.iter().filter(|c| c.status == crate::resume::ChunkStatus::Failed).count();
                println!(" Completed: {}", completed);
                println!(" In Progress: {}", in_progress);
                println!(" Pending: {}", pending);
                println!(" Failed: {}", failed);
            }
            if let Some(ref error) = metadata.error_message { println!("\nError: {}", error); }
            println!("{:-<80}", "");
            Ok(())
        }
        ResumeAction::Resume { url, output, idle_timeout, http3 } => {
            let metadata = resume_manager.load_metadata(&url)?.ok_or_else(|| anyhow!("No resumable download found for: {}", url))?;
            let output_path = output.unwrap_or_else(|| metadata.output_path.clone());
            println!("Resuming download:");
            println!(" URL: {}", url);
            println!(" Output: {}", output_path.display());
            println!(" Progress: {:.1}%", metadata.get_progress_percentage());
            println!(" Idle timeout: {}s", idle_timeout);
            println!(" HTTP/3: {}", http3);
            download_file(&url, &output_path, metadata.chunks.len().max(1), true, idle_timeout, http3, None).await?;
            Ok(())
        }
        ResumeAction::Cleanup { days } => {
            println!("Cleaning up download metadata older than {} days...", days);
            let cleaned = resume_manager.cleanup_old_metadata(days)?;
            println!("Cleaned up {} old download(s)", cleaned);
            Ok(())
        }
        ResumeAction::Delete { url_or_hash } => {
            if resume_manager.delete_metadata(&url_or_hash).is_ok() { println!("Deleted download metadata for: {}", url_or_hash); }
            else {
                let downloads = resume_manager.list_all_downloads()?;
                if let Some(download) = downloads.into_iter().find(|d| d.url_hash.starts_with(&url_or_hash)) {
                    resume_manager.delete_metadata(&download.url)?;
                    println!("Deleted download metadata for: {}", download.url);
                } else { println!("Download not found: {}", url_or_hash); }
            }
            Ok(())
        }
    }
}
