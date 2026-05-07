//! Main application window with tabbed interface.
//!
//! On Windows: Creates a top-level window with a Win32 Tab control.
//! On Linux: Creates a GTK4 Window with a Notebook.

use crate::config::AppConfig;
use crate::dictionary::DictionaryManager;
use crate::history::HistoryManager;
use crate::i18n::I18n;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Shared helpers (platform-independent) ──────────────────────────────

/// Format the ASR status text for the About page.
fn format_asr_status(i18n: &I18n, config: &AppConfig) -> String {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());
    let url = format!("{}/v1/health", config.asr_url);
    match client.get(&url).send() {
        Ok(resp) if resp.status().is_success() => {
            format!("{} ✓", i18n.t("about.asr_connected"))
        }
        _ => {
            format!("{} ✗", i18n.t("about.asr_disconnected"))
        }
    }
}

// ── Windows implementation ─────────────────────────────────────────────

#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicPtr, Ordering};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::*;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::*;

#[cfg(target_os = "windows")]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

#[cfg(target_os = "windows")]
use windows::Win32::System::SystemServices::*;

#[cfg(target_os = "windows")]
use windows::Win32::UI::Controls::*;

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::*;

#[cfg(target_os = "windows")]
use windows::core::{PCWSTR, PWSTR};

#[cfg(target_os = "windows")]
const MAIN_CLASS_NAME: &str = "Qwen3AsrMainWnd";

#[cfg(target_os = "windows")]
const IDC_TABCTRL: usize = 3001;

#[cfg(target_os = "windows")]
const TAB_SETTINGS: i32 = 0;

#[cfg(target_os = "windows")]
const TAB_HISTORY: i32 = 1;

#[cfg(target_os = "windows")]
const TAB_ABOUT: i32 = 2;

#[cfg(target_os = "windows")]
const IDC_ABOUT_VERSION: usize = 3101;

#[cfg(target_os = "windows")]
const IDC_ABOUT_DESC: usize = 3103;

#[cfg(target_os = "windows")]
const IDC_ABOUT_ASR_STATUS: usize = 3104;

#[cfg(target_os = "windows")]
const IDC_ABOUT_GITHUB: usize = 3105;

#[cfg(target_os = "windows")]
static MAIN_HWND: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
static MAIN_CONFIG: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
static MAIN_DICT: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
static MAIN_HISTORY: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
static MAIN_I18N: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
static MAIN_CONFIG_PATH: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "windows")]
pub fn show_main_window(
    config: &mut AppConfig,
    config_path: &std::path::Path,
    dictionary: &mut DictionaryManager,
    history: &HistoryManager,
    i18n: &I18n,
    parent: HWND,
) {
    let raw = MAIN_HWND.load(Ordering::SeqCst);
    if !raw.is_null() {
        let hwnd = HWND(raw);
        if unsafe { IsWindow(hwnd).as_bool() } {
            let _ = unsafe { SetForegroundWindow(hwnd) };
            let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
            return;
        }
    }

    MAIN_CONFIG.store(config as *mut AppConfig as *mut std::ffi::c_void, Ordering::SeqCst);
    MAIN_DICT.store(dictionary as *mut DictionaryManager as *mut std::ffi::c_void, Ordering::SeqCst);
    MAIN_HISTORY.store(history as *const HistoryManager as *const std::ffi::c_void as *mut std::ffi::c_void, Ordering::SeqCst);
    MAIN_I18N.store(i18n as *const I18n as *const std::ffi::c_void as *mut std::ffi::c_void, Ordering::SeqCst);

    let config_path_box = Box::new(config_path.to_path_buf());
    MAIN_CONFIG_PATH.store(Box::into_raw(config_path_box) as *mut std::ffi::c_void, Ordering::SeqCst);

    let _ = unsafe { create_main_window(parent, config, dictionary, history, i18n) };
}

