//! Settings dialog for qwen3-asr-typeless.
//!
//! Uses raw Win32 API (CreateWindowExW) to create a modal dialog
//! for editing application configuration.

use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR};

use crate::config::AppConfig;
use crate::dictionary::DictionaryManager;
use crate::i18n::I18n;

/// Extract the low-order word from a u32 value.
fn loword(v: u32) -> u16 {
    (v & 0xFFFF) as u16
}

/// Extract the high-order word from a u32 value.
fn hiword(v: u32) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

// Control IDs
const IDC_ASRLABEL: usize = 1001;
const IDC_ASRSURL: usize = 1002;
const IDC_APIKEYLABEL: usize = 1003;
const IDC_APIKEY: usize = 1004;
const IDC_MODELABEL: usize = 1005;
const IDC_MODECOMBO: usize = 1006;
const IDC_VADLABEL: usize = 1007;
const IDC_VADTHRESHOLD: usize = 1008;
const IDC_SILENCELABEL: usize = 1009;
const IDC_SILENCEDUR: usize = 1010;
const IDC_PTTKEYLABEL: usize = 1011;
const IDC_PTTKEY: usize = 1012;
const IDC_HFKEYLABEL: usize = 1013;
const IDC_HFKEY: usize = 1014;
const IDC_CANCELKEYLABEL: usize = 1015;
const IDC_CANCELKEY: usize = 1016;
const IDC_PLAYSOUNDS: usize = 1017;
const IDC_SHOWOVERLAY: usize = 1018;
const IDC_POSTPROC: usize = 1019;
const IDC_STARTWITHSYSTEM: usize = 1020;
const IDC_TESTCONN: usize = 1021;
const IDC_CONNSTATUS: usize = 1022;
const IDC_MAXDUR: usize = 1023;
const IDC_SAMPLERATE: usize = 1024;
const IDC_OVERLAYPOS: usize = 1025;
const IDC_MINIMIZETOTRAY: usize = 1026;
const IDC_REMOVEFILLERS: usize = 1027;
const IDC_REMOVEREPT: usize = 1028;
const IDC_AUTOFORMAT: usize = 1029;
const IDC_LLMURL: usize = 1030;
const IDC_LLMAPIKEY: usize = 1031;
const IDC_LLMMODEL: usize = 1032;
const IDC_CUSTOMPROMPT: usize = 1033;
const IDC_HISTRETENTION: usize = 1034;
const IDC_DICTBTN: usize = 1035;
const IDC_LANGUAGE: usize = 1036;
const IDC_TESTPOSTPROC: usize = 1037;
const IDC_OK: usize = 1; // IDOK
const IDC_CANCEL_BTN: usize = 2; // IDCANCEL

// ── Hotkey recording mode ──────────────────────────────────────────────
// When a hotkey edit control (PTT/HF/Cancel) gains focus, it enters
// "recording" mode — displaying "Press a key..." and capturing the next
// keypress to set the hotkey binding.

/// Per-control data stored in GWLP_USERDATA for subclassed hotkey edit controls.
struct HotkeyEditInfo {
    /// Original WndProc before subclassing.
    orig_proc: Option<OrigWndProc>,
    /// Whether this control is currently in recording mode.
    recording: bool,
    /// The original hotkey text before recording started (to restore on cancel).
    original_text: String,
}

/// Type alias for the original WNDPROC used in SetWindowLongPtrW subclassing.
type OrigWndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// Convert a virtual key code + modifiers to a human-readable hotkey string.
/// E.g. VK_F8 → "F8", VK_SPACE with RAlt → "RightAlt+Space"
unsafe fn vk_to_hotkey_string(vk: u32) -> String {
    // Check modifier keys currently held
    let ralt = (GetAsyncKeyState(VK_RMENU.0 as i32) as u16 & 0x8000) != 0;
    let lalt = (GetAsyncKeyState(VK_LMENU.0 as i32) as u16 & 0x8000) != 0;
    let rctrl = (GetAsyncKeyState(VK_RCONTROL.0 as i32) as u16 & 0x8000) != 0;
    let lctrl = (GetAsyncKeyState(VK_LCONTROL.0 as i32) as u16 & 0x8000) != 0;
    let rshift = (GetAsyncKeyState(VK_RSHIFT.0 as i32) as u16 & 0x8000) != 0;
    let lshift = (GetAsyncKeyState(VK_LSHIFT.0 as i32) as u16 & 0x8000) != 0;

    let key_name = vk_to_key_name(vk);

    // If the key itself is a modifier, don't prefix with modifier combo
    let is_modifier = matches!(vk,
        0xA0..=0xA5 | 0x5B | 0x5C // Shift/Ctrl/Alt/Win variants
    );

    if is_modifier {
        return key_name;
    }

    let mut parts: Vec<String> = Vec::new();
    if ralt { parts.push("RightAlt".to_string()); }
    else if lalt { parts.push("LeftAlt".to_string()); }
    if rctrl { parts.push("RightCtrl".to_string()); }
    else if lctrl { parts.push("LeftCtrl".to_string()); }
    if rshift { parts.push("RightShift".to_string()); }
    else if lshift { parts.push("LeftShift".to_string()); }

    parts.push(key_name);
    parts.join("+")
}

/// Convert a virtual key code to its human-readable name.
fn vk_to_key_name(vk: u32) -> String {
    // F1-F24
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    match vk {
        0x20 => "Space".to_string(),
        0x1B => "Escape".to_string(),
        0x0D => "Enter".to_string(),
        0x09 => "Tab".to_string(),
        0x08 => "Backspace".to_string(),
        0x2E => "Delete".to_string(),
        0x2D => "Insert".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "PageUp".to_string(),
        0x22 => "PageDown".to_string(),
        0x26 => "Up".to_string(),
        0x28 => "Down".to_string(),
        0x25 => "Left".to_string(),
        0x27 => "Right".to_string(),
        0xA5 => "RightAlt".to_string(),
        0xA4 => "LeftAlt".to_string(),
        0xA3 => "RightCtrl".to_string(),
        0xA2 => "LeftCtrl".to_string(),
        0xA1 => "RightShift".to_string(),
        0xA0 => "LeftShift".to_string(),
        0x5B => "Win".to_string(),
        _ => {
            // Single character keys (A-Z, 0-9)
            if (0x30..=0x39).contains(&vk) {
                return char::from(vk as u8).to_string(); // '0'-'9'
            }
            if (0x41..=0x5A).contains(&vk) {
                return char::from(vk as u8).to_string(); // 'A'-'Z'
            }
            format!("0x{:02X}", vk)
        }
    }
}

