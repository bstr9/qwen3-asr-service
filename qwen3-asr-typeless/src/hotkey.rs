//! Global hotkey management using Windows low-level keyboard hook (WH_KEYBOARD_LL).
//!
//! Supports both key-down and key-up detection, which is required for
//! push-to-talk (PTT) mode where recording starts on key-down and stops on key-up.
//!
//! Three hotkeys are monitored simultaneously:
//! - **PTT** (push-to-talk): e.g. F8 — key-down starts, key-up stops
//! - **Hands-free**: e.g. RightAlt+Space — toggle recording
//! - **Cancel**: e.g. Escape — abort recording

use anyhow::{bail, Context, Result};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::OnceLock;

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ── Virtual key constants ──────────────────────────────────────────────

const VK_SPACE: u32 = 0x20;
const VK_RMENU: u32 = 0xA5; // Right Alt
const VK_ESCAPE: u32 = 0x1B;

// ── Hotkey event types ─────────────────────────────────────────────────

/// Event emitted by the keyboard hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// A configured hotkey was pressed.
    KeyDown(HotkeyKind),
    /// A configured hotkey was released.
    KeyUp(HotkeyKind),
}

/// Which hotkey was triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyKind {
    /// Push-to-talk key (e.g. F8).
    Ptt,
    /// Hands-free toggle key (e.g. RightAlt+Space).
    HandsFree,
    /// Cancel key (e.g. Escape).
    Cancel,
}

// ── Parsed hotkey descriptor ───────────────────────────────────────────

/// Parsed representation of a hotkey binding from config.
#[derive(Debug, Clone)]
enum HotkeyDef {
    /// Simple single key, e.g. "F8" or "Escape".
    Simple { vk: u32 },
    /// Modifier + key combo, e.g. "RightAlt+Space".
    /// `modifier_vk` is the specific virtual key of the modifier
    /// (e.g. VK_RMENU = 0xA5 for RightAlt).
    Combo { modifier_vk: u32, key_vk: u32 },
}

impl HotkeyDef {
    /// Parse a hotkey string from config into a descriptor.
    fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some((mod_part, key_part)) = s.split_once('+') {
            let modifier_vk = parse_single_key(mod_part.trim()).with_context(|| {
                format!("Unknown modifier key: {:?}", mod_part.trim())
            })?;
            let key_vk = parse_single_key(key_part.trim()).with_context(|| {
                format!("Unknown key: {:?}", key_part.trim())
            })?;
            Ok(HotkeyDef::Combo { modifier_vk, key_vk })
        } else {
            let vk = parse_single_key(s).with_context(|| format!("Unknown key: {:?}", s))?;
            Ok(HotkeyDef::Simple { vk })
        }
    }

    /// Check whether this hotkey is active given the virtual key code
    /// that just fired. For combos, the modifier must currently be held.
    fn matches(&self, fired_vk: u32) -> bool {
        match self {
            HotkeyDef::Simple { vk } => fired_vk == *vk,
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                if fired_vk != *key_vk {
                    return false;
                }
                // Check the specific modifier is currently held down.
                unsafe {
                    (GetAsyncKeyState(*modifier_vk as i32) as u16 & 0x8000) != 0
                }
            }
        }
    }
}

/// Parse a single key name to a virtual key code.
fn parse_single_key(key: &str) -> Result<u32> {
    let lower = key.to_lowercase();

    // F1-F24
    if let Some(num_str) = lower.strip_prefix('f') {
        if let Ok(num) = num_str.parse::<u32>() {
            if (1..=24).contains(&num) {
                return Ok(0x70 + num - 1); // VK_F1 = 0x70
            }
        }
    }

    // Named keys
    match lower.as_str() {
        "space" => return Ok(VK_SPACE),
        "escape" | "esc" => return Ok(VK_ESCAPE),
        "enter" | "return" => return Ok(0x0D),
        "tab" => return Ok(0x09),
        "backspace" | "back" => return Ok(0x08),
        "delete" | "del" => return Ok(0x2E),
        "insert" => return Ok(0x2D),
        "home" => return Ok(0x24),
        "end" => return Ok(0x23),
        "pageup" => return Ok(0x21),
        "pagedown" => return Ok(0x22),
        "up" => return Ok(0x26),
        "down" => return Ok(0x28),
        "left" => return Ok(0x25),
        "right" => return Ok(0x27),
        // Right-side modifiers
        "rightalt" | "ralt" => return Ok(VK_RMENU),
        "rightctrl" | "rctrl" => return Ok(0xA3),
        "rightshift" | "rshift" => return Ok(0xA1),
        // Left-side modifiers
        "leftalt" | "lalt" => return Ok(0xA4), // VK_LMENU
        "leftctrl" | "lctrl" => return Ok(0xA2),
        "leftshift" | "lshift" => return Ok(0xA0),
        // Generic modifiers (resolve to left variant)
        "alt" => return Ok(0xA4),
        "ctrl" | "control" => return Ok(0xA2),
        "shift" => return Ok(0xA0),
        "win" | "super" => return Ok(0x5B), // VK_LWIN
        _ => {}
    }

    // Single letter A-Z or digit 0-9
    if lower.len() == 1 {
        let c = lower.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Ok(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Ok(c as u32);
        }
    }

    bail!("unknown key: {}", key)
}

