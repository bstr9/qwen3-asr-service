//! Application entry point.
//!
//! Runs a Win32 message loop on the main thread (required for tray icon,
//! overlay window, and hotkey hook). A tokio runtime handles async work
//! (ASR requests). All cross-thread communication uses `PostMessageW`
//! with custom `WM_APP+n` messages to avoid blocking the UI thread.

mod app;
mod asr_client;
mod audio;
mod clipboard;
mod config;
mod dictionary;
mod history;
mod history_ui;
mod hotkey;
mod i18n;
mod main_window;
mod overlay;
mod postprocess;
mod settings;
mod sound;
mod tray;
mod vad;

use app::{AppAction, AppEvent, AppState, RecordingMode};
use config::AppConfig;
use dictionary::DictionaryManager;
use history::HistoryManager;
use hotkey::{HotkeyEvent, HotkeyKind, HotkeyManager};
use i18n::I18n;
use overlay::OverlayManager;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray::{TrayAction, TrayManager, TrayState};

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ── Custom Windows messages ────────────────────────────────────────────

/// WM_APP base — all our custom messages start here.
const WM_APP_BASE: u32 = 0x8000;

/// Posted when an ASR result is available. wparam = *mut ThreadString.
const WM_ASR_RESULT: u32 = WM_APP_BASE + 100;
/// Posted when an ASR error occurs. wparam = *mut ThreadString.
const WM_ASR_ERROR: u32 = WM_APP_BASE + 101;
/// Posted when silence timeout fires (hands-free mode).
const WM_SILENCE_TIMEOUT: u32 = WM_APP_BASE + 103;
/// Posted when VAD silence starts (hands-free mode).
const WM_VAD_SILENCE_START: u32 = WM_APP_BASE + 104;
/// Posted when VAD silence ends (hands-free mode).
const WM_VAD_SILENCE_END: u32 = WM_APP_BASE + 105;
/// Posted when a hotkey event fires. wparam = HotkeyEvent encoded as usize.
const WM_HOTKEY_EVENT: u32 = WM_APP_BASE + 106;
/// Posted after paste completes.
const WM_PASTE_COMPLETE: u32 = WM_APP_BASE + 107;
/// Posted from tray ToggleMode action.
const WM_TRAY_TOGGLE_MODE: u32 = WM_APP_BASE + 108;
/// Posted from tray ShowSettings action.
const WM_TRAY_SHOW_SETTINGS: u32 = WM_APP_BASE + 109;
/// Posted from tray ShowHistory action.
const WM_TRAY_SHOW_HISTORY: u32 = WM_APP_BASE + 110;
/// Posted from tray ShowMainWindow action.
const WM_TRAY_SHOW_MAINWINDOW: u32 = WM_APP_BASE + 111;

/// Windows timer ID for max recording duration check.
const TIMER_MAX_DURATION: usize = 1;
/// Check interval for max duration timer (milliseconds).
const MAX_DURATION_CHECK_INTERVAL_MS: u32 = 500;
/// Minimum recording duration in seconds. Recordings shorter than this are discarded.
const MIN_RECORDING_DURATION_SECS: f64 = 1.0;

// ── Hotkey event encoding for WPARAM ──────────────────────────────────

/// Encode a HotkeyEvent as a usize for WPARAM.
fn encode_hotkey_event(event: HotkeyEvent) -> usize {
    match event {
        HotkeyEvent::KeyDown(HotkeyKind::Ptt) => 0x00,
        HotkeyEvent::KeyUp(HotkeyKind::Ptt) => 0x01,
        HotkeyEvent::KeyDown(HotkeyKind::HandsFree) => 0x10,
        HotkeyEvent::KeyUp(HotkeyKind::HandsFree) => 0x11,
        HotkeyEvent::KeyDown(HotkeyKind::Cancel) => 0x20,
        HotkeyEvent::KeyUp(HotkeyKind::Cancel) => 0x21,
    }
}

/// Decode a HotkeyEvent from WPARAM.
fn decode_hotkey_event(val: usize) -> Option<HotkeyEvent> {
    match val {
        0x00 => Some(HotkeyEvent::KeyDown(HotkeyKind::Ptt)),
        0x01 => Some(HotkeyEvent::KeyUp(HotkeyKind::Ptt)),
        0x10 => Some(HotkeyEvent::KeyDown(HotkeyKind::HandsFree)),
        0x11 => Some(HotkeyEvent::KeyUp(HotkeyKind::HandsFree)),
        0x20 => Some(HotkeyEvent::KeyDown(HotkeyKind::Cancel)),
        0x21 => Some(HotkeyEvent::KeyUp(HotkeyKind::Cancel)),
        _ => None,
    }
}

// ── Shared strings passed across threads via PostMessage ──────────────

