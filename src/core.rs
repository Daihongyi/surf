use crate::log::{log_info, log_error, log_debug, log_warn};
use anyhow::{anyhow, Result};
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

#[derive(Debug)]
pub enum TimeoutError {
    IdleTimeout,
    ConnectTimeout,
}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TimeoutError::IdleTimeout => write!(f, "Idle timeout: no data received"),
            TimeoutError::ConnectTimeout => write!(f, "Connection timeout"),
        }
    }
}

impl std::error::Error for TimeoutError {}

pub fn build_client(
    follow_redirects: bool,
    connect_timeout: u64,
    http3: bool,
    headers: Vec<String>,
) -> Result<Client> {
    log_debug(&format!("Building HTTP client - redirects: {}, timeout: {}s, http3: {}",
                       follow_redirects, connect_timeout, http3));

    let mut client_builder = ClientBuilder::new();

    // Set redirect policy
    let redirect_policy = if follow_redirects {
        Policy::limited(10)
    } else {
        Policy::none()
    };
    client_builder = client_builder.redirect(redirect_policy);
    log_debug(&format!("Redirect policy set: {}", if follow_redirects { "limited(10)" } else { "none" }));

    // Set connection timeout only
    client_builder = client_builder.connect_timeout(Duration::from_secs(connect_timeout));

    // Set total timeout for requests
    client_builder = client_builder.timeout(Duration::from_secs(connect_timeout * 3));

    // Add custom headers
    let mut header_map = HeaderMap::new();
    for header in headers {
        if let Some((key, value)) = header.split_once(':') {
            match (HeaderName::from_str(key.trim()), HeaderValue::from_str(value.trim())) {
                (Ok(header_name), Ok(header_value)) => {
                    header_map.insert(header_name, header_value);
                    log_debug(&format!("Added custom header: {} = {}", key.trim(), value.trim()));
                }
                (Err(e), _) => {
                    log_error(&format!("Invalid header name '{}': {}", key.trim(), e));
                    return Err(anyhow!("Invalid header name: {}", key.trim()));
                }
                (_, Err(e)) => {
                    log_error(&format!("Invalid header value '{}': {}", value.trim(), e));
                    return Err(anyhow!("Invalid header value: {}", value.trim()));
                }
            }
        } else {
            log_warn(&format!("Malformed header ignored: '{}'", header));
        }
    }
    client_builder = client_builder.default_headers(header_map);

    // HTTP/3 support with compile-time check
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
            client_builder = client_builder
                .use_rustls_tls()
                .http3_prior_knowledge();
            log_debug("HTTP/3 support enabled");
        }
    }

    match client_builder.build() {
        Ok(client) => {
            log_info("HTTP client built successfully");
            Ok(client)
        }
        Err(e) => {
            log_error(&format!("Failed to build HTTP client: {}", e));
            Err(anyhow!("Failed to build HTTP client: {}", e))
        }
    }
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
    log_debug(&format!("Download settings - output: {}, parallel: {}, continue: {}, idle_timeout: {}s",
                       output.display(), parallel, continue_download, idle_timeout));

    let client = build_client(true, 10, http3, vec![])?;

    // Get file size and check if server supports range requests
    log_debug("Sending HEAD request to get file size");
    let head_response = match client.head(url).send().await {
        Ok(response) => {
            log_debug(&format!("HEAD request successful, status: {}", response.status()));
            response
        }
        Err(e) => {
            log_error(&format!("HEAD request failed: {}", e));
            return Err(e.into());
        }
    };

    let total_size = head_response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|ct_len| ct_len.to_str().ok())
        .and_then(|ct_len| ct_len.parse::<u64>().ok())
        .unwrap_or(0);

    let supports_range = head_response
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|ar| ar.to_str().ok())
        .map(|ar| ar == "bytes")
        .unwrap_or(false);

    log_info(&format!("File size: {}", if total_size > 0 {
        format!("{}", HumanBytes(total_size))
    } else {
        "unknown".to_string()
    }));

    log_debug(&format!("Range requests supported: {}", supports_range));

    // Check if we can resume
    let mut downloaded = 0;
    if continue_download && output.exists() {
        let metadata = fs::metadata(output).await?;
        downloaded = metadata.len();
        log_info(&format!("Resuming download, {} already downloaded", HumanBytes(downloaded)));
    }

    // Use parallel download only if server supports range requests and file is large enough
    let use_parallel = supports_range && total_size > 0 && parallel > 1 && total_size > 10_000_000; // 10MB threshold

    if use_parallel {
        log_info(&format!("Using parallel download with {} connections", parallel));
        download_parallel(url, output, total_size, downloaded, parallel, idle_timeout, http3).await
    } else {
        log_info("Using single connection download");
        download_single(url, output, downloaded, total_size, idle_timeout, http3).await
    }
}

