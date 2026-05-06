//! System tray icon using Windows Shell_NotifyIconW API.
//!
//! Provides a system tray icon with a context menu for:
//! - Opening the main window
//! - Toggling recording mode (PTT / Hands-free)
//! - Opening history
//! - Opening settings
//! - Quitting the application

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// --- Constants ---

/// Custom message sent by the tray icon callback.
const WM_TRAYICON: u32 = WM_APP + 1;

/// Menu item IDs.
const IDM_TOGGLE_MODE: usize = 1001;
const IDM_SHOW_HISTORY: usize = 1002;
const IDM_SHOW_SETTINGS: usize = 1003;
const IDM_ABOUT: usize = 1004;
const IDM_QUIT: usize = 1005;
const IDM_OPEN: usize = 1006;

// --- Types ---

/// Callback for tray menu actions.
pub(crate) type TrayCallback = Box<dyn Fn(TrayAction) + Send + Sync>;

/// Actions that can be triggered from the tray menu.
#[derive(Debug, Clone, PartialEq)]
pub enum TrayAction {
    ToggleMode,
    ShowMainWindow,
    ShowHistory,
    ShowSettings,
    About,
    Quit,
}

/// Tray icon visual state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayState {
    Idle,
    Recording,
    Processing,
    Disconnected,
}

/// Manages the system tray icon and its context menu.
///
/// Note: `TrayManager` is not `Send` because `NOTIFYICONDATAW` contains
/// `HWND` (a raw pointer). It is stored as a leaked `Box` behind a raw
/// pointer inside the global `TRAY_MANAGER` and only accessed from the
/// UI thread inside the window procedure.
pub struct TrayManager {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
    callback: std::sync::Arc<Mutex<Option<TrayCallback>>>,
    visible: AtomicBool,
    is_handsfree: AtomicBool,
    current_state: TrayState,
    idle_icon: HICON,
    recording_icon: HICON,
    processing_icon: HICON,
    disconnected_icon: HICON,
}

impl TrayManager {
    /// Initialize tray icon data. The `hwnd` is the message window that
    /// receives tray notifications.
    pub fn new(hwnd: HWND) -> Result<Self> {
        let idle_icon = unsafe { LoadIconW(None, IDI_APPLICATION)? };

        // Create colored 16x16 icons for different states
        let recording_icon = create_color_icon(0, 200, 80);    // green
        let processing_icon = create_color_icon(66, 133, 244);  // blue
        let disconnected_icon = create_color_icon(255, 193, 7); // yellow

        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;

        // Default icon
        nid.hIcon = idle_icon;

        // Default tooltip
        let tooltip = encode_wide("Qwen3-ASR Typeless");
        let copy_len = tooltip.len().min(128);
        nid.szTip[..copy_len].copy_from_slice(&tooltip[..copy_len]);

        Ok(Self {
            hwnd,
            nid,
            callback: std::sync::Arc::new(Mutex::new(None)),
            visible: AtomicBool::new(false),
            is_handsfree: AtomicBool::new(false),
            current_state: TrayState::Idle,
            idle_icon,
            recording_icon,
            processing_icon,
            disconnected_icon,
        })
    }