/// A heap-allocated string that can be safely sent to the UI thread
/// via PostMessage. The receiver takes ownership via Box::from_raw.
///
/// **Important**: If `PostMessageW` fails (returns `FALSE`), the pointer
/// is never received and must be reclaimed to avoid a memory leak.
/// Use [`ThreadString::post_or_reclaim`] to handle this safely.
struct ThreadString {
    inner: String,
}

unsafe impl Send for ThreadString {}
unsafe impl Sync for ThreadString {}

impl ThreadString {
    fn new(s: String) -> Box<Self> {
        Box::new(ThreadString { inner: s })
    }

    fn into_string(self) -> String {
        self.inner
    }

    unsafe fn from_raw(ptr: *mut Self) -> Box<Self> {
        Box::from_raw(ptr)
    }

    /// Post a pointer to this `ThreadString` via `PostMessageW`.
    ///
    /// If `PostMessageW` succeeds, ownership transfers to the receiving thread.
    /// If it fails, the `Box` is reconstructed and dropped to prevent a memory leak.
    unsafe fn post_or_reclaim(b: Box<Self>, hwnd: HWND, msg: u32) {
        let ptr = Box::into_raw(b);
        if PostMessageW(hwnd, msg, WPARAM(ptr as usize), LPARAM(0)).is_ok() {
            // Ownership transferred — receiver will call from_raw
        } else {
            // PostMessageW failed — reclaim to avoid leak
            log::warn!("PostMessageW({}) failed, reclaiming ThreadString to prevent leak", msg);
            drop(Box::from_raw(ptr));
        }
    }
}

// ── Send-safe wrapper for HWND ─────────────────────────────────────────

/// HWND is `*mut c_void` which is not Send/Sync. We wrap the raw pointer
/// value as `isize` so it can be captured in closures that need to be
/// `Send + Sync` (e.g. tray/hotkey callbacks that post messages).
struct SendHwnd(isize);

unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

impl SendHwnd {
    fn from_hwnd(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }

