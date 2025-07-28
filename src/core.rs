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
    let mut client_builder = ClientBuilder::new();

    // Set redirect policy
    let redirect_policy = if follow_redirects {
        Policy::limited(10)
    } else {
        Policy::none()
    };
    client_builder = client_builder.redirect(redirect_policy);

    // Set connection timeout only
    client_builder = client_builder.connect_timeout(Duration::from_secs(connect_timeout));

    // Add custom headers
    let mut header_map = HeaderMap::new();
    for header in headers {
        if let Some((key, value)) = header.split_once(':') {
            let header_name = HeaderName::from_str(key.trim())?;
            let header_value = HeaderValue::from_str(value.trim())?;
            header_map.insert(header_name, header_value);
        }
    }
    client_builder = client_builder.default_headers(header_map);

    // HTTP/3 support with compile-time check
    if http3 {
        #[cfg(not(feature = "http3"))]
        {
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
        }
    }

    Ok(client_builder.build()?)
}

pub async fn download_file(
    url: &str,
    output: &PathBuf,
    _parallel: usize,
    continue_download: bool,
    idle_timeout: u64,
    http3: bool,
) -> Result<()> {
    let client = build_client(true, 10, http3, vec![])?;

    // Get file size
    let head_response = client.head(url).send().await?;
    let total_size = head_response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|ct_len| ct_len.to_str().ok())
        .and_then(|ct_len| ct_len.parse::<u64>().ok())
        .unwrap_or(0);

    // Check if we can resume
    let mut downloaded = 0;
    if continue_download && output.exists() {
        let metadata = fs::metadata(output).await?;
        downloaded = metadata.len();
    }

    // Create progress bar with timeout status display
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) | {binary_bytes_per_sec} | {msg}")?
            .progress_chars("#>-"),
    );
    pb.set_position(downloaded);
    // 修改点1: Connecting显示为橙色
    pb.set_message("\x1b[33mConnecting...\x1b[0m");

    // Download file
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .await?;

    // Record start time and initial downloaded bytes
    let start_time = Instant::now();
    let initial_downloaded = downloaded;

    if downloaded < total_size {
        let mut request = client.get(url);
        if downloaded > 0 {
            request = request.header("Range", format!("bytes={}-", downloaded));
        }

        // 修改点2: Downloading显示为绿色
        pb.set_message("\x1b[32mDownloading...\x1b[0m");

        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow!(TimeoutError::ConnectTimeout)
            } else {
                anyhow!(e)
            }
        })?;

        if response.status() != StatusCode::OK && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("Failed to download: {}", response.status()));
        }

        let mut stream = response.bytes_stream();
        let mut last_data_time = Instant::now();
        let idle_duration = Duration::from_secs(idle_timeout);

        while let Some(item) = stream.next().await {
            // Check for idle timeout
            if last_data_time.elapsed() > idle_duration {
                // 修改点3: Timeout显示为红色
                pb.set_message("\x1b[31mIDLE TIMEOUT\x1b[0m");
                return Err(anyhow!(TimeoutError::IdleTimeout));
            }

            let chunk = item?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);

            // Update last data time
            last_data_time = Instant::now();
        }
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
    pb.finish_with_message(format!(
        "Downloaded {} in {:.2}s (avg: {}/s) to: {}",
        HumanBytes(downloaded),
        elapsed_time.as_secs_f64(),
        HumanBytes(avg_speed as u64),
        abs_path_str
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
    let client = build_client(true, connect_timeout, http3, vec![])?;

    println!(
        "Benchmarking {} with {} requests, concurrency {} (HTTP/3: {})",
        url, requests, concurrency, http3
    );

    let start = Instant::now();

    let mut tasks = Vec::new();
    for _ in 0..requests {
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
        }
    }

    // Wait for remaining tasks
    for task in tasks {
        let (duration, status) = task.await?;
        println!("Status: {:3} | Time: {:?}", status, duration);
    }

    let total_time = start.elapsed();
    let rps = requests as f64 / total_time.as_secs_f64();

    println!("\nBenchmark complete");
    println!("Total time: {:.2}s", total_time.as_secs_f64());
    println!("Requests per second: {:.2}", rps);

    Ok(())
}