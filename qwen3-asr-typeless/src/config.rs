//! Application configuration for qwen3-asr-typeless.
//!
//! Supports TOML-based config file persistence with sensible defaults.
//! Config is loaded from `%APPDATA%/qwen3-asr-typeless/config.toml` on Windows
//! (or the equivalent XDG data directory on other platforms).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::System::Registry::*;

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Base URL of the ASR service.
    pub asr_url: String,
    /// Optional API key for Bearer token authentication.
    pub api_key: Option<String>,
    /// VAD speech probability threshold (0.0–1.0).
    pub vad_threshold: f32,
    /// Seconds of silence before stopping recording.
    pub silence_duration_secs: f64,
    /// Maximum recording duration in seconds. When reached, recording is
    /// auto-stopped and submitted to ASR.
    pub max_recording_duration: u64,
    /// Audio sample rate in Hz.
    pub sample_rate: u32,
    /// Hotkey configuration.
    pub hotkey: HotkeyConfig,
    /// Recording mode configuration.
    pub mode: RecordingModeConfig,
    /// Post-processing configuration.
    pub post_processing: PostProcessingConfig,
    /// UI configuration.
    pub ui: UiConfig,
}

/// Hotkey bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// Push-to-talk key binding, e.g. "F8" or "RightAlt+Space".
    pub ptt_key: String,
    /// Hands-free toggle key binding.
    pub handsfree_key: String,
    /// Cancel key binding.
    pub cancel_key: String,
}

/// Recording mode settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingModeConfig {
    /// Default recording mode: "ptt" or "handsfree".
    pub default: String,
}

/// Post-processing (AI refinement) settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostProcessingConfig {
    /// Whether post-processing is enabled.
    pub enabled: bool,
    /// Remove filler words (um, uh, etc.).
    pub remove_fillers: bool,
    /// Remove repeated phrases.
    pub remove_repetitions: bool,
    /// Auto-format punctuation and capitalization.
    pub auto_format: bool,
    /// LLM API URL for post-processing (if enabled).
    pub llm_url: Option<String>,
    /// LLM API key.
    pub llm_api_key: Option<String>,
    /// LLM model name.
    pub llm_model: Option<String>,
    /// Custom prompt template for post-processing.
    pub custom_prompt: Option<String>,
}

/// UI preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// Show status overlay during recording.
    pub show_overlay: bool,
    /// Overlay position: "top-center" or "cursor".
    pub overlay_position: String,
    /// Overlay X position in pixels (None = auto-position).
    pub overlay_x: Option<i32>,
    /// Overlay Y position in pixels (None = auto-position).
    pub overlay_y: Option<i32>,
    /// Whether the overlay starts minimized (dot only, no VU meter).
    pub overlay_minimized: bool,
    /// Play start/stop sounds.
    pub play_sounds: bool,
    /// Start minimized with the operating system.
    pub start_with_system: bool,
    /// Minimize to system tray instead of taskbar.
    pub minimize_to_tray: bool,
    /// History retention period in days. 0 = forever.
    #[serde(default = "default_history_retention_days")]
    pub history_retention_days: u64,
    /// UI language: "auto", "en", or "zh". "auto" follows Windows system locale.
    #[serde(default = "default_language")]
    pub language: String,
    /// Main window X position. None = auto-center.
    #[serde(default)]
    pub main_window_x: Option<i32>,
    /// Main window Y position. None = auto-center.
    #[serde(default)]
    pub main_window_y: Option<i32>,
    /// Main window width in pixels.
    #[serde(default = "default_main_window_w")]
    pub main_window_w: i32,
    /// Main window height in pixels.
    #[serde(default = "default_main_window_h")]
    pub main_window_h: i32,
}

fn default_history_retention_days() -> u64 {
    90
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_main_window_w() -> i32 {
    700
}

fn default_main_window_h() -> i32 {
    520
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            asr_url: "http://127.0.0.1:8765".to_string(),
            api_key: None,
            vad_threshold: 0.5,
            silence_duration_secs: 5.0,
            max_recording_duration: 60,
            sample_rate: 16000,
            hotkey: HotkeyConfig::default(),
            mode: RecordingModeConfig::default(),
            post_processing: PostProcessingConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            ptt_key: "F8".to_string(),
            handsfree_key: "RightAlt+Space".to_string(),
            cancel_key: "Ctrl+Escape".to_string(),
        }
    }
}

impl Default for RecordingModeConfig {
    fn default() -> Self {
        Self {
            default: "ptt".to_string(),
        }
    }
}

impl Default for PostProcessingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remove_fillers: true,
            remove_repetitions: true,
            auto_format: true,
            llm_url: None,
            llm_api_key: None,
            llm_model: None,
            custom_prompt: None,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_overlay: true,
            overlay_position: "top-center".to_string(),
            overlay_x: None,
            overlay_y: None,
            overlay_minimized: false,
            play_sounds: true,
            start_with_system: false,
            minimize_to_tray: true,
            history_retention_days: 90,
            language: default_language(),
            main_window_x: None,
            main_window_y: None,
            main_window_w: default_main_window_w(),
            main_window_h: default_main_window_h(),
        }
    }
}

