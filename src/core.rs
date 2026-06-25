use crate::log::{log_info, log_error, log_debug, log_warn};
use crate::resume::{DownloadMetadata, ResumeManager, ChunkStatus};
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

/// 客户端类型枚举，用于区分不同场景的超时策略
#[derive(Debug, Clone, Copy)]
pub enum ClientType {
    Get,
    Download,
    Benchmark,
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

/// 安全截断字符串，确保不切断 UTF-8 字符
fn truncate_url(url: &str, max_len: usize) -> String {
    if url.len() <= max_len {
        url.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !url.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &url[..end])
    }
}

pub fn build_client(
    follow_redirects: bool,
    connect_timeout: u64,
    http3: bool,
    headers: Vec<String>,
    client_type: ClientType,
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

    client_builder = client_builder.connect_timeout(Duration::from_secs(connect_timeout));

    match client_type {
        ClientType::Get => {
            client_builder = client_builder.timeout(Duration::from_secs(300));
            log_debug("Client configured with 300s total timeout for GET requests");
        }
        ClientType::Download => {
            log_debug("Client configured WITHOUT total timeout for downloads (idle timeout only)");
        }
        ClientType::Benchmark => {
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
        output.display(), parallel, continue_download, idle_timeout
    ));

    let resume_manager: ResumeManager = ResumeManager::new()?;
    let client: Client = build_client(true, DEFAULT_CONNECT_TIMEOUT, http3, vec![], ClientType::Download)?;

    let (total_size, supports_range, etag, last_modified) = get_download_info_extended(&client, url).await?;

    log_info(&format!(
        "File size: {}",
        if total_size > 0 {
            format!("{}", HumanBytes(total_size))
        } else {
            "unknown".to_string()
        }
    ));
    log_debug(&format!("Range requests supported: {}", supports_range));

    // 检查是否存在有效的断点续传元数据
    let mut metadata: Option<DownloadMetadata> = if continue_download {
        resume_manager.check_existing_download(
            url,
            output,
            etag.as_deref(),
            last_modified.as_deref(),
        )?
    } else {
        None
    };

    let downloaded = if let Some(ref meta) = metadata {
        log_info(&format!(
            "Resuming download from previous session: {} / {} ({:.1}%)",
            HumanBytes(meta.downloaded),
            HumanBytes(meta.total_size),
            meta.get_progress_percentage()
        ));

        if meta.chunks.len() > 1 {
            let completed_chunks = meta.chunks.iter().filter(|c| c.status == ChunkStatus::Completed).count();
            log_info(&format!(
                "Chunks: {} completed, {} pending",
                completed_chunks,
                meta.chunks.len() - completed_chunks
            ));
        }
        meta.downloaded
    } else if continue_download && output.exists() {
        // 创建新的元数据
        let file_size = fs::metadata(output).await?.len();

        // 修复：校验文件大小不超过总大小
        if file_size > total_size && total_size > 0 {
            log_warn(&format!(
                "Existing file size ({}) exceeds total size ({}), truncating and starting fresh",
                HumanBytes(file_size),
                HumanBytes(total_size)
            ));
            // 截断文件
            fs::File::create(output).await?;
            metadata = Some(DownloadMetadata::new(
                url.to_string(),
                output.clone(),
                total_size,
                supports_range,
                etag.clone(),
                last_modified.clone(),
            ));
            0
        } else {
            metadata = Some(DownloadMetadata::new(
                url.to_string(),
                output.clone(),
                total_size,
                supports_range,
                etag.clone(),
                last_modified.clone(),
            ));
            if let Some(ref mut meta) = metadata {
                meta.downloaded = file_size;
                log_info(&format!(
                    "Creating new resume metadata, {} already downloaded",
                    HumanBytes(file_size)
                ));
            }
            file_size
        }
    } else {
        metadata = Some(DownloadMetadata::new(
            url.to_string(),
            output.clone(),
            total_size,
            supports_range,
            etag.clone(),
            last_modified.clone(),
        ));
        log_info("Starting new download");
        0
    };

    let use_parallel = supports_range
        && total_size > 0
        && parallel > 1
        && total_size > PARALLEL_DOWNLOAD_THRESHOLD
        && downloaded < total_size;

    // 初始化分片
    if let Some(ref mut meta) = metadata {
        if meta.chunks.is_empty() {
            meta.initialize_chunks(if use_parallel { parallel } else { 1 });
            if downloaded > 0 {
                update_chunks_from_progress(meta, downloaded);
            }
        }
    }

    let result = if use_parallel {
        log_info(&format!(
            "Using parallel download with {} connections",
            parallel
        ));
        download_parallel_with_resume(
            &client,
            url,
            output,
            total_size,
            metadata.as_mut().unwrap(),
            &resume_manager,
            idle_timeout,
        )
            .await
    } else {
        log_info("Using single connection download");
        download_single_with_resume(
            &client,
            url,
            output,
            total_size,
            metadata.as_mut().unwrap(),
            &resume_manager,
            idle_timeout,
        )
            .await
    };

    if let Some(ref mut meta) = metadata {
        match &result {
            Ok(_) => {
                meta.mark_completed();
                log_info("Download completed successfully");
                let _ = resume_manager.save_metadata(meta);
            }
            Err(e) => {
                meta.mark_failed(&e.to_string());
                log_error(&format!("Download failed: {}", e));
                let _ = resume_manager.save_metadata(meta);
            }
        }
    }
    result
}

