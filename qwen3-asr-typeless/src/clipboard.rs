//! Clipboard and paste functionality using Windows API.
//!
//! Copies text to the clipboard and simulates Ctrl+V to paste at
//! the current cursor position.

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
};

/// Copy text to the Windows clipboard and simulate Ctrl+V paste.
pub fn paste_text(text: &str) -> Result<()> {
    copy_text(text)?;
    simulate_ctrl_v()?;
    Ok(())
}

/// Saved clipboard content for later restoration.
pub(crate) struct SavedClipboard {
    text: Option<String>,
}

/// Save the current clipboard text content (if any).
/// Call this before paste_text() to preserve the user's existing clipboard.
pub fn save_clipboard() -> SavedClipboard {
    unsafe {
        let mut opened = false;
        for attempt in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if !opened {
            return SavedClipboard { text: None };
        }

        let text = (|| -> Option<String> {
            // CF_UNICODETEXT = 13
            let handle = GetClipboardData(13u32).ok()?;
            // GetClipboardData returns HANDLE, GlobalLock expects HGLOBAL.
            // Both wrap *mut c_void, so convert explicitly.
            let hglobal = HGLOBAL(handle.0);
            let ptr = GlobalLock(hglobal) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(hglobal);
            Some(text)
        })();

        let _ = CloseClipboard();
        SavedClipboard { text }
    }
}

/// Restore previously saved clipboard content.
/// Call this after paste_text() with a delay to allow the paste to complete.
pub fn restore_clipboard(saved: SavedClipboard) {
    if let Some(text) = saved.text {
        // Delay to allow Ctrl+V paste to complete before overwriting clipboard
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = copy_text(&text);
    }
}

/// Copy text to the Windows clipboard (no paste).
///
/// Retries opening the clipboard up to 5 times with 50ms delays,
/// because another application may temporarily hold it open.
pub fn copy_text(text: &str) -> Result<()> {
    unsafe {
        // Retry opening the clipboard — other apps may hold it briefly
        let mut opened = false;
        for attempt in 0..5 {
            if OpenClipboard(None).is_ok() {
                opened = true;
                break;
            }
            if attempt < 4 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if !opened {
            anyhow::bail!("Failed to open clipboard after 5 attempts (another application may be using it)");
        }

        let result = (|| -> Result<()> {
            EmptyClipboard().context("Failed to empty clipboard")?;

            // Allocate global memory for the wide string (including null terminator)
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0u16)).collect();
            let byte_len = wide.len() * std::mem::size_of::<u16>();

            let h_mem = GlobalAlloc(GMEM_MOVEABLE, byte_len).context("Failed to allocate global memory")?;

            let ptr = GlobalLock(h_mem) as *mut u16;
            if ptr.is_null() {
                anyhow::bail!("GlobalLock returned null");
            }

            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h_mem);

            // CF_UNICODETEXT = 13
            // HGLOBAL and HANDLE share the same inner representation (*mut c_void),
            // but SetClipboardData requires Param<HANDLE>. Convert explicitly.
            let handle = HANDLE(h_mem.0);
            SetClipboardData(13u32, handle).context("Failed to set clipboard data")?;

            Ok(())
        })();

        let _ = CloseClipboard();

        result
    }
}

/// Simulate Ctrl+V keystroke.
fn simulate_ctrl_v() -> Result<()> {
    unsafe {
        let mut inputs: [INPUT; 4] = std::mem::zeroed();

        // Key down: Ctrl
        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT {
            wVk: VK_CONTROL,
            wScan: 0,
            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            time: 0,
            dwExtraInfo: 0,
        };

        // Key down: V
        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT {
            wVk: VK_V,
            wScan: 0,
            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            time: 0,
            dwExtraInfo: 0,
        };

        // Key up: V
        inputs[2].r#type = INPUT_KEYBOARD;
        inputs[2].Anonymous.ki = KEYBDINPUT {
            wVk: VK_V,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

        // Key up: Ctrl
        inputs[3].r#type = INPUT_KEYBOARD;
        inputs[3].Anonymous.ki = KEYBDINPUT {
            wVk: VK_CONTROL,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    Ok(())
}
