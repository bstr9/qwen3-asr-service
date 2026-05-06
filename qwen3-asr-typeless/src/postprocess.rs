//! AI post-processing pipeline for ASR output.
//!
//! Provides configurable text cleanup: filler removal, repetition removal,
//! auto-formatting, and optional LLM-based refinement.

use crate::config::PostProcessingConfig;
use anyhow::Result;
use regex::Regex;
use serde_json::json;

/// Remove common Chinese and English filler words.
///
/// Chinese fillers: 嗯、呃、啊、那个、就是说、然后呢、对对对
/// English fillers: um, uh, like, you know, I mean (as standalone words)
pub fn remove_fillers(text: &str) -> String {
    // Chinese fillers — exact character/phrase matches
    let chinese_fillers = [
        "嗯", "呃", "啊", "那个", "就是说", "然后呢", "对对对",
    ];

    let mut result = text.to_string();
    for filler in &chinese_fillers {
        // Replace filler surrounded by word boundaries (or at string edges)
        // For Chinese, we simply replace all occurrences since there are no
        // word boundaries in the same sense as English.
        result = result.replace(filler, "");
    }

    // English fillers — use regex with word boundaries to avoid
    // removing "like" from "likely" etc.
    let english_patterns = [
        (r"\bum\b", "um"),
        (r"\buh\b", "uh"),
        (r"\blike\b", "like"),
        (r"\byou know\b", "you know"),
        (r"\bI mean\b", "I mean"),
    ];

    for (pattern, _label) in &english_patterns {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, "").to_string();
        }
    }

    // Collapse multiple spaces into one, and trim
    if let Ok(re) = Regex::new(r"\s{2,}") {
        result = re.replace_all(&result, " ").to_string();
    }
    result = result.trim().to_string();

    result
}

/// Remove consecutive duplicate phrases (3+ characters).
///
/// Detects immediately repeated substrings and keeps only one instance.
/// E.g. "今天天气很好今天天气很好" → "今天天气很好"
pub fn remove_repetitions(text: &str) -> String {
    if text.len() < 6 {
        return text.to_string();
    }

    let mut result = text.to_string();
    let mut changed = true;

    // Iterate until no more repetitions are found (handles nested repetitions)
    while changed {
        changed = false;
        // Char-based approach for proper Unicode handling
        let chars: Vec<char> = result.chars().collect();
        if chars.len() < 6 {
            break;
        }
        let max_phrase_chars = chars.len() / 2;
        for phrase_char_len in (3..=max_phrase_chars).rev() {
            let mut i = 0;
            while i + phrase_char_len <= chars.len() {
                let phrase: String = chars[i..i + phrase_char_len].iter().collect();
                if i + 2 * phrase_char_len <= chars.len() {
                    let next: String = chars[i + phrase_char_len..i + 2 * phrase_char_len].iter().collect();
                    if phrase == next {
                        // Remove the duplicate: keep chars[i..i+phrase_char_len], skip the duplicate
                        let mut new_chars = Vec::with_capacity(chars.len() - phrase_char_len);
                        new_chars.extend_from_slice(&chars[..i + phrase_char_len]);
                        new_chars.extend_from_slice(&chars[i + 2 * phrase_char_len..]);
                        result = new_chars.into_iter().collect();
                        changed = true;
                        break; // Restart with new string
                    }
                }
                i += 1;
            }
            if changed {
                break; // Restart outer loop
            }
        }
    }

    result
}

