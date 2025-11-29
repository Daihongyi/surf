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
const PROGRESS_UPDATE_INTERVAL: usize = 1000;

// 新增：客户端类型枚举，用于区分不同场景的超时策略
#[derive(Debug, Clone, Copy)]
pub enum ClientType {
    Get,       // GET 请求：需要总超时限制
    Download,  // 下载：只依赖空闲超时，无总超时限制
    Benchmark, // 基准测试：需要较短的总超时
}

#[derive(Debug, thiserror::Error)]
pub enum TimeoutError {
    #[error("Idle timeout: no data received for {0}s")]
    IdleTimeout(u64),
    #[error("Connection timeout")]
    ConnectTimeout,
}

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

// 修改后的 build_client 函数：根据客户端类型设置不同的超时策略
pub fn build_client(
    follow_redirects: bool,
    connect_timeout: u64,
    http3: bool,
    headers: Vec<String>,
    client_type: ClientType, // 新增参数
) -> Result<Client> {
    log_debug(&format!(
        "Building HTTP client - type: {:?}, redirects: {}, timeout: {}s, http3: {}",
        client_type, follow_redirects, connect_timeout, http3
    ));

    let mut client_builder = ClientBuilder::new();

    let redirect_policy = if follow_redirects {
        Policy::limited(MAX_REDIRECTS)
    } else {
        Policy::none()
    };
    client_builder = client_builder.redirect(redirect_policy);

    // 关键修改：根据客户端类型设置不同的超时策略
    client_builder = client_builder.connect_timeout(Duration::from_secs(connect_timeout));

    match client_type {
        ClientType::Get => {
            // GET 请求：设置合理的总超时（5分钟）
            client_builder = client_builder.timeout(Duration::from_secs(300));
            log_debug("Client configured with 300s total timeout for GET requests");
        }
        ClientType::Download => {
            // 下载：不设置总超时，完全依赖空闲超时来判断连接是否断开
            // 这样即使下载大文件超过5分钟也不会超时
            log_debug("Client configured WITHOUT total timeout for downloads (idle timeout only)");
        }
        ClientType::Benchmark => {
            // 基准测试：设置较短的总超时（60秒）
            client_builder = client_builder.timeout(Duration::from_secs(60));
            log_debug("Client configured with 60s total timeout for benchmarks");
        }
    }

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

    // 关键修改：使用 ClientType::Download，不设置总超时
    let client = build_client(true, DEFAULT_CONNECT_TIMEOUT, http3, vec![], ClientType::Download)?;

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
    let idle_duration = Duration::from_secs(idle_timeout);
    let mut chunk_count = 0;
    let mut last_progress_log = Instant::now();

    log_info(&format!(
        "Download started with idle timeout of {}s (no total timeout limit)",
        idle_timeout
    ));

    loop {
        match tokio::time::timeout(idle_duration, stream.next()).await {
            Ok(Some(item)) => {
                let chunk = item.context("Error receiving chunk")?;
                file.write_all(&chunk)
                    .await
                    .context("Error writing to file")?;

                current_downloaded += chunk.len() as u64;
                pb.set_position(current_downloaded);
                chunk_count += 1;

                // 定期记录进度日志
                if last_progress_log.elapsed() >= Duration::from_secs(10) {
                    log_debug(&format!(
                        "Download progress: {} / {} ({:.1}%), elapsed: {:.1}s",
                        HumanBytes(current_downloaded),
                        HumanBytes(total_size),
                        (current_downloaded as f64 / total_size as f64) * 100.0,
                        start_time.elapsed().as_secs_f64()
                    ));
                    last_progress_log = Instant::now();
                }
            }
            Ok(None) => {
                log_info(&format!(
                    "Download stream completed, total chunks: {}, total time: {:.2}s",
                    chunk_count,
                    start_time.elapsed().as_secs_f64()
                ));
                break;
            }
            Err(_) => {
                pb.set_message("\x1b[31mIDLE TIMEOUT\x1b[0m");
                log_error(&format!(
                    "Download failed due to idle timeout ({}s with no data) after {:.2}s total time",
                    idle_timeout,
                    start_time.elapsed().as_secs_f64()
                ));
                return Err(anyhow!(TimeoutError::IdleTimeout(idle_timeout)));
            }
        }
    }

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
    use std::fs::File;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;

    let remaining = total_size - downloaded;
    if remaining == 0 {
        log_info("File already fully downloaded");
        return Ok(());
    }

    let file = File::create(output).context("Failed to create output file")?;
    file.set_len(total_size)
        .context("Failed to pre-allocate file size")?;
    let file = Arc::new(file);

    let chunk_size = remaining / parallel as u64;
    if chunk_size == 0 {
        return download_single(client, url, output, downloaded, total_size, idle_timeout).await;
    }

    log_info(&format!(
        "Parallel download: {} chunks of ~{} bytes each (idle timeout: {}s, no total timeout)",
        parallel,
        HumanBytes(chunk_size),
        idle_timeout
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
        let file = Arc::clone(&file);

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
                return Err(anyhow!("Server doesn't support range requests, status: {}", response.status()));
            }

            let mut stream = response.bytes_stream();
            let mut current_pos = start;

            let idle_duration = Duration::from_secs(idle_timeout);

            loop {
                match tokio::time::timeout(idle_duration, stream.next()).await {
                    Ok(Some(chunk_result)) => {
                        let chunk = chunk_result.context("Error receiving chunk")?;
                        let chunk_len = chunk.len();

                        let file_clone = Arc::clone(&file);
                        let current_chunk_pos = current_pos;

                        tokio::task::spawn_blocking(move || {
                            #[cfg(unix)]
                            {
                                file_clone.write_at(&chunk, current_chunk_pos)?;
                            }
                            #[cfg(not(unix))]
                            {
                                let mut f = &*file_clone;
                                use std::io::{Seek, SeekFrom};
                                f.seek(SeekFrom::Start(current_chunk_pos))?;
                                f.write_all(&chunk)?;
                            }
                            Ok::<(), std::io::Error>(())
                        }).await.context("Spawn blocking write failed")?.context("File write operation failed")?;

                        pb.inc(chunk_len as u64);
                        current_pos += chunk_len as u64;
                    }
                    Ok(None) => {
                        break;
                    }
                    Err(_) => {
                        return Err(anyhow!(TimeoutError::IdleTimeout(idle_timeout)));
                    }
                }
            }

            Ok::<(), anyhow::Error>(())
        });

        tasks.push(task);
    }

    for (i, task) in tasks.into_iter().enumerate() {
        match task.await {
            Ok(Ok(())) => {
                log_debug(&format!("Task {} completed successfully", i));
            }
            Ok(Err(e)) => {
                log_error(&format!("Download task {} failed: {}", i, e));
                pb.abandon_with_message(format!("Task {} failed: {}", i, e));
                return Err(e);
            }
            Err(e) => {
                log_error(&format!("Download task {} panicked: {}", i, e));
                pb.abandon_with_message(format!("Task {} panicked", i));
                return Err(anyhow!("Download task panicked: {}", e));
            }
        }
    }

    let elapsed = start_time.elapsed();
    let speed = if elapsed.as_secs_f64() > 0.0 {
        total_size as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    pb.finish_with_message(format!(
        "Downloaded {} in {:.2}s (avg: {}/s)",
        HumanBytes(total_size),
        elapsed.as_secs_f64(),
        HumanBytes(speed as u64)
    ));

    log_info(&format!(
        "Parallel download completed successfully in {:.2}s",
        elapsed.as_secs_f64()
    ));
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

    // 关键修改：使用 ClientType::Benchmark，设置60秒总超时
    let client = build_client(true, connect_timeout, http3, vec![], ClientType::Benchmark)?;

    println!(
        "Benchmarking {} with {} requests, concurrency {} (HTTP/3: {})",
        url, requests, concurrency, http3
    );

    let start = Instant::now();
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::new();

    let stats = Arc::new(BenchmarkStats::new());

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

            stats.record_request(duration, status_code).await;

            Ok::<(), anyhow::Error>(())
        });

        tasks.push(task);

        if (i + 1) % 50 == 0 {
            log_debug(&format!("Started {} requests", i + 1));
        }
    }

    for task in tasks {
        if let Err(e) = task.await? {
            log_warn(&format!("Benchmark task failed: {}", e));
        }
    }

    let total_time = start.elapsed();
    stats.print_results(requests, total_time).await;

    log_info(&format!(
        "Benchmark completed - Total: {:.2}s, RPS: {:.2}, Success: {}, Failed: {}",
        total_time.as_secs_f64(),
        requests as f64 / total_time.as_secs_f64(),
        stats.successful_requests.load(std::sync::atomic::Ordering::Relaxed),
        stats.failed_requests.load(std::sync::atomic::Ordering::Relaxed)
    ));

    Ok(())
}

