//! Personal dictionary for custom vocabulary.
//!
//! Stores custom word entries as JSON in the app data directory.
//! Dictionary words can be injected into LLM post-processing prompts
//! to improve recognition of domain-specific terms.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// A single dictionary entry representing a custom word and its preferred spelling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryEntry {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// The word or phrase as it might appear in ASR output.
    pub word: String,
    /// The preferred correct spelling to use.
    pub correct_spelling: String,
    /// Optional category (e.g. "medical", "legal", "tech").
    pub category: Option<String>,
}

/// Manages personal dictionary entries with persistence to a JSON file.
pub struct DictionaryManager {
    file_path: PathBuf,
    entries: Vec<DictionaryEntry>,
}

impl DictionaryManager {
    /// Load existing dictionary from `{data_dir}/dictionary.json`, or create empty if missing.
    pub fn new(data_dir: &std::path::Path) -> Result<Self> {
        let file_path = data_dir.join("dictionary.json");
        let entries = if file_path.exists() {
            let data = fs::read_to_string(&file_path)?;
            serde_json::from_str::<Vec<DictionaryEntry>>(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self {
            file_path,
            entries,
        })
    }

    /// Create an in-memory DictionaryManager that writes to a temp file.
    /// Used as a last-resort fallback when no data directory is writable.
    pub fn new_in_memory() -> Self {
        Self {
            file_path: std::env::temp_dir().join("qwen3-asr-typeless-dictionary.json"),
            entries: Vec::new(),
        }
    }

    /// Add a new dictionary entry. Returns an error if a duplicate word already exists.
    pub fn add(
        &mut self,
        word: String,
        correct_spelling: String,
        category: Option<String>,
    ) -> Result<()> {
        // Check for duplicate word (case-insensitive)
        let word_lower = word.to_lowercase();
        if self.entries.iter().any(|e| e.word.to_lowercase() == word_lower) {
            anyhow::bail!("Word '{}' already exists in dictionary", word);
        }
        let entry = DictionaryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            word,
            correct_spelling,
            category,
        };
        self.entries.push(entry);
        self.save()
    }

