use crate::log::{log_info, log_error, log_debug, log_warn};
use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
    Client, ClientBuilder, StatusCode,
};
use std::{
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{fs, io::AsyncWriteExt, sync::Semaphore};

// 常量定义
const DEFAULT_CONNECT_TIMEOUT: u64 = 10;
const PARALLEL_DOWNLOAD_THRESHOLD: u64 = 10_000_000; // 10MB
const MAX_REDIRECTS: usize = 10;
const PROGRESS_UPDATE_INTERVAL: usize = 1000; // 每1000个chunk更新一次日志

#[derive(Debug, thiserror::Error)]
pub enum TimeoutError {
    #[error("Idle timeout: no data received")]
    IdleTimeout,
    #[error("Connection timeout")]
    ConnectTimeout,
}

// 辅助函数：解析header字符串
fn parse_header(header_str: &str) -> Result<(HeaderName, HeaderValue)> {
    let (key, value) = header_str
        .split_once(':')
        .ok_or_else(|| anyhow!("Malformed header: missing colon in '{}'", header_str))?;

    let header_name = HeaderName::from_str(key.trim())
        .with_context(|| format!("Invalid header name: '{}'", key.trim()))?;

    let header_value = HeaderValue::from_str(value.trim())
        .with_context(|| format!("Invalid header value: '{}'", value.trim()))?;

    Ok((header_name, header_value))
}

// 辅助函数：创建进度条
fn create_progress_bar(total_size: u64, initial_pos: u64) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) | {binary_bytes_per_sec} | {msg}")
            .expect("Invalid progress bar template")
            .progress_chars("#>-"),
    );
    pb.set_position(initial_pos);
    pb
}

pub fn build_client(
    follow_redirects: bool,
    connect_timeout: u64,
    http3: bool,
    headers: Vec<String>,
) -> Result<Client> {
    log_debug(&format!(
        "Building HTTP client - redirects: {}, timeout: {}s, http3: {}",
        follow_redirects, connect_timeout, http3
    ));

    let mut client_builder = ClientBuilder::new();

    // 设置重定向策略
    let redirect_policy = if follow_redirects {
        Policy::limited(MAX_REDIRECTS)
    } else {
        Policy::none()
    };
    client_builder = client_builder.redirect(redirect_policy);
    log_debug(&format!(
        "Redirect policy set: {}",
        if follow_redirects {
            "limited(10)"
        } else {
            "none"
        }
    ));

    // 设置连接和请求超时
    client_builder = client_builder
        .connect_timeout(Duration::from_secs(connect_timeout))
        .timeout(Duration::from_secs(connect_timeout * 3));

    // 添加自定义headers
    let mut header_map = HeaderMap::new();
    for header in headers {
        match parse_header(&header) {
            Ok((name, value)) => {
                header_map.insert(name, value);
                log_debug(&format!("Added custom header: {}", header));
            }
            Err(e) => {
                log_warn(&format!("Skipping malformed header '{}': {}", header, e));
            }
        }
    }
    client_builder = client_builder.default_headers(header_map);

    // HTTP/3支持
    if http3 {
        #[cfg(not(feature = "http3"))]
        {
            log_error("HTTP/3 support was not enabled at compile time");
            return Err(anyhow!(
                "HTTP/3 support was not enabled at compile time. \
                Please rebuild with `RUSTFLAGS=\"--cfg reqwest_unstable\"` and the `http3` feature."
            ));
        }
        #[cfg(feature = "http3")]
        {
            client_builder = client_builder.use_rustls_tls().http3_prior_knowledge();
            log_debug("HTTP/3 support enabled");
        }
    }

    client_builder.build().map_err(|e| {
        log_error(&format!("Failed to build HTTP client: {}", e));
        anyhow!("Failed to build HTTP client: {}", e)
    })
}

