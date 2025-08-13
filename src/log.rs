use anyhow::Result;
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;

static LOGGER: OnceCell<Arc<Logger>> = OnceCell::const_new();

pub struct Logger {
    file: Option<Arc<Mutex<std::fs::File>>>,
    enabled: bool,
}

impl Logger {
    pub fn new(enabled: bool, log_dir: Option<PathBuf>) -> Result<Self> {
        let file = if enabled {
            let log_path = if let Some(dir) = log_dir {
                // 确保目录存在
                if let Some(parent) = dir.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                dir.join("surf.log")
            } else {
                // 默认当前目录
                PathBuf::from("surf.log")
            };

            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)?;

            // 记录日志文件位置
            println!("Logging enabled. Log file: {}", log_path.display());

            Some(Arc::new(Mutex::new(file)))
        } else {
            None
        };

        Ok(Logger { file, enabled })
    }

    pub fn log(&self, level: LogLevel, message: &str) {
        if !self.enabled {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_entry = format!("[{}] [{}] {}\n", timestamp, level.as_str(), message);

        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(log_entry.as_bytes());
                let _ = file.flush();
            }
        }
    }

    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message);
    }

    pub fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, message);
    }

    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message);
    }

    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message);
    }
}

#[derive(Debug, Clone)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Debug => "DEBUG",
        }
    }
}

pub async fn init_logger(enabled: bool, log_dir: Option<PathBuf>) -> Result<()> {
    let logger = Arc::new(Logger::new(enabled, log_dir)?);
    LOGGER.set(logger).map_err(|_| anyhow::anyhow!("Logger already initialized"))?;

    if enabled {
        log_info("Logger initialized - logging enabled");
    }

    Ok(())
}

pub fn log_info(message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.info(message);
    }
}

pub fn log_warn(message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.warn(message);
    }
}

pub fn log_error(message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.error(message);
    }
}

pub fn log_debug(message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.debug(message);
    }
}