    fn to_hwnd(&self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// Send-safe wrapper for a raw pointer to OverlayManager.
/// The OverlayManager uses only atomics internally, so set_volume is thread-safe.
struct SendOverlay(*const overlay::OverlayManager);

unsafe impl Send for SendOverlay {}
unsafe impl Sync for SendOverlay {}

impl SendOverlay {
    fn from_overlay(overlay: &overlay::OverlayManager) -> Self {
        Self(overlay as *const overlay::OverlayManager)
    }

    fn set_volume(&self, level: f32) {
        let mgr = unsafe { &*self.0 };
        let _ = mgr.set_volume(level);
    }
}

// ── Application context ───────────────────────────────────────────────

struct AppContext {
    config: AppConfig,
    state: AppState,
    mode: RecordingMode,
    recorder: Option<audio::AudioRecorder>,
    /// The receiver for audio chunks. Stored separately because
    /// we need to drain it after stopping the recorder.
    audio_rx: Option<tokio::sync::mpsc::UnboundedReceiver<audio::AudioChunk>>,
    recorded_samples: Vec<f32>,
    recording_start: Option<Instant>,
    history: HistoryManager,
    dictionary: DictionaryManager,
    overlay: OverlayManager,
    hotkey: HotkeyManager,
    asr_handle: Option<tokio::task::JoinHandle<()>>,
    /// VAD monitor thread handle (hands-free mode only).
    vad_thread: Option<std::thread::JoinHandle<()>>,
    /// The tray HWND — used for PostMessageW from background threads.
    msg_hwnd: HWND,
    /// Internationalization / translation dictionary.
    i18n: I18n,
    /// Path to the config file (for saving settings and passing to dialogs).
    config_path: std::path::PathBuf,
}

impl AppContext {
    fn new(config: AppConfig, msg_hwnd: HWND, config_path: std::path::PathBuf) -> Self {
        let mode = match config.mode.default.as_str() {
            "handsfree" => RecordingMode::HandsFree,
            _ => RecordingMode::PushToTalk,
        };

        let overlay_position = config.ui.overlay_position.clone();
        let overlay_x = config.ui.overlay_x;
        let overlay_y = config.ui.overlay_y;
        let overlay_minimized = config.ui.overlay_minimized;

        let config_dir = AppConfig::config_dir();
        let history = HistoryManager::new(&config_dir)
            .unwrap_or_else(|e| {
                log::warn!("Failed to init history: {}", e);
                // Fallback to a temp directory — HistoryManager::new with a writable
                // dir should always succeed, but if even that fails, use an in-memory
                // instance that writes to a temp location.
                let fallback_dir = std::env::temp_dir().join("qwen3-asr-typeless");
                HistoryManager::new(&fallback_dir).unwrap_or_else(|e2| {
                    log::error!("Failed to init history in fallback dir: {}. History will not persist.", e2);
                    HistoryManager::new_in_memory()
                })
            });

        let dictionary = DictionaryManager::new(&config_dir)
            .unwrap_or_else(|e| {
                log::warn!("Failed to init dictionary: {}", e);
                let fallback_dir = std::env::temp_dir().join("qwen3-asr-typeless");
                DictionaryManager::new(&fallback_dir).unwrap_or_else(|e2| {
                    log::error!("Failed to init dictionary in fallback dir: {}. Dictionary will not persist.", e2);
                    DictionaryManager::new_in_memory()
                })
            });

        let i18n = I18n::from_config(&config.ui.language);

        Self {
            config,
            state: AppState::Idle,
            mode,
            recorder: None,
            audio_rx: None,
            recorded_samples: Vec::new(),
            recording_start: None,
            history,
            dictionary,
            overlay: OverlayManager::new(overlay_position, overlay_x, overlay_y, overlay_minimized),
            hotkey: HotkeyManager::new(),
            asr_handle: None,
            vad_thread: None,
            msg_hwnd,
            i18n,
            config_path,
        }
    }
}

// ── Main entry point ──────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {}] {}",
                buf.timestamp(),
                record.level(),
                record.args()
            )
        })
        .init();

    // Load config
    let config_path = AppConfig::default_config_path();
    let config = AppConfig::load(&config_path).unwrap_or_else(|e| {
        log::warn!("Failed to load config: {}, using defaults", e);
        AppConfig::default()
    });

    log::info!("Qwen3-ASR Typeless starting...");
    log::info!("ASR URL: {}", config.asr_url);
    log::info!("Mode: {}", config.mode.default);

    // Sync auto-start registry state with config
    if config.ui.start_with_system {
        if let Err(e) = config::set_auto_start(true) {
            log::warn!("Failed to set auto-start: {}", e);
        }
    }

    // Create the tokio runtime manually (NOT #[tokio::main])
    // We use Arc<Runtime> so that cleanup can consume it.
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    );

    // Create tray window on the main thread (required by Win32)
    let msg_hwnd = tray::create_tray_window()?;

    // Set up tray icon — capture HWND as SendHwnd for the callback closure
    let send_hwnd = SendHwnd::from_hwnd(msg_hwnd);
    let mut tray_mgr = TrayManager::new(msg_hwnd)?;
    tray_mgr.set_callback(Box::new(move |action: TrayAction| {
        let hwnd = send_hwnd.to_hwnd();
        match action {
            TrayAction::ToggleMode => {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_TRAY_TOGGLE_MODE, WPARAM(0), LPARAM(0));
                }
            }
            TrayAction::ShowMainWindow => {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_TRAY_SHOW_MAINWINDOW, WPARAM(0), LPARAM(0));
                }
            }
            TrayAction::ShowHistory => {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_TRAY_SHOW_HISTORY, WPARAM(0), LPARAM(0));
                }
            }
            TrayAction::ShowSettings => {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_TRAY_SHOW_SETTINGS, WPARAM(0), LPARAM(0));
                }
            }
            TrayAction::About => {
                unsafe {
                    let _ = PostMessageW(hwnd, WM_TRAY_SHOW_MAINWINDOW, WPARAM(0), LPARAM(0));
                }
            }
            TrayAction::Quit => {
                unsafe { PostQuitMessage(0); }
            }
        }
    }));
    tray_mgr.update_mode_display(matches!(config.mode.default.as_str(), "handsfree"))?;
    tray_mgr.show()?;
    tray::set_global_tray(Box::new(tray_mgr));

    // Create overlay window — pass saved position and minimized state from config
    let overlay = OverlayManager::new(
        config.ui.overlay_position.clone(),
        config.ui.overlay_x,
        config.ui.overlay_y,
        config.ui.overlay_minimized,
    );
    overlay.create(Some(msg_hwnd))?;

    // Initialize app context
    let mut ctx = AppContext::new(config, msg_hwnd, config_path.clone());
    ctx.overlay = overlay;

    // Cleanup expired history entries on startup
    match ctx.history.cleanup_expired(ctx.config.ui.history_retention_days) {
        Ok(count) => {
            if count > 0 {
                log::info!("Cleaned up {} expired history entries", count);
            }
        }
        Err(e) => {
            log::warn!("Failed to cleanup expired history: {}", e);
        }
    }

    // Start hotkey hook — posts WM_HOTKEY_EVENT to msg_hwnd
    let send_hwnd2 = SendHwnd::from_hwnd(msg_hwnd);
    ctx.hotkey.start(
        &ctx.config.hotkey.ptt_key,
        &ctx.config.hotkey.handsfree_key,
        &ctx.config.hotkey.cancel_key,
        Box::new(move |event: HotkeyEvent| {
            let encoded = encode_hotkey_event(event);
            let hwnd = send_hwnd2.to_hwnd();
            unsafe {
                let _ = PostMessageW(hwnd, WM_HOTKEY_EVENT, WPARAM(encoded), LPARAM(0));
            }
        }),
    )?;

    log::info!("Hotkey hook installed. Press F8 (PTT) or RightAlt+Space (Hands-free).");

    // Startup health check: verify ASR service is reachable
    {
        let asr_url = ctx.config.asr_url.clone();
        let send_hwnd3 = SendHwnd::from_hwnd(msg_hwnd);
        std::thread::Builder::new()
            .name("health-check".into())
            .spawn(move || {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .build()
                    .unwrap_or_else(|_| reqwest::blocking::Client::new());
                let url = format!("{}/v1/health", asr_url);
                match client.get(&url).send() {
                    Ok(resp) if resp.status().is_success() => {
                        log::info!("ASR service health check passed");
                    }
                    Ok(resp) => {
                        log::warn!("ASR service health check returned status {}", resp.status());
                        let hwnd = send_hwnd3.to_hwnd();
                        unsafe {
                            let msg_ptr = ThreadString::new(format!("Service returned status {}", resp.status()));
                            ThreadString::post_or_reclaim(msg_ptr, hwnd, WM_ASR_ERROR);
                        }
                    }
                    Err(e) => {
                        log::warn!("ASR service unreachable: {}", e);
                        // Set disconnected state and notify
                        tray::set_global_state(TrayState::Disconnected);
                        let hwnd = send_hwnd3.to_hwnd();
                        unsafe {
                            let msg_ptr = ThreadString::new(format!("ASR service unreachable: {}", e));
                            ThreadString::post_or_reclaim(msg_ptr, hwnd, WM_ASR_ERROR);
                        }
                    }
                }
            })?;
    }

    // ── Standard Win32 message loop ────────────────────────────────────

    let mut msg = MSG::default();

    loop {
        // GetMessageW blocks until a message arrives — no CPU spinning.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !got.as_bool() || msg.message == WM_QUIT {
            log::info!("WM_QUIT received, exiting...");
            cleanup(&mut ctx, rt);
            return Ok(());
        }

        // Dispatch custom messages to our handler
        if msg.message >= WM_APP_BASE && msg.message <= WM_APP_BASE + 200 {
            handle_custom_message(&mut ctx, msg.message, msg.wParam, msg.lParam, &rt);
        } else if msg.message == WM_TIMER {
            // Handle max recording duration timer (TIMER_MAX_DURATION on msg_hwnd)
            // Overlay timers (TIMER_RECORDING, TIMER_PROCESSING) are handled by
            // the overlay WndProc via DispatchMessageW.
            let timer_id = msg.wParam.0;
            if timer_id == TIMER_MAX_DURATION {
                handle_timer(&mut ctx, msg.wParam, &rt);
            } else {
                // Dispatch overlay timer messages to the overlay WndProc
                let _ = unsafe { TranslateMessage(&msg) };
                unsafe { DispatchMessageW(&msg); }
            }
        } else {
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg); }
        }
    }
}

