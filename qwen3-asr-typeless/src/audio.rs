//! Microphone audio capture using cpal.
//!
//! Captures 16 kHz mono f32 samples from the default input device
//! and sends them through channels for downstream processing.
//! Supports an optional secondary channel for real-time VAD monitoring.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Samples produced by the audio recorder.
pub type AudioChunk = Vec<f32>;

/// Records audio from the default microphone.
pub struct AudioRecorder {
    sample_rate: u32,
    tx: Option<mpsc::UnboundedSender<AudioChunk>>,
    /// Optional secondary channel for VAD monitoring (std::sync for use in std threads).
    vad_tx: Option<std::sync::mpsc::Sender<AudioChunk>>,
    /// Optional volume level callback (called with RMS 0.0–1.0 on each chunk).
    volume_cb: Option<Box<dyn Fn(f32) + Send + Sync>>,
    stream: Option<Stream>,
    recording: Arc<AtomicBool>,
    buffer: Vec<f32>,
}

impl AudioRecorder {
    /// Create a new recorder targeting the given sample rate.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            tx: None,
            vad_tx: None,
            volume_cb: None,
            stream: None,
            recording: Arc::new(AtomicBool::new(false)),
            buffer: Vec::new(),
        }
    }

    /// Set a secondary channel for VAD monitoring.
    /// Audio chunks will be sent to both the primary tokio channel
    /// and this std::sync channel. Must be called before `start()`.
    pub fn set_vad_channel(&mut self, tx: std::sync::mpsc::Sender<AudioChunk>) {
        self.vad_tx = Some(tx);
    }

    /// Set a volume level callback. Called with RMS level (0.0–1.0)
    /// for each audio chunk. Must be called before `start()`.
    pub fn set_volume_callback(&mut self, cb: Box<dyn Fn(f32) + Send + Sync>) {
        self.volume_cb = Some(cb);
    }

    /// Start capturing audio. Samples are sent in chunks through the returned receiver.
    /// The receiver must be stored by the caller and drained via `collect_chunks()`
    /// after calling `stop()`.
    pub fn start(&mut self) -> Result<mpsc::UnboundedReceiver<AudioChunk>> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("No default input device available")?;

        log::info!("Using input device: {}", device.name().unwrap_or_default());

        // Try to find a supported config matching our desired format,
        // falling back to the device's default input config.
        let supported_config = device
            .supported_input_configs()
            .context("Could not query supported input configs")?
            .find(|c| {
                c.channels() == 1
                    && c.min_sample_rate().0 <= self.sample_rate
                    && c.max_sample_rate().0 >= self.sample_rate
                    && c.sample_format() == SampleFormat::F32
            })
            .or_else(|| {
                // Fallback: accept any mono F32 config we can resample later
                device
                    .supported_input_configs()
                    .ok()?
                    .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::F32)
            })
            .or_else(|| {
                // Fallback: accept any F32 config (will downmix channels)
                device
                    .supported_input_configs()
                    .ok()?
                    .find(|c| c.sample_format() == SampleFormat::F32)
            });

        let (stream_config, actual_channels, actual_rate) = if let Some(sc) = supported_config {
            // Use the closest supported sample rate within the config's range.
            // If our target rate falls within [min, max], use it; otherwise use
            // the config's default (which is always supported by the device).
            let target = cpal::SampleRate(self.sample_rate);
            let rate = if sc.min_sample_rate() <= target && sc.max_sample_rate() >= target {
                target
            } else {
                // Pick the closest boundary rate to minimize resampling distance
                let min_dist = (sc.min_sample_rate().0 as i64 - self.sample_rate as i64).unsigned_abs();
                let max_dist = (sc.max_sample_rate().0 as i64 - self.sample_rate as i64).unsigned_abs();
                if min_dist <= max_dist {
                    sc.min_sample_rate()
                } else {
                    sc.max_sample_rate()
                }
            };
            let config = sc.with_sample_rate(rate).config();
            let ch = config.channels;
            let actual_rate = config.sample_rate.0;
            (config, ch, actual_rate)
        } else {
            // Last resort: use the device's default input config
            let default_config = device
                .default_input_config()
                .context("No suitable input configuration found")?;
            let ch = default_config.channels();
            let rate = default_config.sample_rate().0;
            log::warn!(
                "No F32 config found, using default config: {}ch @ {}Hz {:?}",
                ch, rate, default_config.sample_format()
            );
            (default_config.config(), ch, rate)
        };

        log::info!(
            "Audio stream config: {}ch @ {}Hz (target: {}Hz{})",
            actual_channels,
            actual_rate,
            self.sample_rate,
            if actual_rate != self.sample_rate { " — will resample" } else { "" }
        );

        let (tx, rx) = mpsc::unbounded_channel();
        self.tx = Some(tx.clone());
        self.recording.store(true, Ordering::SeqCst);
        self.buffer.clear();

        let recording = self.recording.clone();
        let sample_rate = self.sample_rate;

        // Clone the optional VAD sender for use inside the audio callback
        let vad_tx = self.vad_tx.clone();

        // Clone the optional volume callback
        let volume_cb = self.volume_cb.take();

        let stream = device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !recording.load(Ordering::SeqCst) {
                    return;
                }

                // Downmix multi-channel to mono by averaging channels
                let chunk: Vec<f32> = if actual_channels == 1 {
                    data.to_vec()
                } else {
                    data.chunks_exact(actual_channels as usize)
                        .map(|frame| frame.iter().sum::<f32>() / actual_channels as f32)
                        .collect()
                };

                // Resample if the device rate differs from our target rate
                let chunk = if actual_rate != sample_rate {
                    simple_resample(&chunk, actual_rate, sample_rate)
                } else {
                    chunk
                };

                // Send to primary tokio channel (for ASR)
                // If the receiver is dropped (recording stopped), that's fine —
                // we just discard the chunk. Log only on unexpected errors.
                if tx.send(chunk.clone()).is_err() {
                    // Receiver dropped — recording has likely stopped
                    return;
                }

                // Compute RMS volume and invoke callback (before moving chunk to VAD)
                if let Some(ref cb) = volume_cb {
                    let rms = compute_rms(&chunk);
                    cb(rms);
                }

                // Send to VAD channel if available (non-blocking, ignore errors)
                if let Some(ref vad) = vad_tx {
                    let _ = vad.send(chunk);
                }
            },
            |err| {
                log::error!("Audio capture error: {}", err);
            },
            None,
        )?;

        stream.play()?;
        self.stream = Some(stream);

        log::info!("Audio recording started at {} Hz", self.sample_rate);
        Ok(rx)
    }

    /// Stop capturing audio. Returns the internal buffer (which is empty unless
    /// samples were manually appended). The caller should use `collect_chunks()`
    /// on the receiver obtained from `start()` to get the actual recorded samples.
    pub fn stop(&mut self) -> Result<Vec<f32>> {
        self.recording.store(false, Ordering::SeqCst);

        // Give the audio callback a moment to flush the last chunk
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Drop the stream to release the device
        self.stream.take();

        // Drop the senders so receivers will get None / try_recv empty
        self.tx.take();
        self.vad_tx.take();

        log::info!("Audio recording stopped");
        Ok(std::mem::take(&mut self.buffer))
    }


}