// ── Callback type ──────────────────────────────────────────────────────

/// Callback type invoked when a hotkey event occurs.
type HookCallback = dyn Fn(HotkeyEvent) + Send + Sync;

// ── Global state for the hook procedure ────────────────────────────────
// The hook procedure is a free function, so it can only access global state.

/// Global hook callback — set once on `start()`.
static CALLBACK: OnceLock<Box<HookCallback>> = OnceLock::new();

/// Hook handle stored as isize (raw pointer value). Zero means no hook.
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);

/// Thread ID of the hook thread so we can post WM_QUIT for clean shutdown.
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// Parsed hotkey definitions — set once on `start()`.
static PTT_DEF: OnceLock<HotkeyDef> = OnceLock::new();
static HANDSFREE_DEF: OnceLock<HotkeyDef> = OnceLock::new();
static CANCEL_DEF: OnceLock<HotkeyDef> = OnceLock::new();

// ── Low-level keyboard hook callback ───────────────────────────────────

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;

/// LLKHF_INJECTED (0x00000010): Event was injected via SendInput or keybd_event.
/// We skip injected events to avoid interfering with our own simulate_ctrl_v()
/// and to prevent the hook from processing synthetic keystrokes.
const LLKHF_INJECTED: u32 = 0x10;

/// The `WH_KEYBOARD_LL` callback. Must be an `unsafe extern "system" fn`
/// with the exact signature Windows expects.
unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // If code < 0, pass directly to next hook (Windows convention).
    if n_code < 0 {
        let hook = HOOK_HANDLE.load(Ordering::Relaxed);
        return CallNextHookEx(
            HHOOK(hook as *mut _),
            n_code,
            w_param,
            l_param,
        );
    }

    let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
    let vk = kb.vkCode;
    let msg = w_param.0 as u32;

    // Skip injected (synthetic) events — these are generated by SendInput
    // or keybd_event (e.g. our own simulate_ctrl_v()). Without this check,
    // the hook would detect the simulated Ctrl+V as a real keystroke and
    // could falsely match hotkey patterns (especially Ctrl+Escape when
    // Ctrl is held during the simulated paste).
    if kb.flags.0 & LLKHF_INJECTED != 0 {
        let hook = HOOK_HANDLE.load(Ordering::Relaxed);
        return CallNextHookEx(
            HHOOK(hook as *mut _),
            n_code,
            w_param,
            l_param,
        );
    }

    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

    if is_down || is_up {
        // Check each configured hotkey in priority order.
        let event = check_hotkey(PTT_DEF.get(), vk, HotkeyKind::Ptt, is_down)
            .or_else(|| check_hotkey(HANDSFREE_DEF.get(), vk, HotkeyKind::HandsFree, is_down))
            .or_else(|| check_hotkey(CANCEL_DEF.get(), vk, HotkeyKind::Cancel, is_down));

        if let Some(evt) = event {
            if let Some(cb) = CALLBACK.get() {
                cb(evt);
            }
        }
    }

    // Always call next hook so other applications are not blocked.
    let hook = HOOK_HANDLE.load(Ordering::Relaxed);
    CallNextHookEx(
        HHOOK(hook as *mut _),
        n_code,
        w_param,
        l_param,
    )
}

/// Helper: if `def` matches the fired `vk`, return the appropriate HotkeyEvent.
fn check_hotkey(def: Option<&HotkeyDef>, vk: u32, kind: HotkeyKind, is_down: bool) -> Option<HotkeyEvent> {
    match def {
        Some(d) if d.matches(vk) => Some(if is_down {
            HotkeyEvent::KeyDown(kind)
        } else {
            HotkeyEvent::KeyUp(kind)
        }),
        _ => None,
    }
}

// ── HotkeyManager ──────────────────────────────────────────────────────

/// Manages a Windows low-level keyboard hook for global hotkey detection.
///
/// The hook is installed on a dedicated thread that runs a message loop
/// (required by Windows for low-level hooks to function). Events are
/// forwarded to the callback provided at `start()` time.
pub struct HotkeyManager {
    /// Handle to the dedicated hook thread, if running.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyManager {
    /// Create a new (inactive) hotkey manager.
    pub fn new() -> Self {
        Self { thread_handle: None }
    }

