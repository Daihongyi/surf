use anyhow::{anyhow, Result};
use futures_util::StreamExt; // 添加 StreamExt 导入
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

pub fn build_client(
    follow_redirects: bool,
    timeout: u64,
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

    // Set timeout
    client_builder = client_builder.timeout(Duration::from_secs(timeout));

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

    // HTTP/3 support (requires rustls and quinn)
    #[cfg(feature = "http3")]
    if http3 {
        client_builder = client_builder
            .use_rustls_tls()
            .http3_prior_knowledge();
    }

    Ok(client_builder.build()?)
}

pub async fn download_file(
    url: &str,
    output: &PathBuf,
    _parallel: usize, // 暂时未使用，添加下划线前缀
    continue_download: bool,
    timeout: u64,
) -> Result<()> {
    let client = build_client(true, timeout, false, vec![])?;

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

    // Create progress bar with speed display
    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta}) | {binary_bytes_per_sec}")?
        .progress_chars("#>-"));
    pb.set_position(downloaded);

    // Download file
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output)
        .await?;

    // 记录下载开始时间和初始下载量
    let start_time = Instant::now();
    let initial_downloaded = downloaded;

    if downloaded < total_size {
        let mut request = client.get(url);
        if downloaded > 0 {
            request = request.header(
                "Range",
                format!("bytes={}-", downloaded), // 修复 Range 头格式
            );
        }

        let response = request.send().await?;
        if response.status() != StatusCode::OK && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("Failed to download: {}", response.status()));
        }

        let mut stream = response.bytes_stream();

        // 使用 StreamExt 的 next() 方法
        while let Some(item) = stream.next().await {
            let chunk = item?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            pb.set_position(downloaded);
        }
    }

    // 计算下载速度和平均速度
    let elapsed_time = start_time.elapsed();
    let download_size = downloaded - initial_downloaded;
    let avg_speed = if elapsed_time.as_secs_f64() > 0.0 {
        download_size as f64 / elapsed_time.as_secs_f64()
    } else {
        download_size as f64
    };

    // 获取绝对路径并格式化输出
    let abs_path = output.canonicalize().unwrap_or_else(|_| output.clone());
    let abs_path_str = abs_path.display().to_string();

    pb.finish_with_message(format!(
        "Downloaded {} in {:.2}s (avg: {}/s) to: {}",
        HumanBytes(downloaded),
        elapsed_time.as_secs_f64(),
        HumanBytes(avg_speed as u64),
        abs_path_str
    ));

    Ok(())
}

pub async fn benchmark_url(url: &str, requests: usize, concurrency: usize, timeout: u64) -> Result<()> {
    let client = build_client(true, timeout, false, vec![])?;

    println!("Benchmarking {} with {} requests, concurrency {}", url, requests, concurrency);

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