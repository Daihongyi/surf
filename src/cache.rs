use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedConfig {
    // Download specific options
    pub parallel: Option<usize>,
    pub continue_download: Option<bool>,
    pub idle_timeout: Option<u64>,
    pub http3: Option<bool>,

    // Get specific options
    pub include: Option<bool>,
    pub location: Option<bool>,
    pub headers: Option<Vec<String>>,
    pub connect_timeout: Option<u64>,
    pub verbose: Option<bool>,
    pub json: Option<bool>,
    pub analyze: Option<bool>,
    pub save_history: Option<bool>,

    // Benchmark specific options
    pub requests: Option<usize>,
    pub concurrency: Option<usize>,

    // Global options
    pub no_color: Option<bool>,
    pub profile: Option<String>,
}

impl CachedConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(CachedConfig::default());
        }

        let content = fs::read_to_string(path)?;
        let config: CachedConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse cached config file: {}", e))?;

        Ok(config)
    }

    pub fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize cached config: {}", e))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)?;
        Ok(())
    }

    pub fn get_cache_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("surf")
            .join("last_config.json")
    }

    // 从Download命令创建缓存配置
    pub fn from_download(
        parallel: usize,
        continue_download: bool,
        idle_timeout: u64,
        http3: bool,
        no_color: bool,
        profile: Option<String>,
    ) -> Self {
        Self {
            parallel: Some(parallel),
            continue_download: Some(continue_download),
            idle_timeout: Some(idle_timeout),
            http3: Some(http3),
            no_color: Some(no_color),
            profile,
            ..Default::default()
        }
    }

    // 从Get命令创建缓存配置
    pub fn from_get(
        include: bool,
        location: bool,
        headers: Vec<String>,
        connect_timeout: u64,
        verbose: bool,
        http3: bool,
        json: bool,
        analyze: bool,
        save_history: bool,
        no_color: bool,
        profile: Option<String>,
    ) -> Self {
        Self {
            include: Some(include),
            location: Some(location),
            headers: if headers.is_empty() { None } else { Some(headers) },
            connect_timeout: Some(connect_timeout),
            verbose: Some(verbose),
            http3: Some(http3),
            json: Some(json),
            analyze: Some(analyze),
            save_history: Some(save_history),
            no_color: Some(no_color),
            profile,
            ..Default::default()
        }
    }

    // 从Benchmark命令创建缓存配置
    pub fn from_bench(
        requests: usize,
        concurrency: usize,
        connect_timeout: u64,
        http3: bool,
        no_color: bool,
        profile: Option<String>,
    ) -> Self {
        Self {
            requests: Some(requests),
            concurrency: Some(concurrency),
            connect_timeout: Some(connect_timeout),
            http3: Some(http3),
            no_color: Some(no_color),
            profile,
            ..Default::default()
        }
    }

    // 检测配置冲突
    pub fn detect_conflicts_download(
        &self,
        parallel: Option<usize>,
        continue_download: Option<bool>,
        idle_timeout: Option<u64>,
        http3: Option<bool>,
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        if let (Some(cached), Some(provided)) = (self.parallel, parallel) {
            if cached != provided {
                conflicts.push(format!("parallel: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.continue_download, continue_download) {
            if cached != provided {
                conflicts.push(format!("continue_download: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.idle_timeout, idle_timeout) {
            if cached != provided {
                conflicts.push(format!("idle_timeout: cached={}s, provided={}s", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.http3, http3) {
            if cached != provided {
                conflicts.push(format!("http3: cached={}, provided={}", cached, provided));
            }
        }

        conflicts
    }

    // 检测Get命令的配置冲突
    pub fn detect_conflicts_get(
        &self,
        include: Option<bool>,
        location: Option<bool>,
        headers: &Option<Vec<String>>,
        connect_timeout: Option<u64>,
        verbose: Option<bool>,
        http3: Option<bool>,
        json: Option<bool>,
        analyze: Option<bool>,
        save_history: Option<bool>,
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        if let (Some(cached), Some(provided)) = (self.include, include) {
            if cached != provided {
                conflicts.push(format!("include: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.location, location) {
            if cached != provided {
                conflicts.push(format!("location: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(ref cached_headers), Some(ref provided_headers)) = (&self.headers, headers) {
            if cached_headers != provided_headers {
                conflicts.push(format!("headers: cached={:?}, provided={:?}", cached_headers, provided_headers));
            }
        }

        if let (Some(cached), Some(provided)) = (self.connect_timeout, connect_timeout) {
            if cached != provided {
                conflicts.push(format!("connect_timeout: cached={}s, provided={}s", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.verbose, verbose) {
            if cached != provided {
                conflicts.push(format!("verbose: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.http3, http3) {
            if cached != provided {
                conflicts.push(format!("http3: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.json, json) {
            if cached != provided {
                conflicts.push(format!("json: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.analyze, analyze) {
            if cached != provided {
                conflicts.push(format!("analyze: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.save_history, save_history) {
            if cached != provided {
                conflicts.push(format!("save_history: cached={}, provided={}", cached, provided));
            }
        }

        conflicts
    }

    // 检测Benchmark命令的配置冲突
    pub fn detect_conflicts_bench(
        &self,
        requests: Option<usize>,
        concurrency: Option<usize>,
        connect_timeout: Option<u64>,
        http3: Option<bool>,
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        if let (Some(cached), Some(provided)) = (self.requests, requests) {
            if cached != provided {
                conflicts.push(format!("requests: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.concurrency, concurrency) {
            if cached != provided {
                conflicts.push(format!("concurrency: cached={}, provided={}", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.connect_timeout, connect_timeout) {
            if cached != provided {
                conflicts.push(format!("connect_timeout: cached={}s, provided={}s", cached, provided));
            }
        }

        if let (Some(cached), Some(provided)) = (self.http3, http3) {
            if cached != provided {
                conflicts.push(format!("http3: cached={}, provided={}", cached, provided));
            }
        }

        conflicts
    }

    // 合并配置，优先使用提供的值，没有提供的使用缓存值，都没有使用默认值
    pub fn merge_download_config(
        &self,
        parallel: Option<usize>,
        continue_download: Option<bool>,
        idle_timeout: Option<u64>,
        http3: Option<bool>,
    ) -> (usize, bool, u64, bool) {
        let merged_parallel = parallel
            .or(self.parallel)
            .unwrap_or(4); // 默认值

        let merged_continue = continue_download
            .or(self.continue_download)
            .unwrap_or(false); // 默认值

        let merged_idle_timeout = idle_timeout
            .or(self.idle_timeout)
            .unwrap_or(30); // 默认值

        let merged_http3 = http3
            .or(self.http3)
            .unwrap_or(false); // 默认值

        (merged_parallel, merged_continue, merged_idle_timeout, merged_http3)
    }

    // 合并Get配置
    pub fn merge_get_config(
        &self,
        include: Option<bool>,
        location: Option<bool>,
        headers: Option<Vec<String>>,
        connect_timeout: Option<u64>,
        verbose: Option<bool>,
        http3: Option<bool>,
        json: Option<bool>,
        analyze: Option<bool>,
        save_history: Option<bool>,
    ) -> (bool, bool, Vec<String>, u64, bool, bool, bool, bool, bool) {
        let merged_include = include
            .or(self.include)
            .unwrap_or(false);

        let merged_location = location
            .or(self.location)
            .unwrap_or(false);

        let merged_headers = headers
            .or_else(|| self.headers.clone())
            .unwrap_or_default();

        let merged_connect_timeout = connect_timeout
            .or(self.connect_timeout)
            .unwrap_or(10);

        let merged_verbose = verbose
            .or(self.verbose)
            .unwrap_or(false);

        let merged_http3 = http3
            .or(self.http3)
            .unwrap_or(false);

        let merged_json = json
            .or(self.json)
            .unwrap_or(false);

        let merged_analyze = analyze
            .or(self.analyze)
            .unwrap_or(false);

        let merged_save_history = save_history
            .or(self.save_history)
            .unwrap_or(true);

        (
            merged_include,
            merged_location,
            merged_headers,
            merged_connect_timeout,
            merged_verbose,
            merged_http3,
            merged_json,
            merged_analyze,
            merged_save_history,
        )
    }

    // 合并Benchmark配置
    pub fn merge_bench_config(
        &self,
        requests: Option<usize>,
        concurrency: Option<usize>,
        connect_timeout: Option<u64>,
        http3: Option<bool>,
    ) -> (usize, usize, u64, bool) {
        let merged_requests = requests
            .or(self.requests)
            .unwrap_or(100);

        let merged_concurrency = concurrency
            .or(self.concurrency)
            .unwrap_or(10);

        let merged_connect_timeout = connect_timeout
            .or(self.connect_timeout)
            .unwrap_or(5);

        let merged_http3 = http3
            .or(self.http3)
            .unwrap_or(false);

        (merged_requests, merged_concurrency, merged_connect_timeout, merged_http3)
    }

    // 更新缓存配置（合并新值）
    pub fn update_with_download(
        &mut self,
        parallel: usize,
        continue_download: bool,
        idle_timeout: u64,
        http3: bool,
        no_color: bool,
        profile: Option<String>,
    ) {
        self.parallel = Some(parallel);
        self.continue_download = Some(continue_download);
        self.idle_timeout = Some(idle_timeout);
        self.http3 = Some(http3);
        self.no_color = Some(no_color);
        self.profile = profile;
    }

    pub fn update_with_get(
        &mut self,
        include: bool,
        location: bool,
        headers: Vec<String>,
        connect_timeout: u64,
        verbose: bool,
        http3: bool,
        json: bool,
        analyze: bool,
        save_history: bool,
        no_color: bool,
        profile: Option<String>,
    ) {
        self.include = Some(include);
        self.location = Some(location);
        self.headers = if headers.is_empty() { None } else { Some(headers) };
        self.connect_timeout = Some(connect_timeout);
        self.verbose = Some(verbose);
        self.http3 = Some(http3);
        self.json = Some(json);
        self.analyze = Some(analyze);
        self.save_history = Some(save_history);
        self.no_color = Some(no_color);
        self.profile = profile;
    }

    pub fn update_with_bench(
        &mut self,
        requests: usize,
        concurrency: usize,
        connect_timeout: u64,
        http3: bool,
        no_color: bool,
        profile: Option<String>,
    ) {
        self.requests = Some(requests);
        self.concurrency = Some(concurrency);
        self.connect_timeout = Some(connect_timeout);
        self.http3 = Some(http3);
        self.no_color = Some(no_color);
        self.profile = profile;
    }

    // 检查缓存是否为空（没有任何配置）
    pub fn is_empty(&self) -> bool {
        self.parallel.is_none()
            && self.continue_download.is_none()
            && self.idle_timeout.is_none()
            && self.http3.is_none()
            && self.include.is_none()
            && self.location.is_none()
            && self.headers.is_none()
            && self.connect_timeout.is_none()
            && self.verbose.is_none()
            && self.json.is_none()
            && self.analyze.is_none()
            && self.save_history.is_none()
            && self.requests.is_none()
            && self.concurrency.is_none()
            && self.no_color.is_none()
            && self.profile.is_none()
    }

    // 显示当前缓存的配置
    pub fn display_cached_config(&self) -> String {
        let mut output = String::new();
        output.push_str("Cached configuration:\n");

        // Download options
        if let Some(parallel) = self.parallel {
            output.push_str(&format!("  parallel: {}\n", parallel));
        }
        if let Some(continue_download) = self.continue_download {
            output.push_str(&format!("  continue_download: {}\n", continue_download));
        }
        if let Some(idle_timeout) = self.idle_timeout {
            output.push_str(&format!("  idle_timeout: {}s\n", idle_timeout));
        }

        // Get options
        if let Some(include) = self.include {
            output.push_str(&format!("  include: {}\n", include));
        }
        if let Some(location) = self.location {
            output.push_str(&format!("  location: {}\n", location));
        }
        if let Some(ref headers) = self.headers {
            output.push_str(&format!("  headers: {:?}\n", headers));
        }
        if let Some(connect_timeout) = self.connect_timeout {
            output.push_str(&format!("  connect_timeout: {}s\n", connect_timeout));
        }
        if let Some(verbose) = self.verbose {
            output.push_str(&format!("  verbose: {}\n", verbose));
        }
        if let Some(json) = self.json {
            output.push_str(&format!("  json: {}\n", json));
        }
        if let Some(analyze) = self.analyze {
            output.push_str(&format!("  analyze: {}\n", analyze));
        }
        if let Some(save_history) = self.save_history {
            output.push_str(&format!("  save_history: {}\n", save_history));
        }

        // Benchmark options
        if let Some(requests) = self.requests {
            output.push_str(&format!("  requests: {}\n", requests));
        }
        if let Some(concurrency) = self.concurrency {
            output.push_str(&format!("  concurrency: {}\n", concurrency));
        }

        // Common options
        if let Some(http3) = self.http3 {
            output.push_str(&format!("  http3: {}\n", http3));
        }
        if let Some(no_color) = self.no_color {
            output.push_str(&format!("  no_color: {}\n", no_color));
        }
        if let Some(ref profile) = self.profile {
            output.push_str(&format!("  profile: {}\n", profile));
        }

        if self.is_empty() {
            output.push_str("  (no cached configuration)\n");
        }

        output
    }
}