pub async fn download_file(
    url: &str,
    output: &PathBuf,
    parallel: usize,
    continue_download: bool,
    idle_timeout: u64,
    http3: bool,
) -> Result<()> {
    log_info(&format!("Starting file download from: {}", url));
    log_debug(&format!(
        "Download settings - output: {}, parallel: {}, continue: {}, idle_timeout: {}s",
        output.display(),
        parallel,
        continue_download,
        idle_timeout
    ));

    let client = build_client(true, DEFAULT_CONNECT_TIMEOUT, http3, vec![])?;

    // 获取文件信息和检查范围请求支持
    let (total_size, supports_range) = get_download_info(&client, url).await?;

    log_info(&format!(
        "File size: {}",
        if total_size > 0 {
            format!("{}", HumanBytes(total_size))
        } else {
            "unknown".to_string()
        }
    ));
    log_debug(&format!("Range requests supported: {}", supports_range));

    // 检查是否可以恢复下载
    let downloaded = if continue_download && output.exists() {
        let metadata = fs::metadata(output).await?;
        metadata.len()
    } else {
        0
    };

    if downloaded > 0 {
        log_info(&format!(
            "Resuming download, {} already downloaded",
            HumanBytes(downloaded)
        ));
    }

    // 决定使用并行还是单连接下载
    let use_parallel = supports_range
        && total_size > 0
        && parallel > 1
        && total_size > PARALLEL_DOWNLOAD_THRESHOLD
        && downloaded < total_size;

    if use_parallel {
        log_info(&format!(
            "Using parallel download with {} connections",
            parallel
        ));
        download_parallel(
            &client,
            url,
            output,
            total_size,
            downloaded,
            parallel,
            idle_timeout,
        )
            .await
    } else {
        log_info("Using single connection download");
        download_single(&client, url, output, downloaded, total_size, idle_timeout).await
    }
}

async fn get_download_info(client: &Client, url: &str) -> Result<(u64, bool)> {
    log_debug("Sending HEAD request to get file info");

    let response = client
        .head(url)
        .send()
        .await
        .context("HEAD request failed")?;

    log_debug(&format!(
        "HEAD request successful, status: {}",
        response.status()
    ));

    let total_size = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|ct_len| ct_len.to_str().ok())
        .and_then(|ct_len| ct_len.parse::<u64>().ok())
        .unwrap_or(0);

    let supports_range = response
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|ar| ar.to_str().ok())
        .map(|ar| ar == "bytes")
        .unwrap_or(false);

    Ok((total_size, supports_range))
}

async fn download_single(
    client: &Client,
    url: &str,
    output: &PathBuf,
    downloaded: u64,
    total_size: u64,
    idle_timeout: u64,
) -> Result<()> {
    let pb = create_progress_bar(total_size, downloaded);
    pb.set_message("\x1b[33mConnecting...\x1b[0m");

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .await
        .with_context(|| format!("Failed to open output file: {}", output.display()))?;

    log_debug(&format!("Opened output file: {}", output.display()));

    let start_time = Instant::now();
    let initial_downloaded = downloaded;
    let mut current_downloaded = downloaded;

    if current_downloaded >= total_size && total_size > 0 {
        log_info("File already fully downloaded");
        pb.finish_with_message("Already completed");
        return Ok(());
    }

    let mut request = client.get(url);
    if current_downloaded > 0 {
        request = request.header("Range", format!("bytes={}-", current_downloaded));
        log_debug(&format!("Using Range header: bytes={}-", current_downloaded));
    }

    pb.set_message("\x1b[32mDownloading...\x1b[0m");

    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            anyhow!(TimeoutError::ConnectTimeout)
        } else {
            anyhow!(e)
        }
    })?;

    log_debug(&format!(
        "Download request successful, status: {}",
        response.status()
    ));

    if !response.status().is_success() {
        let error_msg = format!("Failed to download: {}", response.status());
        log_error(&error_msg);
        return Err(anyhow!(error_msg));
    }

    let mut stream = response.bytes_stream();
    let mut last_data_time = Instant::now();
    let idle_duration = Duration::from_secs(idle_timeout);
    let mut chunk_count = 0;

    while let Some(item) = stream.next().await {
        // 检查空闲超时
        if last_data_time.elapsed() > idle_duration {
            pb.set_message("\x1b[31mIDLE TIMEOUT\x1b[0m");
            log_error("Download failed due to idle timeout");
            return Err(anyhow!(TimeoutError::IdleTimeout));
        }

        let chunk = item.context("Error receiving chunk")?;
        file.write_all(&chunk)
            .await
            .context("Error writing to file")?;

        current_downloaded += chunk.len() as u64;
        pb.set_position(current_downloaded);
        chunk_count += 1;

        // 定期记录进度
        if chunk_count % PROGRESS_UPDATE_INTERVAL == 0 {
            log_debug(&format!("Downloaded {} bytes so far", current_downloaded));
        }

        last_data_time = Instant::now();
    }

    log_info(&format!(
        "Download stream completed, total chunks: {}",
        chunk_count
    ));

    // 计算下载速度并完成进度条
    let elapsed_time = start_time.elapsed();
    let download_size = current_downloaded - initial_downloaded;
    let avg_speed = if elapsed_time.as_secs_f64() > 0.0 {
        download_size as f64 / elapsed_time.as_secs_f64()
    } else {
        download_size as f64
    };

    let abs_path = output.canonicalize().unwrap_or_else(|_| output.clone());
    let completion_msg = format!(
        "Downloaded {} in {:.2}s (avg: {}/s) to: {}",
        HumanBytes(current_downloaded),
        elapsed_time.as_secs_f64(),
        HumanBytes(avg_speed as u64),
        abs_path.display()
    );

    pb.finish_with_message(completion_msg.clone());
    log_info(&format!("Download completed successfully: {}", completion_msg));

    Ok(())
}