async fn download_single(
    url: &str,
    output: &PathBuf,
    downloaded: u64,
    total_size: u64,
    idle_timeout: u64,
    http3: bool,
) -> Result<()> {
    let client = build_client(true, 10, http3, vec![])?;

    // Create progress bar with timeout status display
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) | {binary_bytes_per_sec} | {msg}")?
            .progress_chars("#>-"),
    );
    pb.set_position(downloaded);
    pb.set_message("\x1b[33mConnecting...\x1b[0m");

    // Download file
    let mut file = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .await {
        Ok(file) => {
            log_debug(&format!("Opened output file: {}", output.display()));
            file
        }
        Err(e) => {
            log_error(&format!("Failed to open output file {}: {}", output.display(), e));
            return Err(e.into());
        }
    };

    // Record start time and initial downloaded bytes
    let start_time = Instant::now();
    let initial_downloaded = downloaded;
    let mut current_downloaded = downloaded;

    if current_downloaded < total_size || total_size == 0 {
        let mut request = client.get(url);
        if current_downloaded > 0 {
            request = request.header("Range", format!("bytes={}-", current_downloaded));
            log_debug(&format!("Using Range header: bytes={}-", current_downloaded));
        }

        pb.set_message("\x1b[32mDownloading...\x1b[0m");

        let response = match request.send().await {
            Ok(response) => {
                log_debug(&format!("Download request successful, status: {}", response.status()));
                response
            }
            Err(e) => {
                log_error(&format!("Download request failed: {}", e));
                if e.is_timeout() {
                    return Err(anyhow!(TimeoutError::ConnectTimeout));
                } else {
                    return Err(anyhow!(e));
                }
            }
        };

        if response.status() != StatusCode::OK && response.status() != StatusCode::PARTIAL_CONTENT {
            let error_msg = format!("Failed to download: {}", response.status());
            log_error(&error_msg);
            return Err(anyhow!(error_msg));
        }

        let mut stream = response.bytes_stream();
        let mut last_data_time = Instant::now();
        let idle_duration = Duration::from_secs(idle_timeout);
        let mut chunk_count = 0;

        while let Some(item) = stream.next().await {
            // Check for idle timeout
            if last_data_time.elapsed() > idle_duration {
                pb.set_message("\x1b[31mIDLE TIMEOUT\x1b[0m");
                log_error("Download failed due to idle timeout");
                return Err(anyhow!(TimeoutError::IdleTimeout));
            }

            let chunk = match item {
                Ok(chunk) => chunk,
                Err(e) => {
                    log_error(&format!("Error receiving chunk: {}", e));
                    return Err(e.into());
                }
            };

            if let Err(e) = file.write_all(&chunk).await {
                log_error(&format!("Error writing to file: {}", e));
                return Err(e.into());
            }

            current_downloaded += chunk.len() as u64;
            pb.set_position(current_downloaded);
            chunk_count += 1;

            // Log progress every 1000 chunks to avoid spam
            if chunk_count % 1000 == 0 {
                log_debug(&format!("Downloaded {} bytes so far", current_downloaded));
            }

            // Update last data time
            last_data_time = Instant::now();
        }

        log_info(&format!("Download stream completed, total chunks: {}", chunk_count));
    } else {
        log_info("File already fully downloaded");
    }

    // Calculate download speed
    let elapsed_time = start_time.elapsed();
    let download_size = current_downloaded - initial_downloaded;
    let avg_speed = if elapsed_time.as_secs_f64() > 0.0 {
        download_size as f64 / elapsed_time.as_secs_f64()
    } else {
        download_size as f64
    };

    // Get absolute path for output
    let abs_path = output.canonicalize().unwrap_or_else(|_| output.clone());
    let abs_path_str = abs_path.display().to_string();

    pb.set_message("Completed");
    let completion_msg = format!(
        "Downloaded {} in {:.2}s (avg: {}/s) to: {}",
        HumanBytes(current_downloaded),
        elapsed_time.as_secs_f64(),
        HumanBytes(avg_speed as u64),
        abs_path_str
    );
    pb.finish_with_message(completion_msg.clone());

    log_info(&format!("Download completed successfully: {}", completion_msg));

    Ok(())
}