/// Compute RMS (root-mean-square) volume level from f32 samples.
/// Returns a value in the range 0.0–1.0 where 1.0 corresponds to
/// a full-scale sine wave.
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Simple linear-interpolation resampling from `from_rate` to `to_rate`.
/// Good enough for voice applications; not audiophile-grade.
fn simple_resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((samples.len() as f64) / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;
        let s0 = samples[idx];
        let s1 = if idx + 1 < samples.len() { samples[idx + 1] } else { s0 };
        out.push((s0 as f64 * (1.0 - frac) + s1 as f64 * frac) as f32);
    }
    out
}

/// Drain all chunks from a receiver into a single buffer of f32 samples.
pub fn collect_chunks(rx: &mut mpsc::UnboundedReceiver<AudioChunk>) -> Vec<f32> {
    let mut buffer = Vec::new();
    while let Ok(chunk) = rx.try_recv() {
        buffer.extend_from_slice(&chunk);
    }
    buffer
}

/// Encode f32 samples into WAV format (16-bit PCM) in memory.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut wav_buf = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut wav_buf, spec)?;
        for &sample in samples {
            // Clamp f32 to [-1.0, 1.0] then convert to i16
            let clamped = sample.clamp(-1.0, 1.0);
            let i16_sample = (clamped * 32767.0) as i16;
            writer.write_sample(i16_sample)?;
        }
        writer.finalize()?;
    }

    Ok(wav_buf.into_inner())
}