    /// Remove an entry by its ID. Saves to disk after removal.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() != before {
            self.save()?;
        }
        Ok(())
    }

    /// Return all entries.
    pub fn list(&self) -> &[DictionaryEntry] {
        &self.entries
    }

    /// Case-insensitive search across word, correct_spelling, and category fields.
    /// Used by the dictionary dialog for filtering.
    pub fn search(&self, query: &str) -> Vec<&DictionaryEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.word.to_lowercase().contains(&query_lower)
                    || e.correct_spelling.to_lowercase().contains(&query_lower)
                    || e.category
                        .as_ref()
                        .is_some_and(|c| c.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// Export all entries as pretty-printed JSON.
    pub fn export_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.entries)?)
    }

    /// Import entries from a JSON string, merging with existing entries.
    /// Skips duplicates (by word, case-insensitive). Returns the number of new entries added.
    pub fn import_json(&mut self, json: &str) -> Result<usize> {
        let imported: Vec<DictionaryEntry> = serde_json::from_str(json)?;
        let mut added = 0;
        for entry in imported {
            let word_lower = entry.word.to_lowercase();
            if !self
                .entries
                .iter()
                .any(|e| e.word.to_lowercase() == word_lower)
            {
                self.entries.push(entry);
                added += 1;
            }
        }
        if added > 0 {
            self.save()?;
        }
        Ok(added)
    }

    /// Format dictionary entries as "word → correct_spelling" lines for LLM prompt injection.
    pub fn format_for_prompt(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }
        self.entries
            .iter()
            .map(|e| format!("{} → {}", e.word, e.correct_spelling))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Load preset dictionary entries for common professional terms.
    /// Skips any entries that already exist (by word, case-insensitive).
    /// Returns the number of new entries added.
    pub fn load_preset(&mut self) -> Result<usize> {
        let presets: &[(&str, &str, &str)] = &[
            // Tech
            ("kubernetes", "Kubernetes", "tech"),
            ("k8s", "Kubernetes", "tech"),
            ("docker", "Docker", "tech"),
            ("aws", "AWS", "tech"),
            ("gcp", "GCP", "tech"),
            ("azure", "Azure", "tech"),
            ("gitlab", "GitLab", "tech"),
            ("github", "GitHub", "tech"),
            ("javascript", "JavaScript", "tech"),
            ("typescript", "TypeScript", "tech"),
            ("python", "Python", "tech"),
            ("rustlang", "Rust", "tech"),
            ("linux", "Linux", "tech"),
            ("macos", "macOS", "tech"),
            ("nginx", "nginx", "tech"),
            ("postgresql", "PostgreSQL", "tech"),
            ("mysql", "MySQL", "tech"),
            ("redis", "Redis", "tech"),
            ("mongodb", "MongoDB", "tech"),
            ("openai", "OpenAI", "tech"),
            ("rest api", "REST API", "tech"),
            ("graphql", "GraphQL", "tech"),
            // Medical
            ("covid", "COVID-19", "medical"),
            ("mri", "MRI", "medical"),
            ("ct scan", "CT scan", "medical"),
            ("ecg", "ECG", "medical"),
            ("blood pressure", "blood pressure", "medical"),
            // General
            ("ai", "AI", "general"),
            ("llm", "LLM", "general"),
            ("saas", "SaaS", "general"),
            ("paas", "PaaS", "general"),
            ("iaas", "IaaS", "general"),
            ("ui", "UI", "general"),
            ("ux", "UX", "general"),
            ("api", "API", "general"),
            ("sdk", "SDK", "general"),
            ("ide", "IDE", "general"),
            ("cpu", "CPU", "general"),
            ("gpu", "GPU", "general"),
            ("ram", "RAM", "general"),
            ("ssd", "SSD", "general"),
        ];

        let mut added = 0;
        for (word, correct, category) in presets {
            let word_lower = word.to_lowercase();
            if !self
                .entries
                .iter()
                .any(|e| e.word.to_lowercase() == word_lower)
            {
                self.entries.push(DictionaryEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    word: word.to_string(),
                    correct_spelling: correct.to_string(),
                    category: Some(category.to_string()),
                });
                added += 1;
            }
        }
        if added > 0 {
            self.save()?;
        }
        Ok(added)
    }

    /// Write entries to JSON file atomically (write to temp file, then rename).
    fn save(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let tid = format!("{:?}", std::thread::current().id());
        let dir = std::env::temp_dir().join(format!("qwen3-asr-typeless-dict-test-{}", tid));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &PathBuf) {
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let _ = std::fs::remove_file(dir.join("dictionary.json.tmp"));
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn new_creates_empty_dictionary() {
        let dir = temp_dir();
        // Ensure no leftover file
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let dm = DictionaryManager::new(&dir).unwrap();
        assert!(dm.list().is_empty());
        cleanup(&dir);
    }

    #[test]
    fn add_and_list_entries() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        dm.add("kubernetes".to_string(), "Kubernetes".to_string(), Some("tech".to_string())).unwrap();
        dm.add("docker".to_string(), "Docker".to_string(), None).unwrap();
        assert_eq!(dm.list().len(), 2);
        assert_eq!(dm.list()[0].word, "kubernetes");
        assert_eq!(dm.list()[1].correct_spelling, "Docker");
        cleanup(&dir);
    }

    #[test]
    fn add_duplicate_word_fails() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        dm.add("test".to_string(), "Test".to_string(), None).unwrap();
        let result = dm.add("Test".to_string(), "Test2".to_string(), None);
        assert!(result.is_err());
        cleanup(&dir);
    }

    #[test]
    fn remove_entry() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        dm.add("word1".to_string(), "Word1".to_string(), None).unwrap();
        let id = dm.list()[0].id.clone();
        dm.remove(&id).unwrap();
        assert!(dm.list().is_empty());
        cleanup(&dir);
    }

    #[test]
    fn search_case_insensitive() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        dm.add("kubernetes".to_string(), "Kubernetes".to_string(), Some("tech".to_string())).unwrap();
        dm.add("docker".to_string(), "Docker".to_string(), None).unwrap();
        let results = dm.search("KUBE");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].word, "kubernetes");
        let results = dm.search("tech");
        assert_eq!(results.len(), 1);
        cleanup(&dir);
    }

    #[test]
    fn format_for_prompt() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        assert!(dm.format_for_prompt().is_empty());
        dm.add("k8s".to_string(), "Kubernetes".to_string(), None).unwrap();
        dm.add("aws".to_string(), "AWS".to_string(), None).unwrap();
        let prompt = dm.format_for_prompt();
        assert!(prompt.contains("k8s → Kubernetes"));
        assert!(prompt.contains("aws → AWS"));
        cleanup(&dir);
    }

    #[test]
    fn export_and_import_json() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));
        let mut dm = DictionaryManager::new(&dir).unwrap();
        dm.add("word1".to_string(), "Word1".to_string(), Some("cat1".to_string())).unwrap();
        let json = dm.export_json().unwrap();

        // Import into a new manager
        let dir2 = temp_dir().join("import");
        let _ = std::fs::create_dir_all(&dir2);
        let mut dm2 = DictionaryManager::new(&dir2).unwrap();
        let count = dm2.import_json(&json).unwrap();
        assert_eq!(count, 1);
        assert_eq!(dm2.list().len(), 1);

        // Import same again — should skip duplicate
        let count2 = dm2.import_json(&json).unwrap();
        assert_eq!(count2, 0);
        assert_eq!(dm2.list().len(), 1);

        cleanup(&dir);
        let _ = std::fs::remove_file(dir2.join("dictionary.json"));
        let _ = std::fs::remove_dir(&dir2);
    }

    #[test]
    fn persistence_across_instances() {
        let dir = temp_dir();
        let _ = std::fs::remove_file(dir.join("dictionary.json"));

        // Add entry in first instance
        {
            let mut dm = DictionaryManager::new(&dir).unwrap();
            dm.add("persist".to_string(), "Persist".to_string(), None).unwrap();
        }

        // Load in second instance — should see the entry
        {
            let dm = DictionaryManager::new(&dir).unwrap();
            assert_eq!(dm.list().len(), 1);
            assert_eq!(dm.list()[0].word, "persist");
        }

        cleanup(&dir);
    }
}