// ── Custom message dispatch ───────────────────────────────────────────

fn handle_custom_message(
    ctx: &mut AppContext,
    msg: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
    rt: &Arc<tokio::runtime::Runtime>,
) {
    match msg {
        WM_HOTKEY_EVENT => {
            if let Some(hotkey_event) = decode_hotkey_event(wparam.0) {
                let app_event = match hotkey_event {
                    HotkeyEvent::KeyDown(HotkeyKind::Ptt) => AppEvent::HotKeyDown,
                    HotkeyEvent::KeyUp(HotkeyKind::Ptt) => AppEvent::HotKeyUp,
                    HotkeyEvent::KeyDown(HotkeyKind::HandsFree) => AppEvent::HotKeyDown,
                    HotkeyEvent::KeyUp(HotkeyKind::HandsFree) => AppEvent::HotKeyUp,
                    HotkeyEvent::KeyDown(HotkeyKind::Cancel) => AppEvent::CancelEsc,
                    HotkeyEvent::KeyUp(HotkeyKind::Cancel) => return, // ignore
                };
                dispatch_event(ctx, app_event, rt);
            }
        }

        WM_ASR_RESULT => {
            let ptr = wparam.0 as *mut ThreadString;
            let text = if !ptr.is_null() {
                let boxed = unsafe { ThreadString::from_raw(ptr) };
                boxed.into_string()
            } else {
                String::new()
            };
            dispatch_event(ctx, AppEvent::AsrResult(text), rt);
        }

        WM_ASR_ERROR => {
            let ptr = wparam.0 as *mut ThreadString;
            let error_msg = if !ptr.is_null() {
                let boxed = unsafe { ThreadString::from_raw(ptr) };
                boxed.into_string()
            } else {
                "Unknown ASR error".to_string()
            };
            dispatch_event(ctx, AppEvent::AsrError(error_msg), rt);
        }

        WM_SILENCE_TIMEOUT => {
            dispatch_event(ctx, AppEvent::SilenceTimeout, rt);
        }

        WM_VAD_SILENCE_START => {
            dispatch_event(ctx, AppEvent::VadSilenceStart, rt);
        }

        WM_VAD_SILENCE_END => {
            dispatch_event(ctx, AppEvent::VadSilenceEnd, rt);
        }

        WM_PASTE_COMPLETE => {
            dispatch_event(ctx, AppEvent::PasteComplete, rt);
        }

        WM_TRAY_TOGGLE_MODE => {
            ctx.mode = match ctx.mode {
                RecordingMode::PushToTalk => RecordingMode::HandsFree,
                RecordingMode::HandsFree => RecordingMode::PushToTalk,
            };
            let is_hf = ctx.mode == RecordingMode::HandsFree;
            log::info!("Mode switched to {}", if is_hf { "Hands-free" } else { "Push-to-Talk" });
            tray::update_global_mode_display(is_hf);
        }

        WM_TRAY_SHOW_SETTINGS => {
            let config_path = config::AppConfig::default_config_path();
            settings::show_settings_dialog(&mut ctx.config, &config_path, &mut ctx.dictionary, ctx.msg_hwnd);
        }

        WM_TRAY_SHOW_HISTORY => {
            history_ui::show_history_window(&ctx.history, ctx.msg_hwnd);
        }

        WM_TRAY_SHOW_MAINWINDOW => {
            if !main_window::is_main_window_open() {
                main_window::show_main_window(&mut ctx.config, &ctx.config_path, &mut ctx.dictionary, &ctx.history, &ctx.i18n, ctx.msg_hwnd);
            } else {
                // Already open — just bring to front
                main_window::show_main_window(&mut ctx.config, &ctx.config_path, &mut ctx.dictionary, &ctx.history, &ctx.i18n, ctx.msg_hwnd);
            }
        }

        _ => {}
    }
}