/// Auto-format text: trim, capitalize first letter, fix spacing, add trailing punctuation.
///
/// - Trim whitespace
/// - Capitalize first character (English)
/// - Fix double spaces
/// - Ensure ending punctuation (add 。 for Chinese, . for English if missing)
pub fn auto_format(text: &str) -> String {
    let mut result = text.trim().to_string();

    if result.is_empty() {
        return result;
    }

    // Fix double spaces
    if let Ok(re) = Regex::new(r"  +") {
        result = re.replace_all(&result, " ").to_string();
    }

    // Capitalize first character for English text
    if let Some(first) = result.chars().next() {
        if first.is_ascii_lowercase() {
            let upper = first.to_uppercase().to_string();
            result = format!("{}{}", upper, &result[first.len_utf8()..]);
        }
    }

    // Ensure ending punctuation
    if let Some(last) = result.chars().last() {
        let chinese_end_punctuation = ['。', '！', '？', '…', '；'];
        let english_end_punctuation = ['.', '!', '?', ';', ':'];

        let is_chinese = result.chars().any(|c| c > '\u{4E00}' && c < '\u{9FFF}' || c > '\u{3400}' && c < '\u{4DBF}');
        let has_end_punct = chinese_end_punctuation.contains(&last) || english_end_punctuation.contains(&last);

        if !has_end_punct {
            if is_chinese {
                result.push('。');
            } else {
                result.push('.');
            }
        }
    }

    result
}

/// Run the synchronous post-processing pipeline.
///
/// Applies enabled steps in order: remove_fillers → remove_repetitions → auto_format.
/// Returns the processed text, or the original text if post-processing is disabled.
pub fn postprocess(text: &str, config: &PostProcessingConfig) -> String {
    if !config.enabled {
        return text.to_string();
    }

    let mut result = text.to_string();

    if config.remove_fillers {
        result = remove_fillers(&result);
    }

    if config.remove_repetitions {
        result = remove_repetitions(&result);
    }

    if config.auto_format {
        result = auto_format(&result);
    }

    result
}

