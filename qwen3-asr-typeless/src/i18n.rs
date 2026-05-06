//! Internationalization (i18n) module.
//!
//! Provides simple key→string translation for Chinese (zh) and English (en).
//! Language auto-detects from Windows system locale using `GetUserDefaultUILanguage`.

use std::collections::HashMap;

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Zh,
}

impl Language {
    /// Detect the system UI language using Win32 API.
    pub fn detect_system() -> Self {
        // Use raw FFI to call GetUserDefaultUILanguage from kernel32.
        // This avoids depending on a specific windows crate module path.
        extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        let lang_id = unsafe { GetUserDefaultUILanguage() };
        let primary = lang_id & 0xFF;
        if primary == 0x04 {
            Language::Zh
        } else {
            Language::En
        }
    }

    /// Parse a language string from config ("en", "zh", "auto").
    pub fn from_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "zh" | "zh-cn" | "chinese" => Language::Zh,
            "auto" => Self::detect_system(),
            _ => Language::En,
        }
    }

    /// Get the display name for this language.
    pub fn display_name(&self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Zh => "中文",
        }
    }

    /// Get the config string representation.
    pub fn to_config_str(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Zh => "zh",
        }
    }
}

/// Translation dictionary.
pub struct I18n {
    lang: Language,
    strings: HashMap<&'static str, &'static str>,
}

impl I18n {
    /// Create a new I18n instance for the given language.
    pub fn new(lang: Language) -> Self {
        let strings = build_strings(lang);
        Self { lang, strings }
    }

    /// Create I18n from a config string ("auto", "en", "zh").
    pub fn from_config(lang_str: &str) -> Self {
        let lang = Language::from_config(lang_str);
        Self::new(lang)
    }

    /// Get the current language.
    pub fn lang(&self) -> Language {
        self.lang
    }

    /// Translate a key to the current language string.
    /// Falls back to the key itself if not found.
    pub fn t<'a>(&self, key: &'a str) -> &'a str {
        self.strings.get(key).copied().unwrap_or(key)
    }
}

/// Build the translation map for a given language.
fn build_strings(lang: Language) -> HashMap<&'static str, &'static str> {
    match lang {
        Language::En => EN_STRINGS.iter().map(|&(k, en, _)| (k, en)).collect(),
        Language::Zh => EN_STRINGS.iter().map(|&(k, _, zh)| (k, zh)).collect(),
    }
}