// ── Timer handler ─────────────────────────────────────────────────────

fn handle_timer(ctx: &mut AppContext, wparam: WPARAM, rt: &Arc<tokio::runtime::Runtime>) {
    let timer_id = wparam.0;
    if timer_id != TIMER_MAX_DURATION {
        return;
    }

    // Only act if we're currently recording
    if !matches!(ctx.state, AppState::Recording(_)) {
        return;
    }

    let max_dur = ctx.config.max_recording_duration;
    if max_dur == 0 {
        return;
    }

    if let Some(start) = ctx.recording_start {
        let elapsed = start.elapsed().as_secs();
        if elapsed >= max_dur {
            log::info!("Max recording duration reached ({}s), auto-submitting", max_dur);
            // Stop the timer before dispatching event
            unsafe {
                let _ = KillTimer(ctx.msg_hwnd, TIMER_MAX_DURATION);
            }
            // Show notification about max duration
            if ctx.config.ui.show_overlay {
                ctx.overlay.set_status("Max duration reached, submitting...").ok();
            }
            dispatch_event(ctx, AppEvent::HotKeyUp, rt);
        }
    }
}

// ── State machine dispatch ────────────────────────────────────────────

fn dispatch_event(ctx: &mut AppContext, event: AppEvent, rt: &Arc<tokio::runtime::Runtime>) {
    let (new_state, actions) = app::handle_event(ctx.state.clone(), event, ctx.mode.clone());
    ctx.state = new_state;

    for action in actions {
        execute_action(ctx, action, rt);
    }
}

// ── Action execution ──────────────────────────────────────────────────