/// Optional LLM-based post-processing refinement.
///
/// Sends the text to an OpenAI-compatible chat completion endpoint.
/// If `dictionary_hint` is provided and non-empty, it is appended to the
/// system prompt so the LLM prefers the user's custom spellings.
/// Falls back to returning the input text if the LLM call fails.
pub async fn llm_postprocess(text: &str, config: &PostProcessingConfig, dictionary_hint: Option<&str>) -> Result<String> {
    let url = match &config.llm_url {
        Some(u) => u.clone(),
        None => anyhow::bail!("LLM URL not configured"),
    };

    let model = match &config.llm_model {
        Some(m) => m.clone(),
        None => anyhow::bail!("LLM model not configured"),
    };

    let mut system_prompt = config.custom_prompt.as_deref().unwrap_or(
        "You are a text post-processing assistant. Clean up the following speech-to-text output. \
         Remove filler words, fix grammar, add punctuation, and improve readability while \
         preserving the original meaning. Output only the cleaned text, nothing else."
    ).to_string();

    // Append dictionary hint if provided
    if let Some(hint) = dictionary_hint {
        if !hint.is_empty() {
            system_prompt.push_str(&format!(
                "\n\nThe user has a personal dictionary. Use these preferred spellings:\n{}",
                hint
            ));
        }
    }

    let request_body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": text }
        ],
        "temperature": 0.3,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut request = client.post(&url)
        .json(&request_body);

    if let Some(api_key) = &config.llm_api_key {
        request = request.bearer_auth(api_key);
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("LLM request failed with status {}: {}", status, body);
    }

    let response_json: serde_json::Value = response.json().await?;
    let cleaned = response_json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or(text)
        .to_string();

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> PostProcessingConfig {
        PostProcessingConfig {
            enabled: true,
            remove_fillers: true,
            remove_repetitions: true,
            auto_format: true,
            llm_url: None,
            llm_api_key: None,
            llm_model: None,
            custom_prompt: None,
        }
    }

    // ── remove_fillers tests ──────────────────────────────────────────

    #[test]
    fn remove_chinese_fillers() {
        let result = remove_fillers("嗯今天天气很好呃");
        assert_eq!(result, "今天天气很好");
    }

    #[test]
    fn remove_chinese_phrase_fillers() {
        let result = remove_fillers("那个就是说我想去北京然后呢");
        assert_eq!(result, "我想去北京");
    }

    #[test]
    fn remove_chinese_dupe_filler() {
        let result = remove_fillers("对对对我同意啊");
        assert_eq!(result, "我同意");
    }

    #[test]
    fn remove_english_fillers() {
        let result = remove_fillers("um I think uh this is good");
        assert_eq!(result, "I think this is good");
    }

    #[test]
    fn remove_english_phrase_fillers() {
        let result = remove_fillers("I mean you know it's like great");
        assert_eq!(result, "it's great");
    }

    #[test]
    fn remove_fillers_preserves_normal_text() {
        assert_eq!(remove_fillers("Hello World"), "Hello World");
    }

    #[test]
    fn remove_fillers_empty_string() {
        assert_eq!(remove_fillers(""), "");
    }

    #[test]
    fn remove_fillers_only_fillers() {
        assert_eq!(remove_fillers("嗯呃啊"), "");
    }

    // ── remove_repetitions tests ──────────────────────────────────────

    #[test]
    fn remove_repeated_chinese_phrase() {
        let result = remove_repetitions("今天天气很好今天天气很好");
        assert_eq!(result, "今天天气很好");
    }

    #[test]
    fn remove_repetitions_no_repetition() {
        assert_eq!(remove_repetitions("今天天气很好"), "今天天气很好");
    }

    #[test]
    fn remove_repetitions_short_text() {
        assert_eq!(remove_repetitions("你好"), "你好");
    }

    #[test]
    fn remove_repetitions_empty() {
        assert_eq!(remove_repetitions(""), "");
    }

    #[test]
    fn remove_repetitions_english() {
        let result = remove_repetitions("the quick brown foxthe quick brown fox");
        assert_eq!(result, "the quick brown fox");
    }

    // ── auto_format tests ─────────────────────────────────────────────

    #[test]
    fn auto_format_capitalize_english() {
        let result = auto_format("hello world");
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn auto_format_add_chinese_period() {
        let result = auto_format("你好世界");
        assert_eq!(result, "你好世界。");
    }

    #[test]
    fn auto_format_no_double_period() {
        let result = auto_format("Hello world.");
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn auto_format_no_double_chinese_period() {
        let result = auto_format("你好世界。");
        assert_eq!(result, "你好世界。");
    }

    #[test]
    fn auto_format_trims_whitespace() {
        let result = auto_format("  hello  ");
        assert_eq!(result, "Hello.");
    }

    #[test]
    fn auto_format_fixes_double_spaces() {
        let result = auto_format("hello  world");
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn auto_format_empty_string() {
        assert_eq!(auto_format(""), "");
    }

    #[test]
    fn auto_format_preserves_existing_punctuation() {
        assert_eq!(auto_format("Hello world!"), "Hello world!");
        assert_eq!(auto_format("你好世界？"), "你好世界？");
    }

    // ── postprocess pipeline tests ────────────────────────────────────

    #[test]
    fn postprocess_disabled_returns_original() {
        let mut config = test_config();
        config.enabled = false;
        let result = postprocess("嗯hello  world", &config);
        assert_eq!(result, "嗯hello  world");
    }

    #[test]
    fn postprocess_full_pipeline_chinese() {
        let config = test_config();
        let result = postprocess("嗯今天天气很好今天天气很好", &config);
        assert_eq!(result, "今天天气很好。");
    }

    #[test]
    fn postprocess_full_pipeline_english() {
        let config = test_config();
        let result = postprocess("uh hello  world", &config);
        assert_eq!(result, "Hello world.");
    }

    #[test]
    fn postprocess_selective_steps() {
        let mut config = test_config();
        config.remove_fillers = false;
        config.remove_repetitions = false;
        let result = postprocess("嗯hello  world呃", &config);
        // auto_format still runs: trim, capitalize, fix double spaces, add punctuation
        assert_eq!(result, "嗯hello world呃。");
    }

    #[test]
    fn postprocess_only_fillers() {
        let mut config = test_config();
        config.remove_repetitions = false;
        config.auto_format = false;
        let result = postprocess("嗯hello呃", &config);
        assert_eq!(result, "hello");
    }
}
