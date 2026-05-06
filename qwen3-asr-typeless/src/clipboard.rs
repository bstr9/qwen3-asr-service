//! Clipboard and paste functionality.
//!
//! On Windows, uses Win32 clipboard and SendInput for Ctrl+V.
//! On Linux, uses xdotool type to input text directly into the active window,
//! with clipboard + Ctrl+V as fallback.

use anyhow::{Context, Result};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
#[cfg(target_os = "windows")]
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL, VK_V,
};

/// Copy text to the clipboard and simulate Ctrl+V paste.
#[cfg(target_os = "windows")]
pub fn paste_text(text: &str) -> Result<()> {
    copy_text(text)?;
    simulate_ctrl_v()?;
    Ok(())
}

/// On Linux, set clipboard content and simulate Ctrl+V paste.
/// Uses xsel for clipboard (more reliable than arboard for X11 CLIPBOARD)
/// and xdotool to send Ctrl+V to the specified window.
#[cfg(target_os = "linux")]
pub fn paste_text(text: &str) -> Result<()> {
    // Set clipboard via xsel (more reliable for X11 CLIPBOARD selection)
    set_clipboard_xsel(text)?;
    copy_text(text)?;

    std::thread::sleep(std::time::Duration::from_millis(50));
    simulate_ctrl_v()
}

#[cfg(target_os = "linux")]
fn set_clipboard_xsel(text: &str) -> Result<()> {
    let mut child = std::process::Command::new("xsel")
        .args(["--clipboard", "--input"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("Failed to run xsel (is it installed?)")?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(text.as_bytes()).ok();
    }
    let status = child.wait().context("xsel wait failed")?;
    if !status.success() {
        log::warn!("set_clipboard_xsel: xsel failed, arboard clipboard still set");
    }
    Ok(())
}



/// Saved clipboard content for later restoration.
pub(crate) struct SavedClipboard {
    text: Option<String>,
}

/// Save the current clipboard text content (if any).
#[cfg(target_os = "windows")]
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
            let handle = GetClipboardData(13u32).ok()?;
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

#[cfg(target_os = "linux")]
pub fn save_clipboard() -> SavedClipboard {
    let text = arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok());
    SavedClipboard { text }
}

/// Restore previously saved clipboard content.
#[cfg(target_os = "windows")]
pub fn restore_clipboard(saved: SavedClipboard) {
    if let Some(text) = saved.text {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = copy_text(&text);
    }
}

#[cfg(target_os = "linux")]
pub fn restore_clipboard(saved: SavedClipboard) {
    if let Some(text) = saved.text {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = copy_text(&text);
    }
}

/// Copy text to the clipboard (no paste).
#[cfg(target_os = "windows")]
pub fn copy_text(text: &str) -> Result<()> {
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
            anyhow::bail!("Failed to open clipboard after 5 attempts (another application may be using it)");
        }

        let result = (|| -> Result<()> {
            EmptyClipboard().context("Failed to empty clipboard")?;

            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0u16)).collect();
            let byte_len = wide.len() * std::mem::size_of::<u16>();

            let h_mem = GlobalAlloc(GMEM_MOVEABLE, byte_len).context("Failed to allocate global memory")?;

            let ptr = GlobalLock(h_mem) as *mut u16;
            if ptr.is_null() {
                anyhow::bail!("GlobalLock returned null");
            }

            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h_mem);

            let handle = HANDLE(h_mem.0);
            SetClipboardData(13u32, handle).context("Failed to set clipboard data")?;

            Ok(())
        })();

        let _ = CloseClipboard();

        result
    }
}

#[cfg(target_os = "linux")]
pub fn copy_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .context("Failed to access clipboard")?;
    clipboard.set_text(text)
        .context("Failed to set clipboard text")?;
    log::info!("copy_text: set clipboard to {} chars", text.len());
    Ok(())
}

/// Simulate paste keystroke via xdotool.
/// Terminal apps use Ctrl+Shift+V, others use Ctrl+V.
/// Detects the active window's WM_CLASS to pick the right key combo.
#[cfg(target_os = "linux")]
fn simulate_ctrl_v() -> Result<()> {
    let key_combo = if is_terminal_window() {
        "ctrl+shift+v"
    } else {
        "ctrl+v"
    };
    let status = std::process::Command::new("xdotool")
        .args(["key", key_combo])
        .status()
        .context("Failed to run xdotool (is it installed?)")?;
    if !status.success() {
        anyhow::bail!("xdotool key {} failed", key_combo);
    }
    log::info!("simulate_paste: xdotool key {} succeeded", key_combo);
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_terminal_window() -> bool {
    let output = match std::process::Command::new("xdotool")
        .args(["getactivewindow"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let win_id = stdout.trim();
    if win_id.is_empty() {
        return false;
    }

    let xprop = match std::process::Command::new("xprop")
        .args(["-id", &win_id, "WM_CLASS"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    let class = String::from_utf8_lossy(&xprop.stdout).to_lowercase();

    const TERMINAL_CLASSES: &[&str] = &[
        "terminal",
        "wezterm",
        "alacritty",
        "kitty",
        "gnome-terminal",
        "konsole",
        "xterm",
        "rxvt",
        "urxvt",
        "st",
        "foot",
        "tilix",
        "terminator",
        "sakura",
        "lxterminal",
        "mate-terminal",
        "xfce4-terminal",
        "termite",
    ];

    TERMINAL_CLASSES.iter().any(|t| class.contains(t))
}

/// Simulate Ctrl+V keystroke via Win32 SendInput.
#[cfg(target_os = "windows")]
fn simulate_ctrl_v() -> Result<()> {
    unsafe {
        let mut inputs: [INPUT; 4] = std::mem::zeroed();

        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT {
            wVk: VK_CONTROL,
            wScan: 0,
            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            time: 0,
            dwExtraInfo: 0,
        };

        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT {
            wVk: VK_V,
            wScan: 0,
            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0),
            time: 0,
            dwExtraInfo: 0,
        };

        inputs[2].r#type = INPUT_KEYBOARD;
        inputs[2].Anonymous.ki = KEYBDINPUT {
            wVk: VK_V,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

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