async fn download_parallel(
    client: &Client,
    url: &str,
    output: &PathBuf,
    total_size: u64,
    downloaded: u64,
    parallel: usize,
    idle_timeout: u64,
) -> Result<()> {
    let remaining = total_size - downloaded;
    if remaining == 0 {
        log_info("File already fully downloaded");
        return Ok(());
    }

    let chunk_size = remaining / parallel as u64;
    if chunk_size == 0 {
        return download_single(client, url, output, downloaded, total_size, idle_timeout).await;
    }

    log_info(&format!(
        "Parallel download: {} chunks of ~{} bytes each",
        parallel,
        HumanBytes(chunk_size)
    ));

    let pb = Arc::new(create_progress_bar(total_size, downloaded));
    let semaphore = Arc::new(Semaphore::new(parallel));
    let mut tasks = Vec::new();
    let start_time = Instant::now();

    for i in 0..parallel {
        let start = downloaded + i as u64 * chunk_size;
        let end = if i == parallel - 1 {
            total_size - 1
        } else {
            downloaded + (i + 1) as u64 * chunk_size - 1
        };

        let client = client.clone();
        let url = url.to_string();
        let semaphore = Arc::clone(&semaphore);
        let pb = Arc::clone(&pb);
        let temp_path = output.with_extension(format!("part{}", i));

        let task = tokio::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|e| anyhow!("Failed to acquire semaphore: {}", e))?;

            let response = client
                .get(&url)
                .header("Range", format!("bytes={}-{}", start, end))
                .send()
                .await
                .context("Failed to send range request")?;

            if response.status() != StatusCode::PARTIAL_CONTENT {
                return Err(anyhow!("Server doesn't support range requests"));
            }

            let mut file = fs::File::create(&temp_path)
                .await
                .context("Failed to create temp file")?;

            let mut stream = response.bytes_stream();
            let mut bytes_written = 0u64;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.context("Error receiving chunk")?;
                file.write_all(&chunk)
                    .await
                    .context("Error writing to temp file")?;
                bytes_written += chunk.len() as u64;
                pb.inc(chunk.len() as u64);
            }

            Ok::<(PathBuf, u64), anyhow::Error>((temp_path, bytes_written))
        });

        tasks.push(task);
    }

    // 等待所有下载完成并收集临时文件
    let mut temp_files = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Ok((temp_path, _))) => {
                temp_files.push(temp_path);
            }
            Ok(Err(e)) => {
                log_error(&format!("Download task failed: {}", e));
                return Err(e);
            }
            Err(e) => {
                log_error(&format!("Download task panicked: {}", e));
                return Err(anyhow!("Download task panicked: {}", e));
            }
        }
    }

    // 合并文件
    pb.set_message("Merging files...");
    merge_temp_files(output, &temp_files).await?;

    let elapsed = start_time.elapsed();
    let speed = total_size as f64 / elapsed.as_secs_f64();

    pb.finish_with_message(format!(
        "Downloaded {} in {:.2}s (avg: {}/s)",
        HumanBytes(total_size),
        elapsed.as_secs_f64(),
        HumanBytes(speed as u64)
    ));

    log_info("Parallel download completed successfully");
    Ok(())
}

async fn merge_temp_files(output: &PathBuf, temp_files: &[PathBuf]) -> Result<()> {
    let mut final_file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(output)
        .await
        .context("Failed to create final output file")?;

    for temp_file in temp_files {
        let mut temp = fs::File::open(temp_file)
            .await
            .with_context(|| format!("Failed to open temp file: {}", temp_file.display()))?;

        tokio::io::copy(&mut temp, &mut final_file)
            .await
            .with_context(|| format!("Failed to copy from temp file: {}", temp_file.display()))?;
    }

    // 清理临时文件
    for temp_file in temp_files {
        if let Err(e) = fs::remove_file(temp_file).await {
            log_warn(&format!("Failed to remove temp file {}: {}", temp_file.display(), e));
        }
    }

    Ok(())
}

