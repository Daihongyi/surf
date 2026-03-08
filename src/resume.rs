use anyhow::{anyhow, Context, Result};  // ← 关键修复
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use sha2::{Sha256, Digest};
use hex;
/// 下载元数据，用于断点续传
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadata {
    /// 下载 URL
    pub url: String,
    /// URL 的 SHA256 哈希值，用作唯一标识
    pub url_hash: String,
    /// 目标文件路径
    pub output_path: PathBuf,
    /// 文件总大小（字节）
    pub total_size: u64,
    /// 已下载的字节数
    pub downloaded: u64,
    /// 是否支持范围请求
    pub supports_range: bool,
    /// ETag（如果服务器提供）
    pub etag: Option<String>,
    /// Last-Modified（如果服务器提供）
    pub last_modified: Option<String>,
    /// 下载开始时间（Unix 时间戳）
    pub start_time: u64,
    /// 最后更新时间（Unix 时间戳）
    pub last_update_time: u64,
    /// 下载分片信息（用于并行下载）
    pub chunks: Vec<ChunkInfo>,
    /// 下载状态
    pub status: DownloadStatus,
    /// 错误信息（如果有）
    pub error_message: Option<String>,
}

/// 下载分片信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    /// 分片索引
    pub index: usize,
    /// 分片起始位置
    pub start: u64,
    /// 分片结束位置
    pub end: u64,
    /// 已下载的字节数
    pub downloaded: u64,
    /// 分片状态
    pub status: ChunkStatus,
}

/// 下载状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DownloadStatus {
    /// 进行中
    InProgress,
    /// 已暂停
    Paused,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

/// 分片状态
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChunkStatus {
    /// 等待下载
    Pending,
    /// 下载中
    Downloading,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

impl DownloadMetadata {
    /// 创建新的下载元数据
    pub fn new(
        url: String,
        output_path: PathBuf,
        total_size: u64,
        supports_range: bool,
        etag: Option<String>,
        last_modified: Option<String>,
    ) -> Self {
        let url_hash = Self::hash_url(&url);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            url,
            url_hash,
            output_path,
            total_size,
            downloaded: 0,
            supports_range,
            etag,
            last_modified,
            start_time: now,
            last_update_time: now,
            chunks: Vec::new(),
            status: DownloadStatus::InProgress,
            error_message: None,
        }
    }

    /// 计算 URL 的 SHA256 哈希值
    fn hash_url(url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// 初始化下载分片
    pub fn initialize_chunks(&mut self, num_chunks: usize) {
        if !self.supports_range || self.total_size == 0 || num_chunks <= 1 {
            // 单线程下载，创建一个分片
            self.chunks = vec![ChunkInfo {
                index: 0,
                start: 0,
                end: self.total_size,
                downloaded: 0,
                status: ChunkStatus::Pending,
            }];
            return;
        }

        // 计算每个分片的大小
        let chunk_size = self.total_size / num_chunks as u64;
        let mut chunks = Vec::new();

        for i in 0..num_chunks {
            let start = i as u64 * chunk_size;
            let end = if i == num_chunks - 1 {
                self.total_size
            } else {
                (i as u64 + 1) * chunk_size
            };

            chunks.push(ChunkInfo {
                index: i,
                start,
                end,
                downloaded: 0,
                status: ChunkStatus::Pending,
            });
        }

        self.chunks = chunks;
    }

    /// 更新分片下载进度
    pub fn update_chunk_progress(&mut self, chunk_index: usize, downloaded: u64) {
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.downloaded = downloaded;
            if downloaded >= chunk.end - chunk.start {
                chunk.status = ChunkStatus::Completed;
            }
        }
        self.update_total_downloaded();
        self.update_timestamp();
    }

    /// 标记分片状态
    pub fn set_chunk_status(&mut self, chunk_index: usize, status: ChunkStatus) {
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.status = status;
        }
        self.update_timestamp();
    }

    /// 计算总下载字节数
    fn update_total_downloaded(&mut self) {
        self.downloaded = self.chunks.iter().map(|c| c.downloaded).sum();
    }

    /// 更新时间戳
    fn update_timestamp(&mut self) {
        self.last_update_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    /// 检查下载是否完成
    pub fn is_completed(&self) -> bool {
        self.status == DownloadStatus::Completed
            || (self.downloaded >= self.total_size
            && self.chunks.iter().all(|c| c.status == ChunkStatus::Completed))
    }

    /// 获取未完成的分片
    pub fn get_pending_chunks(&self) -> Vec<&ChunkInfo> {
        self.chunks
            .iter()
            .filter(|c| c.status != ChunkStatus::Completed)
            .collect()
    }

    /// 获取下载进度百分比
    pub fn get_progress_percentage(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            (self.downloaded as f64 / self.total_size as f64) * 100.0
        }
    }

    /// 标记下载完成
    pub fn mark_completed(&mut self) {
        self.status = DownloadStatus::Completed;
        for chunk in &mut self.chunks {
            chunk.status = ChunkStatus::Completed;
        }
        self.update_timestamp();
    }

    /// 标记下载失败
    pub fn mark_failed(&mut self, error: &str) {
        self.status = DownloadStatus::Failed;
        self.error_message = Some(error.to_string());
        self.update_timestamp();
    }

    /// 标记下载暂停
    pub fn mark_paused(&mut self) {
        self.status = DownloadStatus::Paused;
        self.update_timestamp();
    }

    /// 验证元数据是否与当前下载匹配
    pub fn validate(&self, url: &str, etag: Option<&str>, last_modified: Option<&str>) -> bool {
        // URL 必须匹配
        if self.url != url {
            return false;
        }

        // 如果服务器提供了 ETag，检查是否匹配
        if let Some(server_etag) = etag {
            if let Some(ref meta_etag) = self.etag {
                if server_etag != meta_etag {
                    return false;
                }
            }
        }

        // 如果服务器提供了 Last-Modified，检查是否匹配
        if let Some(server_modified) = last_modified {
            if let Some(ref meta_modified) = self.last_modified {
                if server_modified != meta_modified {
                    return false;
                }
            }
        }

        true
    }
}

