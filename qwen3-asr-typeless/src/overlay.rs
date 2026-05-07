//! Floating overlay window with platform-specific implementations.
//!
//! Displays recording status, a VU meter bar, status text, recording duration,
//! and a processing animation in a small, always-on-top, draggable overlay window.
//! Supports minimize mode (dot only) and position memory across sessions.
//!
//! - Windows: Win32 GDI with a layered popup window
//! - Linux: GTK4 with a custom DrawingArea for cairo rendering

use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::sync::OnceLock;

// ── Shared constants ──────────────────────────────────────────────────

// Overlay window dimensions
const OVERLAY_WIDTH: i32 = 320;
const OVERLAY_HEIGHT: i32 = 60;
const OVERLAY_HEIGHT_MINIMIZED: i32 = 30;

// VU meter bar dimensions and position
const BAR_X: i32 = 20;
const BAR_Y: i32 = 34;
const BAR_WIDTH: i32 = 280;
const BAR_HEIGHT: i32 = 8;

// Title area height for drag detection (top portion of overlay)
const TITLE_AREA_HEIGHT: i32 = 26;

// Colors (RGB)
#[cfg(target_os = "linux")]
const COLOR_BG_R: f64 = 30.0 / 255.0;
#[cfg(target_os = "linux")]
const COLOR_BG_G: f64 = 30.0 / 255.0;
#[cfg(target_os = "linux")]
const COLOR_BG_B: f64 = 30.0 / 255.0;

#[cfg(target_os = "linux")]
const COLOR_BAR_BG_R: f64 = 60.0 / 255.0;
#[cfg(target_os = "linux")]
const COLOR_BAR_BG_G: f64 = 60.0 / 255.0;
#[cfg(target_os = "linux")]
const COLOR_BAR_BG_B: f64 = 60.0 / 255.0;

const COLOR_GREEN_R: f64 = 0.0 / 255.0;
const COLOR_GREEN_G: f64 = 200.0 / 255.0;
const COLOR_GREEN_B: f64 = 80.0 / 255.0;

const COLOR_YELLOW_R: f64 = 1.0;
const COLOR_YELLOW_G: f64 = 200.0 / 255.0;
const COLOR_YELLOW_B: f64 = 0.0 / 255.0;

const COLOR_RED_R: f64 = 1.0;
const COLOR_RED_G: f64 = 60.0 / 255.0;
const COLOR_RED_B: f64 = 60.0 / 255.0;

// Semi-transparent alpha (0–255)
const OVERLAY_ALPHA: u8 = 220;

/// Spinner characters for the "Processing..." animation.
const SPINNER_CHARS: &[char] = &['|', '/', '-', '\\'];

/// Timer interval for recording duration repaint (ms).
const TIMER_RECORDING_INTERVAL: u32 = 500;
/// Timer interval for processing spinner (ms).
const TIMER_PROCESSING_INTERVAL: u32 = 200;

/// Determine VU meter color based on volume level.
///
/// - 0–60%: green (0, 200, 80)
/// - 60–85%: yellow (255, 200, 0)
/// - 85–100%: red (255, 60, 60)
fn vu_meter_color(level: f32) -> (f64, f64, f64) {
    if level < 0.60 {
        (COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B)
    } else if level < 0.85 {
        (COLOR_YELLOW_R, COLOR_YELLOW_G, COLOR_YELLOW_B)
    } else {
        (COLOR_RED_R, COLOR_RED_G, COLOR_RED_B)
    }
}

/// Build the display text based on current overlay state.
fn build_display_text(
    status_text: &str,
    is_recording: bool,
    is_processing: bool,
    spinner_idx: i32,
    recording_start_ms: u64,
) -> String {
    if is_processing {
        let spinner_char = SPINNER_CHARS[spinner_idx as usize % SPINNER_CHARS.len()];
        format!("{} Processing...", spinner_char)
    } else if is_recording {
        if recording_start_ms > 0 {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let elapsed_secs = (now_ms - recording_start_ms) / 1000;
            let mins = elapsed_secs / 60;
            let secs = elapsed_secs % 60;
            if status_text.is_empty() {
                format!("Recording {}:{:02}", mins, secs)
            } else {
                format!("{} {}:{:02}", status_text, mins, secs)
            }
        } else if status_text.is_empty() {
            "Recording...".to_string()
        } else {
            status_text.to_string()
        }
    } else if status_text.is_empty() {
        String::new()
    } else {
        status_text.to_string()
    }
}

// ===========================================================================
// Windows implementation
// ===========================================================================

#[cfg(target_os = "windows")]
use std::sync::atomic::AtomicPtr;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::*;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::*;
#[cfg(target_os = "windows")]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::*;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;