#[cfg(target_os = "windows")]
unsafe fn create_main_window(
    parent: HWND,
    config: &AppConfig,
    dictionary: &mut DictionaryManager,
    history: &HistoryManager,
    i18n: &I18n,
) -> anyhow::Result<()> {
    let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();

    let class_name = ew(MAIN_CLASS_NAME);
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(main_wnd_proc),
        hInstance: hinstance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        hbrBackground: GetSysColorBrush(COLOR_3DFACE),
        hIcon: LoadIconW(None, IDI_APPLICATION)?,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..std::mem::zeroed()
    };
    let _ = RegisterClassW(&wc);

    let title = ew(i18n.t("app.title"));
    let (x, y, w, h) = get_window_geometry(config);

    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        x, y, w, h,
        parent,
        None,
        hinstance,
        None,
    )?;

    MAIN_HWND.store(hwnd.0, Ordering::SeqCst);

    let tab_ctrl = create_tab_control(hwnd, hinstance, i18n);
    let _ = SendMessageW(tab_ctrl, WM_SETFONT, WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize), LPARAM(1));

    let settings_page = crate::settings::create_settings_page(hwnd, hinstance, config, dictionary, i18n);
    let history_page = crate::history_ui::create_history_page(hwnd, hinstance, history, i18n);
    let about_page = create_about_page(hwnd, hinstance, i18n, config);

    let pages = Box::new(TabPages {
        tab_ctrl,
        settings_page,
        history_page,
        about_page,
    });
    let pages_ptr = Box::into_raw(pages) as isize;
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, pages_ptr);

    resize_pages(hwnd, tab_ctrl, settings_page, history_page, about_page);

    let _ = ShowWindow(settings_page, SW_SHOW);
    let _ = UpdateWindow(hwnd);

    Ok(())
}

#[cfg(target_os = "windows")]
struct TabPages {
    tab_ctrl: HWND,
    settings_page: HWND,
    history_page: HWND,
    about_page: HWND,
}

#[cfg(target_os = "windows")]
fn get_window_geometry(config: &AppConfig) -> (i32, i32, i32, i32) {
    let x = config.ui.main_window_x.unwrap_or(CW_USEDEFAULT);
    let y = config.ui.main_window_y.unwrap_or(CW_USEDEFAULT);
    let w = config.ui.main_window_w;
    let h = config.ui.main_window_h;
    (x, y, w, h)
}

#[cfg(target_os = "windows")]
unsafe fn create_tab_control(hwnd: HWND, hinstance: HINSTANCE, i18n: &I18n) -> HWND {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TAB_CLASSES,
    };
    let _ = InitCommonControlsEx(&icc);

    let tab = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("SysTabControl32").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        0, 0, 0, 0,
        hwnd,
        HMENU(IDC_TABCTRL as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();

    let tab_labels = [
        i18n.t("main.tab_settings"),
        i18n.t("main.tab_history"),
        i18n.t("main.tab_about"),
    ];

    for (i, label) in tab_labels.iter().enumerate() {
        let mut wlabel = ew(label);
        let mut item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(wlabel.as_mut_ptr()),
            ..std::mem::zeroed()
        };
        let _ = SendMessageW(
            tab,
            TCM_INSERTITEM,
            WPARAM(i),
            LPARAM(&mut item as *mut TCITEMW as isize),
        );
    }

    tab
}

