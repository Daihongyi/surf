use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHistory {
    pub entries: Vec<HistoryEntry>,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub status_code: Option<u16>,
    pub response_time: Option<u64>, // in milliseconds
    pub response_size: Option<u64>, // in bytes
    pub success: bool,
    pub error_message: Option<String>,
}

impl Default for RequestHistory {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 1000,
        }
    }
}

impl RequestHistory {
    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(RequestHistory::default());
        }

        let content = fs::read_to_string(path)?;
        let history: RequestHistory = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse history file: {}", e))?;

        Ok(history)
    }

    pub fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow!("Failed to serialize history: {}", e))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, content)?;
        Ok(())
    }

    pub fn add_entry(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);

        // Keep only the most recent entries
        if self.entries.len() > self.max_entries {
            self.entries.drain(0..self.entries.len() - self.max_entries);
        }
    }

    pub fn get_recent(&self, limit: usize) -> &[HistoryEntry] {
        let start = if self.entries.len() > limit {
            self.entries.len() - limit
        } else {
            0
        };
        &self.entries[start..]
    }

    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.url.contains(query) ||
                    entry.method.contains(query) ||
                    entry.error_message.as_ref().map_or(false, |msg| msg.contains(query))
            })
            .collect()
    }

    pub fn get_by_id(&self, id: &str) -> Option<&HistoryEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get_history_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("surf")
            .join("history.json")
    }
}

impl HistoryEntry {
    pub fn new(method: &str, url: &str, headers: HashMap<String, String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            method: method.to_string(),
            url: url.to_string(),
            headers,
            status_code: None,
            response_time: None,
            response_size: None,
            success: false,
            error_message: None,
        }
    }

    pub fn with_response(mut self, status_code: u16, response_time: u64, response_size: u64) -> Self {
        self.status_code = Some(status_code);
        self.response_time = Some(response_time);
        self.response_size = Some(response_size);
        self.success = status_code >= 200 && status_code < 400;
        self
    }

    pub fn with_error(mut self, error_message: String) -> Self {
        self.error_message = Some(error_message);
        self.success = false;
        self
    }
}