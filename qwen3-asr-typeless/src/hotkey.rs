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
use std::collections::HashSet;
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
    fn matches_key(&self, key: rdev::Key, pressed_modifiers: &HashSet<rdev::Key>) -> bool {
        match self {
            HotkeyDef::Simple { vk } => vk_to_rdev_key(*vk) == Some(key),
            HotkeyDef::Combo { modifier_vk, key_vk } => {
                if vk_to_rdev_key(*key_vk) != Some(key) {
                    return false;
                }
                vk_to_rdev_key(*modifier_vk)
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

// ── Linux: VK → rdev::Key mapping ──────────────────────────────────────

#[cfg(target_os = "linux")]
fn vk_to_rdev_key(vk: u32) -> Option<rdev::Key> {
    match vk {
        0x41 => Some(rdev::Key::KeyA),
        0x42 => Some(rdev::Key::KeyB),
        0x43 => Some(rdev::Key::KeyC),
        0x44 => Some(rdev::Key::KeyD),
        0x45 => Some(rdev::Key::KeyE),
        0x46 => Some(rdev::Key::KeyF),
        0x47 => Some(rdev::Key::KeyG),
        0x48 => Some(rdev::Key::KeyH),
        0x49 => Some(rdev::Key::KeyI),
        0x4A => Some(rdev::Key::KeyJ),
        0x4B => Some(rdev::Key::KeyK),
        0x4C => Some(rdev::Key::KeyL),
        0x4D => Some(rdev::Key::KeyM),
        0x4E => Some(rdev::Key::KeyN),
        0x4F => Some(rdev::Key::KeyO),
        0x50 => Some(rdev::Key::KeyP),
        0x51 => Some(rdev::Key::KeyQ),
        0x52 => Some(rdev::Key::KeyR),
        0x53 => Some(rdev::Key::KeyS),
        0x54 => Some(rdev::Key::KeyT),
        0x55 => Some(rdev::Key::KeyU),
        0x56 => Some(rdev::Key::KeyV),
        0x57 => Some(rdev::Key::KeyW),
        0x58 => Some(rdev::Key::KeyX),
        0x59 => Some(rdev::Key::KeyY),
        0x5A => Some(rdev::Key::KeyZ),
        0x70 => Some(rdev::Key::F1),
        0x71 => Some(rdev::Key::F2),
        0x72 => Some(rdev::Key::F3),
        0x73 => Some(rdev::Key::F4),
        0x74 => Some(rdev::Key::F5),
        0x75 => Some(rdev::Key::F6),
        0x76 => Some(rdev::Key::F7),
        0x77 => Some(rdev::Key::F8),
        0x78 => Some(rdev::Key::F9),
        0x79 => Some(rdev::Key::F10),
        0x7A => Some(rdev::Key::F11),
        0x7B => Some(rdev::Key::F12),
        0x30 => Some(rdev::Key::Num0),
        0x31 => Some(rdev::Key::Num1),
        0x32 => Some(rdev::Key::Num2),
        0x33 => Some(rdev::Key::Num3),
        0x34 => Some(rdev::Key::Num4),
        0x35 => Some(rdev::Key::Num5),
        0x36 => Some(rdev::Key::Num6),
        0x37 => Some(rdev::Key::Num7),
        0x38 => Some(rdev::Key::Num8),
        0x39 => Some(rdev::Key::Num9),
        0x20 => Some(rdev::Key::Space),
        0x1B => Some(rdev::Key::Escape),
        0x0D => Some(rdev::Key::Return),
        0x09 => Some(rdev::Key::Tab),
        0x08 => Some(rdev::Key::Backspace),
        0x2E => Some(rdev::Key::Delete),
        0x2D => Some(rdev::Key::Insert),
        0x24 => Some(rdev::Key::Home),
        0x23 => Some(rdev::Key::End),
        0x21 => Some(rdev::Key::PageUp),
        0x22 => Some(rdev::Key::PageDown),
        0x26 => Some(rdev::Key::UpArrow),
        0x28 => Some(rdev::Key::DownArrow),
        0x25 => Some(rdev::Key::LeftArrow),
        0x27 => Some(rdev::Key::RightArrow),
        0xA5 => Some(rdev::Key::AltGr),
        0xA4 => Some(rdev::Key::Alt),
        0xA3 => Some(rdev::Key::ControlRight),
        0xA1 => Some(rdev::Key::ShiftRight),
        0xA2 => Some(rdev::Key::ControlLeft),
        0xA0 => Some(rdev::Key::ShiftLeft),
        0x5B => Some(rdev::Key::MetaLeft),
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
                .name("hotkey-hook".into())
                .spawn(move || {
                    let pressed_modifiers: std::sync::Mutex<HashSet<rdev::Key>> =
                        std::sync::Mutex::new(HashSet::new());

                    // X11 auto-repeat emits KeyPress→KeyRelease→KeyPress→KeyRelease...
                    // Track physical key state to suppress repeat events.
                    let key_state: std::sync::Mutex<HashSet<rdev::Key>> =
                        std::sync::Mutex::new(HashSet::new());

                    let callback_fn = move |event: rdev::Event| {
                        if !running.load(Ordering::Relaxed) {
                            return;
                        }

                        let (key, is_down) = match event.event_type {
                            rdev::EventType::KeyPress(k) => (k, true),
                            rdev::EventType::KeyRelease(k) => (k, false),
                            _ => return,
                        };

                        {
                            let Ok(mut state) = key_state.lock() else { return };
                            if is_down {
                                if state.contains(&key) { return; }
                                state.insert(key);
                            } else {
                                if !state.contains(&key) { return; }
                                state.remove(&key);
                            }
                        }

                        let is_modifier = matches!(
                            key,
                            rdev::Key::ShiftLeft
                                | rdev::Key::ShiftRight
                                | rdev::Key::ControlLeft
                                | rdev::Key::ControlRight
                                | rdev::Key::Alt
                                | rdev::Key::AltGr
                                | rdev::Key::MetaLeft
                                | rdev::Key::MetaRight
                        );

                        if is_modifier {
                            if let Ok(mut mods) = pressed_modifiers.lock() {
                                if is_down {
                                    mods.insert(key);
                                } else {
                                    mods.remove(&key);
                                }
                            }
                        }

                        let event = match pressed_modifiers.lock() {
                            Ok(guard) => check_hotkey_linux(
                                PTT_DEF.get(),
                                HANDSFREE_DEF.get(),
                                CANCEL_DEF.get(),
                                key,
                                &*guard,
                                is_down,
                            ),
                            Err(_) => None,
                        };

                        if let Some(evt) = event {
                            if let Some(cb) = CALLBACK.get() {
                                cb(evt);
                            }
                        }
                    };

                    if let Err(e) = rdev::listen(callback_fn) {
                        log::error!("rdev::listen error: {:?}", e);
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

    fn check_hotkey_linux(
        ptt_def: Option<&HotkeyDef>,
        handsfree_def: Option<&HotkeyDef>,
        cancel_def: Option<&HotkeyDef>,
        key: rdev::Key,
        pressed_modifiers: &HashSet<rdev::Key>,
        is_down: bool,
    ) -> Option<HotkeyEvent> {
        ptt_def
            .and_then(|d| {
                if d.matches_key(key, pressed_modifiers) {
                    Some(if is_down { HotkeyEvent::KeyDown(HotkeyKind::Ptt) } else { HotkeyEvent::KeyUp(HotkeyKind::Ptt) })
                } else {
                    None
                }
            })
            .or_else(|| {
                handsfree_def.and_then(|d| {
                    if d.matches_key(key, pressed_modifiers) {
                        Some(if is_down { HotkeyEvent::KeyDown(HotkeyKind::HandsFree) } else { HotkeyEvent::KeyUp(HotkeyKind::HandsFree) })
                    } else {
                        None
                    }
                })
            })
            .or_else(|| {
                cancel_def.and_then(|d| {
                    if d.matches_key(key, pressed_modifiers) {
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