fn execute_action(ctx: &mut AppContext, action: AppAction, rt: &Arc<tokio::runtime::Runtime>) {
    match action {
        AppAction::StartRecording => {
            tray::set_global_state(TrayState::Recording);
            let mut recorder = audio::AudioRecorder::new(ctx.config.sample_rate);

            // Set up volume callback for the VU meter overlay
            let send_overlay = SendOverlay::from_overlay(&ctx.overlay);
            recorder.set_volume_callback(Box::new(move |level: f32| {
                send_overlay.set_volume(level);
            }));

            // For hands-free mode, set up VAD monitoring channel
            if ctx.mode == RecordingMode::HandsFree {
                let (vad_tx, vad_rx) = std::sync::mpsc::channel::<audio::AudioChunk>();
                recorder.set_vad_channel(vad_tx);

                // Spawn VAD monitor thread (OnnxModel is not Send, must stay in one thread)
                let vad_threshold = ctx.config.vad_threshold;
                let silence_duration = ctx.config.silence_duration_secs;
                let sample_rate = ctx.config.sample_rate;
                let send_hwnd = SendHwnd::from_hwnd(ctx.msg_hwnd);

                let vad_handle = match std::thread::Builder::new()
                    .name("vad-monitor".into())
                    .spawn(move || {
                        vad_monitor_thread(
                            vad_rx,
                            vad_threshold,
                            silence_duration,
                            sample_rate,
                            send_hwnd,
                        );
                    }) {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("Failed to spawn VAD monitor thread: {}", e);
                        // Stop recording since we can't monitor for silence
                        if let Some(ref mut recorder) = ctx.recorder {
                            let _ = recorder.stop();
                        }
                        ctx.recorder = None;
                        ctx.audio_rx = None;
                        ctx.state = AppState::Idle;
                        ctx.overlay.set_status("VAD thread failed").ok();
                        ctx.overlay.hide().ok();
                        return;
                    }
                };

                ctx.vad_thread = Some(vad_handle);
            }

            match recorder.start() {
                Ok(rx) => {
                    ctx.recorder = Some(recorder);
                    ctx.audio_rx = Some(rx);
                    ctx.recorded_samples.clear();
                    ctx.recording_start = Some(Instant::now());
                    ctx.overlay.set_recording(true).ok();

                    // Set recording start timestamp for duration display
                    let start_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    ctx.overlay.set_recording_start(start_ms).ok();

                    // Start a periodic timer to check for max recording duration
                    if ctx.config.max_recording_duration > 0 {
                        unsafe {
                            let _ = SetTimer(
                                ctx.msg_hwnd,
                                TIMER_MAX_DURATION,
                                MAX_DURATION_CHECK_INTERVAL_MS,
                                None,
                            );
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to start recording: {}", e);
                    ctx.state = AppState::Idle;
                    // Clean up VAD thread if it was started
                    ctx.vad_thread = None;
                }
            }
        }

        AppAction::StopRecording => {
            // Stop the recorder first (drops the stream, which stops audio capture)
            if let Some(ref mut recorder) = ctx.recorder {
                let _ = recorder.stop();
            }
            ctx.recorder = None;

            // Drain the receiver to collect all recorded samples
            if let Some(ref mut rx) = ctx.audio_rx {
                ctx.recorded_samples = audio::collect_chunks(rx);
            }
            ctx.audio_rx = None;

            // Stop VAD monitor thread (dropping the sender in stop() will cause
            // the receiver to return Err, which exits the VAD thread loop)
            if let Some(vad_handle) = ctx.vad_thread.take() {
                let _ = vad_handle.join();
            }

            // Kill the max duration timer
            unsafe { let _ = KillTimer(ctx.msg_hwnd, TIMER_MAX_DURATION); }

            ctx.overlay.set_recording(false).ok();

            let sample_count = ctx.recorded_samples.len();
            let duration = sample_count as f64 / ctx.config.sample_rate as f64;
            log::info!("Stopped recording: {} samples ({:.2}s)", sample_count, duration);

            // Min recording duration check: discard recordings shorter than 1 second
            if duration < MIN_RECORDING_DURATION_SECS {
                log::info!("Recording too short ({:.2}s < {:.1}s), discarded", duration, MIN_RECORDING_DURATION_SECS);
                ctx.state = AppState::Idle;
                ctx.overlay.set_status("Too short, discarded").ok();
                tray::set_global_state(TrayState::Idle);
                return;
            }

            // Update tray state to processing
            tray::set_global_state(TrayState::Processing);

            // If we have samples, encode to WAV and send to ASR
            if !ctx.recorded_samples.is_empty() {
                match audio::encode_wav(&ctx.recorded_samples, ctx.config.sample_rate) {
                    Ok(wav_data) => {
                        send_to_asr(ctx, wav_data, rt);
                    }
                    Err(e) => {
                        log::error!("Failed to encode WAV: {}", e);
                        ctx.state = AppState::Idle;
                        ctx.overlay.set_status("Encoding failed").ok();
                    }
                }
            } else {
                log::warn!("No audio samples captured");
                ctx.state = AppState::Idle;
                ctx.overlay.set_status("No audio captured").ok();
            }
        }

        AppAction::PasteText(raw_asr_text) => {
            // Mark overlay as processing (spinner animation)
            ctx.overlay.set_processing(true).ok();

            // Store the raw ASR text before post-processing
            let original_text = raw_asr_text.clone();

            // Run synchronous post-processing pipeline
            let mut processed = postprocess::postprocess(&raw_asr_text, &ctx.config.post_processing);

            // Run optional LLM post-processing if configured
            if ctx.config.post_processing.llm_url.is_some() {
                let dict_hint = ctx.dictionary.format_for_prompt();
                let hint_opt = if dict_hint.is_empty() { None } else { Some(dict_hint.as_str()) };
                match rt.block_on(postprocess::llm_postprocess(&processed, &ctx.config.post_processing, hint_opt)) {
                    Ok(refined) => {
                        log::info!("LLM post-processing applied");
                        processed = refined;
                    }
                    Err(e) => {
                        log::warn!("LLM post-processing failed, using local result: {}", e);
                    }
                }
            }

            // Save clipboard before pasting, then paste and schedule restore
            let saved = clipboard::save_clipboard();
            match clipboard::paste_text(&processed) {
                Ok(()) => {
                    log::info!("Text pasted successfully");
                    ctx.overlay.set_processing(false).ok();
                    // Save to history (use processed text, store original ASR text as raw)
                    let duration_secs = ctx.recording_start
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    let mode_str = match ctx.mode {
                        RecordingMode::PushToTalk => "ptt",
                        RecordingMode::HandsFree => "handsfree",
                    };
                    let entry = history::HistoryEntry::new(
                        processed,
                        Some(original_text),
                        duration_secs,
                        mode_str.to_string(),
                    );
                    ctx.history.add(entry).ok();
                    ctx.recording_start = None;

                    // Schedule clipboard restore on a background thread
                    std::thread::spawn(move || {
                        clipboard::restore_clipboard(saved);
                    });

                    // Post PasteComplete to transition state
                    unsafe {
                        let _ = PostMessageW(ctx.msg_hwnd, WM_PASTE_COMPLETE, WPARAM(0), LPARAM(0));
                    }
                }
                Err(e) => {
                    log::error!("Failed to paste text: {}", e);
                    ctx.overlay.set_processing(false).ok();
                    ctx.state = AppState::Idle;
                    ctx.overlay.hide().ok();
                }
            }
        }

        AppAction::PlayStopSound => {
            if ctx.config.ui.play_sounds {
                sound::play_stop_sound();
            }
        }

        AppAction::PlayErrorSound => {
            if ctx.config.ui.play_sounds {
                sound::play_error_sound();
            }
        }

        AppAction::PlayWarningSound => {
            if ctx.config.ui.play_sounds {
                sound::play_warning_sound();
            }
        }

        AppAction::PlayStartSound => {
            if ctx.config.ui.play_sounds {
                sound::play_start_sound();
            }
        }

        AppAction::ShowOverlay(text) => {
            if ctx.config.ui.show_overlay {
                ctx.overlay.set_status(&text).ok();
                ctx.overlay.show().ok();
            }
        }

        AppAction::HideOverlay => {
            // Save overlay position before hiding
            let (pos_x, pos_y) = ctx.overlay.save_position();
            if pos_x >= 0 && pos_y >= 0 {
                ctx.config.ui.overlay_x = Some(pos_x);
                ctx.config.ui.overlay_y = Some(pos_y);
            }
            ctx.overlay.hide().ok();
            tray::set_global_state(TrayState::Idle);
        }

        AppAction::ShowNotification(msg) => {
            log::info!("Notification: {}", msg);
            tray::show_global_balloon("Qwen3-ASR", &msg);
        }

        AppAction::CancelRecording(partial_text) => {
            // Stop the recorder if still running
            if let Some(ref mut recorder) = ctx.recorder {
                let _ = recorder.stop();
            }
            ctx.recorder = None;
            ctx.audio_rx = None;

            // Stop VAD thread
            if let Some(vad_handle) = ctx.vad_thread.take() {
                let _ = vad_handle.join();
            }

            // Kill the max duration timer
            unsafe { let _ = KillTimer(ctx.msg_hwnd, TIMER_MAX_DURATION); }

            ctx.overlay.set_recording(false).ok();
            ctx.overlay.set_processing(false).ok();
            ctx.overlay.hide().ok();

            // Save cancelled entry to history
            let duration_secs = ctx.recording_start
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            ctx.recording_start = None;

            let mode_str = match ctx.mode {
                RecordingMode::PushToTalk => "ptt",
                RecordingMode::HandsFree => "handsfree",
            };
            let text = partial_text.unwrap_or_default();
            let entry = history::HistoryEntry::new_cancelled(
                if text.is_empty() { "(cancelled)".to_string() } else { text },
                None,
                duration_secs,
                mode_str.to_string(),
            );
            if let Err(e) = ctx.history.add(entry) {
                log::warn!("Failed to save cancelled entry to history: {}", e);
            } else {
                log::info!("Saved cancelled recording to history ({:.1}s)", duration_secs);
            }

            tray::set_global_state(TrayState::Idle);
        }
    }
}

// ── ASR request ───────────────────────────────────────────────────────

fn send_to_asr(ctx: &mut AppContext, wav_data: Vec<u8>, rt: &Arc<tokio::runtime::Runtime>) {
    let asr_url = ctx.config.asr_url.clone();
    let api_key = ctx.config.api_key.clone();
    let send_hwnd = SendHwnd::from_hwnd(ctx.msg_hwnd);

    ctx.asr_handle = Some(rt.spawn(async move {
        let client = asr_client::AsrClient::new(asr_url, api_key);
        let result = client.transcribe(&wav_data).await;
        // Reconstruct HWND only after the await, so it's not held across it
        let post_hwnd = send_hwnd.to_hwnd();
        match result {
            Ok(text) => {
                let boxed = ThreadString::new(text);
                unsafe {
                    ThreadString::post_or_reclaim(boxed, post_hwnd, WM_ASR_RESULT);
                }
            }
            Err(e) => {
                let msg = format!("{}", e);
                let boxed = ThreadString::new(msg);
                unsafe {
                    ThreadString::post_or_reclaim(boxed, post_hwnd, WM_ASR_ERROR);
                }
            }
        }
    }));
}

// ── VAD Monitor Thread ────────────────────────────────────────────────

/// Runs on a dedicated std::thread. Reads audio chunks from the channel,
/// runs Silero VAD on each chunk, and posts silence/speech events to the
/// UI thread via PostMessageW.
///
/// The thread exits when the sender is dropped (i.e. when recording stops).
fn vad_monitor_thread(
    rx: std::sync::mpsc::Receiver<audio::AudioChunk>,
    vad_threshold: f32,
    silence_duration_secs: f64,
    sample_rate: u32,
    send_hwnd: SendHwnd,
) {
    log::info!("VAD monitor thread started (threshold={}, silence={}s)", vad_threshold, silence_duration_secs);

    // Initialize VAD detector
    let mut vad = match vad::VadDetector::new(sample_rate, vad_threshold) {
        Ok(v) => v,
        Err(e) => {
            log::error!("Failed to initialize VAD: {}. Falling back to timer-based silence.", e);
            // Fallback: simple timer-based silence detection
            fallback_silence_timer(rx, silence_duration_secs, send_hwnd);
            return;
        }
    };

    let chunk_size = vad.chunk_size();
    let mut sample_buffer: Vec<f32> = Vec::with_capacity(chunk_size * 2);

    // State tracking
    let mut is_silence = false;
    let mut silence_start: Option<Instant> = None;

    // Accumulate enough samples for one VAD chunk, then process
    for chunk in rx.iter() {
        sample_buffer.extend_from_slice(&chunk);

        // Process in VAD-sized chunks
        while sample_buffer.len() >= chunk_size {
            let vad_chunk: Vec<f32> = sample_buffer.drain(..chunk_size).collect();
            let has_speech = vad.is_speech(&vad_chunk);

            let hwnd = send_hwnd.to_hwnd();

            if has_speech {
                if is_silence {
                    // Speech resumed after silence
                    log::debug!("VAD: speech resumed");
                    is_silence = false;
                    silence_start = None;
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_VAD_SILENCE_END, WPARAM(0), LPARAM(0));
                    }
                }
            } else {
                // No speech detected
                if !is_silence {
                    // Transition to silence
                    log::debug!("VAD: silence started");
                    is_silence = true;
                    silence_start = Some(Instant::now());
                    unsafe {
                        let _ = PostMessageW(hwnd, WM_VAD_SILENCE_START, WPARAM(0), LPARAM(0));
                    }
                } else if let Some(start) = silence_start {
                    // Check if silence has exceeded the threshold
                    let elapsed = start.elapsed().as_secs_f64();
                    if elapsed >= silence_duration_secs {
                        log::info!("VAD: silence timeout ({:.1}s >= {:.1}s)", elapsed, silence_duration_secs);
                        unsafe {
                            let _ = PostMessageW(hwnd, WM_SILENCE_TIMEOUT, WPARAM(0), LPARAM(0));
                        }
                        // Exit the loop — recording will be stopped by the UI thread
                        return;
                    }
                }
            }
        }
    }

    // Channel closed — recording stopped
    log::info!("VAD monitor thread exiting (channel closed)");
}