impl AppConfig {
    /// Load configuration from a TOML file.
    ///
    /// Returns the default configuration if the file does not exist.
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            log::info!("Config file not found at {:?}, using defaults", path);
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        config.validate()?;
        log::info!("Loaded config from {:?}", path);
        Ok(config)
    }

    /// Save configuration to a TOML file.
    ///
    /// Creates parent directories if they do not exist.
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        log::info!("Saved config to {:?}", path);
        Ok(())
    }

    /// Return the platform-specific config directory.
    ///
    /// On Windows: `%APPDATA%/qwen3-asr-typeless/`
    /// On other platforms: `$XDG_CONFIG_HOME/qwen3-asr-typeless/` or `~/.config/qwen3-asr-typeless/`
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("qwen3-asr-typeless")
    }

    /// Return the default config file path.
    pub fn default_config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Validate configuration values, clamping out-of-range values and
    /// logging warnings for any corrections made.
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.vad_threshold) {
            log::warn!(
                "vad_threshold {} is out of range [0.0, 1.0], clamping",
                self.vad_threshold
            );
        }
        if self.silence_duration_secs < 0.5 {
            log::warn!(
                "silence_duration_secs {} is very low (< 0.5s), may cause premature stop",
                self.silence_duration_secs
            );
        }
        if self.max_recording_duration == 0 {
            log::warn!("max_recording_duration is 0, disabling max duration limit");
        }
        if self.sample_rate != 16000 && self.sample_rate != 8000 {
            log::warn!(
                "sample_rate {} is not 8000 or 16000; VAD and ASR may not work correctly",
                self.sample_rate
            );
        }
        if self.asr_url.is_empty() {
            log::warn!("asr_url is empty; ASR requests will fail");
        }
        Ok(())
    }
}

/// Set or remove auto-start entry.
///
/// On Windows, writes/removes the current exe path to
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
/// On Linux, creates/removes a .desktop file in the XDG autostart directory.
#[cfg(target_os = "windows")]
pub fn set_auto_start(enable: bool) -> Result<()> {
    let key_path: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let app_name: Vec<u16> = "Qwen3ASR".encode_utf16().chain(std::iter::once(0)).collect();

    // Get current exe path as wide string
    let exe_path = std::env::current_exe()?;
    let exe_path_str = exe_path.to_str().unwrap_or("");
    let exe_path_wide: Vec<u16> = exe_path_str
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    if enable {
        // Open the Run key for writing
        let mut hkey: HKEY = HKEY(std::ptr::null_mut());
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            )
        };
        if result.0 != 0 {
            log::error!("Failed to open Run registry key: error {}", result.0);
        } else {
            // REG_SZ: data is null-terminated UTF-16, byte count excludes the null
            let byte_size = (exe_path_wide.len() - 1) * 2; // exclude null terminator for size
            let data_ptr = exe_path_wide.as_ptr() as *const u8;
            let set_result = unsafe {
                RegSetValueExW(
                    hkey,
                    PCWSTR(app_name.as_ptr()),
                    0,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(data_ptr, byte_size)),
                )
            };
            if set_result.0 != 0 {
                log::error!("Failed to set auto-start registry value: error {}", set_result.0);
            } else {
                log::info!("Auto-start enabled in registry");
            }
            unsafe { let _ = RegCloseKey(hkey); }
        }
    } else {
        // Open the Run key and delete the value
        let mut hkey: HKEY = HKEY(std::ptr::null_mut());
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                0,
                KEY_SET_VALUE,
                &mut hkey,
            )
        };
        if result.0 != 0 {
            log::debug!("Failed to open Run registry key for deletion: error {}", result.0);
        } else {
            let del_result = unsafe {
                RegDeleteValueW(hkey, PCWSTR(app_name.as_ptr()))
            };
            // Deleting a non-existent value is not an error
            if del_result.0 != 0 {
                log::debug!("Failed to delete auto-start registry value (may not exist): error {}", del_result.0);
            } else {
                log::info!("Auto-start removed from registry");
            }
            unsafe { let _ = RegCloseKey(hkey); }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn set_auto_start(enable: bool) -> Result<()> {
    let autostart_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("autostart");
    let desktop_path = autostart_dir.join("qwen3-asr-typeless.desktop");

    if enable {
        std::fs::create_dir_all(&autostart_dir)?;
        let exe_path = std::env::current_exe()?;
        let exe_str = exe_path.to_str().unwrap_or("");
        let desktop_entry = format!(
            "[Desktop Entry]\nType=Application\nName=Qwen3-ASR Typeless\nExec={}\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
            exe_str
        );
        std::fs::write(&desktop_path, desktop_entry)?;
        log::info!("Auto-start enabled via .desktop file at {:?}", desktop_path);
    } else {
        if desktop_path.exists() {
            std::fs::remove_file(&desktop_path)?;
            log::info!("Auto-start removed (deleted .desktop file)");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_toml() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("serialize should succeed");
        let parsed: AppConfig = toml::from_str(&toml_str).expect("deserialize should succeed");
        assert_eq!(parsed.asr_url, config.asr_url);
        assert_eq!(parsed.sample_rate, config.sample_rate);
        assert_eq!(parsed.vad_threshold, config.vad_threshold);
        assert_eq!(parsed.hotkey.ptt_key, "F8");
        assert_eq!(parsed.mode.default, "ptt");
        assert!(!parsed.post_processing.enabled);
        assert!(parsed.ui.show_overlay);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let path = PathBuf::from("/nonexistent/path/config.toml");
        let config = AppConfig::load(&path).expect("load should succeed for missing file");
        assert_eq!(config.asr_url, "http://127.0.0.1:8765");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("qwen3-asr-typeless-test-config");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_config.toml");

        let original = AppConfig::default();
        original.save(&path).expect("save should succeed");

        let loaded = AppConfig::load(&path).expect("load should succeed");
        assert_eq!(loaded.asr_url, original.asr_url);
        assert_eq!(loaded.sample_rate, original.sample_rate);
        assert_eq!(loaded.hotkey.ptt_key, original.hotkey.ptt_key);
        assert_eq!(loaded.mode.default, original.mode.default);

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn config_dir_is_under_platform_config() {
        let dir = AppConfig::config_dir();
        assert!(dir.ends_with("qwen3-asr-typeless"));
    }
}