async fn download_parallel(
    url: &str,
    output: &PathBuf,
    total_size: u64,
    downloaded: u64,
    parallel: usize,
    idle_timeout: u64,
    http3: bool,
) -> Result<()> {
    let remaining = total_size - downloaded;
    let chunk_size = remaining / parallel as u64;

    if chunk_size == 0 {
        return download_single(url, output, downloaded, total_size, idle_timeout, http3).await;
    }

    log_info(&format!("Parallel download: {} chunks of ~{} bytes each",
                      parallel, HumanBytes(chunk_size)));

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) | {binary_bytes_per_sec}")?
            .progress_chars("#>-"),
    );
    pb.set_position(downloaded);

    let client = build_client(true, 10, http3, vec![])?;
    let semaphore = Arc::new(Semaphore::new(parallel));
    let pb = Arc::new(pb);

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
            let _permit = semaphore.acquire().await?;

            let mut request = client.get(&url);
            request = request.header("Range", format!("bytes={}-{}", start, end));

            let response = request.send().await?;
            if response.status() != StatusCode::PARTIAL_CONTENT {
                return Err(anyhow!("Server doesn't support range requests"));
            }

            let mut file = fs::File::create(&temp_path).await?;
            let mut stream = response.bytes_stream();
            let mut bytes_written = 0u64;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                bytes_written += chunk.len() as u64;
                pb.inc(chunk.len() as u64);
            }

            Ok::<(PathBuf, u64), anyhow::Error>((temp_path, bytes_written))
        });

        tasks.push(task);
    }

    // Wait for all downloads to complete
    let mut temp_files = Vec::new();
    for task in tasks {
        let (temp_path, _) = task.await??;
        temp_files.push(temp_path);
    }

    // Merge all temporary files
    pb.set_message("Merging files...");
    let mut final_file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(output)
        .await?;

    for temp_file in &temp_files {
        let mut temp = fs::File::open(temp_file).await?;
        tokio::io::copy(&mut temp, &mut final_file).await?;
    }

    // Clean up temporary files
    for temp_file in temp_files {
        let _ = fs::remove_file(temp_file).await;
    }

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

pub async fn benchmark_url(
    url: &str,
    requests: usize,
    concurrency: usize,
    connect_timeout: u64,
    http3: bool,
) -> Result<()> {
    log_info(&format!("Starting benchmark - URL: {}, requests: {}, concurrency: {}",
                      url, requests, concurrency));

    let client = build_client(true, connect_timeout, http3, vec![])?;

    println!(
        "Benchmarking {} with {} requests, concurrency {} (HTTP/3: {})",
        url, requests, concurrency, http3
    );

    let start = Instant::now();
    let mut successful_requests = 0;
    let mut failed_requests = 0;
    let mut response_times = Vec::new();
    let mut status_codes = std::collections::HashMap::new();

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = Vec::new();

    for i in 0..requests {
        let client = client.clone();
        let url = url.to_string();
        let semaphore = Arc::clone(&semaphore);

        let task = tokio::spawn(async move {
            let _permit = semaphore.acquire().await?;
            let start = Instant::now();
            let response = client.get(&url).send().await;
            let duration = start.elapsed();
            match response {
                Ok(resp) => Ok((duration, resp.status().as_u16())),
                Err(_) => Ok((duration, 0)),
            }
        });
        tasks.push(task);

        // Log progress every 50 requests
        if (i + 1) % 50 == 0 {
            log_debug(&format!("Started {} requests", i + 1));
        }
    }

    // Wait for all tasks
    for task in tasks {
        let result: Result<(Duration, u16), anyhow::Error> = task.await?;
        let (duration, status) = result?;

        response_times.push(duration.as_millis() as u64);
        *status_codes.entry(status).or_insert(0) += 1;

        if status >= 200 && status < 400 {
            successful_requests += 1;
        } else {
            failed_requests += 1;
            if status != 0 {
                log_warn(&format!("Request failed with status: {}", status));
            }
        }
    }

    let total_time = start.elapsed();
    let rps = requests as f64 / total_time.as_secs_f64();

    // Calculate statistics
    response_times.sort_unstable();
    let min_time = response_times.first().copied().unwrap_or(0);
    let max_time = response_times.last().copied().unwrap_or(0);
    let avg_time = response_times.iter().sum::<u64>() / response_times.len() as u64;
    let p50 = response_times.get(response_times.len() / 2).copied().unwrap_or(0);
    let p95 = response_times.get((response_times.len() as f64 * 0.95) as usize).copied().unwrap_or(0);
    let p99 = response_times.get((response_times.len() as f64 * 0.99) as usize).copied().unwrap_or(0);

    println!("\n=== Benchmark Results ===");
    println!("Total time: {:.2}s", total_time.as_secs_f64());
    println!("Requests per second: {:.2}", rps);
    println!("Successful requests: {}", successful_requests);
    println!("Failed requests: {}", failed_requests);
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
    for (status, count) in status_codes.iter() {
        println!("  {}: {} ({:.1}%)", status, count, (*count as f64 / requests as f64) * 100.0);
    }

    log_info(&format!("Benchmark completed - Total: {:.2}s, RPS: {:.2}, Success: {}, Failed: {}",
                      total_time.as_secs_f64(), rps, successful_requests, failed_requests));

    Ok(())
}