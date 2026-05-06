//! Global hotkey management using platform-specific keyboard hooks.
//!
//! Supports both key-down and key-up detection, which is required for
//! push-to-talk (PTT) mode where recording starts on key-down and stops on key-up.
//!
//! Three hotkeys are monitored simultaneously:
//! - **PTT** (push-to-talk): e.g. F8 — key-down starts, key-up stops
//! - **Hands-free**: e.g. RightAlt+Space — toggle recording
//! - **Cancel**: e.g. Escape — abort recording
//!
//! Platform implementations:
//! - **Windows**: WH_KEYBOARD_LL low-level keyboard hook
//! - **Linux**: rdev global keyboard listener

use anyhow::{bail, Context, Result};
use std::sync::OnceLock;

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::*;
#[cfg(target_os = "windows")]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::GetCurrentThreadId;
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::*;

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Arc;

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
    #[cfg(target_os = "windows")]
    fn matches(&self, fired_vk: u32) -> bool {
        match self {
            HotkeyDef::Simple { vk } => fired_vk == *vk,
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                if fired_vk != *key_vk {
                    return false;
                }
                unsafe {
                    (GetAsyncKeyState(*modifier_vk as i32) as u16 & 0x8000) != 0
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl HotkeyDef {
    fn matches_evdev(&self, code: u16, pressed_modifiers: &[u16]) -> bool {
        match self {
            HotkeyDef::Simple { vk } => vk_to_evdev_code(*vk) == Some(code),
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                if vk_to_evdev_code(*key_vk) != Some(code) {
                    return false;
                }
                vk_to_evdev_code(*modifier_vk)
                    .map(|mk| pressed_modifiers.contains(&mk))
                    .unwrap_or(false)
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

// ── Shared globals ─────────────────────────────────────────────────────

/// Global hook callback — set once on `start()`.
static CALLBACK: OnceLock<Box<HookCallback>> = OnceLock::new();

/// Parsed hotkey definitions — set once on `start()`.
static PTT_DEF: OnceLock<HotkeyDef> = OnceLock::new();
static HANDSFREE_DEF: OnceLock<HotkeyDef> = OnceLock::new();
static CANCEL_DEF: OnceLock<HotkeyDef> = OnceLock::new();

// ── Linux: VK → evdev key code mapping ──────────────────────────────────

#[cfg(target_os = "linux")]
fn vk_to_evdev_code(vk: u32) -> Option<u16> {
    match vk {
        // Letters
        0x41 => Some(0x1E), 0x42 => Some(0x30), 0x43 => Some(0x2E), 0x44 => Some(0x20),
        0x45 => Some(0x12), 0x46 => Some(0x21), 0x47 => Some(0x22), 0x48 => Some(0x23),
        0x49 => Some(0x17), 0x4A => Some(0x24), 0x4B => Some(0x25), 0x4C => Some(0x26),
        0x4D => Some(0x32), 0x4E => Some(0x31), 0x4F => Some(0x18), 0x50 => Some(0x19),
        0x51 => Some(0x10), 0x52 => Some(0x13), 0x53 => Some(0x1F), 0x54 => Some(0x14),
        0x55 => Some(0x16), 0x56 => Some(0x2F), 0x57 => Some(0x11), 0x58 => Some(0x2D),
        0x59 => Some(0x15), 0x5A => Some(0x2C),
        // F-keys
        0x70 => Some(0x3B), 0x71 => Some(0x3C), 0x72 => Some(0x3D), 0x73 => Some(0x3E),
        0x74 => Some(0x3F), 0x75 => Some(0x40), 0x76 => Some(0x41), 0x77 => Some(0x42),
        0x78 => Some(0x43), 0x79 => Some(0x44), 0x7A => Some(0x45), 0x7B => Some(0x46),
        // Digits
        0x30 => Some(0x0B), 0x31 => Some(0x02), 0x32 => Some(0x03), 0x33 => Some(0x04),
        0x34 => Some(0x05), 0x35 => Some(0x06), 0x36 => Some(0x07), 0x37 => Some(0x08),
        0x38 => Some(0x09), 0x39 => Some(0x0A),
        // Special keys
        0x20 => Some(0x39), 0x1B => Some(0x01), 0x0D => Some(0x1C), 0x09 => Some(0x0F),
        0x08 => Some(0x0E), 0x2E => Some(0x6F), 0x2D => Some(0x6E), 0x24 => Some(0x66),
        0x23 => Some(0x6B), 0x21 => Some(0x68), 0x22 => Some(0x6D),
        0x26 => Some(0x67), 0x28 => Some(0x6C), 0x25 => Some(0x69), 0x27 => Some(0x6A),
        // Modifiers
        0xA5 => Some(0x64), 0xA4 => Some(0x38), 0xA3 => Some(0x61), 0xA2 => Some(0x1D),
        0xA1 => Some(0x36), 0xA0 => Some(0x2A), 0x5B => Some(0x7D),
        _ => None,
    }
}

// ── Windows implementation ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;

    /// Hook handle stored as isize (raw pointer value). Zero means no hook.
    static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);

    /// Thread ID of the hook thread so we can post WM_QUIT for clean shutdown.
    static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;

    const LLKHF_INJECTED: u32 = 0x10;

    unsafe extern "system" fn low_level_keyboard_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
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
            let event = check_hotkey(PTT_DEF.get(), vk, HotkeyKind::Ptt, is_down)
                .or_else(|| check_hotkey(HANDSFREE_DEF.get(), vk, HotkeyKind::HandsFree, is_down))
                .or_else(|| check_hotkey(CANCEL_DEF.get(), vk, HotkeyKind::Cancel, is_down));

            if let Some(evt) = event {
                if let Some(cb) = CALLBACK.get() {
                    cb(evt);
                }
            }
        }

        let hook = HOOK_HANDLE.load(Ordering::Relaxed);
        CallNextHookEx(
            HHOOK(hook as *mut _),
            n_code,
            w_param,
            l_param,
        )
    }

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

    pub struct HotkeyManager {
        thread_handle: Option<std::thread::JoinHandle<()>>,
    }

    impl HotkeyManager {
        pub fn new() -> Self {
            Self { thread_handle: None }
        }

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

            let ptt_def = HotkeyDef::parse(ptt_key)?;
            let handsfree_def = HotkeyDef::parse(handsfree_key)?;
            let cancel_def = HotkeyDef::parse(cancel_key)?;

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

                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }

                    let h = HOOK_HANDLE.swap(0, Ordering::SeqCst);
                    if h != 0 {
                        let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
                        log::info!("WH_KEYBOARD_LL hook removed");
                    }
                    HOOK_THREAD_ID.store(0, Ordering::SeqCst);
                })
                .context("Failed to spawn hotkey hook thread")?;

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

        pub fn stop(&mut self) -> Result<()> {
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
}

#[cfg(target_os = "windows")]
pub use windows_impl::HotkeyManager;

// ── Linux implementation ───────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;

    const EV_KEY: u16 = 0x01;
    const KEY_LEFTSHIFT: u16 = 0x2A;
    const KEY_RIGHTSHIFT: u16 = 0x36;
    const KEY_LEFTCTRL: u16 = 0x1D;
    const KEY_RIGHTCTRL: u16 = 0x61;
    const KEY_LEFTALT: u16 = 0x38;
    const KEY_RIGHTALT: u16 = 0x64;
    const KEY_LEFTMETA: u16 = 0x7D;
    const KEY_RIGHTMETA: u16 = 0x7E;
    const KEY_SPACE: u16 = 0x39;

    pub struct HotkeyManager {
        thread_handle: Option<std::thread::JoinHandle<()>>,
        running: Arc<AtomicBool>,
    }

    impl HotkeyManager {
        pub fn new() -> Self {
            Self {
                thread_handle: None,
                running: Arc::new(AtomicBool::new(false)),
            }
        }

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

            let ptt_def = HotkeyDef::parse(ptt_key)?;
            let handsfree_def = HotkeyDef::parse(handsfree_key)?;
            let cancel_def = HotkeyDef::parse(cancel_key)?;

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

            self.running.store(true, Ordering::SeqCst);
            let running = self.running.clone();

            let handle = std::thread::Builder::new()
                .name("hotkey-evdev".into())
                .spawn(move || {
                    let mut devices: Vec<evdev::Device> = evdev::enumerate()
                        .filter(|(_, dev)| {
                            dev.supported_keys().map_or(false, |keys| {
                                keys.contains(evdev::KeyCode(KEY_SPACE))
                            })
                        })
                        .map(|(_, dev)| dev)
                        .collect();

                    if devices.is_empty() {
                        log::error!("No keyboard devices found via evdev");
                        return;
                    }

                    log::info!("evdev: monitoring {} keyboard device(s)", devices.len());

                    let mut pressed_modifiers: Vec<u16> = Vec::new();

                    const MODIFIER_CODES: &[u16] = &[
                        KEY_LEFTSHIFT, KEY_RIGHTSHIFT,
                        KEY_LEFTCTRL, KEY_RIGHTCTRL,
                        KEY_LEFTALT, KEY_RIGHTALT,
                        KEY_LEFTMETA, KEY_RIGHTMETA,
                    ];

                    while running.load(Ordering::Relaxed) {
                        for dev in &mut devices {
                            let events = match dev.fetch_events() {
                                Ok(ev) => ev,
                                Err(e) => {
                                    if e.kind() != std::io::ErrorKind::WouldBlock {
                                        log::warn!("evdev fetch error: {:?}", e);
                                    }
                                    continue;
                                }
                            };

                            for event in events {
                                if event.event_type().0 != EV_KEY {
                                    continue;
                                }

                                let code = event.code();
                                let value = event.value();
                                // value: 1=down, 0=up, 2=repeat
                                if value == 2 { continue; }
                                let is_down = value == 1;

                                if MODIFIER_CODES.contains(&code) {
                                    if is_down {
                                        if !pressed_modifiers.contains(&code) {
                                            pressed_modifiers.push(code);
                                        }
                                    } else {
                                        pressed_modifiers.retain(|&c| c != code);
                                    }
                                }

                                let hotkey_event = check_hotkey_evdev(
                                    PTT_DEF.get(),
                                    HANDSFREE_DEF.get(),
                                    CANCEL_DEF.get(),
                                    code,
                                    &pressed_modifiers,
                                    is_down,
                                );

                                if let Some(evt) = hotkey_event {
                                    if let Some(cb) = CALLBACK.get() {
                                        cb(evt);
                                    }
                                }
                            }
                        }

                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                })
                .context("Failed to spawn hotkey listener thread")?;

            self.thread_handle = Some(handle);
            Ok(())
        }

        pub fn stop(&mut self) -> Result<()> {
            self.running.store(false, Ordering::SeqCst);

            if let Some(handle) = self.thread_handle.take() {
                let _ = handle.join();
            }

            Ok(())
        }
    }

    impl Drop for HotkeyManager {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    fn check_hotkey_evdev(
        ptt_def: Option<&HotkeyDef>,
        handsfree_def: Option<&HotkeyDef>,
        cancel_def: Option<&HotkeyDef>,
        code: u16,
        pressed_modifiers: &[u16],
        is_down: bool,
    ) -> Option<HotkeyEvent> {
        ptt_def
            .and_then(|d| {
                if d.matches_evdev(code, pressed_modifiers) {
                    Some(if is_down { HotkeyEvent::KeyDown(HotkeyKind::Ptt) } else { HotkeyEvent::KeyUp(HotkeyKind::Ptt) })
                } else {
                    None
                }
            })
            .or_else(|| {
                handsfree_def.and_then(|d| {
                    if d.matches_evdev(code, pressed_modifiers) {
                        Some(if is_down { HotkeyEvent::KeyDown(HotkeyKind::HandsFree) } else { HotkeyEvent::KeyUp(HotkeyKind::HandsFree) })
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                cancel_def.and_then(|d| {
                    if d.matches_evdev(code, pressed_modifiers) {
                        Some(if is_down { HotkeyEvent::KeyDown(HotkeyKind::Cancel) } else { HotkeyEvent::KeyUp(HotkeyKind::Cancel) })
                    } else {
                        None
                    }
                })
            })
    }
}

#[cfg(target_os = "linux")]
pub use linux_impl::HotkeyManager;

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

    #[cfg(target_os = "windows")]
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