/// Subclass window procedure for hotkey edit controls.
/// When focused, captures the next keypress and sets it as the hotkey.
unsafe extern "system" fn hotkey_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Retrieve per-control info from GWLP_USERDATA
    let info_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HotkeyEditInfo;
    let info = if !info_ptr.is_null() { &mut *info_ptr } else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    match msg {
        WM_SETFOCUS => {
            // Enter recording mode — show placeholder text
            // Save the current text so we can restore it if the user cancels
            let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
            let mut buf: Vec<u16> = vec![0u16; len + 1];
            let _ = SendMessageW(hwnd, WM_GETTEXT, WPARAM(buf.len()), LPARAM(buf.as_mut_ptr() as isize));
            info.original_text = String::from_utf16_lossy(&buf[..len]);
            info.recording = true;
            let placeholder = ew("Press a key...");
            let _ = SendMessageW(hwnd, WM_SETTEXT, WPARAM(0), LPARAM(placeholder.as_ptr() as isize));
            let _ = SendMessageW(hwnd, EM_SETSEL, WPARAM(0), LPARAM(0));
            // Call original WndProc for focus handling
            if let Some(orig) = info.orig_proc {
                return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            // If still in recording mode (no key was captured), restore original text
            if info.recording {
                let wtext = ew(&info.original_text);
                let _ = SendMessageW(hwnd, WM_SETTEXT, WPARAM(0), LPARAM(wtext.as_ptr() as isize));
                info.recording = false;
            }
            if let Some(orig) = info.orig_proc {
                return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if info.recording {
                let vk = wparam.0 as u32;
                // Ignore standalone modifier presses — wait for the actual key
                let is_modifier = matches!(vk,
                    0xA0..=0xA5 | 0x5B | 0x5C
                    | 0x10 | 0x11 | 0x12 // generic Shift/Ctrl/Alt
                );
                if !is_modifier {
                    let hotkey_str = vk_to_hotkey_string(vk);
                    let wtext = ew(&hotkey_str);
                    let _ = SendMessageW(hwnd, WM_SETTEXT, WPARAM(0), LPARAM(wtext.as_ptr() as isize));
                    info.recording = false;
                }
                return LRESULT(0); // Consume the key
            }
            if let Some(orig) = info.orig_proc {
                return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_CHAR => {
            // Suppress character input in recording mode
            if info.recording {
                return LRESULT(0);
            }
            if let Some(orig) = info.orig_proc {
                return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // Clean up: free the HotkeyEditInfo before the control is destroyed
            if !info_ptr.is_null() {
                let _ = Box::from_raw(info_ptr);
            }
            // Don't call orig_proc after freeing — use DefWindowProcW
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => {
            if let Some(orig) = info.orig_proc {
                return CallWindowProcW(Some(orig), hwnd, msg, wparam, lparam);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

/// Subclass a hotkey edit control to capture keypresses for hotkey recording.
unsafe fn subclass_hotkey_edit(edit_hwnd: HWND) {
    let orig_proc = SetWindowLongPtrW(edit_hwnd, GWLP_WNDPROC, hotkey_edit_proc as *mut std::ffi::c_void as isize);
    let orig: Option<OrigWndProc> = if orig_proc != 0 {
        Some(std::mem::transmute::<isize, OrigWndProc>(orig_proc))
    } else {
        None
    };

    // Read current text from the edit control as the original value
    let len = SendMessageW(edit_hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
    let mut buf: Vec<u16> = vec![0u16; len + 1];
    let _ = SendMessageW(edit_hwnd, WM_GETTEXT, WPARAM(buf.len()), LPARAM(buf.as_mut_ptr() as isize));
    let original_text = String::from_utf16_lossy(&buf[..len]);

    // Allocate per-control info on the heap, store pointer in GWLP_USERDATA
    let info = Box::new(HotkeyEditInfo {
        orig_proc: orig,
        recording: false,
        original_text,
    });
    SetWindowLongPtrW(edit_hwnd, GWLP_USERDATA, Box::into_raw(info) as isize);

    // Make the edit read-only appearance but still focusable
    let _ = SendMessageW(edit_hwnd, EM_SETREADONLY, WPARAM(1), LPARAM(0));
}

/// Class name for the settings window.
const SETTINGS_CLASS_NAME: &str = "Qwen3AsrSettingsWnd";

/// Class name for the settings page (child window embedded in tab).
const SETTINGS_PAGE_CLASS_NAME: &str = "Qwen3AsrSettingsPage";

/// Scroll position for the settings page (vertical scroll offset in pixels).
static SETTINGS_PAGE_SCROLL_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Global: the config pointer being edited.
///
/// # Safety
///
/// This raw pointer is only safe because:
/// 1. `show_settings_dialog` runs a **modal message loop** on the main thread,
///    so no other main-thread code can access `AppContext.config` concurrently.
/// 2. The pointer is set before the modal loop and cleared after it returns.
/// 3. Background threads (ASR client, VAD monitor) read `config` values that
///    were captured **before** the dialog opened (they clone Strings/ints into
///    their closures), so they are not affected by mid-dialog edits.
///
/// If a background thread were to read `ctx.config` *by reference* during the
/// dialog, that would be a data race. Currently this does not happen because
/// all background work captures config values by value at spawn time.
static SETTINGS_CONFIG: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Global: result of the dialog (true = OK, false = Cancel).
static SETTINGS_RESULT: AtomicBool = AtomicBool::new(false);

/// Global: the DictionaryManager pointer for the Dictionary dialog.
///
/// # Safety
///
/// Same reasoning as SETTINGS_CONFIG — only accessed during the modal
/// settings dialog loop on the main thread.
static SETTINGS_DICT: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Global: the DictionaryManager pointer for the Add Entry sub-dialog.
static ADD_DICT: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Global: result of the Add Entry sub-dialog (true = entry added).
static ADD_RESULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Show the settings dialog. Returns `true` if the user clicked OK
/// and settings were changed, `false` if cancelled.
pub fn show_settings_dialog(
    config: &mut AppConfig,
    config_path: &std::path::PathBuf,
    dictionary: &mut DictionaryManager,
    parent: HWND,
) -> bool {
    SETTINGS_CONFIG.store(config as *mut AppConfig as *mut std::ffi::c_void, Ordering::SeqCst);
    SETTINGS_DICT.store(dictionary as *mut DictionaryManager as *mut std::ffi::c_void, Ordering::SeqCst);
    SETTINGS_RESULT.store(false, Ordering::SeqCst);

    let _ = unsafe { create_settings_window(parent) };

    let result = SETTINGS_RESULT.load(Ordering::SeqCst);

    if result {
        if let Err(e) = config.save(config_path) {
            log::error!("Failed to save config: {}", e);
        }
        // Sync auto-start registry with the updated config
        if let Err(e) = crate::config::set_auto_start(config.ui.start_with_system) {
            log::warn!("Failed to update auto-start registry: {}", e);
        }
    }

    SETTINGS_CONFIG.store(std::ptr::null_mut(), Ordering::SeqCst);
    SETTINGS_DICT.store(std::ptr::null_mut(), Ordering::SeqCst);
    result
}

unsafe fn create_settings_window(parent: HWND) -> Result<()> {
    let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();

    // Register window class (ignore failure — may already be registered)
    let class_name = ew(SETTINGS_CLASS_NAME);
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(settings_wnd_proc),
        hInstance: hinstance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hbrBackground: GetSysColorBrush(COLOR_3DFACE),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..std::mem::zeroed()
    };
    let _ = RegisterClassW(&wc);

    let title = ew("Qwen3-ASR Settings");
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        480,
        970,
        parent,
        None,
        hinstance,
        None,
    )?;

    // Read config and create controls
    let config_ptr = SETTINGS_CONFIG.load(Ordering::SeqCst);
    let config = &*(config_ptr as *const AppConfig);
    create_controls(hwnd, hinstance, config);

    // Center the dialog on screen
    center_window(hwnd);
    let _ = UpdateWindow(hwnd);

    // Modal message loop
    let mut msg = MSG::default();
    loop {
        let ret = GetMessageW(&mut msg, None, 0, 0);
        if ret.0 <= 0 {
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if !IsWindow(hwnd).as_bool() {
            break;
        }
    }

    Ok(())
}

unsafe fn create_controls(hwnd: HWND, hinstance: HINSTANCE, config: &AppConfig) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let ctx = ControlCtx { hwnd, hinstance, font };
    let mut y: i32 = 15;
    let label_w: i32 = 120;
    let edit_w: i32 = 290;
    let edit_h: i32 = 22;
    let spacing: i32 = 30;

    // Row 1: ASR URL
    create_label(&ctx, IDC_ASRLABEL, "ASR URL:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_ASRSURL, &config.asr_url, CtrlRect { x: 140, y, w: edit_w - 110, h: edit_h });
    // Test Connection button (next to ASR URL)
    let test_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Test").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140 + edit_w - 105,
        y,
        100,
        edit_h + 4,
        hwnd,
        HMENU(IDC_TESTCONN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(test_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += spacing;

    // Connection status label (hidden until test is clicked)
    let _ = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WINDOW_STYLE(SS_LEFT.0),
        140,
        y - 6,
        edit_w,
        edit_h,
        hwnd,
        HMENU(IDC_CONNSTATUS as *mut core::ffi::c_void),
        hinstance,
        None,
    );
    // Status text will be set by the Test Connection handler

    // Row 2: API Key (password-style display)
    create_label(&ctx, IDC_APIKEYLABEL, "API Key:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let api_key = config.api_key.as_deref().unwrap_or("");
    create_password_edit(&ctx, IDC_APIKEY, api_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 3: Default Mode
    create_label(&ctx, IDC_MODELABEL, "Default Mode:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let combo = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("COMBOBOX").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
        140,
        y,
        150,
        200,
        hwnd,
        HMENU(IDC_MODECOMBO as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    let ptt_str = ew("Push-to-Talk");
    let hf_str = ew("Hands-free");
    let _ = SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(ptt_str.as_ptr() as isize));
    let _ = SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(hf_str.as_ptr() as isize));
    let sel = if config.mode.default == "handsfree" { 1 } else { 0 };
    let _ = SendMessageW(combo, CB_SETCURSEL, WPARAM(sel as usize), LPARAM(0));
    y += spacing;

    // Row 4: VAD Threshold
    create_label(&ctx, IDC_VADLABEL, "VAD Threshold:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_VADTHRESHOLD, &format!("{:.2}", config.vad_threshold), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 5: Silence Duration
    create_label(&ctx, IDC_SILENCELABEL, "Silence (sec):", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_SILENCEDUR, &format!("{:.1}", config.silence_duration_secs), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: Max Duration
    create_label(&ctx, 0, "Max Duration (sec):", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_MAXDUR, &config.max_recording_duration.to_string(), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 6: PTT Key
    create_label(&ctx, IDC_PTTKEYLABEL, "PTT Key:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_PTTKEY, &config.hotkey.ptt_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 7: Hands-free Key
    create_label(&ctx, IDC_HFKEYLABEL, "Hands-free Key:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_HFKEY, &config.hotkey.handsfree_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 8: Cancel Key
    create_label(&ctx, IDC_CANCELKEYLABEL, "Cancel Key:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_CANCELKEY, &config.hotkey.cancel_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });

    // Subclass hotkey edit controls for key-recording mode (click → press key → captured)
    if let Ok(ptt_edit) = GetDlgItem(hwnd, IDC_PTTKEY as i32) {
        if !ptt_edit.is_invalid() { subclass_hotkey_edit(ptt_edit); }
    }
    if let Ok(hf_edit) = GetDlgItem(hwnd, IDC_HFKEY as i32) {
        if !hf_edit.is_invalid() { subclass_hotkey_edit(hf_edit); }
    }
    if let Ok(cancel_edit) = GetDlgItem(hwnd, IDC_CANCELKEY as i32) {
        if !cancel_edit.is_invalid() { subclass_hotkey_edit(cancel_edit); }
    }

    y += spacing + 5;

    // Row: Sample Rate combo
    create_label(&ctx, 0, "Sample Rate:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let sr_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_SAMPLERATE as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sr_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let sr_16k = ew("16000");
        let sr_8k = ew("8000");
        let _ = SendMessageW(sr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(sr_16k.as_ptr() as isize));
        let _ = SendMessageW(sr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(sr_8k.as_ptr() as isize));
        let sr_sel = if config.sample_rate == 8000 { 1 } else { 0 };
        let _ = SendMessageW(sr_combo, CB_SETCURSEL, WPARAM(sr_sel as usize), LPARAM(0));
    }
    y += spacing - 5;

    // Row 9: Play Sounds
    create_checkbox(&ctx, IDC_PLAYSOUNDS, "Play start/stop sounds", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.play_sounds);
    y += spacing - 5;

    // Row 10: Show Overlay
    create_checkbox(&ctx, IDC_SHOWOVERLAY, "Show overlay during recording", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.show_overlay);
    y += spacing - 5;

    // Row 11: Post-processing
    create_checkbox(&ctx, IDC_POSTPROC, "Enable post-processing", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.enabled);
    y += spacing - 5;

    // Row: Remove fillers
    create_checkbox(&ctx, IDC_REMOVEFILLERS, "Remove fillers", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.remove_fillers);
    y += spacing - 5;

    // Row: Remove repetitions
    create_checkbox(&ctx, IDC_REMOVEREPT, "Remove repetitions", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.remove_repetitions);
    y += spacing - 5;

    // Row: Auto-format
    create_checkbox(&ctx, IDC_AUTOFORMAT, "Auto-format", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.auto_format);
    y += spacing;

    // Row: Test Post-Processing button
    let test_pp_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Test Post-Processing").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140,
        y,
        160,
        28,
        hwnd,
        HMENU(IDC_TESTPOSTPROC as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(test_pp_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += spacing;

    // Row 12: Start with system
    create_checkbox(&ctx, IDC_STARTWITHSYSTEM, "Start with system", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.start_with_system);
    y += spacing - 5;

    // Row: Minimize to tray
    create_checkbox(&ctx, IDC_MINIMIZETOTRAY, "Minimize to tray", CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.minimize_to_tray);
    y += spacing - 5;

    // Row: History Retention
    create_label(&ctx, 0, "History Retain:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let hr_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_HISTRETENTION as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(hr_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let hr_7 = ew("7 Days");
        let hr_30 = ew("30 Days");
        let hr_90 = ew("90 Days");
        let hr_forever = ew("Forever");
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_7.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_30.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_90.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_forever.as_ptr() as isize));
        let hr_sel = match config.ui.history_retention_days {
            7 => 0,
            30 => 1,
            90 => 2,
            0 => 3,
            _ => 2, // default to 90 days
        };
        let _ = SendMessageW(hr_combo, CB_SETCURSEL, WPARAM(hr_sel as usize), LPARAM(0));
    }
    y += spacing;

    // Row: Overlay Position combo
    create_label(&ctx, 0, "Overlay Position:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let op_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_OVERLAYPOS as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(op_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let op_tc = ew("top-center");
        let op_cur = ew("cursor");
        let _ = SendMessageW(op_combo, CB_ADDSTRING, WPARAM(0), LPARAM(op_tc.as_ptr() as isize));
        let _ = SendMessageW(op_combo, CB_ADDSTRING, WPARAM(0), LPARAM(op_cur.as_ptr() as isize));
        let op_sel = if config.ui.overlay_position == "cursor" { 1 } else { 0 };
        let _ = SendMessageW(op_combo, CB_SETCURSEL, WPARAM(op_sel as usize), LPARAM(0));
    }
    y += spacing;

    // Row: LLM URL
    create_label(&ctx, 0, "LLM URL:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_url = config.post_processing.llm_url.as_deref().unwrap_or("");
    create_edit(&ctx, IDC_LLMURL, llm_url, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: LLM API Key (password-style)
    create_label(&ctx, 0, "LLM API Key:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_api_key = config.post_processing.llm_api_key.as_deref().unwrap_or("");
    create_password_edit(&ctx, IDC_LLMAPIKEY, llm_api_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: LLM Model
    create_label(&ctx, 0, "LLM Model:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_model = config.post_processing.llm_model.as_deref().unwrap_or("");
    create_edit(&ctx, IDC_LLMMODEL, llm_model, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: Custom Prompt (multi-line)
    create_label(&ctx, 0, "Custom Prompt:", CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let custom_prompt = config.post_processing.custom_prompt.as_deref().unwrap_or("");
    {
        let wtext = ew(custom_prompt);
        let prompt_h: i32 = 60;
        let prompt_ctrl = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("EDIT").as_ptr()),
            PCWSTR(wtext.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL
                | WINDOW_STYLE(ES_MULTILINE as u32)
                | WINDOW_STYLE(ES_AUTOVSCROLL as u32),
            140,
            y,
            edit_w,
            prompt_h,
            hwnd,
            HMENU(IDC_CUSTOMPROMPT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(prompt_ctrl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        y += prompt_h + 10;
    }

    // Row: Dictionary button
    let dict_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Dictionary...").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140,
        y,
        130,
        28,
        hwnd,
        HMENU(IDC_DICTBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(dict_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += spacing + 5;

    // OK / Cancel buttons
    let ok_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("OK").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        140,
        y,
        100,
        28,
        hwnd,
        HMENU(IDC_OK as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(ok_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let cancel_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Cancel").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        260,
        y,
        100,
        28,
        hwnd,
        HMENU(IDC_CANCEL_BTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(cancel_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

/// Shared context for creating dialog controls.
struct ControlCtx {
    hwnd: HWND,
    hinstance: HINSTANCE,
    font: HGDIOBJ,
}

/// Position and size for a control.
struct CtrlRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

unsafe fn create_label(
    ctx: &ControlCtx, id: usize, text: &str, r: CtrlRect,
) -> HWND {
    let wtext = ew(text);
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(wtext.as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        r.x, r.y, r.w, r.h,
        ctx.hwnd,
        HMENU(id as *mut core::ffi::c_void),
        ctx.hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(ctx.font.0 as usize), LPARAM(1));
    h
}

unsafe fn create_edit(
    ctx: &ControlCtx, id: usize, text: &str, r: CtrlRect,
) -> HWND {
    let wtext = ew(text);
    let h = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR(wtext.as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        r.x, r.y, r.w, r.h,
        ctx.hwnd,
        HMENU(id as *mut core::ffi::c_void),
        ctx.hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(ctx.font.0 as usize), LPARAM(1));
    h
}

/// Create a password-style edit control (shows ●●● instead of text).
unsafe fn create_password_edit(
    ctx: &ControlCtx, id: usize, text: &str, r: CtrlRect,
) -> HWND {
    let wtext = ew(text);
    let h = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR(wtext.as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WINDOW_STYLE(ES_PASSWORD as u32),
        r.x, r.y, r.w, r.h,
        ctx.hwnd,
        HMENU(id as *mut core::ffi::c_void),
        ctx.hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(ctx.font.0 as usize), LPARAM(1));
    h
}

unsafe fn create_checkbox(
    ctx: &ControlCtx, id: usize, text: &str, r: CtrlRect,
    checked: bool,
) -> HWND {
    let wtext = ew(text);
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(wtext.as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        r.x, r.y, r.w, r.h,
        ctx.hwnd,
        HMENU(id as *mut core::ffi::c_void),
        ctx.hinstance,
        None,
    )
    .unwrap_or_default();
    if checked {
        let _ = SendMessageW(h, BM_SETCHECK, WPARAM(BST_CHECKED.0 as usize), LPARAM(0));
    }
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(ctx.font.0 as usize), LPARAM(1));
    h
}

unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let cmd_id = loword(wparam.0 as u32) as usize;
            let notification = hiword(wparam.0 as u32) as usize;

            if notification == BN_CLICKED as usize || cmd_id == IDC_OK || cmd_id == IDC_CANCEL_BTN {
                match cmd_id {
                    IDC_TESTCONN => {
                        // Test connection to ASR service
                        let asr_url = get_edit_text(hwnd, IDC_ASRSURL);
                        let status_text = test_asr_connection(&asr_url);
                        let status_ctrl = GetDlgItem(hwnd, IDC_CONNSTATUS as i32).unwrap_or_default();
                        if !status_ctrl.is_invalid() {
                            let wtext = ew(&status_text);
                            let _ = SendMessageW(status_ctrl, WM_SETTEXT, WPARAM(0), LPARAM(wtext.as_ptr() as isize));
                            let _ = ShowWindow(status_ctrl, SW_SHOW);
                        }
                        return LRESULT(0);
                    }
                    IDC_OK => {
                        let config_ptr = SETTINGS_CONFIG.load(Ordering::SeqCst);
                        if !config_ptr.is_null() {
                            let config = &mut *(config_ptr as *mut AppConfig);
                            read_controls_to_config(hwnd, config);
                            if let Err(e) = config.validate() {
                                log::warn!("Config validation after settings edit: {}", e);
                            }
                        }
                        SETTINGS_RESULT.store(true, Ordering::SeqCst);
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    IDC_CANCEL_BTN => {
                        SETTINGS_RESULT.store(false, Ordering::SeqCst);
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    IDC_DICTBTN => {
                        let dict_ptr = SETTINGS_DICT.load(Ordering::SeqCst);
                        if !dict_ptr.is_null() {
                            let dictionary = &mut *(dict_ptr as *mut DictionaryManager);
                            show_dictionary_dialog(dictionary, hwnd);
                        }
                        return LRESULT(0);
                    }
                    IDC_TESTPOSTPROC => {
                        // Read current post-processing config from dialog controls
                        let pp_config = read_postproc_config(hwnd);
                        let sample = "嗯，那个，今天我们讨论一下，讨论一下项目进度";
                        let result = crate::postprocess::postprocess(sample, &pp_config);
                        let msg_text = format!("Original:\n{}\n\nProcessed:\n{}", sample, result);
                        let title_w = ew("Post-Processing Test");
                        let msg_w = ew(&msg_text);
                        MessageBoxW(hwnd, PCWSTR(msg_w.as_ptr()), PCWSTR(title_w.as_ptr()), MB_OK);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            SETTINGS_RESULT.store(false, Ordering::SeqCst);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn read_controls_to_config(hwnd: HWND, config: &mut AppConfig) {
    config.asr_url = get_edit_text(hwnd, IDC_ASRSURL);

    let api_key = get_edit_text(hwnd, IDC_APIKEY);
    config.api_key = if api_key.is_empty() { None } else { Some(api_key) };

    let combo = GetDlgItem(hwnd, IDC_MODECOMBO as i32).unwrap_or_default();
    let sel = SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as i32;
    config.mode.default = if sel == 1 { "handsfree".to_string() } else { "ptt".to_string() };

    let vad_text = get_edit_text(hwnd, IDC_VADTHRESHOLD);
    if let Ok(v) = vad_text.parse::<f32>() {
        config.vad_threshold = v.clamp(0.0, 1.0);
    }

    let silence_text = get_edit_text(hwnd, IDC_SILENCEDUR);
    if let Ok(v) = silence_text.parse::<f64>() {
        config.silence_duration_secs = v.clamp(1.0, 60.0);
    }

    let maxdur_text = get_edit_text(hwnd, IDC_MAXDUR);
    if let Ok(v) = maxdur_text.parse::<u64>() {
        config.max_recording_duration = v.clamp(0, 3600);
    }

    config.hotkey.ptt_key = get_edit_text(hwnd, IDC_PTTKEY);
    config.hotkey.handsfree_key = get_edit_text(hwnd, IDC_HFKEY);
    config.hotkey.cancel_key = get_edit_text(hwnd, IDC_CANCELKEY);

    // Sample Rate from combo (0=16000, 1=8000)
    let sr_combo = GetDlgItem(hwnd, IDC_SAMPLERATE as i32).unwrap_or_default();
    let sr_sel = SendMessageW(sr_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as i32;
    config.sample_rate = if sr_sel == 1 { 8000 } else { 16000 };

    config.ui.play_sounds = is_checkbox_checked(hwnd, IDC_PLAYSOUNDS);
    config.ui.show_overlay = is_checkbox_checked(hwnd, IDC_SHOWOVERLAY);
    config.post_processing.enabled = is_checkbox_checked(hwnd, IDC_POSTPROC);
    config.post_processing.remove_fillers = is_checkbox_checked(hwnd, IDC_REMOVEFILLERS);
    config.post_processing.remove_repetitions = is_checkbox_checked(hwnd, IDC_REMOVEREPT);
    config.post_processing.auto_format = is_checkbox_checked(hwnd, IDC_AUTOFORMAT);
    config.ui.start_with_system = is_checkbox_checked(hwnd, IDC_STARTWITHSYSTEM);
    config.ui.minimize_to_tray = is_checkbox_checked(hwnd, IDC_MINIMIZETOTRAY);

    // History retention from combo (0=7 days, 1=30 days, 2=90 days, 3=Forever)
    let hr_combo = GetDlgItem(hwnd, IDC_HISTRETENTION as i32).unwrap_or_default();
    let hr_sel = SendMessageW(hr_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as i32;
    config.ui.history_retention_days = match hr_sel {
        0 => 7,
        1 => 30,
        2 => 90,
        3 => 0, // Forever
        _ => 90,
    };

    // Overlay Position from combo (0=top-center, 1=cursor)
    let op_combo = GetDlgItem(hwnd, IDC_OVERLAYPOS as i32).unwrap_or_default();
    let op_sel = SendMessageW(op_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as i32;
    config.ui.overlay_position = if op_sel == 1 { "cursor".to_string() } else { "top-center".to_string() };

    let llm_url = get_edit_text(hwnd, IDC_LLMURL);
    config.post_processing.llm_url = if llm_url.is_empty() { None } else { Some(llm_url) };

    let llm_api_key = get_edit_text(hwnd, IDC_LLMAPIKEY);
    config.post_processing.llm_api_key = if llm_api_key.is_empty() { None } else { Some(llm_api_key) };

    let llm_model = get_edit_text(hwnd, IDC_LLMMODEL);
    config.post_processing.llm_model = if llm_model.is_empty() { None } else { Some(llm_model) };

    let custom_prompt = get_edit_text(hwnd, IDC_CUSTOMPROMPT);
    config.post_processing.custom_prompt = if custom_prompt.is_empty() { None } else { Some(custom_prompt) };
}

unsafe fn get_edit_text(hwnd: HWND, ctrl_id: usize) -> String {
    let ctrl = match GetDlgItem(hwnd, ctrl_id as i32) {
        Ok(h) if !h.is_invalid() => h,
        _ => return String::new(),
    };
    let len = SendMessageW(ctrl, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
    if len == 0 {
        return String::new();
    }
    let mut buf: Vec<u16> = vec![0u16; len + 1];
    let _ = SendMessageW(ctrl, WM_GETTEXT, WPARAM(buf.len()), LPARAM(buf.as_mut_ptr() as isize));
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn is_checkbox_checked(hwnd: HWND, ctrl_id: usize) -> bool {
    let ctrl = match GetDlgItem(hwnd, ctrl_id as i32) {
        Ok(h) if !h.is_invalid() => h,
        _ => return false,
    };
    let state = SendMessageW(ctrl, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize;
    state == BST_CHECKED.0 as usize
}

/// Read post-processing config from the dialog controls.
unsafe fn read_postproc_config(hwnd: HWND) -> crate::config::PostProcessingConfig {
    let llm_url = get_edit_text(hwnd, IDC_LLMURL);
    let llm_api_key = get_edit_text(hwnd, IDC_LLMAPIKEY);
    let llm_model = get_edit_text(hwnd, IDC_LLMMODEL);
    let custom_prompt = get_edit_text(hwnd, IDC_CUSTOMPROMPT);

    crate::config::PostProcessingConfig {
        enabled: is_checkbox_checked(hwnd, IDC_POSTPROC),
        remove_fillers: is_checkbox_checked(hwnd, IDC_REMOVEFILLERS),
        remove_repetitions: is_checkbox_checked(hwnd, IDC_REMOVEREPT),
        auto_format: is_checkbox_checked(hwnd, IDC_AUTOFORMAT),
        llm_url: if llm_url.is_empty() { None } else { Some(llm_url) },
        llm_api_key: if llm_api_key.is_empty() { None } else { Some(llm_api_key) },
        llm_model: if llm_model.is_empty() { None } else { Some(llm_model) },
        custom_prompt: if custom_prompt.is_empty() { None } else { Some(custom_prompt) },
    }
}

/// Test the ASR service connection by hitting the /v1/health endpoint.
fn test_asr_connection(asr_url: &str) -> String {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let url = format!("{}/v1/health", asr_url.trim_end_matches('/'));

    match client.get(&url).send() {
        Ok(resp) => {
            if resp.status().is_success() {
                "✓ Connected".to_string()
            } else {
                format!("✗ HTTP {}", resp.status())
            }
        }
        Err(e) => {
            let msg = if e.is_connect() {
                "Connection refused".to_string()
            } else if e.is_timeout() {
                "Timeout".to_string()
            } else {
                format!("{}", e)
            };
            format!("✗ {}", msg)
        }
    }
}

// ── Dictionary Dialog ──────────────────────────────────────────────────

/// LVS_FULLROWSELECT for the dictionary ListView.
const DICT_LVS_FULLROWSELECT: u32 = 0x0020;

// Dictionary dialog control IDs
const IDC_DICT_LISTVIEW: usize = 3001;
const IDC_DICT_ADDBTN: usize = 3002;
const IDC_DICT_DELETEBTN: usize = 3003;
const IDC_DICT_IMPORTBTN: usize = 3004;
const IDC_DICT_EXPORTBTN: usize = 3005;
const IDC_DICT_PRESETBTN: usize = 3009;
const IDC_DICT_SEARCHEDIT: usize = 3007;
const IDC_DICT_SEARCHLABEL: usize = 3008;
const IDC_DICT_CLOSEBTN: usize = 3006;

// Add-entry sub-dialog control IDs
const IDC_ADDWORD_LABEL: usize = 3010;
const IDC_ADDWORD_EDIT: usize = 3011;
const IDC_ADDCORRECT_LABEL: usize = 3012;
const IDC_ADDCORRECT_EDIT: usize = 3013;
const IDC_ADDCAT_LABEL: usize = 3014;
const IDC_ADDCAT_EDIT: usize = 3015;
const IDC_ADDOK: usize = 3016;
const IDC_ADDCANCEL: usize = 3017;

const DICT_CLASS_NAME: &str = "Qwen3AsrDictWnd";
const ADDENTRY_CLASS_NAME: &str = "Qwen3AsrAddEntryWnd";

/// Show the dictionary dialog as a modal window.
fn show_dictionary_dialog(dictionary: &mut DictionaryManager, parent: HWND) {
    let _ = unsafe { create_dictionary_window(dictionary, parent) };
}

unsafe fn create_dictionary_window(dictionary: &mut DictionaryManager, parent: HWND) -> anyhow::Result<()> {
    let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();

    // Register window class (ignore failure — may already be registered)
    let class_name = ew(DICT_CLASS_NAME);
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(dict_wnd_proc),
        hInstance: hinstance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hbrBackground: GetSysColorBrush(COLOR_3DFACE),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..std::mem::zeroed()
    };
    let _ = RegisterClassW(&wc);

    let title = ew("Personal Dictionary");
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        520,
        400,
        parent,
        None,
        hinstance,
        None,
    )?;

    // Store DictionaryManager pointer in window user data
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, dictionary as *mut DictionaryManager as isize);

    // Create controls
    let font = GetStockObject(DEFAULT_GUI_FONT);
    create_dict_controls(hwnd, hinstance, font);

    // Populate the listview
    populate_dict_listview(hwnd, dictionary.list());

    // Center on screen
    center_window(hwnd);
    let _ = UpdateWindow(hwnd);

    // Modal message loop
    let mut msg = MSG::default();
    loop {
        let ret = GetMessageW(&mut msg, None, 0, 0);
        if ret.0 <= 0 {
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if !IsWindow(hwnd).as_bool() {
            break;
        }
    }

    Ok(())
}

unsafe fn create_dict_controls(hwnd: HWND, hinstance: HINSTANCE, font: HGDIOBJ) {
    // Search label + edit above the listview
    let search_label = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew("Search:").as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        10, 10, 55, 20,
        hwnd,
        HMENU(IDC_DICT_SEARCHLABEL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(search_label, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let search_edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        70, 8, 420, 22,
        hwnd,
        HMENU(IDC_DICT_SEARCHEDIT as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(search_edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    // ListView — main area (shifted down to make room for search)
    InitCommonControls();
    let lv = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("SysListView32").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP
            | WINDOW_STYLE(LVS_REPORT)
            | WINDOW_STYLE(LVS_SINGLESEL)
            | WINDOW_STYLE(DICT_LVS_FULLROWSELECT),
        10, 38, 480, 252,
        hwnd,
        HMENU(IDC_DICT_LISTVIEW as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(lv, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    // Add columns
    add_dict_listview_column(lv, 0, "Word", 160);
    add_dict_listview_column(lv, 1, "Correct Spelling", 180);
    add_dict_listview_column(lv, 2, "Category", 120);

    // Buttons at bottom
    let add_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Add").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        10, 300, 80, 28,
        hwnd,
        HMENU(IDC_DICT_ADDBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(add_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let del_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Delete").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        100, 300, 80, 28,
        hwnd,
        HMENU(IDC_DICT_DELETEBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(del_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let imp_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Import").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        190, 300, 80, 28,
        hwnd,
        HMENU(IDC_DICT_IMPORTBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(imp_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let exp_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Export").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        280, 300, 80, 28,
        hwnd,
        HMENU(IDC_DICT_EXPORTBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(exp_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let preset_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Presets").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        370, 300, 60, 28,
        hwnd,
        HMENU(IDC_DICT_PRESETBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(preset_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let close_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Close").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        435, 300, 55, 28,
        hwnd,
        HMENU(IDC_DICT_CLOSEBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(close_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

unsafe fn add_dict_listview_column(lv: HWND, index: i32, title: &str, width: i32) {
    let mut wtitle = ew(title);
    let mut col = LVCOLUMNW {
        mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT,
        fmt: LVCFMT_LEFT,
        cx: width,
        pszText: PWSTR(wtitle.as_mut_ptr()),
        ..std::mem::zeroed()
    };
    let _ = SendMessageW(lv, LVM_INSERTCOLUMNW, WPARAM(index as usize), LPARAM(&mut col as *mut _ as isize));
}

/// Populate the dictionary ListView with entries.
unsafe fn populate_dict_listview(hwnd: HWND, entries: &[crate::dictionary::DictionaryEntry]) {
    let lv = match GetDlgItem(hwnd, IDC_DICT_LISTVIEW as i32) {
        Ok(h) if !h.is_invalid() => h,
        _ => return,
    };

    // Clear existing items
    let _ = SendMessageW(lv, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));

    for (i, entry) in entries.iter().enumerate() {
        let mut word_str = ew(&entry.word);
        let mut correct_str = ew(&entry.correct_spelling);
        let mut cat_str = ew(entry.category.as_deref().unwrap_or(""));

        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: i as i32,
            iSubItem: 0,
            pszText: PWSTR(word_str.as_mut_ptr()),
            ..std::mem::zeroed()
        };
        let _ = SendMessageW(lv, LVM_INSERTITEMW, WPARAM(0), LPARAM(&mut item as *mut _ as isize));

        item.iSubItem = 1;
        item.pszText = PWSTR(correct_str.as_mut_ptr());
        let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

        item.iSubItem = 2;
        item.pszText = PWSTR(cat_str.as_mut_ptr());
        let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));
    }
}

/// Get the index of the currently selected ListView item in the dictionary dialog.
unsafe fn get_dict_selected_index(hwnd: HWND) -> i32 {
    let lv = GetDlgItem(hwnd, IDC_DICT_LISTVIEW as i32).unwrap_or_default();
    if lv.is_invalid() {
        return -1;
    }
    SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0 as i32
}

unsafe extern "system" fn dict_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let dict_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DictionaryManager;
    let dictionary = if dict_ptr.is_null() {
        None
    } else {
        Some(&mut *dict_ptr)
    };

    match msg {
        WM_COMMAND => {
            let cmd_id = loword(wparam.0 as u32) as usize;
            let notification = hiword(wparam.0 as u32) as usize;

            // Handle search edit text change (EN_CHANGE from the search box)
            if cmd_id == IDC_DICT_SEARCHEDIT && notification == EN_CHANGE as usize {
                if let Some(dictionary) = dictionary {
                    let query = get_edit_text(hwnd, IDC_DICT_SEARCHEDIT);
                    if query.is_empty() {
                        populate_dict_listview(hwnd, dictionary.list());
                    } else {
                        let results = dictionary.search(&query);
                        populate_dict_listview(hwnd, &results.iter().map(|r| (*r).clone()).collect::<Vec<_>>());
                    }
                }
                return LRESULT(0);
            }

            if notification == BN_CLICKED as usize {
                match cmd_id {
                    IDC_DICT_ADDBTN => {
                        if let Some(dictionary) = dictionary {
                            if show_add_entry_dialog(dictionary, hwnd) {
                                populate_dict_listview(hwnd, dictionary.list());
                            }
                        }
                        return LRESULT(0);
                    }
                    IDC_DICT_DELETEBTN => {
                        if let Some(dictionary) = dictionary {
                            let idx = get_dict_selected_index(hwnd);
                            if idx >= 0 {
                                // Clone id and word before mutable borrow from remove()
                                let (id, word) = {
                                    let entries = dictionary.list();
                                    entries.get(idx as usize)
                                        .map(|e| (e.id.clone(), e.word.clone()))
                                        .unwrap_or_default()
                                };
                                if !id.is_empty() {
                                    if let Err(e) = dictionary.remove(&id) {
                                        log::error!("Failed to delete dictionary entry: {}", e);
                                    } else {
                                        log::info!("Deleted dictionary entry '{}'", word);
                                        populate_dict_listview(hwnd, dictionary.list());
                                    }
                                }
                            }
                        }
                        return LRESULT(0);
                    }
                    IDC_DICT_IMPORTBTN => {
                        if let Some(dictionary) = dictionary {
                            if let Some(path) = open_json_file_dialog(hwnd, "Import Dictionary", false) {
                                match std::fs::read_to_string(&path) {
                                    Ok(json) => {
                                        match dictionary.import_json(&json) {
                                            Ok(count) => {
                                                log::info!("Imported {} dictionary entries", count);
                                                populate_dict_listview(hwnd, dictionary.list());
                                            }
                                            Err(e) => {
                                                log::error!("Failed to import dictionary: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to read import file: {}", e);
                                    }
                                }
                            }
                        }
                        return LRESULT(0);
                    }
                    IDC_DICT_EXPORTBTN => {
                        if let Some(dictionary) = dictionary {
                            if let Some(path) = open_json_file_dialog(hwnd, "Export Dictionary", true) {
                                match dictionary.export_json() {
                                    Ok(json) => {
                                        if let Err(e) = std::fs::write(&path, &json) {
                                            log::error!("Failed to write export file: {}", e);
                                        } else {
                                            log::info!("Exported dictionary to {:?}", path);
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("Failed to export dictionary: {}", e);
                                    }
                                }
                            }
                        }
                        return LRESULT(0);
                    }
                    IDC_DICT_PRESETBTN => {
                        if let Some(dictionary) = dictionary {
                            match dictionary.load_preset() {
                                Ok(count) => {
                                    log::info!("Loaded {} preset dictionary entries", count);
                                    populate_dict_listview(hwnd, dictionary.list());
                                }
                                Err(e) => {
                                    log::error!("Failed to load presets: {}", e);
                                }
                            }
                        }
                        return LRESULT(0);
                    }
                    IDC_DICT_CLOSEBTN => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Show the "Add Entry" sub-dialog. Returns true if an entry was added.
fn show_add_entry_dialog(dictionary: &mut DictionaryManager, parent: HWND) -> bool {
    ADD_DICT.store(dictionary as *mut DictionaryManager as *mut std::ffi::c_void, Ordering::SeqCst);
    ADD_RESULT.store(false, Ordering::SeqCst);

    let _ = unsafe { create_add_entry_window(parent) };

    let result = ADD_RESULT.load(Ordering::SeqCst);
    ADD_DICT.store(std::ptr::null_mut(), Ordering::SeqCst);
    result
}

unsafe fn create_add_entry_window(parent: HWND) -> anyhow::Result<()> {
    let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();

    // Register window class
    let class_name = ew(ADDENTRY_CLASS_NAME);
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(add_entry_wnd_proc),
        hInstance: hinstance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hbrBackground: GetSysColorBrush(COLOR_3DFACE),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..std::mem::zeroed()
    };
    let _ = RegisterClassW(&wc);

    let title = ew("Add Dictionary Entry");
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        360,
        220,
        parent,
        None,
        hinstance,
        None,
    )?;

    let font = GetStockObject(DEFAULT_GUI_FONT);

    // Word field
    let wl = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew("Word:").as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        15, 15, 80, 20,
        hwnd,
        HMENU(IDC_ADDWORD_LABEL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(wl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let we = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        100, 12, 230, 22,
        hwnd,
        HMENU(IDC_ADDWORD_EDIT as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(we, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    // Correct Spelling field
    let cl = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew("Correct Spelling:").as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        15, 50, 100, 20,
        hwnd,
        HMENU(IDC_ADDCORRECT_LABEL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(cl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let ce = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        100, 47, 230, 22,
        hwnd,
        HMENU(IDC_ADDCORRECT_EDIT as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(ce, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    // Category field
    let catl = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew("Category:").as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        15, 85, 80, 20,
        hwnd,
        HMENU(IDC_ADDCAT_LABEL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(catl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let cate = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("EDIT").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        100, 82, 230, 22,
        hwnd,
        HMENU(IDC_ADDCAT_EDIT as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(cate, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    // OK / Cancel buttons
    let ok_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("OK").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        100, 130, 100, 28,
        hwnd,
        HMENU(IDC_ADDOK as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(ok_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let cancel_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew("Cancel").as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        210, 130, 100, 28,
        hwnd,
        HMENU(IDC_ADDCANCEL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(cancel_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    center_window(hwnd);
    let _ = UpdateWindow(hwnd);

    // Modal message loop
    let mut msg = MSG::default();
    loop {
        let ret = GetMessageW(&mut msg, None, 0, 0);
        if ret.0 <= 0 {
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if !IsWindow(hwnd).as_bool() {
            break;
        }
    }

    Ok(())
}

unsafe extern "system" fn add_entry_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let cmd_id = loword(wparam.0 as u32) as usize;
            let notification = hiword(wparam.0 as u32) as usize;

            if notification == BN_CLICKED as usize || cmd_id == IDC_ADDOK || cmd_id == IDC_ADDCANCEL {
                match cmd_id {
                    IDC_ADDOK => {
                        let word = get_edit_text(hwnd, IDC_ADDWORD_EDIT);
                        let correct = get_edit_text(hwnd, IDC_ADDCORRECT_EDIT);
                        let category = {
                            let cat = get_edit_text(hwnd, IDC_ADDCAT_EDIT);
                            if cat.is_empty() { None } else { Some(cat) }
                        };

                        if word.is_empty() || correct.is_empty() {
                            // Don't add empty entries
                            return LRESULT(0);
                        }

                        let dict_ptr = ADD_DICT.load(Ordering::SeqCst);
                        if !dict_ptr.is_null() {
                            let dictionary = &mut *(dict_ptr as *mut DictionaryManager);
                            if let Err(e) = dictionary.add(word, correct, category) {
                                log::warn!("Failed to add dictionary entry: {}", e);
                            } else {
                                ADD_RESULT.store(true, Ordering::SeqCst);
                            }
                        }
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    IDC_ADDCANCEL => {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Open a file dialog for JSON import/export.
/// If `save` is true, shows a Save dialog; otherwise shows an Open dialog.
/// Returns the selected file path, or None if cancelled.
fn open_json_file_dialog(parent: HWND, title: &str, save: bool) -> Option<std::path::PathBuf> {
    use windows::Win32::UI::Controls::Dialogs::*;

    unsafe {
        let mut file_buf = [0u16; 260];
        let filter = ew("JSON Files\0*.json\0All Files\0*.*\0\0");
        let title_w = ew(title);

        if save {
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: parent,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(file_buf.as_mut_ptr()),
                nMaxFile: file_buf.len() as u32,
                lpstrTitle: PCWSTR(title_w.as_ptr()),
                Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
                ..std::mem::zeroed()
            };
            if GetSaveFileNameW(&mut ofn).as_bool() {
                let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
                return Some(std::path::PathBuf::from(String::from_utf16_lossy(&file_buf[..len])));
            }
        } else {
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: parent,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(file_buf.as_mut_ptr()),
                nMaxFile: file_buf.len() as u32,
                lpstrTitle: PCWSTR(title_w.as_ptr()),
                Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
                ..std::mem::zeroed()
            };
            if GetOpenFileNameW(&mut ofn).as_bool() {
                let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
                return Some(std::path::PathBuf::from(String::from_utf16_lossy(&file_buf[..len])));
            }
        }
    }
    None
}

// ── Settings Page (child window for tab embedding) ─────────────────────

/// Create a settings page as a child window suitable for embedding in a Tab control.
///
/// The caller should set `SETTINGS_CONFIG` before calling this function so that
/// the page WndProc can read the config for Test Connection logic.
///
/// Returns the `HWND` of the newly created child window.
pub fn create_settings_page(
    parent: HWND,
    hinstance: HINSTANCE,
    config: &AppConfig,
    dictionary: &DictionaryManager,
    i18n: &I18n,
) -> HWND {
    unsafe {
        // Register window class (ignore failure — may already be registered)
        let class_name = ew(SETTINGS_PAGE_CLASS_NAME);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(settings_page_wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_3DFACE),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..std::mem::zeroed()
        };
        let _ = RegisterClassW(&wc);

        // Create child window with WS_VSCROLL for scrolling
        let hwnd = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_VSCROLL,
            0, 0, 460, 600,
            parent,
            None,
            hinstance,
            None,
        )
        .unwrap_or_default();

        if hwnd.is_invalid() {
            return hwnd;
        }

        // Store DictionaryManager pointer in GWLP_USERDATA
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            dictionary as *const DictionaryManager as isize,
        );

        // Store I18n pointer as a prop so WndProc can access it later
        // (We pass i18n via a window prop because GWLP_USERDATA is used for dict)
        let i18n_box = Box::new(i18n as *const I18n);
        let i18n_raw = Box::into_raw(i18n_box);
        let _ = SetPropW(
            hwnd,
            PCWSTR(ew("SettingsPageI18n").as_ptr()),
            HANDLE(i18n_raw as *mut core::ffi::c_void),
        );

        // Create all controls inside the child window
        create_page_controls(hwnd, hinstance, config, i18n);

        // Set up scrollbar info
        let content_height = 1200i32; // approximate total content height
        let si = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: content_height,
            nPage: 600,
            nPos: 0,
            nTrackPos: 0,
        };
        let _ = SetScrollInfo(hwnd, SB_VERT, &si, BOOL(0));

        SETTINGS_PAGE_SCROLL_Y.store(0, Ordering::SeqCst);

        hwnd
    }
}

/// Create all controls for the settings page (mirrors create_controls but uses i18n).
unsafe fn create_page_controls(hwnd: HWND, hinstance: HINSTANCE, config: &AppConfig, i18n: &I18n) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let ctx = ControlCtx { hwnd, hinstance, font };
    let mut y: i32 = 15;
    let label_w: i32 = 120;
    let edit_w: i32 = 290;
    let edit_h: i32 = 22;
    let spacing: i32 = 30;

    // Row 1: ASR URL
    create_label(&ctx, IDC_ASRLABEL, i18n.t("settings.asr_url"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_ASRSURL, &config.asr_url, CtrlRect { x: 140, y, w: edit_w - 110, h: edit_h });
    // Test Connection button
    let test_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew(i18n.t("settings.test")).as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140 + edit_w - 105,
        y,
        100,
        edit_h + 4,
        hwnd,
        HMENU(IDC_TESTCONN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(test_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += spacing;

    // Connection status label
    let _ = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WINDOW_STYLE(SS_LEFT.0),
        140,
        y - 6,
        edit_w,
        edit_h,
        hwnd,
        HMENU(IDC_CONNSTATUS as *mut core::ffi::c_void),
        hinstance,
        None,
    );

    // Row 2: API Key
    create_label(&ctx, IDC_APIKEYLABEL, i18n.t("settings.api_key"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let api_key = config.api_key.as_deref().unwrap_or("");
    create_password_edit(&ctx, IDC_APIKEY, api_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 3: Default Mode
    create_label(&ctx, IDC_MODELABEL, i18n.t("settings.default_mode"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let combo = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(ew("COMBOBOX").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
        140,
        y,
        150,
        200,
        hwnd,
        HMENU(IDC_MODECOMBO as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    let ptt_str = ew(i18n.t("combo.ptt"));
    let hf_str = ew(i18n.t("combo.handsfree"));
    let _ = SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(ptt_str.as_ptr() as isize));
    let _ = SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(hf_str.as_ptr() as isize));
    let sel = if config.mode.default == "handsfree" { 1 } else { 0 };
    let _ = SendMessageW(combo, CB_SETCURSEL, WPARAM(sel as usize), LPARAM(0));
    y += spacing;

    // Row 4: VAD Threshold
    create_label(&ctx, IDC_VADLABEL, i18n.t("settings.vad_threshold"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_VADTHRESHOLD, &format!("{:.2}", config.vad_threshold), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 5: Silence Duration
    create_label(&ctx, IDC_SILENCELABEL, i18n.t("settings.silence_dur"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_SILENCEDUR, &format!("{:.1}", config.silence_duration_secs), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: Max Duration
    create_label(&ctx, 0, i18n.t("settings.max_dur"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_MAXDUR, &config.max_recording_duration.to_string(), CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 6: PTT Key
    create_label(&ctx, IDC_PTTKEYLABEL, i18n.t("settings.ptt_key"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_PTTKEY, &config.hotkey.ptt_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 7: Hands-free Key
    create_label(&ctx, IDC_HFKEYLABEL, i18n.t("settings.hf_key"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_HFKEY, &config.hotkey.handsfree_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row 8: Cancel Key
    create_label(&ctx, IDC_CANCELKEYLABEL, i18n.t("settings.cancel_key"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    create_edit(&ctx, IDC_CANCELKEY, &config.hotkey.cancel_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });

    // Subclass hotkey edit controls for key-recording mode (click → press key → captured)
    if let Ok(ptt_edit) = GetDlgItem(hwnd, IDC_PTTKEY as i32) {
        if !ptt_edit.is_invalid() { subclass_hotkey_edit(ptt_edit); }
    }
    if let Ok(hf_edit) = GetDlgItem(hwnd, IDC_HFKEY as i32) {
        if !hf_edit.is_invalid() { subclass_hotkey_edit(hf_edit); }
    }
    if let Ok(cancel_edit) = GetDlgItem(hwnd, IDC_CANCELKEY as i32) {
        if !cancel_edit.is_invalid() { subclass_hotkey_edit(cancel_edit); }
    }

    y += spacing + 5;

    // Row: Sample Rate combo
    create_label(&ctx, 0, i18n.t("settings.sample_rate"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let sr_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_SAMPLERATE as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sr_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let sr_16k = ew("16000");
        let sr_8k = ew("8000");
        let _ = SendMessageW(sr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(sr_16k.as_ptr() as isize));
        let _ = SendMessageW(sr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(sr_8k.as_ptr() as isize));
        let sr_sel = if config.sample_rate == 8000 { 1 } else { 0 };
        let _ = SendMessageW(sr_combo, CB_SETCURSEL, WPARAM(sr_sel as usize), LPARAM(0));
    }
    y += spacing - 5;

    // Row: Play Sounds
    create_checkbox(&ctx, IDC_PLAYSOUNDS, i18n.t("settings.play_sounds"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.play_sounds);
    y += spacing - 5;

    // Row: Show Overlay
    create_checkbox(&ctx, IDC_SHOWOVERLAY, i18n.t("settings.show_overlay"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.show_overlay);
    y += spacing - 5;

    // Row: Post-processing
    create_checkbox(&ctx, IDC_POSTPROC, i18n.t("settings.postproc"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.enabled);
    y += spacing - 5;

    // Row: Remove fillers
    create_checkbox(&ctx, IDC_REMOVEFILLERS, i18n.t("settings.remove_fillers"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.remove_fillers);
    y += spacing - 5;

    // Row: Remove repetitions
    create_checkbox(&ctx, IDC_REMOVEREPT, i18n.t("settings.remove_rept"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.remove_repetitions);
    y += spacing - 5;

    // Row: Auto-format
    create_checkbox(&ctx, IDC_AUTOFORMAT, i18n.t("settings.auto_format"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.post_processing.auto_format);
    y += spacing;

    // Row: Test Post-Processing button
    let test_pp_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew(i18n.t("settings.test_postproc")).as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140,
        y,
        160,
        28,
        hwnd,
        HMENU(IDC_TESTPOSTPROC as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(test_pp_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += spacing;

    // Row: Start with system
    create_checkbox(&ctx, IDC_STARTWITHSYSTEM, i18n.t("settings.start_with_system"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.start_with_system);
    y += spacing - 5;

    // Row: Minimize to tray
    create_checkbox(&ctx, IDC_MINIMIZETOTRAY, i18n.t("settings.minimize_to_tray"), CtrlRect { x: 140, y, w: edit_w, h: edit_h }, config.ui.minimize_to_tray);
    y += spacing - 5;

    // Row: History Retention
    create_label(&ctx, 0, i18n.t("settings.history_retain"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let hr_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_HISTRETENTION as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(hr_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let hr_7 = ew(i18n.t("combo.7days"));
        let hr_30 = ew(i18n.t("combo.30days"));
        let hr_90 = ew(i18n.t("combo.90days"));
        let hr_forever = ew(i18n.t("combo.forever"));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_7.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_30.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_90.as_ptr() as isize));
        let _ = SendMessageW(hr_combo, CB_ADDSTRING, WPARAM(0), LPARAM(hr_forever.as_ptr() as isize));
        let hr_sel = match config.ui.history_retention_days {
            7 => 0,
            30 => 1,
            90 => 2,
            0 => 3,
            _ => 2,
        };
        let _ = SendMessageW(hr_combo, CB_SETCURSEL, WPARAM(hr_sel as usize), LPARAM(0));
    }
    y += spacing;

    // Row: Overlay Position combo
    create_label(&ctx, 0, i18n.t("settings.overlay_pos"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let op_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_OVERLAYPOS as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(op_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let op_tc = ew(i18n.t("combo.top_center"));
        let op_cur = ew(i18n.t("combo.cursor"));
        let _ = SendMessageW(op_combo, CB_ADDSTRING, WPARAM(0), LPARAM(op_tc.as_ptr() as isize));
        let _ = SendMessageW(op_combo, CB_ADDSTRING, WPARAM(0), LPARAM(op_cur.as_ptr() as isize));
        let op_sel = if config.ui.overlay_position == "cursor" { 1 } else { 0 };
        let _ = SendMessageW(op_combo, CB_SETCURSEL, WPARAM(op_sel as usize), LPARAM(0));
    }
    y += spacing;

    // Row: LLM URL
    create_label(&ctx, 0, i18n.t("settings.llm_url"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_url = config.post_processing.llm_url.as_deref().unwrap_or("");
    create_edit(&ctx, IDC_LLMURL, llm_url, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: LLM API Key
    create_label(&ctx, 0, i18n.t("settings.llm_api_key"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_api_key = config.post_processing.llm_api_key.as_deref().unwrap_or("");
    create_password_edit(&ctx, IDC_LLMAPIKEY, llm_api_key, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: LLM Model
    create_label(&ctx, 0, i18n.t("settings.llm_model"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let llm_model = config.post_processing.llm_model.as_deref().unwrap_or("");
    create_edit(&ctx, IDC_LLMMODEL, llm_model, CtrlRect { x: 140, y, w: edit_w, h: edit_h });
    y += spacing;

    // Row: Custom Prompt (multi-line)
    create_label(&ctx, 0, i18n.t("settings.custom_prompt"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    let custom_prompt = config.post_processing.custom_prompt.as_deref().unwrap_or("");
    {
        let wtext = ew(custom_prompt);
        let prompt_h: i32 = 60;
        let prompt_ctrl = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("EDIT").as_ptr()),
            PCWSTR(wtext.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL
                | WINDOW_STYLE(ES_MULTILINE as u32)
                | WINDOW_STYLE(ES_AUTOVSCROLL as u32),
            140,
            y,
            edit_w,
            prompt_h,
            hwnd,
            HMENU(IDC_CUSTOMPROMPT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(prompt_ctrl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        y += prompt_h + 10;
    }

    // Row: Language combo
    create_label(&ctx, 0, i18n.t("settings.language"), CtrlRect { x: 15, y, w: label_w, h: edit_h });
    {
        let lang_combo = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("COMBOBOX").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            140,
            y,
            150,
            200,
            hwnd,
            HMENU(IDC_LANGUAGE as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(lang_combo, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let lang_auto = ew(i18n.t("combo.auto"));
        let lang_en = ew("English");
        let lang_zh = ew("中文");
        let _ = SendMessageW(lang_combo, CB_ADDSTRING, WPARAM(0), LPARAM(lang_auto.as_ptr() as isize));
        let _ = SendMessageW(lang_combo, CB_ADDSTRING, WPARAM(0), LPARAM(lang_en.as_ptr() as isize));
        let _ = SendMessageW(lang_combo, CB_ADDSTRING, WPARAM(0), LPARAM(lang_zh.as_ptr() as isize));
        let lang_sel = match config.ui.language.as_str() {
            "en" => 1,
            "zh" => 2,
            _ => 0, // auto
        };
        let _ = SendMessageW(lang_combo, CB_SETCURSEL, WPARAM(lang_sel as usize), LPARAM(0));
    }
    y += spacing;

    // Row: Dictionary button
    let dict_btn = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("BUTTON").as_ptr()),
        PCWSTR(ew(i18n.t("settings.dictionary_btn")).as_ptr()),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        140,
        y,
        130,
        28,
        hwnd,
        HMENU(IDC_DICTBTN as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(dict_btn, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    // No OK/Cancel buttons — the main window handles saving
}

/// Window procedure for the settings page child window.
unsafe extern "system" fn settings_page_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let cmd_id = loword(wparam.0 as u32) as usize;
            let notification = hiword(wparam.0 as u32) as usize;

            if notification == BN_CLICKED as usize {
                match cmd_id {
                    IDC_TESTCONN => {
                        // Test connection to ASR service
                        let asr_url = get_edit_text(hwnd, IDC_ASRSURL);
                        let status_text = test_asr_connection(&asr_url);
                        let status_ctrl = GetDlgItem(hwnd, IDC_CONNSTATUS as i32).unwrap_or_default();
                        if !status_ctrl.is_invalid() {
                            let wtext = ew(&status_text);
                            let _ = SendMessageW(status_ctrl, WM_SETTEXT, WPARAM(0), LPARAM(wtext.as_ptr() as isize));
                            let _ = ShowWindow(status_ctrl, SW_SHOW);
                        }
                        return LRESULT(0);
                    }
                    IDC_DICTBTN => {
                        // Open dictionary dialog
                        let dict_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DictionaryManager;
                        if !dict_ptr.is_null() {
                            let dictionary = &mut *dict_ptr;
                            show_dictionary_dialog(dictionary, hwnd);
                        }
                        return LRESULT(0);
                    }
                    IDC_TESTPOSTPROC => {
                        // Read current post-processing config from dialog controls
                        let pp_config = read_postproc_config(hwnd);
                        let sample = "嗯，那个，今天我们讨论一下，讨论一下项目进度";
                        let result = crate::postprocess::postprocess(sample, &pp_config);
                        let msg_text = format!("Original:\n{}\n\nProcessed:\n{}", sample, result);
                        let title_w = ew("Post-Processing Test");
                        let msg_w = ew(&msg_text);
                        MessageBoxW(hwnd, PCWSTR(msg_w.as_ptr()), PCWSTR(title_w.as_ptr()), MB_OK);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            // Handle language combo selection change
            if notification == CBN_SELCHANGE as usize && cmd_id == IDC_LANGUAGE {
                let combo = GetDlgItem(hwnd, IDC_LANGUAGE as i32).unwrap_or_default();
                if !combo.is_invalid() {
                    let sel = SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0));
                    let lang_str = match sel.0 {
                        1 => "en",
                        2 => "zh",
                        _ => "auto",
                    };
                    // Store the selected language in the I18n prop for later retrieval
                    let i18n = crate::i18n::I18n::from_config(lang_str);
                    let lang_config = i18n.lang().to_config_str().to_string();
                    // Remove old prop
                    let old_prop = GetPropW(hwnd, PCWSTR(ew("SettingsPageI18n").as_ptr()));
                    if !old_prop.is_invalid() {
                        let old_ptr = old_prop.0 as *mut *const crate::i18n::I18n;
                        if !old_ptr.is_null() {
                            let _ = Box::from_raw(old_ptr);
                        }
                    }
                    let _ = RemovePropW(hwnd, PCWSTR(ew("SettingsPageI18n").as_ptr()));
                    // Store new I18n
                    let i18n_box = Box::new(i18n);
                    let i18n_raw = Box::into_raw(i18n_box);
                    let _ = SetPropW(
                        hwnd,
                        PCWSTR(ew("SettingsPageI18n").as_ptr()),
                        HANDLE(i18n_raw as *mut core::ffi::c_void),
                    );
                    // Also store the lang config string for retrieval on close
                    let _ = SetPropW(
                        hwnd,
                        PCWSTR(ew("SettingsPageLang").as_ptr()),
                        HANDLE(lang_config.as_ptr() as *mut core::ffi::c_void),
                    );
                    // Note: We intentionally leak the lang_config string here;
                    // it will be cleaned up when the settings page is destroyed.
                    std::mem::forget(lang_config);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_VSCROLL => {
            let scroll_code = SCROLLBAR_COMMAND(loword(wparam.0 as u32) as i32);
            let content_height = 1200i32;

            let mut si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                nMin: 0,
                nMax: 0,
                nPage: 0,
                nPos: 0,
                nTrackPos: 0,
            };
            let _ = GetScrollInfo(hwnd, SB_VERT, &mut si);

            let old_pos = si.nPos;
            let page_size = si.nPage as i32;

            match scroll_code {
                SB_LINEUP => {
                    si.nPos = si.nPos.saturating_sub(20);
                }
                SB_LINEDOWN => {
                    si.nPos = (si.nPos + 20).min(content_height - page_size);
                }
                SB_PAGEUP => {
                    si.nPos = si.nPos.saturating_sub(page_size);
                }
                SB_PAGEDOWN => {
                    si.nPos = (si.nPos + page_size).min(content_height - page_size);
                }
                SB_THUMBTRACK => {
                    si.nPos = si.nTrackPos;
                }
                _ => {}
            }

            si.fMask = SIF_POS;
            let _ = SetScrollInfo(hwnd, SB_VERT, &si, BOOL(1));

            // Get the actual position after clamping
            let mut si2 = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_POS,
                nMin: 0,
                nMax: 0,
                nPage: 0,
                nPos: 0,
                nTrackPos: 0,
            };
            let _ = GetScrollInfo(hwnd, SB_VERT, &mut si2);
            let new_pos = si2.nPos;

            if new_pos != old_pos {
                let delta = old_pos - new_pos;
                SETTINGS_PAGE_SCROLL_Y.store(new_pos, Ordering::SeqCst);
                let _ = ScrollWindowEx(
                    hwnd,
                    0,
                    delta,
                    None,
                    None,
                    None,
                    None,
                    SW_INVALIDATE | SW_ERASE | SW_SCROLLCHILDREN,
                );
                let _ = UpdateWindow(hwnd);
            }

            LRESULT(0)
        }
        WM_DESTROY => {
            // Clean up the I18n prop
            let prop = GetPropW(hwnd, PCWSTR(ew("SettingsPageI18n").as_ptr()));
            if !prop.is_invalid() {
                let i18n_box_ptr = prop.0 as *mut *const I18n;
                if !i18n_box_ptr.is_null() {
                    let _ = Box::from_raw(i18n_box_ptr);
                }
            }
            let _ = RemovePropW(hwnd, PCWSTR(ew("SettingsPageI18n").as_ptr()));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn center_window(hwnd: HWND) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let x = (screen_w - w) / 2;
        let y = (screen_h - h) / 2;
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
    }
}

/// Encode a Rust string as a null-terminated UTF-16 wide string.
fn ew(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0u16)).collect()
}