/// 根据已下载字节数更新分片状态
fn update_chunks_from_progress(metadata: &mut DownloadMetadata, downloaded: u64) {
    let mut remaining = downloaded;
    for chunk in &mut metadata.chunks {
        let chunk_size = chunk.end - chunk.start;
        if remaining >= chunk_size {
            chunk.downloaded = chunk_size;
            chunk.status = ChunkStatus::Completed;
            remaining -= chunk_size;
        } else if remaining > 0 {
            chunk.downloaded = remaining;
            chunk.status = ChunkStatus::Downloading;
            remaining = 0;
        } else {
            chunk.downloaded = 0;
            chunk.status = ChunkStatus::Pending;
        }
    }
}

async fn get_download_info_extended(
    client: &Client,
    url: &str,
) -> Result<(u64, bool, Option<String>, Option<String>)> {
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

    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|et| et.to_str().ok())
        .map(|s| s.to_string());

    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|lm| lm.to_str().ok())
        .map(|s| s.to_string());

    Ok((total_size, supports_range, etag, last_modified))
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

    let client: Client = build_client(true, connect_timeout, http3, vec![], ClientType::Benchmark)?;

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

// ========== 断点续传支持函数 ==========

async fn download_single_with_resume(
    client: &Client,
    url: &str,
    output: &PathBuf,
    total_size: u64,
    metadata: &mut DownloadMetadata,
    resume_manager: &ResumeManager,
    idle_timeout: u64,
) -> Result<()> {
    log_debug("Starting single download with resume support");

    let chunk = &metadata.chunks[0];
    let start_from = chunk.start + chunk.downloaded;

    if start_from >= total_size && total_size > 0 {
        log_info("File already fully downloaded");
        return Ok(());
    }

    log_debug(&format!(
        "Downloading chunk 0: {}-{} (already downloaded: {})",
        chunk.start, chunk.end, chunk.downloaded
    ));

    let mut request = client.get(url);
    if start_from > 0 {
        log_debug(&format!("Adding Range header: bytes={}-", start_from));
        request = request.header(
            reqwest::header::RANGE,
            format!("bytes={}-", start_from),
        );
    }

    let response = request.send().await.context("Request failed")?;
    let status = response.status();
    log_debug(&format!("Response status: {}", status));

    if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        log_error(&format!("Server returned error status: {}", status));
        return Err(anyhow!("Server returned error status: {}", status));
    }

    let file = if start_from > 0 {
        log_debug("Opening file in append mode");
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output)
            .await
            .context("Failed to open file for appending")?
    } else {
        log_debug("Creating new file");
        fs::File::create(output)
            .await
            .context("Failed to create file")?
    };

    let pb = create_progress_bar(total_size, start_from);
    let mut stream = response.bytes_stream();
    let mut writer = tokio::io::BufWriter::new(file);
    let start_time = Instant::now();
    let idle_duration = Duration::from_secs(idle_timeout);
    let mut bytes_downloaded = start_from;
    let mut save_counter = 0;

    loop {
        match tokio::time::timeout(idle_duration, stream.next()).await {
            Ok(Some(chunk_result)) => {
                let chunk = chunk_result.context("Error receiving chunk")?;
                let chunk_len = chunk.len();
                writer
                    .write_all(&chunk)
                    .await
                    .context("Failed to write to file")?;
                pb.inc(chunk_len as u64);
                bytes_downloaded += chunk_len as u64;

                metadata.update_chunk_progress(0, bytes_downloaded - metadata.chunks[0].start);

                // 每下载 10MB 保存一次元数据
                save_counter += chunk_len;
                if save_counter >= 10_000_000 {
                    writer.flush().await?;
                    resume_manager.save_metadata(metadata)?;
                    save_counter = 0;
                    log_debug("Saved progress checkpoint");
                }
            }
            Ok(None) => {
                log_debug("Stream ended normally");
                break;
            }
            Err(_) => {
                writer.flush().await?;
                resume_manager.save_metadata(metadata)?;
                log_error(&format!("Idle timeout: no data received for {}s", idle_timeout));
                return Err(anyhow!(TimeoutError::IdleTimeout(idle_timeout)));
            }
        }
    }

    writer.flush().await.context("Failed to flush file")?;
    metadata.set_chunk_status(0, ChunkStatus::Completed);
    resume_manager.save_metadata(metadata)?;

    let elapsed = start_time.elapsed();
    let speed = if elapsed.as_secs_f64() > 0.0 {
        (bytes_downloaded - start_from) as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    pb.finish_with_message(format!(
        "Downloaded {} in {:.2}s (avg: {}/s)",
        HumanBytes(bytes_downloaded),
        elapsed.as_secs_f64(),
        HumanBytes(speed as u64)
    ));

    log_info(&format!(
        "Download completed successfully in {:.2}s",
        elapsed.as_secs_f64()
    ));

    Ok(())
}

