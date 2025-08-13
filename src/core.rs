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
    io::Write,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::{fs, io::AsyncWriteExt};

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
    _parallel: usize,
    continue_download: bool,
    idle_timeout: u64,
    http3: bool,
) -> Result<()> {
    log_info(&format!("Starting file download from: {}", url));
    log_debug(&format!("Download settings - output: {}, continue: {}, idle_timeout: {}s",
                       output.display(), continue_download, idle_timeout));

    let client = build_client(true, 10, http3, vec![])?;

    // Get file size
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

    log_info(&format!("File size: {}", if total_size > 0 {
        format!("{}", HumanBytes(total_size))
    } else {
        "unknown".to_string()
    }));

    // Check if we can resume
    let mut downloaded = 0;
    if continue_download && output.exists() {
        let metadata = fs::metadata(output).await?;
        downloaded = metadata.len();
        log_info(&format!("Resuming download, {} already downloaded", HumanBytes(downloaded)));
    }

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

    if downloaded < total_size {
        let mut request = client.get(url);
        if downloaded > 0 {
            request = request.header("Range", format!("bytes={}-", downloaded));
            log_debug(&format!("Using Range header: bytes={}-", downloaded));
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

            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
            chunk_count += 1;

            // Log progress every 1000 chunks to avoid spam
            if chunk_count % 1000 == 0 {
                log_debug(&format!("Downloaded {} bytes so far", downloaded));
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
    let download_size = downloaded - initial_downloaded;
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
        HumanBytes(downloaded),
        elapsed_time.as_secs_f64(),
        HumanBytes(avg_speed as u64),
        abs_path_str
    );
    pb.finish_with_message(completion_msg.clone());

    log_info(&format!("Download completed successfully: {}", completion_msg));

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

    let mut tasks = Vec::new();
    for i in 0..requests {
        let client = client.clone();
        let url = url.to_string();
        let task = tokio::spawn(async move {
            let start = Instant::now();
            let response = client.get(&url).send().await;
            let duration = start.elapsed();
            match response {
                Ok(resp) => (duration, resp.status().as_u16()),
                Err(_) => (duration, 0),
            }
        });
        tasks.push(task);

        if tasks.len() >= concurrency {
            let task = tasks.remove(0);
            let (duration, status) = task.await?;
            println!("Status: {:3} | Time: {:?}", status, duration);

            if status >= 200 && status < 400 {
                successful_requests += 1;
            } else {
                failed_requests += 1;
                log_warn(&format!("Request failed with status: {}", status));
            }

            // Log progress every 50 requests
            if (i + 1) % 50 == 0 {
                log_debug(&format!("Completed {} requests", i + 1));
            }
        }
    }

    // Wait for remaining tasks
    for task in tasks {
        let (duration, status) = task.await?;
        println!("Status: {:3} | Time: {:?}", status, duration);

        if status >= 200 && status < 400 {
            successful_requests += 1;
        } else {
            failed_requests += 1;
            log_warn(&format!("Request failed with status: {}", status));
        }
    }

    let total_time = start.elapsed();
    let rps = requests as f64 / total_time.as_secs_f64();

    println!("\nBenchmark complete");
    println!("Total time: {:.2}s", total_time.as_secs_f64());
    println!("Requests per second: {:.2}", rps);
    println!("Successful requests: {}", successful_requests);
    println!("Failed requests: {}", failed_requests);

    log_info(&format!("Benchmark completed - Total: {:.2}s, RPS: {:.2}, Success: {}, Failed: {}",
                      total_time.as_secs_f64(), rps, successful_requests, failed_requests));

    Ok(())
}