/// 断点续传管理器
pub struct ResumeManager {
    metadata_dir: PathBuf,
}

impl ResumeManager {
    /// 创建新的断点续传管理器
    pub fn new() -> Result<Self> {
        let metadata_dir = Self::get_metadata_dir()?;
        fs::create_dir_all(&metadata_dir)?;

        Ok(Self { metadata_dir })
    }

    /// 获取元数据目录
    fn get_metadata_dir() -> Result<PathBuf> {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("surf")
            .join("downloads");
        Ok(dir)
    }

    /// 获取元数据文件路径
    fn get_metadata_path(&self, url_hash: &str) -> PathBuf {
        self.metadata_dir.join(format!("{}.json", url_hash))
    }

    /// 保存下载元数据
    pub fn save_metadata(&self, metadata: &DownloadMetadata) -> Result<()> {
        let path = self.get_metadata_path(&metadata.url_hash);
        let content = serde_json::to_string_pretty(metadata)
            .context("Failed to serialize download metadata")?;
        fs::write(&path, content).context("Failed to write metadata file")?;
        Ok(())
    }

    /// 加载下载元数据
    pub fn load_metadata(&self, url: &str) -> Result<Option<DownloadMetadata>> {
        let url_hash = DownloadMetadata::hash_url(url);
        let path = self.get_metadata_path(&url_hash);

        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path).context("Failed to read metadata file")?;
        let metadata: DownloadMetadata =
            serde_json::from_str(&content).context("Failed to parse metadata file")?;