pub async fn benchmark_url(
    url: &str,
    requests: usize,
    concurrency: usize,
    connect_timeout: u64,
    http3: bool,
) -> Result<()> {
    log_info(&format!(
        "Starting benchmark - URL: {}, requests: {}, concurrency: {}",
        url, requests, concurrency
    ));

    let client = build_client(true, connect_timeout, http3, vec![])?;

    println!(
        "Benchmarking {} with {} requests, concurrency {} (HTTP/3: {})",
        url, requests, concurrency, http3
    );

    let start = Instant::now();
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::new();

    // 使用更高效的数据结构收集统计信息
    let stats = Arc::new(tokio::sync::Mutex::new(BenchmarkStats::new()));

    for i in 0..requests {
        let client = client.clone();
        let url = url.to_string();
        let semaphore = Arc::clone(&semaphore);
        let stats = Arc::clone(&stats);

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await?;
            let request_start = Instant::now();

            let result = client.get(&url).send().await;
            let duration = request_start.elapsed();

            let status_code = match &result {
                Ok(resp) => Some(resp.status().as_u16()),
                Err(_) => None,
            };

            let mut stats_guard = stats.lock().await;
            stats_guard.record_request(duration, status_code);

            Ok::<(), anyhow::Error>(())
        });

        tasks.push(task);

        if (i + 1) % 50 == 0 {
            log_debug(&format!("Started {} requests", i + 1));
        }
    }

    // 等待所有任务完成
    for task in tasks {
        if let Err(e) = task.await? {
            log_warn(&format!("Benchmark task failed: {}", e));
        }
    }

    let total_time = start.elapsed();
    let stats = stats.lock().await;
    stats.print_results(requests, total_time);

    log_info(&format!(
        "Benchmark completed - Total: {:.2}s, RPS: {:.2}, Success: {}, Failed: {}",
        total_time.as_secs_f64(),
        requests as f64 / total_time.as_secs_f64(),
        stats.successful_requests,
        stats.failed_requests
    ));

    Ok(())
}

// 基准测试统计数据结构
struct BenchmarkStats {
    response_times: Vec<u64>,
    status_codes: std::collections::HashMap<u16, u32>,
    successful_requests: u32,
    failed_requests: u32,
}

impl BenchmarkStats {
    fn new() -> Self {
        Self {
            response_times: Vec::new(),
            status_codes: std::collections::HashMap::new(),
            successful_requests: 0,
            failed_requests: 0,
        }
    }

    fn record_request(&mut self, duration: Duration, status_code: Option<u16>) {
        let ms = duration.as_millis() as u64;
        self.response_times.push(ms);

        if let Some(code) = status_code {
            *self.status_codes.entry(code).or_insert(0) += 1;

            if code >= 200 && code < 400 {
                self.successful_requests += 1;
            } else {
                self.failed_requests += 1;
                log_warn(&format!("Request failed with status: {}", code));
            }
        } else {
            self.failed_requests += 1;
        }
    }

    fn print_results(&self, total_requests: usize, total_time: Duration) {
        let rps = total_requests as f64 / total_time.as_secs_f64();

        // 计算百分位数
        let mut sorted_times = self.response_times.clone();
        sorted_times.sort_unstable();

        let min_time = sorted_times.first().copied().unwrap_or(0);
        let max_time = sorted_times.last().copied().unwrap_or(0);
        let avg_time = sorted_times.iter().sum::<u64>() / sorted_times.len() as u64;

        let p50 = percentile(&sorted_times, 0.5);
        let p95 = percentile(&sorted_times, 0.95);
        let p99 = percentile(&sorted_times, 0.99);

        println!("\n=== Benchmark Results ===");
        println!("Total time: {:.2}s", total_time.as_secs_f64());
        println!("Requests per second: {:.2}", rps);
        println!("Successful requests: {}", self.successful_requests);
        println!("Failed requests: {}", self.failed_requests);
        println!();
        println!("Response Times (ms):");
        println!("  Min: {}", min_time);
        println!("  Max: {}", max_time);
        println!("  Avg: {}", avg_time);
        println!("  50th percentile: {}", p50);
        println!("  95th percentile: {}", p95);
        println!("  99th percentile: {}", p99);
        println!();
        println!("Status Code Distribution:");

        for (status, count) in &self.status_codes {
            println!(
                "  {}: {} ({:.1}%)",
                status,
                count,
                (*count as f64 / total_requests as f64) * 100.0
            );
        }
    }
}

// 计算百分位数的辅助函数
fn percentile(sorted_data: &[u64], percentile: f64) -> u64 {
    if sorted_data.is_empty() {
        return 0;
    }

    let index = (sorted_data.len() as f64 * percentile) as usize;
    sorted_data[std::cmp::min(index, sorted_data.len() - 1)]
}