// Windows-specific color constants (BGR format for Win32 GDI)
#[cfg(target_os = "windows")]
const COLOR_BG: u32 = 0x001E1E1E; // RGB(30, 30, 30)
#[cfg(target_os = "windows")]
const COLOR_BAR_BG: u32 = 0x003C3C3C; // RGB(60, 60, 60)
#[cfg(target_os = "windows")]
const COLOR_YELLOW: u32 = 0x0000C8FF; // RGB(255, 200, 0)
#[cfg(target_os = "windows")]
const COLOR_TEXT: u32 = 0x00FFFFFF; // RGB(255, 255, 255)
#[cfg(target_os = "windows")]
const COLOR_RECORDING_DOT: u32 = 0x0050C800; // RGB(0, 200, 80)

/// Timer ID for recording duration repaints.
#[cfg(target_os = "windows")]
const TIMER_RECORDING: usize = 1;
/// Timer ID for processing spinner animation.
#[cfg(target_os = "windows")]
const TIMER_PROCESSING: usize = 2;

/// Class name for the overlay window.
#[cfg(target_os = "windows")]
const OVERLAY_CLASS_NAME: &str = "Qwen3AsrOverlay";

/// Global reference to the OverlayManager, used by WndProc.
/// We store a raw pointer (wrapped for safety) because the OverlayManager
/// is owned by AppContext on the main thread, and the WndProc needs access.
#[cfg(target_os = "windows")]
static OVERLAY_MANAGER: OnceLock<OverlayManagerRef> = OnceLock::new();

/// Wrapper for a raw pointer to OverlayManager. Only dereferenced on the
/// UI thread inside the WndProc. The OverlayManager is owned by AppContext
/// and must outlive the overlay window.
#[cfg(target_os = "windows")]
struct OverlayManagerRef {
    ptr: *const OverlayManager,
}

#[cfg(target_os = "windows")]
unsafe impl Send for OverlayManagerRef {}
#[cfg(target_os = "windows")]
unsafe impl Sync for OverlayManagerRef {}

/// Manages the floating overlay window.
#[cfg(target_os = "windows")]
pub struct OverlayManager {
    hwnd: AtomicPtr<core::ffi::c_void>,
    visible: AtomicBool,
    /// Current volume level stored as f32 bits in an AtomicI32.
    volume: AtomicI32,
    status_text: Arc<Mutex<String>>,
    is_recording: AtomicBool,
    /// Overlay position mode: "top-center" or "cursor".
    overlay_position: String,
    /// Whether the overlay is in processing state (shows spinner).
    processing: AtomicBool,
    /// Spinner frame index (0–3), advanced by WM_TIMER.
    spinner_index: AtomicI32,
    /// Whether the overlay is minimized (dot only, no VU meter).
    minimized: AtomicBool,
    /// Recording start timestamp as milliseconds since Unix epoch.
    /// 0 means not recording.
    recording_start_ms: AtomicU64,
    /// Saved overlay X position for persistence.
    saved_x: AtomicI32,
    /// Saved overlay Y position for persistence.
    saved_y: AtomicI32,
}

#[cfg(target_os = "windows")]
impl OverlayManager {
    /// Create an uninitialized overlay manager.
    ///
    /// `overlay_position` controls where the overlay appears: "top-center" for
    /// the default top-center position, or "cursor" to place it near the mouse.
    /// `overlay_x` and `overlay_y` are saved positions from a previous session
    /// (None = auto-position).
    /// `minimized` controls whether the overlay starts in minimized mode.
    pub fn new(
        overlay_position: String,
        overlay_x: Option<i32>,
        overlay_y: Option<i32>,
        minimized: bool,
    ) -> Self {
        Self {
            hwnd: AtomicPtr::new(std::ptr::null_mut()),
            visible: AtomicBool::new(false),
            volume: AtomicI32::new(0.0f32.to_bits() as i32),
            status_text: Arc::new(Mutex::new(String::new())),
            is_recording: AtomicBool::new(false),
            overlay_position,
            processing: AtomicBool::new(false),
            spinner_index: AtomicI32::new(0),
            minimized: AtomicBool::new(minimized),
            recording_start_ms: AtomicU64::new(0),
            saved_x: AtomicI32::new(overlay_x.unwrap_or(-1)),
            saved_y: AtomicI32::new(overlay_y.unwrap_or(-1)),
        }
    }

    /// Create the overlay window.
    ///
    /// Registers the window class (if not already registered) and creates
    /// a layered, top-most popup window. If saved positions exist from a
    /// previous session, those are used; otherwise the window is positioned
    /// based on the configured overlay_position mode.
    pub fn create(&self, parent_hwnd: Option<HWND>) -> Result<HWND> {
        let h_instance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };

        // Register the window class (safe to call multiple times — if already
        // registered the call simply fails, which we ignore).
        let class_name: Vec<u16> = OVERLAY_CLASS_NAME
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(overlay_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
        };

        unsafe {
            RegisterClassW(&wc);
        }

        // Determine overlay position: use saved position if available,
        // otherwise compute based on configured mode.
        let screen_cx = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_cy = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let height = self.current_height();