        Ok(Some(metadata))
    }

    /// 删除下载元数据
    pub fn delete_metadata(&self, url: &str) -> Result<()> {
        let url_hash = DownloadMetadata::hash_url(url);
        let path = self.get_metadata_path(&url_hash);

        if path.exists() {
            fs::remove_file(&path).context("Failed to delete metadata file")?;
        }

        Ok(())
    }

    /// 列出所有下载元数据
    pub fn list_all_downloads(&self) -> Result<Vec<DownloadMetadata>> {
        let mut downloads = Vec::new();

        if !self.metadata_dir.exists() {
            return Ok(downloads);
        }

        for entry in fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<DownloadMetadata>(&content) {
                        downloads.push(metadata);
                    }
                }
            }
        }

        // 按最后更新时间排序
        downloads.sort_by(|a, b| b.last_update_time.cmp(&a.last_update_time));

        Ok(downloads)
    }

    /// 清理旧的元数据（删除超过指定天数的已完成或失败的下载）
    pub fn cleanup_old_metadata(&self, days: u64) -> Result<usize> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let threshold = now - (days * 24 * 60 * 60);

        let mut cleaned = 0;

        for entry in fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(metadata) = serde_json::from_str::<DownloadMetadata>(&content) {
                        // 只清理已完成或失败的下载
                        if (metadata.status == DownloadStatus::Completed
                            || metadata.status == DownloadStatus::Failed)
                            && metadata.last_update_time < threshold
                        {
                            if fs::remove_file(&path).is_ok() {
                                cleaned += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }

    /// 检查并验证已存在的下载
    pub fn check_existing_download(
        &self,
        url: &str,
        output_path: &Path,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<Option<DownloadMetadata>> {
        if let Some(metadata) = self.load_metadata(url)? {
            // 验证元数据是否有效
            if !metadata.validate(url, etag, last_modified) {
                return Ok(None);
            }

            // 检查输出文件是否存在
            if !output_path.exists() {
                return Ok(None);
            }

            // 检查文件大小是否匹配
            if let Ok(file_metadata) = fs::metadata(output_path) {
                let file_size = file_metadata.len();

                // 如果文件大小与元数据不符，重新开始
                if file_size != metadata.downloaded {
                    return Ok(None);
                }

                // 如果已完成，验证文件大小
                if metadata.is_completed() && file_size != metadata.total_size {
                    return Ok(None);
                }
            }

            Ok(Some(metadata))
        } else {
            Ok(None)
        }
    }
}

impl Default for ResumeManager {
    fn default() -> Self {
        Self::new().expect("Failed to create ResumeManager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_metadata_creation() {
        let url = "https://example.com/file.zip".to_string();
        let output = PathBuf::from("/tmp/file.zip");
        let metadata = DownloadMetadata::new(
            url.clone(),
            output.clone(),
            1000,
            true,
            Some("etag123".to_string()),
            None,
        );

        assert_eq!(metadata.url, url);
        assert_eq!(metadata.output_path, output);
        assert_eq!(metadata.total_size, 1000);
        assert_eq!(metadata.downloaded, 0);
        assert!(metadata.supports_range);
    }

    #[test]
    fn test_chunk_initialization() {
        let mut metadata = DownloadMetadata::new(
            "https://example.com/file.zip".to_string(),
            PathBuf::from("/tmp/file.zip"),
            1000,
            true,
            None,
            None,
        );

        metadata.initialize_chunks(4);
        assert_eq!(metadata.chunks.len(), 4);
        assert_eq!(metadata.chunks[0].start, 0);
        assert_eq!(metadata.chunks[0].end, 250);
        assert_eq!(metadata.chunks[3].start, 750);
        assert_eq!(metadata.chunks[3].end, 1000);
    }

    #[test]
    fn test_progress_calculation() {
        let mut metadata = DownloadMetadata::new(
            "https://example.com/file.zip".to_string(),
            PathBuf::from("/tmp/file.zip"),
            1000,
            true,
            None,
            None,
        );

        metadata.initialize_chunks(2);
        metadata.update_chunk_progress(0, 250);
        metadata.update_chunk_progress(1, 250);

        assert_eq!(metadata.downloaded, 500);
        assert_eq!(metadata.get_progress_percentage(), 50.0);
    }

    #[test]
    fn test_validation() {
        let metadata = DownloadMetadata::new(
            "https://example.com/file.zip".to_string(),
            PathBuf::from("/tmp/file.zip"),
            1000,
            true,
            Some("etag123".to_string()),
            Some("Mon, 01 Jan 2024 00:00:00 GMT".to_string()),
        );

        // 相同的 URL 和 ETag 应该通过验证
        assert!(metadata.validate(
            "https://example.com/file.zip",
            Some("etag123"),
            Some("Mon, 01 Jan 2024 00:00:00 GMT")
        ));

        // 不同的 ETag 应该失败
        assert!(!metadata.validate(
            "https://example.com/file.zip",
            Some("different_etag"),
            Some("Mon, 01 Jan 2024 00:00:00 GMT")
        ));

        // 不同的 URL 应该失败
        assert!(!metadata.validate("https://example.com/other.zip", Some("etag123"), None));
    }
}