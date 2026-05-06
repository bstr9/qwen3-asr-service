# Typeless — Android ASR Client

Android native client for the [Qwen3-ASR Service](https://github.com/LanceLRQ/qwen3-asr-service) voice dictation system.

## Features

- **Voice Recording** — Capture audio via device microphone (16 kHz, mono, 16-bit PCM)
- **Silero VAD** — Voice activity detection for hands-free auto-stop on silence
- **ASR Integration** — Upload audio to Qwen3-ASR service, poll for transcription results
- **Two Modes**:
  - **PTT (Push-to-Talk)** — Tap to start, tap to stop
  - **Hands-free** — Auto-stop when silence is detected via VAD
- **Foreground Service** — Continue recording even when app is minimized
- **History** — Room database stores all transcriptions with search and delete
- **Material Design 3** — Clean, modern UI with recording states
- **Clipboard** — Transcription results are automatically copied to clipboard

## Prerequisites

- Android Studio Hedgehog (2023.1.1) or later
- Android SDK 34
- Kotlin 1.9+
- A running Qwen3-ASR service on your local network

## Setup

1. Clone this repository into your project
2. Open the `qwen3-asr-android` directory in Android Studio
3. Download the Silero VAD ONNX model:
   ```bash
   # Place the model in app/src/main/assets/
   mkdir -p app/src/main/assets/
   wget https://github.com/snakers4/silero-vad/raw/master/src/silero_vad.onnx \
     -O app/src/main/assets/silero_vad.onnx
   ```
4. Build and run on a device or emulator

## Configuration

Open **Settings** in the app to configure:

| Setting | Default | Description |
|---------|---------|-------------|
| ASR Service URL | `http://192.168.1.100:8765` | URL of your Qwen3-ASR service |
| API Key | _(empty)_ | Optional Bearer token for API authentication |
| Default Mode | PTT | Recording mode: PTT or Hands-free |
| VAD Threshold | 0.50 | Speech probability threshold (0.0–1.0) |
| Silence Duration | 2.0 seconds | How long silence must persist before auto-stop |
| Post-processing | Enabled | Enable punctuation cleanup |

## Usage

1. Ensure your Qwen3-ASR service is running on the local network
2. Configure the ASR URL in Settings
3. Tap the microphone button to start recording
4. In PTT mode: tap again to stop
5. In Hands-free mode: speak naturally, recording auto-stops after silence
6. Wait for transcription to complete
7. Result is displayed and copied to clipboard

## Project Structure

```
app/src/main/java/com/qwen3/asr/typeless/
├── App.kt              — Application class, singleton AsrClient
├── MainActivity.kt     — Main UI with recording button + status
├── AsrClient.kt        — HTTP client for ASR service (multipart + polling)
├── AudioRecorder.kt    — Microphone capture + PCM→WAV conversion
├── VadDetector.kt      — Silero VAD wrapper (ONNX Runtime)
├── RecordingService.kt — Foreground service for recording
├── SettingsActivity.kt — Settings screen
├── HistoryActivity.kt  — History viewer with search
└── HistoryDatabase.kt  — Room database for transcription history
```

## ASR Service API

The client communicates with the Qwen3-ASR service via:

1. **POST `/v1/asr`** — Submit audio file (multipart form: `file` + optional `language`)
   → Returns `{"task_id": "..."}`

2. **GET `/v1/tasks/{task_id}`** — Poll for transcription result
   → Returns `{"status": "completed", "result": {"full_text": "..."}}`

Optional `Authorization: Bearer <token>` header for API authentication.

## License

This project is provided as-is for use with the Qwen3-ASR service.
