//! Sound effects for recording state feedback.
//!
//! Generates short beep tones and plays them via the platform audio API.
//! On Windows, uses waveOut and MessageBeep. On Linux, uses rodio.
//! All sounds play asynchronously on background threads to avoid
//! blocking the caller.

#[cfg(target_os = "windows")]
use std::thread;
#[cfg(target_os = "windows")]
use windows::Win32::Media::Audio::{
    waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
    waveOutWrite, CALLBACK_NULL, HWAVEOUT, WAVEFORMATEX, WAVEHDR, WAVE_FORMAT_PCM, WAVE_MAPPER,
    WHDR_DONE,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION;

/// Sample rate for generated tones.
const SAMPLE_RATE: u32 = 44100;

/// Generate a 16-bit mono PCM sine wave buffer.
///
/// Returns a `Vec<i16>` containing `duration_ms` milliseconds of a
/// sine wave at the given frequency, with a short fade-in/fade-out
/// envelope to avoid clicks.
fn generate_tone(frequency: u32, duration_ms: u32) -> Vec<i16> {
    let num_samples = (SAMPLE_RATE as u64 * duration_ms as u64 / 1000) as usize;
    let mut samples = Vec::with_capacity(num_samples);

    let fade_samples = (SAMPLE_RATE as f64 * 0.005) as usize; // 5ms fade
    let amplitude = 16000.0_f64; // Leave headroom

    for i in 0..num_samples {
        let t = i as f64 / SAMPLE_RATE as f64;
        let raw = amplitude * (2.0 * std::f64::consts::PI * frequency as f64 * t).sin();

        // Apply fade envelope to avoid clicks at start/end
        let envelope = if i < fade_samples {
            i as f64 / fade_samples as f64
        } else if i > num_samples.saturating_sub(fade_samples) {
            (num_samples - i) as f64 / fade_samples as f64
        } else {
            1.0
        };

        samples.push((raw * envelope) as i16);
    }

    samples
}

/// Generate a composite buffer: tone → silence → tone.
fn generate_double_tone(
    freq: u32,
    tone_ms: u32,
    gap_ms: u32,
) -> Vec<i16> {
    let tone1 = generate_tone(freq, tone_ms);
    let gap_samples = (SAMPLE_RATE as u64 * gap_ms as u64 / 1000) as usize;
    let tone2 = generate_tone(freq, tone_ms);

    let mut buf = Vec::with_capacity(tone1.len() + gap_samples + tone2.len());
    buf.extend_from_slice(&tone1);
    buf.extend(std::iter::repeat_n(0i16, gap_samples));
    buf.extend_from_slice(&tone2);
    buf
}

// ── Windows: waveOut API ──────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn play_pcm(samples: &[i16]) {
    let byte_len = std::mem::size_of_val(samples);

    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * std::mem::size_of::<i16>() as u32,
        nBlockAlign: std::mem::size_of::<i16>() as u16,
        wBitsPerSample: 16,
        cbSize: 0,
    };

    unsafe {
        let mut hwo: HWAVEOUT = std::mem::zeroed();
        let result = waveOutOpen(
            Some(&mut hwo),
            WAVE_MAPPER,
            &format,
            0,
            0,
            CALLBACK_NULL,
        );
        if result != 0 {
            log::warn!("waveOutOpen failed with code {}", result);
            return;
        }

        let mut header: WAVEHDR = std::mem::zeroed();
        header.lpData = windows::core::PSTR(samples.as_ptr() as *mut u8);
        header.dwBufferLength = byte_len as u32;

        let result = waveOutPrepareHeader(hwo, std::ptr::addr_of_mut!(header), std::mem::size_of::<WAVEHDR>() as u32);
        if result != 0 {
            log::warn!("waveOutPrepareHeader failed with code {}", result);
            let _ = waveOutClose(hwo);
            return;
        }

        let result = waveOutWrite(hwo, std::ptr::addr_of_mut!(header), std::mem::size_of::<WAVEHDR>() as u32);
        if result != 0 {
            log::warn!("waveOutWrite failed with code {}", result);
            let _ = waveOutUnprepareHeader(hwo, std::ptr::addr_of_mut!(header), std::mem::size_of::<WAVEHDR>() as u32);
            let _ = waveOutClose(hwo);
            return;
        }

        let max_wait = std::time::Duration::from_secs(5);
        let start = std::time::Instant::now();
        loop {
            if header.dwFlags & WHDR_DONE != 0 {
                break;
            }
            if start.elapsed() > max_wait {
                log::warn!("Timed out waiting for waveOut playback to complete");
                let _ = waveOutReset(hwo);
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }

        let _ = waveOutUnprepareHeader(hwo, std::ptr::addr_of_mut!(header), std::mem::size_of::<WAVEHDR>() as u32);
        let _ = waveOutClose(hwo);
    }
}

// ── Linux: rodio ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn play_pcm(samples: &[i16]) {
    use rodio::{OutputStream, Sink};

    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("Failed to get audio output: {}", e);
            return;
        }
    };
    let sink = match Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to create audio sink: {}", e);
            return;
        }
    };
    let source = rodio::buffer::SamplesBuffer::new(1, 44100, samples.to_vec());
    sink.append(source);
    sink.sleep_until_end();
}

// ── Public API (platform-agnostic) ────────────────────────────────────

/// Play a short high-pitched beep to indicate recording started.
///
/// 880 Hz sine wave for 150 ms, played on a background thread.
pub fn play_start_sound() {
    let samples = generate_tone(880, 150);
    std::thread::spawn(move || play_pcm(&samples));
}

/// Play a short low-pitched double-beep to indicate recording stopped.
///
/// 440 Hz × 100 ms, 80 ms gap, 440 Hz × 100 ms, played on a background thread.
pub fn play_stop_sound() {
    let samples = generate_double_tone(440, 100, 80);
    std::thread::spawn(move || play_pcm(&samples));
}

/// Play an error sound.
#[cfg(target_os = "windows")]
pub fn play_error_sound() {
    unsafe {
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    }
}

#[cfg(target_os = "linux")]
pub fn play_error_sound() {
    let samples = generate_tone(440, 200);
    std::thread::spawn(move || play_pcm(&samples));
}

/// Play a warning tone to indicate VAD silence detected (auto-stop imminent).
///
/// 660 Hz × 200 ms, 100 ms gap, 660 Hz × 200 ms, played on a background thread.
pub fn play_warning_sound() {
    let samples = generate_double_tone(660, 200, 100);
    std::thread::spawn(move || play_pcm(&samples));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_tone_produces_correct_length() {
        let samples = generate_tone(440, 100);
        // 44100 Hz × 0.1 s = 4410 samples
        assert_eq!(samples.len(), 4410);
    }

    #[test]
    fn generate_double_tone_produces_correct_length() {
        let samples = generate_double_tone(440, 100, 80);
        // 4410 + (44100 * 0.08) + 4410 = 4410 + 3528 + 4410 = 12348
        assert_eq!(samples.len(), 12348);
    }

    #[test]
    fn generate_tone_fade_envelope_starts_near_zero() {
        let samples = generate_tone(880, 150);
        // First sample should be very small due to fade-in
        assert!(samples[0].unsigned_abs() < 500);
    }
}
