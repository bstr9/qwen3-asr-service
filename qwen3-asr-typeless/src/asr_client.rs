//! HTTP client for the Qwen3-ASR service.
//!
//! Submits audio via multipart POST to `/v1/asr`, then polls
//! `/v1/tasks/{task_id}` until the result is ready.

use anyhow::{bail, Context, Result};
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

/// ASR service client.
pub struct AsrClient {
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

/// Response from submitting an ASR task.
#[derive(Debug, Deserialize)]
struct SubmitResponse {
    task_id: String,
}

/// Response from polling a task.
#[derive(Debug, Deserialize)]
pub(crate) struct TaskResult {
    pub task_id: String,
    pub status: String,
    pub result: Option<AsrResult>,
    pub error: Option<String>,
}

/// The ASR recognition result.
#[derive(Debug, Deserialize)]
pub struct AsrResult {
    pub full_text: String,
    pub segments: Option<Vec<Segment>>,
}

/// A single segment of recognized text.
#[derive(Debug, Deserialize)]
pub(crate) struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Polling configuration.
const POLL_INTERVAL_MS: u64 = 500;
const POLL_TIMEOUT_SECS: u64 = 300;

impl AsrClient {
    /// Create a new ASR client.
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Submit audio data for transcription.
    /// Returns the task_id for polling.
    pub async fn submit_audio(&self, audio_data: &[u8], filename: &str) -> Result<String> {
        let url = format!("{}/v1/asr", self.base_url);

        let file_part = multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename.to_string())
            .mime_str("audio/wav")?;

        let form = multipart::Form::new().part("file", file_part);

        let mut request = self.client.post(&url).multipart(form);

        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.context("Failed to connect to ASR service (is it running?)")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("ASR submit failed ({}): {}", status, body);
        }

        let submit: SubmitResponse = response
            .json()
            .await
            .context("Failed to parse ASR submit response — service may be returning an unexpected format")?;

        log::info!("ASR task submitted: {}", submit.task_id);
        Ok(submit.task_id)
    }

    /// Poll a task until completion or failure.
    pub async fn poll_result(&self, task_id: &str) -> Result<TaskResult> {
        let url = format!("{}/v1/tasks/{}", self.base_url, task_id);
        let timeout = Duration::from_secs(POLL_TIMEOUT_SECS);
        let start = std::time::Instant::now();

        loop {
            let mut request = self.client.get(&url);

            if let Some(ref key) = self.api_key {
                request = request.bearer_auth(key);
            }

            let response = request.send().await.context("Failed to poll ASR task — connection lost?")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                bail!("ASR poll failed ({}): {}", status, body);
            }

            let result: TaskResult = response
                .json()
                .await
                .context("Failed to parse ASR task poll response")?;

            match result.status.as_str() {
                "completed" => {
                    log::info!("ASR task {} completed", task_id);
                    return Ok(result);
                }
                "failed" => {
                    let error = result.error.as_deref().unwrap_or("unknown error");
                    bail!("ASR task {} failed: {}", task_id, error);
                }
                "cancelled" => {
                    bail!("ASR task {} was cancelled", task_id);
                }
                _ => {
                    // pending or processing — keep polling
                    if start.elapsed() > timeout {
                        bail!("ASR task {} timed out after {}s", task_id, POLL_TIMEOUT_SECS);
                    }
                    tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                }
            }
        }
    }

    /// Submit audio and poll until the transcription is ready.
    /// Returns the recognized text.
    ///
    /// Retries up to 3 times with exponential backoff on network errors
    /// (connection refused, timeout, DNS failure). Does NOT retry on
    /// 4xx client errors (those are permanent).
    pub async fn transcribe(&self, audio_data: &[u8]) -> Result<String> {
        if audio_data.is_empty() {
            bail!("Cannot transcribe empty audio data");
        }

        const MAX_RETRIES: u32 = 3;
        let mut attempt = 0;

        loop {
            attempt += 1;
            match self.submit_audio(audio_data, "recording.wav").await {
                Ok(task_id) => {
                    // Poll for result (polling has its own retry via re-polling)
                    match self.poll_result(&task_id).await {
                        Ok(result) => {
                            log::info!("ASR task {} status={}", result.task_id, result.status);
                            return match result.result {
                                Some(asr_result) => {
                                    if let Some(ref segments) = asr_result.segments {
                                        log::debug!("ASR returned {} segments", segments.len());
                                        for (i, seg) in segments.iter().enumerate() {
                                            log::debug!(
                                                "  segment {}: [{:.2}-{:.2}] {}",
                                                i, seg.start, seg.end, seg.text
                                            );
                                        }
                                    }
                                    Ok(asr_result.full_text)
                                }
                                None => bail!("ASR task completed but no result returned"),
                            };
                        }
                        Err(e) => {
                            // Poll errors are not retried at this level — the task
                            // was already submitted successfully.
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    // Check if this is a retriable network error
                    let is_network = is_network_error(&e);
                    if is_network && attempt <= MAX_RETRIES {
                        let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                        log::warn!(
                            "ASR submit failed (attempt {}/{}): {}, retrying in {:?}",
                            attempt, MAX_RETRIES, e, delay
                        );
                        tokio::time::sleep(delay).await;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }
}

/// Check if an error is a network-related error that can be retried.
/// Does NOT consider 4xx client errors (those are permanent).
fn is_network_error(err: &anyhow::Error) -> bool {
    // Check for reqwest error types
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>() {
        return req_err.is_connect() || req_err.is_timeout() || req_err.is_request();
    }
    // Check the error chain for connection-related messages
    let msg = format!("{}", err);
    msg.contains("connect") || msg.contains("timeout") || msg.contains("connection")
}