    /// Add the tray icon (NIM_ADD).
    pub fn show(&mut self) -> Result<()> {
        if self.visible.load(Ordering::SeqCst) {
            return Ok(());
        }
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &self.nid) };
        if !ok.as_bool() {
            return Err(anyhow::anyhow!("Shell_NotifyIconW NIM_ADD failed"));
        }
        self.visible.store(true, Ordering::SeqCst);
        Ok(())
    }



    /// Show a balloon notification from the tray icon.
    pub fn show_balloon(&mut self, title: &str, message: &str) -> Result<()> {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = self.nid.uID;
        nid.uFlags = NIF_INFO;

        let title_wide = encode_wide(title);
        let msg_wide = encode_wide(message);
        let title_len = title_wide.len().min(64);
        let msg_len = msg_wide.len().min(256);
        nid.szInfoTitle[..title_len].copy_from_slice(&title_wide[..title_len]);
        nid.szInfo[..msg_len].copy_from_slice(&msg_wide[..msg_len]);

        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        if !ok.as_bool() {
            return Err(anyhow::anyhow!("Shell_NotifyIconW balloon failed"));
        }
        Ok(())
    }

    /// Update tooltip text (NIM_MODIFY).
    pub fn set_tooltip(&mut self, text: &str) -> Result<()> {
        let tooltip = encode_wide(text);
        let copy_len = tooltip.len().min(128);
        self.nid.szTip = [0u16; 128];
        self.nid.szTip[..copy_len].copy_from_slice(&tooltip[..copy_len]);
        if self.visible.load(Ordering::SeqCst) {
            let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.nid) };
            if !ok.as_bool() {
                return Err(anyhow::anyhow!("Shell_NotifyIconW NIM_MODIFY failed"));
            }
        }
        Ok(())
    }

    /// Create and show a popup context menu at the cursor position.
    pub fn show_context_menu(&self) -> Result<()> {
        unsafe {
            let h_menu = CreatePopupMenu()?;

            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_OPEN,
                PCWSTR(encode_wide_null("Open").as_ptr()),
            )?;

            let mode_text = if self.is_handsfree.load(Ordering::SeqCst) {
                "Mode: Hands-free"
            } else {
                "Mode: Push-to-Talk"
            };

            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_TOGGLE_MODE,
                PCWSTR(encode_wide_null(mode_text).as_ptr()),
            )?;
            AppendMenuW(h_menu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_SHOW_HISTORY,
                PCWSTR(encode_wide_null("History").as_ptr()),
            )?;
            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_SHOW_SETTINGS,
                PCWSTR(encode_wide_null("Settings").as_ptr()),
            )?;
            AppendMenuW(h_menu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_ABOUT,
                PCWSTR(encode_wide_null("About").as_ptr()),
            )?;
            AppendMenuW(
                h_menu,
                MF_STRING,
                IDM_QUIT,
                PCWSTR(encode_wide_null("Quit").as_ptr()),
            )?;

            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point)?;

            // Required for the menu to dismiss properly
            let _ = SetForegroundWindow(self.hwnd);

            let _ = TrackPopupMenu(
                h_menu,
                TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                self.hwnd,
                None,
            );

            DestroyMenu(h_menu)?;
        }
        Ok(())
    }

    /// Set the action callback.
    pub fn set_callback(&mut self, callback: TrayCallback) {
        if let Ok(mut cb) = self.callback.lock() {
            *cb = Some(callback);
        }
    }

    /// Handle tray notification messages.
    ///
    /// - When `lparam == WM_RBUTTONUP`, show context menu.
    /// - When `lparam == WM_LBUTTONDBLCLK`, open main window.
    /// - When `lparam == WM_LBUTTONUP`, toggle mode.
    /// - When `msg` comes from `WM_COMMAND` (menu item selected), invoke callback.
    pub fn handle_message(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        if msg == WM_TRAYICON {
            let event = lparam.0 as u32;
            match event {
                WM_RBUTTONUP => {
                    if let Err(e) = self.show_context_menu() {
                        log::error!("Failed to show tray context menu: {}", e);
                    }
                }
                WM_LBUTTONDBLCLK => {
                    self.invoke_callback(TrayAction::ShowMainWindow);
                }
                WM_LBUTTONUP => {
                    self.invoke_callback(TrayAction::ToggleMode);
                }
                _ => {}
            }
        } else if msg == WM_COMMAND {
            let menu_id = loword(wparam.0 as u32);
            let action = match menu_id {
                1001 => Some(TrayAction::ToggleMode),
                1002 => Some(TrayAction::ShowHistory),
                1003 => Some(TrayAction::ShowSettings),
                1004 => Some(TrayAction::About),
                1005 => Some(TrayAction::Quit),
                1006 => Some(TrayAction::ShowMainWindow),
                _ => None,
            };
            if let Some(action) = action {
                self.invoke_callback(action);
            }
        }
    }

    /// Update tooltip to show current mode.
    pub fn update_mode_display(&mut self, is_handsfree: bool) -> Result<()> {
        self.is_handsfree.store(is_handsfree, Ordering::SeqCst);
        let tooltip = if is_handsfree {
            "Qwen3-ASR Typeless [Hands-free]"
        } else {
            "Qwen3-ASR Typeless [Push-to-Talk]"
        };
        self.set_tooltip(tooltip)
    }

    fn invoke_callback(&self, action: TrayAction) {
        if let Ok(cb) = self.callback.lock() {
            if let Some(ref callback) = *cb {
                callback(action);
            }
        }
    }

    /// Set the tray icon state, changing the icon color.
    pub fn set_state(&mut self, state: TrayState) -> Result<()> {
        if state == self.current_state {
            return Ok(());
        }
        self.current_state = state;
        let icon = match state {
            TrayState::Idle => self.idle_icon,
            TrayState::Recording => self.recording_icon,
            TrayState::Processing => self.processing_icon,
            TrayState::Disconnected => self.disconnected_icon,
        };
        self.nid.hIcon = icon;
        if self.visible.load(Ordering::SeqCst) {
            let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.nid) };
            if !ok.as_bool() {
                return Err(anyhow::anyhow!("Shell_NotifyIconW NIM_MODIFY (set_state) failed"));
            }
        }
        Ok(())
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        if self.visible.load(Ordering::SeqCst) {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            }
        }
    }
}

// --- Global tray manager for window procedure ---

/// Thread-safe wrapper for the global TrayManager.
/// TrayManager is not Send (NOTIFYICONDATAW contains HWND), so we use
/// a raw pointer. The pointer is only ever dereferenced on the UI thread
/// inside the window procedure.
struct GlobalTray {
    ptr: *mut TrayManager,
}