use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;

struct BenchmarkStats {
    response_times: Arc<Mutex<Vec<u64>>>,
    status_codes: Arc<Mutex<std::collections::HashMap<u16, u32>>>,
    successful_requests: AtomicU32,
    failed_requests: AtomicU32,
}

impl BenchmarkStats {
    fn new() -> Self {
        Self {
            response_times: Arc::new(Mutex::new(Vec::new())),
            status_codes: Arc::new(Mutex::new(std::collections::HashMap::new())),
            successful_requests: AtomicU32::new(0),
            failed_requests: AtomicU32::new(0),
        }
    }

    async fn record_request(&self, duration: Duration, status_code: Option<u16>) {
        let ms = duration.as_millis() as u64;
        self.response_times.lock().await.push(ms);

        if let Some(code) = status_code {
            *self.status_codes.lock().await.entry(code).or_insert(0) += 1;

            if (200..400).contains(&code) {
                self.successful_requests.fetch_add(1, Ordering::Relaxed);
            } else {
                self.failed_requests.fetch_add(1, Ordering::Relaxed);
                log_warn(&format!("Request failed with status: {}", code));
            }
        } else {
            self.failed_requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn print_results(&self, total_requests: usize, total_time: Duration) {
        let rps = total_requests as f64 / total_time.as_secs_f64();

        let mut sorted_times = self.response_times.lock().await.clone();
        sorted_times.sort_unstable();

        let min_time = sorted_times.first().copied().unwrap_or(0);
        let max_time = sorted_times.last().copied().unwrap_or(0);
        let avg_time = if !sorted_times.is_empty() {
            sorted_times.iter().sum::<u64>() / sorted_times.len() as u64
        } else {
            0
        };

        let p50 = percentile(&sorted_times, 0.5);
        let p95 = percentile(&sorted_times, 0.95);
        let p99 = percentile(&sorted_times, 0.99);

        println!("\n=== Benchmark Results ===");
        println!("Total time: {:.2}s", total_time.as_secs_f64());
        println!("Requests per second: {:.2}", rps);
        println!("Successful requests: {}", self.successful_requests.load(Ordering::Relaxed));
        println!("Failed requests: {}", self.failed_requests.load(Ordering::Relaxed));
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

        for (status, count) in &*self.status_codes.lock().await {
            println!(
                "  {}: {} ({:.1}%)",
                status,
                count,
                (*count as f64 / total_requests as f64) * 100.0
            );
        }
    }
}

fn percentile(sorted_data: &[u64], percentile: f64) -> u64 {
    if sorted_data.is_empty() {
        return 0;
    }

    let index = (sorted_data.len() as f64 * percentile) as usize;
    sorted_data[std::cmp::min(index, sorted_data.len() - 1)]
}