/// 修复后的并行下载函数：使用共享原子计数器跟踪各分片进度，
/// 并在下载过程中定期保存元数据，避免中断后丢失进度。
async fn download_parallel_with_resume(
    client: &Client,
    url: &str,
    output: &PathBuf,
    total_size: u64,
    metadata: &mut DownloadMetadata,
    resume_manager: &ResumeManager,
    idle_timeout: u64,
) -> Result<()> {
    use futures_util::stream::FuturesUnordered;
    use std::sync::atomic::AtomicU64;

    log_info("Starting parallel download with resume support");

    let pending_chunks: Vec<usize> = metadata
        .chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.status != ChunkStatus::Completed)
        .map(|(i, _)| i)
        .collect();

    if pending_chunks.is_empty() {
        log_info("All chunks already downloaded");
        return Ok(());
    }

    log_info(&format!(
        "Resuming {} pending chunks out of {}",
        pending_chunks.len(),
        metadata.chunks.len()
    ));

    let pb = create_progress_bar(total_size, metadata.downloaded);
    let start_time = Instant::now();

    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(output)
        .context("Failed to open file for writing")?;

    // 修复：预分配文件大小，避免稀疏文件
    if total_size > 0 {
        file.set_len(total_size)
            .context("Failed to pre-allocate file size")?;
    }

    let file = Arc::new(file);
    let semaphore = Arc::new(Semaphore::new(pending_chunks.len()));

    // 修复：使用共享原子计数器跟踪每个分片的实时下载进度
    let chunk_progress: Vec<Arc<AtomicU64>> = metadata
        .chunks
        .iter()
        .map(|c| Arc::new(AtomicU64::new(c.downloaded)))
        .collect();

    let mut tasks = FuturesUnordered::new();

    for &chunk_index in &pending_chunks {
        let client = client.clone();
        let url = url.to_string();
        let pb = pb.clone();
        let file = Arc::clone(&file);
        let semaphore = Arc::clone(&semaphore);
        let chunk_info = metadata.chunks[chunk_index].clone();
        let progress = Arc::clone(&chunk_progress[chunk_index]);
        let idx = chunk_index;

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await?;

            let start = chunk_info.start + chunk_info.downloaded;
            // 修复：使用 saturating_sub 防止整数下溢
            let end = chunk_info.end.saturating_sub(1);

            if start >= chunk_info.end {
                log_debug(&format!("Chunk {} already completed", idx));
                return Ok::<(), anyhow::Error>(());
            }

            log_debug(&format!(
                "Downloading chunk {}: bytes={}-{} (resume from {})",
                idx, chunk_info.start, end, start
            ));

            let response = client
                .get(&url)
                .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
                .send()
                .await
                .context("Request failed")?;

            if !response.status().is_success() && response.status() != StatusCode::PARTIAL_CONTENT {
                return Err(anyhow!("Chunk {} failed with status: {}", idx, response.status()));
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
                                use std::io::{Seek, SeekFrom, Write};
                                let mut f = &*file_clone;
                                f.seek(SeekFrom::Start(current_chunk_pos))?;
                                f.write_all(&chunk)?;
                            }
                            Ok::<(), std::io::Error>(())
                        })
                            .await
                            .context("Spawn blocking write failed")?
                            .context("File write operation failed")?;

                        pb.inc(chunk_len as u64);
                        current_pos += chunk_len as u64;
                        // 修复：实时更新共享进度计数器
                        progress.store(current_pos - chunk_info.start, Ordering::Relaxed);
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

        tasks.push(async move {
            let result = task.await;
            (chunk_index, result)
        });
    }

    let mut last_save = Instant::now();
    let save_interval = Duration::from_secs(10);

    // 修复：使用 select! 在等待任务完成的同时定期保存进度
    while !tasks.is_empty() {
        tokio::select! {
            Some((chunk_index, result)) = tasks.next() => {
                // 从共享原子计数器读取最终进度
                let downloaded = chunk_progress[chunk_index].load(Ordering::Relaxed);
                metadata.update_chunk_progress(chunk_index, downloaded);

                match result {
                    Ok(Ok(())) => {
                        log_debug(&format!("Chunk {} completed successfully", chunk_index));
                        metadata.set_chunk_status(chunk_index, ChunkStatus::Completed);
                        resume_manager.save_metadata(metadata)?;
                        last_save = Instant::now();
                    }
                    Ok(Err(e)) => {
                        log_error(&format!("Chunk {} failed: {}", chunk_index, e));
                        metadata.set_chunk_status(chunk_index, ChunkStatus::Failed);
                        resume_manager.save_metadata(metadata)?;
                        pb.abandon_with_message(format!("Chunk {} failed: {}", chunk_index, e));
                        return Err(e);
                    }
                    Err(e) => {
                        log_error(&format!("Chunk {} task panicked: {}", chunk_index, e));
                        metadata.set_chunk_status(chunk_index, ChunkStatus::Failed);
                        resume_manager.save_metadata(metadata)?;
                        pb.abandon_with_message(format!("Chunk {} panicked", chunk_index));
                        return Err(anyhow!("Chunk task panicked: {}", e));
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                // 修复：定期从原子计数器读取进度并保存元数据
                if last_save.elapsed() >= save_interval {
                    for (i, progress) in chunk_progress.iter().enumerate() {
                        let downloaded = progress.load(Ordering::Relaxed);
                        metadata.update_chunk_progress(i, downloaded);
                    }
                    let _ = resume_manager.save_metadata(metadata);
                    last_save = Instant::now();
                    log_debug("Saved progress checkpoint during parallel download");
                }
            }
        }
    }

    // 最终保存：从原子计数器读取所有分片最终进度
    for (i, progress) in chunk_progress.iter().enumerate() {
        let downloaded = progress.load(Ordering::Relaxed);
        metadata.update_chunk_progress(i, downloaded);
    }
    resume_manager.save_metadata(metadata)?;

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