    /// Install the global keyboard hook.
    ///
    /// `ptt_key`, `handsfree_key`, and `cancel_key` are the hotkey strings
    /// from config (e.g. "F8", "RightAlt+Space", "Escape").
    /// `callback` is invoked on every matching key-down / key-up event.
    ///
    /// The hook runs on a dedicated background thread that owns a Windows
    /// message loop — required for low-level hooks to receive events.
    pub fn start(
        &mut self,
        ptt_key: &str,
        handsfree_key: &str,
        cancel_key: &str,
        callback: Box<HookCallback>,
    ) -> Result<()> {
        if self.thread_handle.is_some() {
            bail!("HotkeyManager hook is already running");
        }

        // Parse hotkey definitions.
        let ptt_def = HotkeyDef::parse(ptt_key)?;
        let handsfree_def = HotkeyDef::parse(handsfree_key)?;
        let cancel_def = HotkeyDef::parse(cancel_key)?;

        // Store the callback and definitions in globals.
        CALLBACK
            .set(callback)
            .map_err(|_| anyhow::anyhow!("HotkeyManager callback already set (OnceLock)"))?;
        PTT_DEF
            .set(ptt_def)
            .map_err(|_| anyhow::anyhow!("PTT def already set (OnceLock)"))?;
        HANDSFREE_DEF
            .set(handsfree_def)
            .map_err(|_| anyhow::anyhow!("HandsFree def already set (OnceLock)"))?;
        CANCEL_DEF
            .set(cancel_def)
            .map_err(|_| anyhow::anyhow!("Cancel def already set (OnceLock)"))?;

        let handle = std::thread::Builder::new()
            .name("hotkey-hook".into())
            .spawn(move || unsafe {
                let h_instance: HINSTANCE = match GetModuleHandleW(None) {
                    Ok(h) => h.into(),
                    Err(e) => {
                        log::error!("GetModuleHandleW failed: {}", e);
                        return;
                    }
                };

                let hook_result = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    h_instance,
                    0,
                );

                let hook_handle = match hook_result {
                    Ok(h) => h,
                    Err(e) => {
                        log::error!("SetWindowsHookExW failed: {}", e);
                        return;
                    }
                };

                HOOK_HANDLE.store(hook_handle.0 as isize, Ordering::SeqCst);
                let tid = GetCurrentThreadId();
                HOOK_THREAD_ID.store(tid, Ordering::SeqCst);
                log::info!("WH_KEYBOARD_LL hook installed successfully");

                // Run a message loop. Low-level hooks require the
                // installing thread to pump messages.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                // Unhook on shutdown.
                let h = HOOK_HANDLE.swap(0, Ordering::SeqCst);
                if h != 0 {
                    let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
                    log::info!("WH_KEYBOARD_LL hook removed");
                }
                HOOK_THREAD_ID.store(0, Ordering::SeqCst);
            })
            .context("Failed to spawn hotkey hook thread")?;

        // Wait for the hook to be installed (with timeout).
        let start = std::time::Instant::now();
        while HOOK_HANDLE.load(Ordering::SeqCst) == 0 {
            if start.elapsed() > std::time::Duration::from_secs(5) {
                bail!("timeout waiting for keyboard hook to install");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        self.thread_handle = Some(handle);
        Ok(())
    }

    /// Remove the keyboard hook and join the background thread.
    pub fn stop(&mut self) -> Result<()> {
        // Post WM_QUIT to the hook thread so GetMessageW returns FALSE.
        let thread_id = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }

        if let Some(handle) = self.thread_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Hotkey hook thread panicked"))?;
        }

        Ok(())
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_f8() {
        let def = HotkeyDef::parse("F8").unwrap();
        match def {
            HotkeyDef::Simple { vk } => assert_eq!(vk, 0x77),
            _ => panic!("expected Simple"),
        }
    }

    #[test]
    fn parse_simple_escape() {
        let def = HotkeyDef::parse("Escape").unwrap();
        match def {
            HotkeyDef::Simple { vk } => assert_eq!(vk, VK_ESCAPE),
            _ => panic!("expected Simple"),
        }
    }

    #[test]
    fn parse_combo_rightalt_space() {
        let def = HotkeyDef::parse("RightAlt+Space").unwrap();
        match def {
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                assert_eq!(modifier_vk, VK_RMENU);
                assert_eq!(key_vk, VK_SPACE);
            }
            _ => panic!("expected Combo"),
        }
    }

    #[test]
    fn parse_unknown_key_fails() {
        assert!(HotkeyDef::parse("UnknownKey").is_err());
    }

    #[test]
    fn simple_matches_vk() {
        let def = HotkeyDef::parse("F8").unwrap();
        assert!(def.matches(0x77));
        assert!(!def.matches(VK_ESCAPE));
    }

    #[test]
    fn parse_ralt_shortcut() {
        let def = HotkeyDef::parse("RAlt+Space").unwrap();
        match def {
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                assert_eq!(modifier_vk, VK_RMENU);
                assert_eq!(key_vk, VK_SPACE);
            }
            _ => panic!("expected Combo"),
        }
    }

    #[test]
    fn parse_single_letter() {
        let def = HotkeyDef::parse("A").unwrap();
        match def {
            HotkeyDef::Simple { vk } => assert_eq!(vk, 0x41), // 'A' = 0x41
            _ => panic!("expected Simple"),
        }
    }
}