        let (x, y) = if self.saved_x.load(Ordering::SeqCst) >= 0
            && self.saved_y.load(Ordering::SeqCst) >= 0
        {
            // Use saved position from previous session
            let sx = self.saved_x.load(Ordering::SeqCst);
            let sy = self.saved_y.load(Ordering::SeqCst);
            // Clamp to current screen bounds
            let sx = sx.clamp(0, (screen_cx - OVERLAY_WIDTH).max(0));
            let sy = sy.clamp(0, (screen_cy - height).max(0));
            (sx, sy)
        } else if self.overlay_position == "cursor" {
            let mut point = POINT { x: 0, y: 0 };
            unsafe { let _ = GetCursorPos(&mut point); }
            let cx = point.x - (OVERLAY_WIDTH / 2);
            let cy = point.y + 20; // slightly below cursor
            // Clamp to screen bounds
            let cx = cx.clamp(0, (screen_cx - OVERLAY_WIDTH).max(0));
            let cy = cy.clamp(0, (screen_cy - height).max(0));
            (cx, cy)
        } else {
            // Default: top-center
            ((screen_cx - OVERLAY_WIDTH) / 2, 8)
        };

        // NOTE: WS_EX_TRANSPARENT is removed so the overlay is draggable.
        // WM_NCHITTEST controls which areas are interactive.
        let dw_ex_style = WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_LAYERED
            | WS_EX_NOACTIVATE;

        let hwnd = unsafe {
            CreateWindowExW(
                dw_ex_style,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                OVERLAY_WIDTH,
                height,
                parent_hwnd.unwrap_or_default(),
                None,
                h_instance,
                None,
            )?
        };

        // Make the window semi-transparent.
        unsafe {
            SetLayeredWindowAttributes(hwnd, COLORREF(0), OVERLAY_ALPHA, LWA_ALPHA)?;
        }

        self.hwnd.store(hwnd.0, Ordering::SeqCst);

        // Store a pointer to self in the global so the WndProc can access it.
        // This avoids creating a second OverlayManager instance — the WndProc
        // reads state directly from the original OverlayManager owned by AppContext.
        let self_ptr: *const OverlayManager = self as *const OverlayManager;
        let _ = OVERLAY_MANAGER.set(OverlayManagerRef { ptr: self_ptr });