#[cfg(target_os = "windows")]
unsafe fn create_about_page(parent: HWND, hinstance: HINSTANCE, i18n: &I18n, config: &AppConfig) -> HWND {
    let page = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR::null(),
        WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
        0, 0, 0, 0,
        parent,
        None,
        hinstance,
        None,
    )
    .unwrap_or_default();

    let font = GetStockObject(DEFAULT_GUI_FONT);
    let mut y: i32 = 20;
    let left: i32 = 20;
    let label_w: i32 = 640;

    let title_text = format!("Qwen3-ASR Typeless v{}", VERSION);
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(&title_text).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        left, y, label_w, 24,
        page,
        HMENU(IDC_ABOUT_VERSION as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += 35;

    let desc = i18n.t("about.description");
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(desc).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        left, y, label_w, 20,
        page,
        HMENU(IDC_ABOUT_DESC as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += 30;

    let github_text = "https://github.com/LanceLRQ/qwen3-asr-service";
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(github_text).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0) | WINDOW_STYLE(SS_NOTIFY.0),
        left, y, label_w, 20,
        page,
        HMENU(IDC_ABOUT_GITHUB as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += 35;

    let status_label = i18n.t("about.asr_status");
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(&format!("{}: ", status_label)).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        left, y, 120, 20,
        page,
        None,
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    let status_text = format_asr_status(i18n, config);
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(&status_text).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        left + 130, y, 300, 20,
        page,
        HMENU(IDC_ABOUT_ASR_STATUS as *mut core::ffi::c_void),
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    y += 30;

    let lang_text = format!("Language: {} ({})", i18n.lang().display_name(), config.ui.language);
    let h = CreateWindowExW(
        WS_EX_LEFT,
        PCWSTR(ew("STATIC").as_ptr()),
        PCWSTR(ew(&lang_text).as_ptr()),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
        left, y, label_w, 20,
        page,
        None,
        hinstance,
        None,
    )
    .unwrap_or_default();
    let _ = SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

    page
}

#[cfg(target_os = "windows")]
unsafe fn resize_pages(hwnd: HWND, tab_ctrl: HWND, settings_page: HWND, history_page: HWND, about_page: HWND) {
    let mut rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut rect);

    let _ = MoveWindow(tab_ctrl, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, true);

    let _ = SendMessageW(tab_ctrl, TCM_ADJUSTRECT, WPARAM(0), LPARAM(&mut rect as *mut RECT as isize));

    let left = rect.left + 2;
    let top = rect.top + 2;
    let width = rect.right - rect.left - 4;
    let height = rect.bottom - rect.top - 4;

    for page in [settings_page, history_page, about_page] {
        let _ = MoveWindow(page, left, top, width, height, true);
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NOTIFY => {
            let nmhdr = &*(lparam.0 as *const NMHDR);
            if nmhdr.hwndFrom == get_tab_ctrl(hwnd) && nmhdr.code == TCN_SELCHANGE {
                let pages = get_pages(hwnd);
                if let Some(pages) = pages {
                    let sel = SendMessageW(pages.tab_ctrl, TCM_GETCURSEL, WPARAM(0), LPARAM(0));
                    let selected = sel.0 as i32;

                    let _ = ShowWindow(pages.settings_page, SW_HIDE);
                    let _ = ShowWindow(pages.history_page, SW_HIDE);
                    let _ = ShowWindow(pages.about_page, SW_HIDE);

                    match selected {
                        TAB_SETTINGS => { let _ = ShowWindow(pages.settings_page, SW_SHOW); }
                        TAB_HISTORY => { let _ = ShowWindow(pages.history_page, SW_SHOW); }
                        TAB_ABOUT => { let _ = ShowWindow(pages.about_page, SW_SHOW); }
                        _ => {}
                    }
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_SIZE => {
            let pages = get_pages(hwnd);
            if let Some(pages) = pages {
                resize_pages(hwnd, pages.tab_ctrl, pages.settings_page, pages.history_page, pages.about_page);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 500;
            mmi.ptMinTrackSize.y = 400;
            LRESULT(0)
        }

        WM_CLOSE => {
            save_window_geometry(hwnd);

            let pages = get_pages(hwnd);
            if let Some(pages) = pages {
                let _ = DestroyWindow(pages.settings_page);
                let _ = DestroyWindow(pages.history_page);
                let _ = DestroyWindow(pages.about_page);
                let _ = DestroyWindow(pages.tab_ctrl);
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TabPages;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
            }

            MAIN_HWND.store(std::ptr::null_mut(), Ordering::SeqCst);
            MAIN_CONFIG.store(std::ptr::null_mut(), Ordering::SeqCst);
            MAIN_DICT.store(std::ptr::null_mut(), Ordering::SeqCst);
            MAIN_HISTORY.store(std::ptr::null_mut(), Ordering::SeqCst);
            MAIN_I18N.store(std::ptr::null_mut(), Ordering::SeqCst);

            let path_ptr = MAIN_CONFIG_PATH.swap(std::ptr::null_mut(), Ordering::SeqCst);
            if !path_ptr.is_null() {
                drop(Box::from_raw(path_ptr as *mut std::path::PathBuf));
            }

            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }

        WM_DESTROY => {
            MAIN_HWND.store(std::ptr::null_mut(), Ordering::SeqCst);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(target_os = "windows")]
unsafe fn get_tab_ctrl(hwnd: HWND) -> HWND {
    let pages = get_pages(hwnd);
    match pages {
        Some(p) => p.tab_ctrl,
        None => HWND::default(),
    }
}

#[cfg(target_os = "windows")]
unsafe fn get_pages(hwnd: HWND) -> Option<&'static TabPages> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TabPages;
    if ptr.is_null() {
        None
    } else {
        Some(&*ptr)
    }
}

#[cfg(target_os = "windows")]
fn save_window_geometry(hwnd: HWND) {
    let config_ptr = MAIN_CONFIG.load(Ordering::SeqCst);
    if config_ptr.is_null() {
        return;
    }

    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };

    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
    if style as u32 & WS_MINIMIZE.0 != 0 {
        return;
    }

    let config = unsafe { &mut *(config_ptr as *mut AppConfig) };
    config.ui.main_window_x = Some(rect.left);
    config.ui.main_window_y = Some(rect.top);
    config.ui.main_window_w = rect.right - rect.left;
    config.ui.main_window_h = rect.bottom - rect.top;

    let path_ptr = MAIN_CONFIG_PATH.load(Ordering::SeqCst);
    if !path_ptr.is_null() {
        let config_path = unsafe { &*(path_ptr as *const std::path::PathBuf) };
        if let Err(e) = config.save(config_path) {
            log::warn!("Failed to save window geometry: {}", e);
        }
    }
}

#[cfg(target_os = "windows")]
pub fn close_main_window() {
    let raw = MAIN_HWND.load(Ordering::SeqCst);
    if !raw.is_null() {
        let hwnd = HWND(raw);
        if unsafe { IsWindow(hwnd).as_bool() } {
            let _ = unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
    }
}

#[cfg(target_os = "windows")]
fn ew(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0u16)).collect()
}

// ── Linux implementation ──────────────────────────────────────────────

#[cfg(target_os = "linux")]
use gtk4::prelude::*;

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "linux")]
static MAIN_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
static mut MAIN_GTK_WINDOW_WEAK: Option<gtk4::glib::WeakRef<gtk4::Window>> = None;

#[cfg(target_os = "linux")]
pub fn show_main_window(
    config: &mut AppConfig,
    config_path: &std::path::Path,
    dictionary: &mut DictionaryManager,
    history: &HistoryManager,
    i18n: &I18n,
) {
    unsafe {
        if let Some(ref weak) = MAIN_GTK_WINDOW_WEAK {
            if let Some(existing) = weak.upgrade() {
                if existing.is_visible() {
                    existing.present();
                    return;
                }
            }
        }
    }

    let window = gtk4::Window::builder()
        .title(i18n.t("app.title"))
        .default_width(config.ui.main_window_w)
        .default_height(config.ui.main_window_h)
        .build();

    let notebook = gtk4::Notebook::new();

    let settings_label = gtk4::Label::new(Some(i18n.t("main.tab_settings")));
    let settings_page = crate::settings::create_settings_page_gtk(config, dictionary, i18n);
    notebook.append_page(&settings_page, Some(&settings_label));

    let history_label = gtk4::Label::new(Some(i18n.t("main.tab_history")));
    let history_page = crate::history_ui::create_history_page_gtk(history, i18n);
    notebook.append_page(&history_page, Some(&history_label));

    let about_label = gtk4::Label::new(Some(i18n.t("main.tab_about")));
    let about_page = create_about_page_gtk(i18n, config);
    notebook.append_page(&about_page, Some(&about_label));

    window.set_child(Some(&notebook));

    let config_path_owned = config_path.to_path_buf();
    let config_ptr = config as *mut AppConfig as *mut std::ffi::c_void;
    window.connect_close_request(move |_win| {
        MAIN_WINDOW_OPEN.store(false, Ordering::SeqCst);
        unsafe {
            MAIN_GTK_WINDOW_WEAK = None;
        }
        let config = unsafe { &mut *(config_ptr as *mut AppConfig) };
        let _ = config.save(&config_path_owned);
        gtk4::glib::Propagation::Proceed
    });

    let weak = gtk4::glib::WeakRef::new();
    weak.set(Some(&window));
    unsafe {
        MAIN_GTK_WINDOW_WEAK = Some(weak);
    }
    MAIN_WINDOW_OPEN.store(true, Ordering::SeqCst);

    window.show();
}

#[cfg(target_os = "linux")]
fn create_about_page_gtk(i18n: &I18n, config: &AppConfig) -> gtk4::Box {
    let box_ = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    box_.set_margin_top(20);
    box_.set_margin_bottom(20);
    box_.set_margin_start(20);
    box_.set_margin_end(20);

    let title = format!("Qwen3-ASR Typeless v{}", VERSION);
    let title_label = gtk4::Label::new(Some(&title));
    title_label.set_halign(gtk4::Align::Start);
    box_.append(&title_label);

    let desc = gtk4::Label::new(Some(i18n.t("about.description")));
    desc.set_halign(gtk4::Align::Start);
    box_.append(&desc);

    let github = gtk4::Label::new(Some("https://github.com/LanceLRQ/qwen3-asr-service"));
    github.set_halign(gtk4::Align::Start);
    box_.append(&github);

    let status_text = format_asr_status(i18n, config);
    let status = gtk4::Label::new(Some(&format!("{}: {}", i18n.t("about.asr_status"), status_text)));
    status.set_halign(gtk4::Align::Start);
    box_.append(&status);

    let lang_text = format!("Language: {} ({})", i18n.lang().display_name(), config.ui.language);
    let lang = gtk4::Label::new(Some(&lang_text));
    lang.set_halign(gtk4::Align::Start);
    box_.append(&lang);

    box_
}

#[cfg(target_os = "linux")]
pub fn close_main_window() {
    unsafe {
        if let Some(ref weak) = MAIN_GTK_WINDOW_WEAK {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        }
    }
}
