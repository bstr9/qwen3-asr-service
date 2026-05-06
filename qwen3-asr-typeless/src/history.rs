//! Dictation history storage.
//!
//! Stores dictation entries as JSON in the app data directory.
//! Uses a simple append-only JSON array file for reliability.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,               // UUID v4
    pub text: String,             // Final text (after post-processing)
    pub raw_text: Option<String>, // Before post-processing
    pub timestamp: i64,           // Unix timestamp (seconds)
    pub duration_secs: f64,       // Recording duration
    pub mode: String,             // "ptt" or "handsfree"
    pub language: Option<String>, // Detected language from ASR
    /// "completed" or "cancelled". Defaults to "completed" for backward compat.
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "completed".to_string()
}

impl HistoryEntry {
    pub fn new(text: String, raw_text: Option<String>, duration_secs: f64, mode: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            raw_text,
            timestamp: chrono::Utc::now().timestamp(),
            duration_secs,
            mode,
            language: None,
            status: "completed".to_string(),
        }
    }

    /// Create a cancelled history entry for partial/cancelled recordings.
    pub fn new_cancelled(text: String, raw_text: Option<String>, duration_secs: f64, mode: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text,
            raw_text,
            timestamp: chrono::Utc::now().timestamp(),
            duration_secs,
            mode,
            language: None,
            status: "cancelled".to_string(),
        }
    }

    pub fn formatted_timestamp(&self) -> String {
        chrono::DateTime::from_timestamp(self.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.status == "cancelled"
    }
}

pub struct HistoryManager {
    file_path: PathBuf,
    entries: Vec<HistoryEntry>,
    max_entries: usize,
}

impl HistoryManager {
    /// Load existing history from `{data_dir}/history.json`, or create empty if missing.
    /// Default max_entries = 1000.
    pub fn new(data_dir: &std::path::Path) -> Result<Self> {
        let file_path = data_dir.join("history.json");
        let entries = if file_path.exists() {
            let data = fs::read_to_string(&file_path)?;
            serde_json::from_str::<Vec<HistoryEntry>>(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self {
            file_path,
            entries,
            max_entries: 1000,
        })
    }

    /// Create an in-memory HistoryManager that writes to a temp file.
    /// Used as a last-resort fallback when no data directory is writable.
    pub fn new_in_memory() -> Self {
        Self {
            file_path: std::env::temp_dir().join("qwen3-asr-typeless-history.json"),
            entries: Vec::new(),
            max_entries: 1000,
        }
    }

    /// Add entry, auto-trim oldest if exceeding max_entries, then save.
    pub fn add(&mut self, entry: HistoryEntry) -> Result<()> {
        // Insert at front so newest is first (reverse chronological order)
        self.entries.insert(0, entry);
        // Trim from the end (oldest entries) if over limit
        if self.entries.len() > self.max_entries {
            self.entries.truncate(self.max_entries);
        }
        self.save()
    }

    /// Return all entries (newest first).
    pub fn list(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Simple case-insensitive substring search in text field.
    #[cfg(target_os = "windows")]
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.text.to_lowercase().contains(&query_lower))
            .collect()
    }

    /// Remove entry by ID, save to disk.
    #[cfg(target_os = "windows")]
    pub fn delete(&mut self, id: &str) -> Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() != before {
            self.save()?;
        }
        Ok(())
    }

    /// Remove entries older than `retention_days` days.
    ///
    /// Returns the number of entries removed. If `retention_days` is 0,
    /// no cleanup is performed (forever retention). Saves after cleanup.
    pub fn cleanup_expired(&mut self, retention_days: u64) -> Result<usize> {
        if retention_days == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - (retention_days as i64 * 86400);
        let before = self.entries.len();
        self.entries.retain(|e| e.timestamp >= cutoff);
        let removed = before - self.entries.len();
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    /// Export history entries as pretty-printed JSON.
    #[cfg(target_os = "windows")]
    pub fn export_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.entries)?)
    }

    /// Export history entries as CSV with columns: timestamp, text, status, mode, duration.
    #[cfg(target_os = "windows")]
    pub fn export_csv(&self) -> Result<String> {
        let mut w = String::from("timestamp,text,status,mode,duration\n");
        for e in &self.entries {
            let text = e.text.replace('"', "\"\"");
            w.push_str(&format!(
                "\"{}\",\"{}\",\"{}\",\"{}\",\"{:.1}s\"\n",
                e.formatted_timestamp(),
                text,
                e.status,
                e.mode,
                e.duration_secs,
            ));
        }
        Ok(w)
    }

    /// Export history entries as plain text, one entry per line:
    /// `YYYY-MM-DD HH:MM:SS | text`
    #[cfg(target_os = "windows")]
    pub fn export_txt(&self) -> Result<String> {
        let mut w = String::new();
        for e in &self.entries {
            w.push_str(&format!("{} | {}\n", e.formatted_timestamp(), e.text));
        }
        Ok(w)
    }

    /// Write entries to JSON file atomically (write to temp file, then rename).
    fn save(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)?;
        let temp_path = self.file_path.with_extension("json.tmp");
        fs::write(&temp_path, &json)?;
        fs::rename(&temp_path, &self.file_path)?;
        Ok(())
    }
}