        Ok(hwnd)
    }

    /// Show the overlay window without activating it.
    ///
    /// When `overlay_position` is "cursor" and no saved position exists,
    /// the window is repositioned near the current mouse cursor before
    /// being shown.
    pub fn show(&self) -> Result<()> {
        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            anyhow::bail!("Overlay window not created");
        }
        let hwnd = HWND(ptr);
        let height = self.current_height();

        // Reposition for cursor mode each time the overlay is shown
        // (only if no saved position from a drag)
        if self.overlay_position == "cursor"
            && self.saved_x.load(Ordering::SeqCst) < 0
        {
            let screen_cx = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let screen_cy = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            let mut point = POINT { x: 0, y: 0 };
            unsafe { let _ = GetCursorPos(&mut point); }
            let x = (point.x - OVERLAY_WIDTH / 2).clamp(0, (screen_cx - OVERLAY_WIDTH).max(0));
            let y = (point.y + 20).clamp(0, (screen_cy - height).max(0));
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    x,
                    y,
                    OVERLAY_WIDTH,
                    height,
                    SWP_NOACTIVATE,
                );
            }
        } else {
            // Ensure correct size (may have changed due to minimize toggle)
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    OVERLAY_WIDTH,
                    height,
                    SWP_NOMOVE | SWP_NOACTIVATE,
                );
            }
        }

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        self.visible.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Hide the overlay window.
    pub fn hide(&self) -> Result<()> {
        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            anyhow::bail!("Overlay window not created");
        }
        let hwnd = HWND(ptr);

        // Kill any active timers
        unsafe {
            let _ = KillTimer(hwnd, TIMER_RECORDING);
            let _ = KillTimer(hwnd, TIMER_PROCESSING);
        }

        // Save current window position before hiding
        let mut rect = RECT::default();
        unsafe { let _ = GetWindowRect(hwnd, &mut rect); }
        self.saved_x.store(rect.left, Ordering::SeqCst);
        self.saved_y.store(rect.top, Ordering::SeqCst);

        // Reset state
        self.processing.store(false, Ordering::SeqCst);
        self.recording_start_ms.store(0, Ordering::SeqCst);

        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        self.visible.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Update the status text and trigger a repaint.
    pub fn set_status(&self, text: &str) -> Result<()> {
        {
            let mut status = self.status_text.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            status.clear();
            status.push_str(text);
        }
        self.invalidate()?;
        Ok(())
    }

    /// Update the VU meter level (0.0–1.0) and trigger a repaint.
    pub fn set_volume(&self, level: f32) -> Result<()> {
        let clamped = level.clamp(0.0, 1.0);
        let bits = clamped.to_bits() as i32;
        self.volume.store(bits, Ordering::SeqCst);
        self.invalidate()?;
        Ok(())
    }

    /// Toggle VU meter visibility and start/stop the recording duration timer.
    pub fn set_recording(&self, is_recording: bool) -> Result<()> {
        self.is_recording.store(is_recording, Ordering::SeqCst);

        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            return Ok(());
        }
        let hwnd = HWND(ptr);

        if is_recording {
            // Start recording duration timer
            unsafe {
                let _ = SetTimer(hwnd, TIMER_RECORDING, TIMER_RECORDING_INTERVAL, None);
            }
        } else {
            // Stop recording duration timer
            unsafe {
                let _ = KillTimer(hwnd, TIMER_RECORDING);
            }
            self.recording_start_ms.store(0, Ordering::SeqCst);
        }

        self.invalidate()?;
        Ok(())
    }

    /// Set the processing state. When true, shows a spinning "Processing..."
    /// animation. When false, returns to normal display.
    pub fn set_processing(&self, is_processing: bool) -> Result<()> {
        self.processing.store(is_processing, Ordering::SeqCst);

        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            return Ok(());
        }
        let hwnd = HWND(ptr);

        if is_processing {
            self.spinner_index.store(0, Ordering::SeqCst);
            // Start spinner animation timer
            unsafe {
                let _ = SetTimer(hwnd, TIMER_PROCESSING, TIMER_PROCESSING_INTERVAL, None);
            }
        } else {
            // Stop spinner timer
            unsafe {
                let _ = KillTimer(hwnd, TIMER_PROCESSING);
            }
        }

        self.invalidate()?;
        Ok(())
    }

    /// Set the minimized state. When minimized, the overlay shows only
    /// the recording dot and status text without the VU meter bar.
    pub fn set_minimized(&self, minimized: bool) -> Result<()> {
        let was_minimized = self.minimized.swap(minimized, Ordering::SeqCst);
        if was_minimized == minimized {
            return Ok(());
        }

        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            return Ok(());
        }
        let hwnd = HWND(ptr);
        let height = self.current_height();

        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                OVERLAY_WIDTH,
                height,
                SWP_NOMOVE | SWP_NOACTIVATE,
            );
        }

        self.invalidate()?;
        Ok(())
    }

    /// Set the recording start timestamp for the duration display.
    /// `timestamp_ms` is milliseconds since Unix epoch (0 = not recording).
    pub fn set_recording_start(&self, timestamp_ms: u64) -> Result<()> {
        self.recording_start_ms.store(timestamp_ms, Ordering::SeqCst);
        Ok(())
    }

    /// Return the current overlay position for config persistence.
    /// Returns (-1, -1) if the window has never been positioned.
    pub fn save_position(&self) -> (i32, i32) {
        let x = self.saved_x.load(Ordering::SeqCst);
        let y = self.saved_y.load(Ordering::SeqCst);
        (x, y)
    }

    /// Destroy the overlay window.
    pub fn destroy(&self) -> Result<()> {
        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            return Ok(());
        }
        let hwnd = HWND(ptr);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        self.hwnd.store(std::ptr::null_mut(), Ordering::SeqCst);
        self.visible.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Trigger a repaint of the overlay window.
    fn invalidate(&self) -> Result<()> {
        let ptr = self.hwnd.load(Ordering::SeqCst);
        if ptr.is_null() {
            return Ok(());
        }
        let hwnd = HWND(ptr);
        unsafe {
            let _ = InvalidateRect(hwnd, None, true);
        }
        Ok(())
    }

    /// Get current overlay height based on minimized state.
    fn current_height(&self) -> i32 {
        if self.minimized.load(Ordering::SeqCst) {
            OVERLAY_HEIGHT_MINIMIZED
        } else {
            OVERLAY_HEIGHT
        }
    }
}