unsafe impl Send for GlobalTray {}
unsafe impl Sync for GlobalTray {}

/// Global reference to the TrayManager, used by the window procedure.
static TRAY_MANAGER: OnceLock<Mutex<Option<GlobalTray>>> = OnceLock::new();

/// Sets the global tray manager reference. Call once after creating the TrayManager.
///
/// The TrayManager is boxed and leaked via `Box::into_raw`; the pointer remains
/// valid until the application exits. This is intentional — the tray icon should
/// persist for the entire lifetime of the application, and the OS will reclaim
/// the memory on process exit. The `Drop` impl handles removing the tray icon.
pub fn set_global_tray(tray: Box<TrayManager>) {
    let ptr = Box::into_raw(tray);
    // SAFETY: The leaked Box is intentionally never freed. The TrayManager's
    // Drop impl removes the tray icon (NIM_DELETE), and the pointer is only
    // dereferenced on the UI thread inside the window procedure.
    let _ = TRAY_MANAGER.get_or_init(|| Mutex::new(Some(GlobalTray { ptr })));
}

/// Update the mode display on the global tray icon.
/// Must only be called from the UI thread.
pub fn update_global_mode_display(is_handsfree: bool) {
    if let Some(global) = TRAY_MANAGER.get() {
        if let Ok(mut guard) = global.lock() {
            if let Some(ref mut g) = *guard {
                let tray = unsafe { &mut *g.ptr };
                let _ = tray.update_mode_display(is_handsfree);
            }
        }
    }
}

/// Show a balloon notification from the global tray icon.
/// Must only be called from the UI thread.
pub fn show_global_balloon(title: &str, message: &str) {
    if let Some(global) = TRAY_MANAGER.get() {
        if let Ok(mut guard) = global.lock() {
            if let Some(ref mut g) = *guard {
                let tray = unsafe { &mut *g.ptr };
                if let Err(e) = tray.show_balloon(title, message) {
                    log::warn!("Failed to show tray balloon: {}", e);
                }
            }
        }
    }
}

/// Set the global tray icon state (changes icon color).
/// Must only be called from the UI thread.
pub fn set_global_state(state: TrayState) {
    if let Some(global) = TRAY_MANAGER.get() {
        if let Ok(mut guard) = global.lock() {
            if let Some(ref mut g) = *guard {
                let tray = unsafe { &mut *g.ptr };
                if let Err(e) = tray.set_state(state) {
                    log::warn!("Failed to set tray state: {}", e);
                }
            }
        }
    }
}

/// Creates a hidden window for receiving tray messages.
pub fn create_tray_window() -> Result<HWND> {
    unsafe {
        let class_name = encode_wide_null("Qwen3ASRTypelessTrayClass");

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: GetModuleHandleW(None)?.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..std::mem::zeroed()
        };

        let atom = RegisterClassW(&wnd_class);
        if atom == 0 {
            return Err(anyhow::anyhow!(
                "RegisterClassW failed: {}",
                GetLastError().0
            ));
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(encode_wide_null("Qwen3-ASR Typeless Tray").as_ptr()),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            wnd_class.hInstance,
            None,
        )?;

        Ok(hwnd)
    }
}

/// Window procedure for the hidden tray message window.
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON | WM_COMMAND => {
            if let Some(global) = TRAY_MANAGER.get() {
                if let Ok(guard) = global.lock() {
                    if let Some(ref g) = *guard {
                        let tray = &*g.ptr;
                        tray.handle_message(msg, wparam, lparam);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// --- Utility ---

/// Create a 16x16 solid-color icon using GDI.
fn create_color_icon(r: u8, g: u8, b: u8) -> HICON {
    const SIZE: i32 = 16;
    unsafe {
        let hdc_screen = GetDC(None);
        let hdc = CreateCompatibleDC(hdc_screen);
        let _ = ReleaseDC(None, hdc_screen);

        // COLORREF is 0x00BBGGRR
        let color = COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16));
        let hbm_color = CreateCompatibleBitmap(hdc_screen, SIZE, SIZE);
        SelectObject(hdc, hbm_color);

        let brush = CreateSolidBrush(color);
        let rect = RECT { left: 0, top: 0, right: SIZE, bottom: SIZE };
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush);
        let _ = DeleteObject(hbm_color);

        // Monochrome AND mask (all zeros = fully opaque)
        let hbm_mask = CreateBitmap(SIZE, SIZE, 1, 1, None);

        let icon_info = ICONINFO {
            fIcon: BOOL::from(true),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let icon = CreateIconIndirect(&icon_info).unwrap_or_default();
        let _ = DeleteObject(hbm_mask);
        let _ = DeleteDC(hdc);
        icon
    }
}

/// Encode a Rust string to a wide (UTF-16) vector without null terminator.
fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Encode a Rust string to a wide (UTF-16) vector with null terminator.
fn encode_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Extract the low-order word from a WPARAM value.
fn loword(wparam: u32) -> u32 {
    wparam & 0xFFFF
}