/// Fallback timer-based silence detection when VAD fails to initialize.
/// Simply waits for the configured silence duration and posts WM_SILENCE_TIMEOUT.
fn fallback_silence_timer(
    rx: std::sync::mpsc::Receiver<audio::AudioChunk>,
    silence_duration_secs: f64,
    send_hwnd: SendHwnd,
) {
    let start = Instant::now();
    // Drain the channel to keep it alive, but ignore the data
    for _chunk in rx.iter() {
        if start.elapsed().as_secs_f64() >= silence_duration_secs {
            let hwnd = send_hwnd.to_hwnd();
            unsafe {
                let _ = PostMessageW(hwnd, WM_SILENCE_TIMEOUT, WPARAM(0), LPARAM(0));
            }
            return;
        }
    }
}

// ── Cleanup ───────────────────────────────────────────────────────────

fn cleanup(ctx: &mut AppContext, rt: Arc<tokio::runtime::Runtime>) {
    // Close the main window if it's open
    main_window::close_main_window();

    if ctx.recorder.is_some() {
        if let Some(ref mut recorder) = ctx.recorder {
            let _ = recorder.stop();
        }
        ctx.recorder = None;
        ctx.audio_rx = None;
    }

    // Join VAD thread
    if let Some(vad_handle) = ctx.vad_thread.take() {
        let _ = vad_handle.join();
    }

    let _ = ctx.hotkey.stop();
    ctx.overlay.destroy().ok();

    if let Some(handle) = ctx.asr_handle.take() {
        handle.abort();
    }

    // shutdown_timeout takes Runtime by value — we own Arc, so we can
    // try to get exclusive ownership. If other references exist, just
    // drop our reference and let the background threads finish naturally.
    match Arc::try_unwrap(rt) {
        Ok(runtime) => {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
        Err(rt) => {
            drop(rt);
        }
    }
    log::info!("Qwen3-ASR Typeless shut down.");
}