// Translation table: (key, english, chinese)
static EN_STRINGS: &[(&str, &str, &str)] = &[
    // Main window
    ("app.title", "Qwen3-ASR Typeless", "Qwen3-ASR Typeless"),
    ("main.tab_settings", "Settings", "设置"),
    ("main.tab_history", "History", "历史"),
    ("main.tab_about", "About", "关于"),
    // Tray
    ("tray.open", "Open", "打开"),
    ("tray.mode_ptt", "Mode: Push-to-Talk", "模式: 按住说话"),
    ("tray.mode_handsfree", "Mode: Hands-free", "模式: 免手"),
    ("tray.history", "History", "历史"),
    ("tray.settings", "Settings", "设置"),
    ("tray.about", "About", "关于"),
    ("tray.quit", "Quit", "退出"),
    // Settings
    ("settings.title", "Settings", "设置"),
    ("settings.asr_url", "ASR URL:", "ASR 地址:"),
    ("settings.api_key", "API Key:", "API 密钥:"),
    ("settings.default_mode", "Default Mode:", "默认模式:"),
    ("settings.vad_threshold", "VAD Threshold:", "VAD 阈值:"),
    ("settings.silence_dur", "Silence (sec):", "静音时长(秒):"),
    ("settings.max_dur", "Max Duration (sec):", "最长录音(秒):"),
    ("settings.ptt_key", "PTT Key:", "PTT 按键:"),
    ("settings.hf_key", "Hands-free Key:", "免手按键:"),
    ("settings.cancel_key", "Cancel Key:", "取消按键:"),
    ("settings.sample_rate", "Sample Rate:", "采样率:"),
    ("settings.play_sounds", "Play start/stop sounds", "播放提示音"),
    ("settings.show_overlay", "Show overlay during recording", "录音时显示悬浮窗"),
    ("settings.postproc", "Enable post-processing", "启用后处理"),
    ("settings.remove_fillers", "Remove fillers", "去除语气词"),
    ("settings.remove_rept", "Remove repetitions", "去除重复"),
    ("settings.auto_format", "Auto-format", "自动格式化"),
    ("settings.start_with_system", "Start with system", "开机自启"),
    ("settings.minimize_to_tray", "Minimize to tray", "最小化到托盘"),
    ("settings.history_retain", "History Retain:", "历史保留:"),
    ("settings.overlay_pos", "Overlay Position:", "悬浮窗位置:"),
    ("settings.llm_url", "LLM URL:", "LLM 地址:"),
    ("settings.llm_api_key", "LLM API Key:", "LLM API 密钥:"),
    ("settings.llm_model", "LLM Model:", "LLM 模型:"),
    ("settings.custom_prompt", "Custom Prompt:", "自定义提示词:"),
    ("settings.dictionary_btn", "Dictionary...", "词典..."),
    ("settings.test", "Test", "测试"),
    ("settings.test_postproc", "Test Post-Processing", "测试后处理"),
    ("settings.ok", "OK", "确定"),
    ("settings.cancel", "Cancel", "取消"),
    ("settings.language", "Language:", "语言:"),
    // Dictionary dialog
    ("dict.title", "Personal Dictionary", "个人词典"),
    ("dict.search", "Search:", "搜索:"),
    ("dict.word", "Word", "词语"),
    ("dict.correct", "Correct Spelling", "正确拼写"),
    ("dict.category", "Category", "分类"),
    ("dict.add", "Add", "添加"),
    ("dict.delete", "Delete", "删除"),
    ("dict.import", "Import", "导入"),
    ("dict.export", "Export", "导出"),
    ("dict.close", "Close", "关闭"),
    ("dict.add_title", "Add Dictionary Entry", "添加词典条目"),
    // History
    ("history.title", "Dictation History", "听写历史"),
    ("history.search", "Search:", "搜索:"),
    ("history.search_btn", "Search", "搜索"),
    ("history.time", "Time", "时间"),
    ("history.text", "Text", "文本"),
    ("history.status", "Status", "状态"),
    ("history.mode", "Mode", "模式"),
    ("history.duration", "Duration", "时长"),
    ("history.copy", "Copy", "复制"),
    ("history.cancelled", "[Cancelled]", "[已取消]"),
    // About
    ("about.version", "Version", "版本"),
    ("about.project", "Project", "项目"),
    ("about.asr_status", "ASR Service Status", "ASR 服务状态"),
    ("about.asr_connected", "Connected", "已连接"),
    ("about.asr_disconnected", "Disconnected", "未连接"),
    ("about.asr_checking", "Checking...", "检查中..."),
    ("about.description", "A voice dictation client powered by Qwen3-ASR", "基于 Qwen3-ASR 的语音听写客户端"),
    // Overlay
    ("overlay.recording", "Recording...", "录音中..."),
    ("overlay.processing", "Processing...", "处理中..."),
    ("overlay.too_short", "Too short, discarded", "录音过短，已丢弃"),
    ("overlay.max_duration", "Max duration reached", "已达最长录音时间"),
    ("overlay.no_audio", "No audio captured", "未捕获音频"),
    ("overlay.encoding_failed", "Encoding failed", "编码失败"),
    // Combo items
    ("combo.ptt", "Push-to-Talk", "按住说话"),
    ("combo.handsfree", "Hands-free", "免手"),
    ("combo.7days", "7 Days", "7 天"),
    ("combo.30days", "30 Days", "30 天"),
    ("combo.90days", "90 Days", "90 天"),
    ("combo.forever", "Forever", "永久"),
    ("combo.top_center", "top-center", "屏幕顶部居中"),
    ("combo.cursor", "cursor", "光标位置"),
    ("combo.auto", "Auto (follow system)", "自动(跟随系统)"),
    // Export
    ("export.json_btn", "JSON", "JSON"),
    ("export.csv_btn", "CSV", "CSV"),
    ("export.txt_btn", "TXT", "TXT"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_en_translations() {
        let i18n = I18n::new(Language::En);
        assert_eq!(i18n.t("settings.title"), "Settings");
        assert_eq!(i18n.t("history.title"), "Dictation History");
        assert_eq!(i18n.t("about.version"), "Version");
    }

    #[test]
    fn test_zh_translations() {
        let i18n = I18n::new(Language::Zh);
        assert_eq!(i18n.t("settings.title"), "设置");
        assert_eq!(i18n.t("history.title"), "听写历史");
        assert_eq!(i18n.t("about.version"), "版本");
    }

    #[test]
    fn test_fallback() {
        let i18n = I18n::new(Language::En);
        assert_eq!(i18n.t("nonexistent.key"), "nonexistent.key");
    }

    #[test]
    fn test_from_config() {
        assert_eq!(Language::from_config("en"), Language::En);
        assert_eq!(Language::from_config("zh"), Language::Zh);
        assert_eq!(Language::from_config("ZH-CN"), Language::Zh);
        assert_eq!(Language::from_config("auto"), Language::detect_system());
    }

    #[test]
    fn test_to_config_str() {
        assert_eq!(Language::En.to_config_str(), "en");
        assert_eq!(Language::Zh.to_config_str(), "zh");
    }

    #[test]
    fn test_display_name() {
        assert_eq!(Language::En.display_name(), "English");
        assert_eq!(Language::Zh.display_name(), "中文");
    }
}
