//! Silero VAD wrapper for speech detection.
//!
//! Uses the `silero-vad-rust` crate which bundles ONNX model weights
//! and handles ONNX Runtime session creation internally.
//!
//! If ONNX Runtime is missing or has an incompatible version, `VadDetector::new()`
//! returns an error instead of panicking, allowing the app to fall back to
//! timer-based silence detection.

use anyhow::{Context, Result};
use silero_vad_rust::load_silero_vad;
use silero_vad_rust::silero_vad::model::OnnxModel;

/// Voice Activity Detector wrapper.
pub struct VadDetector {
    model: OnnxModel,
    threshold: f32,
    sample_rate: u32,
    chunk_size: usize,
}

impl VadDetector {
    /// Create a new VAD detector.
    ///
    /// `sample_rate` should be 16000 or 8000 (Silero supported rates).
    /// `threshold` is the speech probability cutoff (typically 0.5).
    ///
    /// Uses `catch_unwind` to intercept panics from ort's version check
    /// (e.g., system onnxruntime.dll is 1.10.0 but ort expects 1.22.x),
    /// converting them into a regular `Err` so the caller can fall back.
    pub fn new(sample_rate: u32, threshold: f32) -> Result<Self> {
        let model = std::panic::catch_unwind(load_silero_vad)
            .map_err(|_| anyhow::anyhow!(
                "ONNX Runtime initialization panicked — likely a version mismatch. \
                 ort 2.0.0-rc.10 requires ONNX Runtime 1.22.x, but the system DLL may be an older version. \
                 Place the correct onnxruntime.dll next to the executable or set ORT_DYLIB_PATH."
            ))?
            .context("Failed to load Silero VAD model")?;

        // Silero VAD expects chunks of 512, 768, 1024, or 1536 samples at 16 kHz
        let chunk_size = match sample_rate {
            16000 => 512,
            8000 => 256,
            _ => 512, // default fallback
        };

        Ok(Self {
            model,
            threshold,
            sample_rate,
            chunk_size,
        })
    }

    /// Process a buffer of samples in chunk_size windows.
    /// Returns a vector of speech probabilities, one per chunk.
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut probs = Vec::new();

        for chunk in samples.chunks(self.chunk_size) {
            // Pad last chunk with zeros if needed
            let padded = if chunk.len() < self.chunk_size {
                let mut tmp = vec![0.0f32; self.chunk_size];
                tmp[..chunk.len()].copy_from_slice(chunk);
                tmp
            } else {
                chunk.to_vec()
            };

            match self.model.forward_chunk(&padded, self.sample_rate) {
                Ok(prob_tensor) => {
                    // Extract scalar probability from the ndarray
                    let prob = prob_tensor[[0, 0]];
                    probs.push(prob);
                }
                Err(e) => {
                    log::warn!("VAD forward_chunk error: {}", e);
                    probs.push(0.0);
                }
            }
        }

        probs
    }

    /// Convenience: returns true if any chunk in the buffer exceeds the threshold.
    pub fn is_speech(&mut self, samples: &[f32]) -> bool {
        let probs = self.process(samples);
        probs.iter().any(|&p| p >= self.threshold)
    }

    /// Get the expected chunk size.
    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }
}