/// Window procedure for the overlay window.
#[cfg(target_os = "windows")]
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => paint_overlay(hwnd),

        WM_NCHITTEST => {
            // Allow dragging by treating the title area as a caption.
            // The VU meter area (below title area) is transparent so it
            // doesn't interfere with other windows.
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            // Convert screen coordinates to client coordinates
            let mut point = POINT { x, y };
            unsafe { let _ = ScreenToClient(hwnd, &mut point); }

            let is_minimized = if let Some(mgr_ref) = OVERLAY_MANAGER.get() {
                let mgr = unsafe { &*mgr_ref.ptr };
                mgr.minimized.load(Ordering::SeqCst)
            } else {
                false
            };

            // In minimized mode, the entire window is the title area (draggable).
            // In normal mode, the top portion is draggable, the VU meter area is not.
            if is_minimized || point.y < TITLE_AREA_HEIGHT {
                LRESULT(HTCAPTION as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }

        WM_LBUTTONDBLCLK => {
            // Double-click toggles minimized mode
            if let Some(mgr_ref) = OVERLAY_MANAGER.get() {
                let mgr = unsafe { &*mgr_ref.ptr };
                let current = mgr.minimized.load(Ordering::SeqCst);
                let _ = mgr.set_minimized(!current);
            }
            LRESULT(0)
        }

        WM_MOVE => {
            // Save position on move
            if let Some(mgr_ref) = OVERLAY_MANAGER.get() {
                let mgr = unsafe { &*mgr_ref.ptr };
                let mut rect = RECT::default();
                unsafe { let _ = GetWindowRect(hwnd, &mut rect); }
                mgr.saved_x.store(rect.left, Ordering::SeqCst);
                mgr.saved_y.store(rect.top, Ordering::SeqCst);
            }
            LRESULT(0)
        }

        WM_TIMER => {
            let timer_id = wparam.0;
            if timer_id == TIMER_RECORDING {
                // Repaint to update recording duration display
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
            } else if timer_id == TIMER_PROCESSING {
                // Advance spinner frame
                if let Some(mgr_ref) = OVERLAY_MANAGER.get() {
                    let mgr = unsafe { &*mgr_ref.ptr };
                    let idx = mgr.spinner_index.fetch_add(1, Ordering::SeqCst);
                    mgr.spinner_index.store(idx % SPINNER_CHARS.len() as i32, Ordering::SeqCst);
                }
                unsafe { let _ = InvalidateRect(hwnd, None, false); }
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Handle WM_PAINT — draw the overlay content.
#[cfg(target_os = "windows")]
unsafe fn paint_overlay(hwnd: HWND) -> LRESULT {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };

    // Get client area for background fill
    let mut client_rect = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client_rect); }

    // Fill background
    let bg_brush = unsafe { CreateSolidBrush(COLORREF(COLOR_BG)) };
    unsafe { FillRect(hdc, &client_rect, bg_brush); }
    unsafe { let _ = DeleteObject(bg_brush); }

    // Read state from the global manager reference
    let (status_text, is_recording, volume, is_processing, spinner_idx, is_minimized, recording_start) =
        if let Some(mgr_ref) = OVERLAY_MANAGER.get() {
            let mgr = unsafe { &*mgr_ref.ptr };
            let text = mgr
                .status_text
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            let recording = mgr.is_recording.load(Ordering::SeqCst);
            let vol = f32::from_bits(mgr.volume.load(Ordering::SeqCst) as u32);
            let processing = mgr.processing.load(Ordering::SeqCst);
            let spin = mgr.spinner_index.load(Ordering::SeqCst);
            let minimized = mgr.minimized.load(Ordering::SeqCst);
            let start_ms = mgr.recording_start_ms.load(Ordering::SeqCst);
            (text, recording, vol, processing, spin, minimized, start_ms)
        } else {
            (String::new(), false, 0.0, false, 0, false, 0)
        };

    // Set up text rendering
    unsafe { SetBkMode(hdc, TRANSPARENT); }
    unsafe { SetTextColor(hdc, COLORREF(COLOR_TEXT)); }

    // Select a reasonable font (DEFAULT_GUI_FONT)
    let h_font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let old_font = unsafe { SelectObject(hdc, h_font) };

    // Build the display text
    let display_text = build_display_text(
        &status_text, is_recording, is_processing, spinner_idx, recording_start,
    );

    let text_x = if is_recording || is_processing { 28 } else { 12 };

    // Draw the dot when recording or processing
    if is_recording || is_processing {
        let dot_color = if is_processing {
            COLOR_YELLOW // Yellow dot for processing
        } else {
            COLOR_RECORDING_DOT // Green dot for recording
        };
        let dot_brush = unsafe { CreateSolidBrush(COLORREF(dot_color)) };
        let null_pen = unsafe { GetStockObject(NULL_PEN) };
        let old_pen = unsafe { SelectObject(hdc, null_pen) };
        let old_brush = unsafe { SelectObject(hdc, dot_brush) };
        // Dot position: slightly higher in minimized mode
        let dot_top = if is_minimized { 8 } else { 10 };
        let dot_bottom = dot_top + 12;
        unsafe { let _ = Ellipse(hdc, 10, dot_top, 22, dot_bottom); }
        unsafe { SelectObject(hdc, old_brush); }
        unsafe { SelectObject(hdc, old_pen); }
        unsafe { let _ = DeleteObject(dot_brush); }
    }

    // Draw status text
    if !display_text.is_empty() {
        let mut text_wide: Vec<u16> = display_text.encode_utf16().chain(std::iter::once(0u16)).collect();
        let text_top = if is_minimized { 4 } else { 6 };
        let text_bottom = if is_minimized { OVERLAY_HEIGHT_MINIMIZED - 4 } else { 30 };
        let mut text_rect = RECT {
            left: text_x,
            top: text_top,
            right: OVERLAY_WIDTH - 10,
            bottom: text_bottom,
        };
        unsafe {
            DrawTextW(
                hdc,
                &mut text_wide,
                &mut text_rect,
                DT_SINGLELINE | DT_VCENTER,
            );
        }
    }

    // Draw VU meter bar if recording and not minimized
    if is_recording && !is_minimized {
        // Bar background
        let bar_bg_brush = unsafe { CreateSolidBrush(COLORREF(COLOR_BAR_BG)) };
        let bar_bg_rect = RECT {
            left: BAR_X,
            top: BAR_Y,
            right: BAR_X + BAR_WIDTH,
            bottom: BAR_Y + BAR_HEIGHT,
        };
        unsafe { FillRect(hdc, &bar_bg_rect, bar_bg_brush); }
        unsafe { let _ = DeleteObject(bar_bg_brush); }

        // Bar fill based on volume level
        let fill_width = (volume * BAR_WIDTH as f32) as i32;
        if fill_width > 0 {
            let (r, g, b) = vu_meter_color(volume);
            // Convert f64 RGB back to BGR u32 for Win32
            let fill_color = ((b * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (r * 255.0) as u32;
            let fill_brush = unsafe { CreateSolidBrush(COLORREF(fill_color)) };
            let fill_rect = RECT {
                left: BAR_X,
                top: BAR_Y,
                right: BAR_X + fill_width,
                bottom: BAR_Y + BAR_HEIGHT,
            };
            unsafe { FillRect(hdc, &fill_rect, fill_brush); }
            unsafe { let _ = DeleteObject(fill_brush); }
        }
    }

    // Restore original font
    unsafe { SelectObject(hdc, old_font); }

    unsafe { let _ = EndPaint(hwnd, &ps); };

    LRESULT(0)
}

// ===========================================================================
// Linux implementation (GTK4)
// ===========================================================================

#[cfg(target_os = "linux")]
use gtk4::glib;
#[cfg(target_os = "linux")]
use gtk4::prelude::*;
#[cfg(target_os = "linux")]
use std::cell::RefCell;

#[cfg(target_os = "linux")]
struct OverlayState {
    window: Arc<RefCell<Option<gtk4::Window>>>,
    visible: Arc<AtomicBool>,
    volume: Arc<AtomicI32>,
    status_text: Arc<Mutex<String>>,
    is_recording: Arc<AtomicBool>,
    processing: Arc<AtomicBool>,
    spinner_index: Arc<AtomicI32>,
    minimized: Arc<AtomicBool>,
    recording_start_ms: Arc<AtomicU64>,
    saved_x: Arc<AtomicI32>,
    saved_y: Arc<AtomicI32>,
    recording_timer_id: Arc<RefCell<Option<glib::SourceId>>>,
    processing_timer_id: Arc<RefCell<Option<glib::SourceId>>>,
}

#[cfg(target_os = "linux")]
pub struct OverlayManager {
    state: OverlayState,
}

#[cfg(target_os = "linux")]
impl OverlayManager {
    pub fn new(
        _overlay_position: String,
        overlay_x: Option<i32>,
        overlay_y: Option<i32>,
        minimized: bool,
    ) -> Self {
        Self {
            state: OverlayState {
                window: Arc::new(RefCell::new(None)),
                visible: Arc::new(AtomicBool::new(false)),
                volume: Arc::new(AtomicI32::new(0.0f32.to_bits() as i32)),
                status_text: Arc::new(Mutex::new(String::new())),
                is_recording: Arc::new(AtomicBool::new(false)),
                processing: Arc::new(AtomicBool::new(false)),
                spinner_index: Arc::new(AtomicI32::new(0)),
                minimized: Arc::new(AtomicBool::new(minimized)),
                recording_start_ms: Arc::new(AtomicU64::new(0)),
                saved_x: Arc::new(AtomicI32::new(overlay_x.unwrap_or(-1))),
                saved_y: Arc::new(AtomicI32::new(overlay_y.unwrap_or(-1))),
                recording_timer_id: Arc::new(RefCell::new(None)),
                processing_timer_id: Arc::new(RefCell::new(None)),
            },
        }
    }

    fn ensure_window(&self) -> Option<gtk4::Window> {
        if let Some(win) = self.state.window.borrow().clone() {
            return Some(win);
        }

        let height = self.current_height();

        let win = gtk4::Window::builder()
            .decorated(false)
            .resizable(false)
            .opacity(OVERLAY_ALPHA as f64 / 255.0)
            .focus_on_click(false)
            .can_focus(false)
            .hide_on_close(true)
            .default_width(OVERLAY_WIDTH)
            .default_height(height)
            .build();

        let css = gtk4::CssProvider::new();
        css.load_from_data(
            ".overlay-window { background-color: rgb(30,30,30); border-radius: 8px; }"
        );
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        win.add_css_class("overlay-window");

        let drawing_area = gtk4::DrawingArea::new();
        drawing_area.set_draw_func({
            let st = self.state.status_text.clone();
            let vol = self.state.volume.clone();
            let rec = self.state.is_recording.clone();
            let proc = self.state.processing.clone();
            let spin = self.state.spinner_index.clone();
            let min = self.state.minimized.clone();
            let start_ms = self.state.recording_start_ms.clone();

            move |_area, cr, _width, _height| {
                draw_overlay(cr, &st, &vol, &rec, &proc, &spin, &min, &start_ms);
            }
        });
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);

        win.set_child(Some(&drawing_area));

        // Drag support: use GestureDrag to initiate Toplevel::begin_move
        let drag = gtk4::GestureDrag::new();
        let win_ref = self.state.window.clone();

        drag.connect_drag_begin({
            let w = win_ref.clone();
            move |gesture, _start_x, _start_y| {
                if let Some(win) = w.borrow().as_ref() {
                    if let Some(surface) = win.surface() {
                        if let Some(toplevel) = surface.downcast_ref::<gtk4::gdk::Toplevel>() {
                            if let Some(display) = gtk4::gdk::Display::default() {
                                if let Some(seat) = display.default_seat() {
                                    if let Some(device) = seat.pointer() {
                                        let event = gesture.last_event(None::<&gtk4::gdk::EventSequence>);
                                        let timestamp = event
                                            .as_ref()
                                            .map(|e| e.time())
                                            .unwrap_or(0);
                                        toplevel.begin_move(
                                            &device,
                                            1,
                                            0.0,
                                            0.0,
                                            timestamp,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            }
        });

        let sx = self.state.saved_x.clone();
        let sy = self.state.saved_y.clone();
        win.connect_close_request({
            let sx = sx.clone();
            let sy = sy.clone();
            move |win: &gtk4::Window| {
                if let Some(surface) = win.surface() {
                    let w = surface.width();
                    let h = surface.height();
                    let _ = (w, h);
                }
                sx.store(-1, Ordering::SeqCst);
                sy.store(-1, Ordering::SeqCst);
                glib::Propagation::Proceed
            }
        });

        let dbl_click = gtk4::GestureClick::new();
        dbl_click.set_button(gtk4::gdk::BUTTON_PRIMARY);
        dbl_click.connect_pressed({
            let min_clone = self.state.minimized.clone();
            let win_clone = self.state.window.clone();
            move |_gesture, n_press, _x, _y| {
                if n_press == 2 {
                    let current = min_clone.load(Ordering::SeqCst);
                    min_clone.store(!current, Ordering::SeqCst);
                    let h = if !current { OVERLAY_HEIGHT_MINIMIZED } else { OVERLAY_HEIGHT };
                    if let Some(w) = win_clone.borrow().as_ref() {
                        w.set_default_size(OVERLAY_WIDTH, h);
                        w.queue_draw();
                    }
                }
            }
        });

        drawing_area.add_controller(drag);
        drawing_area.add_controller(dbl_click);

        *self.state.window.borrow_mut() = Some(win.clone());
        Some(win)
    }

    pub fn show(&self) -> Result<()> {
        let win = match self.ensure_window() {
            Some(w) => w,
            None => anyhow::bail!("Failed to create overlay window"),
        };

        let height = self.current_height();
        win.set_default_size(OVERLAY_WIDTH, height);

        win.show();
        self.state.visible.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn hide(&self) -> Result<()> {
        if let Some(id) = self.state.recording_timer_id.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.state.processing_timer_id.borrow_mut().take() {
            id.remove();
        }

        if let Some(win) = self.state.window.borrow().as_ref() {
            win.hide();
        }

        self.state.processing.store(false, Ordering::SeqCst);
        self.state.recording_start_ms.store(0, Ordering::SeqCst);
        self.state.visible.store(false, Ordering::SeqCst);
        Ok(())
    }

    pub fn set_status(&self, text: &str) -> Result<()> {
        {
            let mut status = self.state.status_text.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
            status.clear();
            status.push_str(text);
        }
        self.invalidate()
    }

    pub fn set_volume(&self, level: f32) -> Result<()> {
        let clamped = level.clamp(0.0, 1.0);
        self.state.volume.store(clamped.to_bits() as i32, Ordering::SeqCst);
        self.invalidate()
    }

    pub fn set_recording(&self, is_recording: bool) -> Result<()> {
        self.state.is_recording.store(is_recording, Ordering::SeqCst);

        if is_recording {
            let window = self.state.window.clone();
            let timer_id = glib::timeout_add_local(std::time::Duration::from_millis(
                TIMER_RECORDING_INTERVAL as u64,
            ), {
                let win = window.clone();
                move || {
                    if let Some(w) = win.borrow().as_ref() {
                        w.queue_draw();
                    }
                    glib::ControlFlow::Continue
                }
            });
            *self.state.recording_timer_id.borrow_mut() = Some(timer_id);
        } else {
            if let Some(id) = self.state.recording_timer_id.borrow_mut().take() {
                id.remove();
            }
            self.state.recording_start_ms.store(0, Ordering::SeqCst);
        }

        self.invalidate()
    }

    pub fn set_processing(&self, is_processing: bool) -> Result<()> {
        self.state.processing.store(is_processing, Ordering::SeqCst);

        if is_processing {
            self.state.spinner_index.store(0, Ordering::SeqCst);
            let spinner_index = self.state.spinner_index.clone();
            let window = self.state.window.clone();
            let timer_id = glib::timeout_add_local(std::time::Duration::from_millis(
                TIMER_PROCESSING_INTERVAL as u64,
            ), {
                let spin = spinner_index.clone();
                let win = window.clone();
                move || {
                    let idx = spin.fetch_add(1, Ordering::SeqCst);
                    spin.store(idx % SPINNER_CHARS.len() as i32, Ordering::SeqCst);
                    if let Some(w) = win.borrow().as_ref() {
                        w.queue_draw();
                    }
                    glib::ControlFlow::Continue
                }
            });
            *self.state.processing_timer_id.borrow_mut() = Some(timer_id);
        } else {
            if let Some(id) = self.state.processing_timer_id.borrow_mut().take() {
                id.remove();
            }
        }

        self.invalidate()
    }

    pub fn set_recording_start(&self, timestamp_ms: u64) -> Result<()> {
        self.state.recording_start_ms.store(timestamp_ms, Ordering::SeqCst);
        Ok(())
    }

    pub fn save_position(&self) -> (i32, i32) {
        (self.state.saved_x.load(Ordering::SeqCst), self.state.saved_y.load(Ordering::SeqCst))
    }

    pub fn destroy(&self) -> Result<()> {
        if let Some(id) = self.state.recording_timer_id.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.state.processing_timer_id.borrow_mut().take() {
            id.remove();
        }

        if let Some(win) = self.state.window.borrow_mut().take() {
            win.close();
        }
        self.state.visible.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn invalidate(&self) -> Result<()> {
        if let Some(win) = self.state.window.borrow().as_ref() {
            win.queue_draw();
        }
        Ok(())
    }

    fn current_height(&self) -> i32 {
        if self.state.minimized.load(Ordering::SeqCst) {
            OVERLAY_HEIGHT_MINIMIZED
        } else {
            OVERLAY_HEIGHT
        }
    }
}

#[cfg(target_os = "linux")]
fn draw_overlay(
    cr: &gtk4::cairo::Context,
    status_text: &Arc<Mutex<String>>,
    volume: &Arc<AtomicI32>,
    is_recording: &Arc<AtomicBool>,
    processing: &Arc<AtomicBool>,
    spinner_index: &Arc<AtomicI32>,
    minimized: &Arc<AtomicBool>,
    recording_start_ms: &Arc<AtomicU64>,
) {
    let text = status_text.lock().map(|g| g.clone()).unwrap_or_default();
    let rec = is_recording.load(Ordering::SeqCst);
    let vol = f32::from_bits(volume.load(Ordering::SeqCst) as u32);
    let proc = processing.load(Ordering::SeqCst);
    let spin = spinner_index.load(Ordering::SeqCst);
    let min = minimized.load(Ordering::SeqCst);
    let start_ms = recording_start_ms.load(Ordering::SeqCst);

    let height = if min { OVERLAY_HEIGHT_MINIMIZED } else { OVERLAY_HEIGHT };

    cr.set_source_rgb(COLOR_BG_R, COLOR_BG_G, COLOR_BG_B);
    cr.rectangle(0.0, 0.0, OVERLAY_WIDTH as f64, height as f64);
    let _ = cr.fill();

    let display_text = build_display_text(&text, rec, proc, spin, start_ms);
    let text_x = if rec || proc { 28.0 } else { 12.0 };

    if rec || proc {
        let (dot_r, dot_g, dot_b) = if proc {
            (COLOR_YELLOW_R, COLOR_YELLOW_G, COLOR_YELLOW_B)
        } else {
            (COLOR_GREEN_R, COLOR_GREEN_G, COLOR_GREEN_B)
        };
        cr.set_source_rgb(dot_r, dot_g, dot_b);
        let dot_cy = if min { 15.0 } else { 16.0 };
        cr.arc(16.0, dot_cy, 6.0, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.fill();
    }

    if !display_text.is_empty() {
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let font_size = 13.0;
        cr.set_font_size(font_size);
        cr.select_font_face(
            "sans-serif",
            gtk4::cairo::FontSlant::Normal,
            gtk4::cairo::FontWeight::Normal,
        );
        let title_h = if min { OVERLAY_HEIGHT_MINIMIZED as f64 } else { TITLE_AREA_HEIGHT as f64 };
        let text_y = title_h / 2.0 + font_size / 2.0 - 2.0;
        let _ = cr.move_to(text_x, text_y);
        let _ = cr.show_text(&display_text);
    }

    if rec && !min {
        cr.set_source_rgb(COLOR_BAR_BG_R, COLOR_BAR_BG_G, COLOR_BAR_BG_B);
        cr.rectangle(BAR_X as f64, BAR_Y as f64, BAR_WIDTH as f64, BAR_HEIGHT as f64);
        let _ = cr.fill();

        let fill_width = (vol * BAR_WIDTH as f32) as f64;
        if fill_width > 0.0 {
            let (fr, fg, fb) = vu_meter_color(vol);
            cr.set_source_rgb(fr, fg, fb);
            cr.rectangle(BAR_X as f64, BAR_Y as f64, fill_width, BAR_HEIGHT as f64);
            let _ = cr.fill();
        }
    }
}
