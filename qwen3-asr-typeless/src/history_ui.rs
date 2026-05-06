//! History viewer window for qwen3-asr-typeless.
//!
//! Uses raw Win32 API (CreateWindowExW + ListView) to display
//! dictation history entries with copy/delete functionality.
#[cfg(target_os = "windows")]
mod windows_impl {
    use std::sync::atomic::{AtomicPtr, Ordering};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::SystemServices::*;
    use windows::Win32::UI::Controls::*;
    use windows::Win32::UI::Controls::Dialogs::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::{PCWSTR, PWSTR};

    use crate::clipboard;
    use crate::history::HistoryManager;
    use crate::i18n::I18n;

    /// LVS_FULLROWSELECT is not defined in windows 0.58; it's 0x0020.
    const LVS_FULLROWSELECT: u32 = 0x0020;

    /// Extract the low-order word from a u32 value.
    fn loword(v: u32) -> u16 {
        (v & 0xFFFF) as u16
    }

    /// Extract the high-order word from a u32 value.
    fn hiword(v: u32) -> u16 {
        ((v >> 16) & 0xFFFF) as u16
    }

    // Control IDs
    const IDC_LISTVIEW: usize = 2001;
    const IDC_COPYBTN: usize = 2002;
    const IDC_DELETEBTN: usize = 2003;
    const IDC_CLOSEBTN: usize = 2004;
    const IDC_SEARCHLABEL: usize = 2005;
    const IDC_SEARCHEDIT: usize = 2006;
    const IDC_SEARCHBTN: usize = 2007;
    const IDC_EXPORTJSON: usize = 2010;
    const IDC_EXPORTCSV: usize = 2011;
    const IDC_EXPORTTXT: usize = 2012;

    // Page control IDs (for tab page child window)
    const IDC_PAGE_SEARCHLABEL: usize = 4001;
    const IDC_PAGE_SEARCHEDIT: usize = 4002;
    const IDC_PAGE_SEARCHBTN: usize = 4003;
    const IDC_PAGE_LISTVIEW: usize = 4004;
    const IDC_PAGE_COPYBTN: usize = 4005;
    const IDC_PAGE_DELETEBTN: usize = 4006;
    const IDC_PAGE_EXPORTJSON: usize = 4007;
    const IDC_PAGE_EXPORTCSV: usize = 4008;
    const IDC_PAGE_EXPORTTXT: usize = 4009;

    const HISTORY_CLASS_NAME: &str = "Qwen3AsrHistoryWnd";
    const HISTORY_PAGE_CLASS_NAME: &str = "Qwen3AsrHistoryPage";

    /// Track the open history window using AtomicPtr (HWND is not safe in statics).
    static HISTORY_HWND: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    /// Show the history viewer window. If already open, bring to front.
    pub fn show_history_window(history: &HistoryManager, parent: HWND) {
        let raw = HISTORY_HWND.load(Ordering::SeqCst);
        if !raw.is_null() {
            let hwnd = HWND(raw);
            if unsafe { IsWindow(hwnd).as_bool() } {
                let _ = unsafe { SetForegroundWindow(hwnd) };
                return;
            }
        }

        let _ = unsafe { create_history_window(history, parent) };
    }

    unsafe fn create_history_window(history: &HistoryManager, parent: HWND) -> anyhow::Result<()> {
        let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();

        // Register window class (ignore failure — may already be registered)
        let class_name = ew(HISTORY_CLASS_NAME);
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(history_wnd_proc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_3DFACE),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..std::mem::zeroed()
        };
        let _ = RegisterClassW(&wc);

        let title = ew("Dictation History");
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            450,
            parent,
            None,
            hinstance,
            None,
        )?;

        // Store the HistoryManager pointer in window user data so WndProc can access it.
        // Safe: HistoryManager is owned by AppContext on the main thread, and this window
        // runs on the same thread (same message loop), so no cross-thread access.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, history as *const HistoryManager as isize);

        // Store the HWND
        HISTORY_HWND.store(hwnd.0, Ordering::SeqCst);

        // Create controls
        let font = GetStockObject(DEFAULT_GUI_FONT);
        create_history_controls(hwnd, hinstance, font);

        // Populate the listview
        populate_listview(hwnd, history.list());

        // Center on screen
        center_window(hwnd);
        let _ = UpdateWindow(hwnd);

        // Modeless: the main app's GetMessageW loop will dispatch messages
        // for this window. We just create it and return.
        Ok(())
    }

    unsafe fn create_history_controls(hwnd: HWND, hinstance: HINSTANCE, font: HGDIOBJ) {
        // Search bar at top
        let sl = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("STATIC").as_ptr()),
            PCWSTR(ew("Search:").as_ptr()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
            10, 8, 50, 20,
            hwnd,
            HMENU(IDC_SEARCHLABEL as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let se = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("EDIT").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            65, 6, 400, 22,
            hwnd,
            HMENU(IDC_SEARCHEDIT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(se, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let sb = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("Search").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            475, 5, 70, 24,
            hwnd,
            HMENU(IDC_SEARCHBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sb, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // ListView — main area
        InitCommonControls();
        let lv = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("SysListView32").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(LVS_REPORT) | WINDOW_STYLE(LVS_SINGLESEL) | WINDOW_STYLE(LVS_FULLROWSELECT),
            10, 35, 660, 310,
            hwnd,
            HMENU(IDC_LISTVIEW as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(lv, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // Add columns
        add_listview_column(lv, 0, "Time", 130);
        add_listview_column(lv, 1, "Text", 310);
        add_listview_column(lv, 2, "Status", 65);
        add_listview_column(lv, 3, "Mode", 65);
        add_listview_column(lv, 4, "Duration", 65);

        // Buttons at bottom
        let cb = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("Copy").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            260, 355, 80, 28,
            hwnd,
            HMENU(IDC_COPYBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(cb, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let db = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("Delete").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            350, 355, 80, 28,
            hwnd,
            HMENU(IDC_DELETEBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(db, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let ej = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("JSON").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            10, 355, 60, 28,
            hwnd,
            HMENU(IDC_EXPORTJSON as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(ej, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let ec = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("CSV").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            75, 355, 60, 28,
            hwnd,
            HMENU(IDC_EXPORTCSV as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(ec, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let et = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("TXT").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            140, 355, 60, 28,
            hwnd,
            HMENU(IDC_EXPORTTXT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(et, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let clb = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew("Close").as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            570, 355, 80, 28,
            hwnd,
            HMENU(IDC_CLOSEBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(clb, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    }

    unsafe fn add_listview_column(lv: HWND, index: i32, title: &str, width: i32) {
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

    /// Populate the ListView with history entries.
    unsafe fn populate_listview(hwnd: HWND, entries: &[crate::history::HistoryEntry]) {
        let lv = match GetDlgItem(hwnd, IDC_LISTVIEW as i32) {
            Ok(h) if !h.is_invalid() => h,
            _ => return,
        };

        // Clear existing items
        let _ = SendMessageW(lv, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));

        for (i, entry) in entries.iter().enumerate() {
            let mut time_str = ew(&entry.formatted_timestamp());
            let display_text = if entry.is_cancelled() {
                format!("[Cancelled] {}", entry.text)
            } else {
                entry.text.clone()
            };
            let mut text_str = ew(&display_text);
            let mut status_str = ew(&entry.status);
            let mut mode_str = ew(&entry.mode);
            let mut dur_str = ew(&format!("{:.1}s", entry.duration_secs));

            let mut item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                iSubItem: 0,
                pszText: PWSTR(time_str.as_mut_ptr()),
                ..std::mem::zeroed()
            };
            let _ = SendMessageW(lv, LVM_INSERTITEMW, WPARAM(0), LPARAM(&mut item as *mut _ as isize));

            // Sub-items
            item.iSubItem = 1;
            item.pszText = PWSTR(text_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 2;
            item.pszText = PWSTR(status_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 3;
            item.pszText = PWSTR(mode_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 4;
            item.pszText = PWSTR(dur_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));
        }
    }

    /// Populate the ListView with history entries using a specific control ID.
    unsafe fn populate_listview_by_id(hwnd: HWND, ctrl_id: usize, entries: &[crate::history::HistoryEntry]) {
        let lv = match GetDlgItem(hwnd, ctrl_id as i32) {
            Ok(h) if !h.is_invalid() => h,
            _ => return,
        };

        // Clear existing items
        let _ = SendMessageW(lv, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));

        for (i, entry) in entries.iter().enumerate() {
            let mut time_str = ew(&entry.formatted_timestamp());
            let display_text = if entry.is_cancelled() {
                format!("[Cancelled] {}", entry.text)
            } else {
                entry.text.clone()
            };
            let mut text_str = ew(&display_text);
            let mut status_str = ew(&entry.status);
            let mut mode_str = ew(&entry.mode);
            let mut dur_str = ew(&format!("{:.1}s", entry.duration_secs));

            let mut item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                iSubItem: 0,
                pszText: PWSTR(time_str.as_mut_ptr()),
                ..std::mem::zeroed()
            };
            let _ = SendMessageW(lv, LVM_INSERTITEMW, WPARAM(0), LPARAM(&mut item as *mut _ as isize));

            // Sub-items
            item.iSubItem = 1;
            item.pszText = PWSTR(text_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 2;
            item.pszText = PWSTR(status_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 3;
            item.pszText = PWSTR(mode_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));

            item.iSubItem = 4;
            item.pszText = PWSTR(dur_str.as_mut_ptr());
            let _ = SendMessageW(lv, LVM_SETITEMTEXTW, WPARAM(i), LPARAM(&mut item as *mut _ as isize));
        }
    }

    /// Get the text of the currently selected ListView item using a specific control ID.
    unsafe fn get_selected_item_text_by_id(hwnd: HWND, ctrl_id: usize) -> Option<String> {
        let lv = GetDlgItem(hwnd, ctrl_id as i32).unwrap_or_default();
        if lv.is_invalid() {
            return None;
        }

        let sel = SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0 as i32;
        if sel < 0 {
            return None;
        }

        let mut buf = [0u16; 4096];
        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: sel,
            iSubItem: 1,
            pszText: PWSTR(buf.as_mut_ptr()),
            cchTextMax: buf.len() as i32,
            ..std::mem::zeroed()
        };
        let _ = SendMessageW(lv, LVM_GETITEMTEXTW, WPARAM(sel as usize), LPARAM(&mut item as *mut _ as isize));

        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// Get the index of the currently selected ListView item using a specific control ID.
    unsafe fn get_selected_index_by_id(hwnd: HWND, ctrl_id: usize) -> i32 {
        let lv = GetDlgItem(hwnd, ctrl_id as i32).unwrap_or_default();
        if lv.is_invalid() {
            return -1;
        }
        SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0 as i32
    }

    /// Get the text of the currently selected ListView item (sub-item 1 = "Text" column).
    unsafe fn get_selected_item_text(hwnd: HWND) -> Option<String> {
        let lv = GetDlgItem(hwnd, IDC_LISTVIEW as i32).unwrap_or_default();
        if lv.is_invalid() {
            return None;
        }

        let sel = SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0 as i32;
        if sel < 0 {
            return None;
        }

        let mut buf = [0u16; 4096];
        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: sel,
            iSubItem: 1,
            pszText: PWSTR(buf.as_mut_ptr()),
            cchTextMax: buf.len() as i32,
            ..std::mem::zeroed()
        };
        let _ = SendMessageW(lv, LVM_GETITEMTEXTW, WPARAM(sel as usize), LPARAM(&mut item as *mut _ as isize));

        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// Get the index of the currently selected ListView item.
    unsafe fn get_selected_index(hwnd: HWND) -> i32 {
        let lv = GetDlgItem(hwnd, IDC_LISTVIEW as i32).unwrap_or_default();
        if lv.is_invalid() {
            return -1;
        }
        SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(usize::MAX), LPARAM(LVNI_SELECTED as isize)).0 as i32
    }

    unsafe extern "system" fn history_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Retrieve HistoryManager pointer from window user data
        let history_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HistoryManager;
        let history = if history_ptr.is_null() {
            None
        } else {
            Some(&mut *history_ptr)
        };

        match msg {
            WM_COMMAND => {
                let cmd_id = loword(wparam.0 as u32) as usize;
                let notification = hiword(wparam.0 as u32) as usize;

                if notification == BN_CLICKED as usize {
                    match cmd_id {
                        IDC_COPYBTN => {
                            if let Some(text) = get_selected_item_text(hwnd) {
                                if let Err(e) = clipboard::copy_text(&text) {
                                    log::error!("Failed to copy to clipboard: {}", e);
                                } else {
                                    log::info!("Copied text to clipboard");
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_DELETEBTN => {
                            if let Some(history) = history {
                                let idx = get_selected_index(hwnd);
                                if idx >= 0 {
                                    let entries = history.list();
                                    if let Some(entry) = entries.get(idx as usize) {
                                        let id = entry.id.clone();
                                        if let Err(e) = history.delete(&id) {
                                            log::error!("Failed to delete history entry: {}", e);
                                        } else {
                                            log::info!("Deleted history entry {}", id);
                                            populate_listview(hwnd, history.list());
                                        }
                                    }
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_CLOSEBTN => {
                            let _ = DestroyWindow(hwnd);
                            return LRESULT(0);
                        }
                        IDC_SEARCHBTN => {
                            if let Some(history) = history {
                                let query = get_edit_text(hwnd, IDC_SEARCHEDIT);
                                if query.is_empty() {
                                    populate_listview(hwnd, history.list());
                                } else {
                                    let matches: Vec<crate::history::HistoryEntry> =
                                        history.search(&query).into_iter().cloned().collect();
                                    populate_listview(hwnd, &matches);
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_EXPORTJSON => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "json");
                            }
                            return LRESULT(0);
                        }
                        IDC_EXPORTCSV => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "csv");
                            }
                            return LRESULT(0);
                        }
                        IDC_EXPORTTXT => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "txt");
                            }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NOTIFY => {
                // Handle ListView double-click to show entry details
                let nmhdr = &*(lparam.0 as *const NMHDR);
                if nmhdr.idFrom == IDC_LISTVIEW && nmhdr.code == NM_DBLCLK {
                    if let Some(history) = history {
                        let idx = get_selected_index(hwnd);
                        if idx >= 0 {
                            let entries = history.list();
                            if let Some(entry) = entries.get(idx as usize) {
                                show_detail_dialog(entry, hwnd);
                            }
                        }
                    }
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // Clear user data so dangling pointer is not used after window is destroyed
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                HISTORY_HWND.store(std::ptr::null_mut(), Ordering::SeqCst);
                LRESULT(0)
            }
            WM_SIZE => {
                // Resize the ListView to fit the window
                let width = loword(lparam.0 as u32) as i32;
                let height = hiword(lparam.0 as u32) as i32;
                let lv = GetDlgItem(hwnd, IDC_LISTVIEW as i32).unwrap_or_default();
                if !lv.is_invalid() {
                    let _ = MoveWindow(lv, 10, 35, width - 20, height - 75, true);
                }
                // Move buttons to bottom
                let btn_y = height - 40;
                let export_json_btn = GetDlgItem(hwnd, IDC_EXPORTJSON as i32).unwrap_or_default();
                let _ = MoveWindow(export_json_btn, 10, btn_y, 60, 28, true);
                let export_csv_btn = GetDlgItem(hwnd, IDC_EXPORTCSV as i32).unwrap_or_default();
                let _ = MoveWindow(export_csv_btn, 75, btn_y, 60, 28, true);
                let export_txt_btn = GetDlgItem(hwnd, IDC_EXPORTTXT as i32).unwrap_or_default();
                let _ = MoveWindow(export_txt_btn, 140, btn_y, 60, 28, true);
                let copy_btn = GetDlgItem(hwnd, IDC_COPYBTN as i32).unwrap_or_default();
                let _ = MoveWindow(copy_btn, width - 340, btn_y, 80, 28, true);
                let del_btn = GetDlgItem(hwnd, IDC_DELETEBTN as i32).unwrap_or_default();
                let _ = MoveWindow(del_btn, width - 250, btn_y, 80, 28, true);
                let close_btn = GetDlgItem(hwnd, IDC_CLOSEBTN as i32).unwrap_or_default();
                let _ = MoveWindow(close_btn, width - 90, btn_y, 80, 28, true);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
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

    /// Show a Save File dialog and return the chosen path, or None if cancelled.
    unsafe fn save_file_dialog(hwnd: HWND, default_name: &str, filter: &str) -> Option<std::path::PathBuf> {
        let mut file_buf = [0u16; 260];
        let default_name_w = ew(default_name);
        let name_len = default_name_w.len().min(260) - 1; // exclude null
        file_buf[..name_len].copy_from_slice(&default_name_w[..name_len]);

        let filter_w: Vec<u16> = filter
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();

        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = hwnd;
        ofn.lpstrFilter = PCWSTR(filter_w.as_ptr());
        ofn.lpstrFile = PWSTR(file_buf.as_mut_ptr());
        ofn.nMaxFile = file_buf.len() as u32;
        ofn.Flags = OFN_OVERWRITEPROMPT;

        if GetSaveFileNameW(&mut ofn).as_bool() {
            let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
            if len > 0 {
                let path_str = String::from_utf16_lossy(&file_buf[..len]);
                return Some(std::path::PathBuf::from(path_str));
            }
        }
        None
    }

    /// Export history to file via Save As dialog.
    fn do_export(history: &HistoryManager, hwnd: HWND, format: &str) {
        let (content, default_name, filter) = match format {
            "json" => (
                history.export_json(),
                "history.json",
                "JSON Files\0*.json\0All Files\0*.*\0",
            ),
            "csv" => (
                history.export_csv(),
                "history.csv",
                "CSV Files\0*.csv\0All Files\0*.*\0",
            ),
            "txt" => (
                history.export_txt(),
                "history.txt",
                "Text Files\0*.txt\0All Files\0*.*\0",
            ),
            _ => return,
        };

        let content = match content {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to export history as {}: {}", format, e);
                return;
            }
        };

        let path = unsafe { save_file_dialog(hwnd, default_name, filter) };
        if let Some(path) = path {
            match std::fs::write(&path, &content) {
                Ok(()) => log::info!("History exported to {:?}", path),
                Err(e) => log::error!("Failed to write export file {:?}: {}", path, e),
            }
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

    /// Create a history page as a child window suitable for embedding in a Tab control.
    ///
    /// Returns the HWND of the child page window. The page is not visible initially
    /// (the caller should show/hide it based on tab selection).
    pub fn create_history_page(parent: HWND, hinstance: HINSTANCE, history: &HistoryManager, i18n: &I18n) -> HWND {
        unsafe {
            // Register the page window class (ignore failure — may already be registered)
            let class_name = ew(HISTORY_PAGE_CLASS_NAME);
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(history_page_wnd_proc),
                hInstance: hinstance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hbrBackground: GetSysColorBrush(COLOR_3DFACE),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..std::mem::zeroed()
            };
            let _ = RegisterClassW(&wc);

            // Create as a child window filling the parent's client area
            let mut parent_rect = RECT::default();
            let _ = GetClientRect(parent, &mut parent_rect);
            let pw = parent_rect.right - parent_rect.left;
            let ph = parent_rect.bottom - parent_rect.top;

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN, // not visible initially
                0,
                0,
                pw,
                ph,
                parent,
                None,
                hinstance,
                None,
            )
            .unwrap_or_default();

            if hwnd.is_invalid() {
                return hwnd;
            }

            // Store the HistoryManager pointer in window user data
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, history as *const HistoryManager as isize);

            // Create controls
            let font = GetStockObject(DEFAULT_GUI_FONT);
            create_history_page_controls(hwnd, hinstance, font, i18n);

            // Populate the listview
            populate_listview_by_id(hwnd, IDC_PAGE_LISTVIEW, history.list());

            hwnd
        }
    }

    unsafe fn create_history_page_controls(hwnd: HWND, hinstance: HINSTANCE, font: HGDIOBJ, i18n: &I18n) {
        // Search bar at top
        let sl = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("STATIC").as_ptr()),
            PCWSTR(ew(i18n.t("history.search")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_LEFT.0),
            10, 8, 50, 20,
            hwnd,
            HMENU(IDC_PAGE_SEARCHLABEL as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let se = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("EDIT").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            65, 6, 400, 22,
            hwnd,
            HMENU(IDC_PAGE_SEARCHEDIT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(se, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let sb = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("history.search_btn")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            475, 5, 70, 24,
            hwnd,
            HMENU(IDC_PAGE_SEARCHBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(sb, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // ListView — main area
        InitCommonControls();
        let lv = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            PCWSTR(ew("SysListView32").as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(LVS_REPORT) | WINDOW_STYLE(LVS_SINGLESEL) | WINDOW_STYLE(LVS_FULLROWSELECT),
            10, 35, 660, 310,
            hwnd,
            HMENU(IDC_PAGE_LISTVIEW as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(lv, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // Add columns with i18n
        add_listview_column(lv, 0, i18n.t("history.time"), 130);
        add_listview_column(lv, 1, i18n.t("history.text"), 310);
        add_listview_column(lv, 2, i18n.t("history.status"), 65);
        add_listview_column(lv, 3, i18n.t("history.mode"), 65);
        add_listview_column(lv, 4, i18n.t("history.duration"), 65);

        // Export buttons at bottom-left
        let ej = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("export.json_btn")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            10, 355, 60, 28,
            hwnd,
            HMENU(IDC_PAGE_EXPORTJSON as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(ej, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let ec = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("export.csv_btn")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            75, 355, 60, 28,
            hwnd,
            HMENU(IDC_PAGE_EXPORTCSV as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(ec, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let et = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("export.txt_btn")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            140, 355, 60, 28,
            hwnd,
            HMENU(IDC_PAGE_EXPORTTXT as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(et, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // Copy and Delete buttons
        let cb = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("history.copy")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            260, 355, 80, 28,
            hwnd,
            HMENU(IDC_PAGE_COPYBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(cb, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        let db = CreateWindowExW(
            WS_EX_LEFT,
            PCWSTR(ew("BUTTON").as_ptr()),
            PCWSTR(ew(i18n.t("dict.delete")).as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            350, 355, 80, 28,
            hwnd,
            HMENU(IDC_PAGE_DELETEBTN as *mut core::ffi::c_void),
            hinstance,
            None,
        )
        .unwrap_or_default();
        let _ = SendMessageW(db, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));

        // No Close button — the tab page doesn't need one
    }

    unsafe extern "system" fn history_page_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Retrieve HistoryManager pointer from window user data
        let history_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HistoryManager;
        let history = if history_ptr.is_null() {
            None
        } else {
            Some(&mut *history_ptr)
        };

        match msg {
            WM_COMMAND => {
                let cmd_id = loword(wparam.0 as u32) as usize;
                let notification = hiword(wparam.0 as u32) as usize;

                if notification == BN_CLICKED as usize {
                    match cmd_id {
                        IDC_PAGE_COPYBTN => {
                            if let Some(text) = get_selected_item_text_by_id(hwnd, IDC_PAGE_LISTVIEW) {
                                if let Err(e) = clipboard::copy_text(&text) {
                                    log::error!("Failed to copy to clipboard: {}", e);
                                } else {
                                    log::info!("Copied text to clipboard");
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_PAGE_DELETEBTN => {
                            if let Some(history) = history {
                                let idx = get_selected_index_by_id(hwnd, IDC_PAGE_LISTVIEW);
                                if idx >= 0 {
                                    let entries = history.list();
                                    if let Some(entry) = entries.get(idx as usize) {
                                        let id = entry.id.clone();
                                        if let Err(e) = history.delete(&id) {
                                            log::error!("Failed to delete history entry: {}", e);
                                        } else {
                                            log::info!("Deleted history entry {}", id);
                                            populate_listview_by_id(hwnd, IDC_PAGE_LISTVIEW, history.list());
                                        }
                                    }
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_PAGE_SEARCHBTN => {
                            if let Some(history) = history {
                                let query = get_edit_text(hwnd, IDC_PAGE_SEARCHEDIT);
                                if query.is_empty() {
                                    populate_listview_by_id(hwnd, IDC_PAGE_LISTVIEW, history.list());
                                } else {
                                    let matches: Vec<crate::history::HistoryEntry> =
                                        history.search(&query).into_iter().cloned().collect();
                                    populate_listview_by_id(hwnd, IDC_PAGE_LISTVIEW, &matches);
                                }
                            }
                            return LRESULT(0);
                        }
                        IDC_PAGE_EXPORTJSON => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "json");
                            }
                            return LRESULT(0);
                        }
                        IDC_PAGE_EXPORTCSV => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "csv");
                            }
                            return LRESULT(0);
                        }
                        IDC_PAGE_EXPORTTXT => {
                            if let Some(history) = history {
                                do_export(history, hwnd, "txt");
                            }
                            return LRESULT(0);
                        }
                        _ => {}
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NOTIFY => {
                // Handle ListView double-click to show entry details
                let nmhdr = &*(lparam.0 as *const NMHDR);
                if nmhdr.idFrom == IDC_PAGE_LISTVIEW && nmhdr.code == NM_DBLCLK {
                    if let Some(history) = history {
                        let idx = get_selected_index_by_id(hwnd, IDC_PAGE_LISTVIEW);
                        if idx >= 0 {
                            let entries = history.list();
                            if let Some(entry) = entries.get(idx as usize) {
                                show_detail_dialog(entry, hwnd);
                            }
                        }
                    }
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DESTROY => {
                // Clear user data so dangling pointer is not used after window is destroyed
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                LRESULT(0)
            }
            WM_SIZE => {
                // Resize the ListView to fit the page
                let width = loword(lparam.0 as u32) as i32;
                let height = hiword(lparam.0 as u32) as i32;
                let lv = GetDlgItem(hwnd, IDC_PAGE_LISTVIEW as i32).unwrap_or_default();
                if !lv.is_invalid() {
                    let _ = MoveWindow(lv, 10, 35, width - 20, height - 75, true);
                }
                // Move buttons to bottom
                let btn_y = height - 40;
                let export_json_btn = GetDlgItem(hwnd, IDC_PAGE_EXPORTJSON as i32).unwrap_or_default();
                let _ = MoveWindow(export_json_btn, 10, btn_y, 60, 28, true);
                let export_csv_btn = GetDlgItem(hwnd, IDC_PAGE_EXPORTCSV as i32).unwrap_or_default();
                let _ = MoveWindow(export_csv_btn, 75, btn_y, 60, 28, true);
                let export_txt_btn = GetDlgItem(hwnd, IDC_PAGE_EXPORTTXT as i32).unwrap_or_default();
                let _ = MoveWindow(export_txt_btn, 140, btn_y, 60, 28, true);
                let copy_btn = GetDlgItem(hwnd, IDC_PAGE_COPYBTN as i32).unwrap_or_default();
                let _ = MoveWindow(copy_btn, width - 340, btn_y, 80, 28, true);
                let del_btn = GetDlgItem(hwnd, IDC_PAGE_DELETEBTN as i32).unwrap_or_default();
                let _ = MoveWindow(del_btn, width - 250, btn_y, 80, 28, true);
                // No Close button to reposition
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Show a detail dialog for a history entry, displaying both the processed
    /// text and the original ASR text if they differ.
    unsafe fn show_detail_dialog(entry: &crate::history::HistoryEntry, parent: HWND) {
        let mut detail = format!("{}\n\nTime: {}\nMode: {}\nDuration: {:.1}s\nStatus: {}",
            entry.text,
            entry.formatted_timestamp(),
            entry.mode,
            entry.duration_secs,
            entry.status,
        );

        // Show raw ASR text if it differs from the processed text
        if let Some(raw) = &entry.raw_text {
            if raw != &entry.text {
                detail = format!("Processed:\n{}\n\n─── Original ASR ───\n{}", entry.text, raw);
            }
        }

        let title = ew("History Detail");
        let msg = ew(&detail);
        MessageBoxW(parent, PCWSTR(msg.as_ptr()), PCWSTR(title.as_ptr()), MB_OK);
    }

    /// Encode a Rust string as a null-terminated UTF-16 wide string.
    fn ew(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0u16)).collect()
    }

}

#[cfg(target_os = "windows")]
pub use windows_impl::show_history_window;

#[cfg(target_os = "windows")]
pub use windows_impl::create_history_page;

#[cfg(target_os = "linux")]
use gtk4::prelude::*;

#[cfg(target_os = "linux")]
fn populate_gtk_model(model: &gtk4::ListStore, entries: &[crate::history::HistoryEntry]) {
    for entry in entries {
        let display_text = if entry.is_cancelled() {
            format!("[Cancelled] {}", entry.text)
        } else {
            entry.text.clone()
        };
        model.insert_with_values(
            None,
            &[
                (0, &entry.formatted_timestamp()),
                (1, &display_text),
                (2, &entry.status),
                (3, &entry.mode),
                (4, &format!("{:.1}s", entry.duration_secs)),
            ],
        );
    }
}

#[cfg(target_os = "linux")]
pub fn create_history_page_gtk(history: &crate::history::HistoryManager, i18n: &crate::i18n::I18n) -> gtk4::Box {
    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    vbox.set_margin_top(8);
    vbox.set_margin_bottom(8);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);

    let search_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let search_entry = gtk4::Entry::new();
    search_entry.set_hexpand(true);
    search_entry.set_placeholder_text(Some(i18n.t("history.search")));
    let search_btn = gtk4::Button::with_label(i18n.t("history.search_btn"));
    search_bar.append(&search_entry);
    search_bar.append(&search_btn);
    vbox.append(&search_bar);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic);

    let model = gtk4::ListStore::new(&[
        gtk4::glib::Type::STRING,
        gtk4::glib::Type::STRING,
        gtk4::glib::Type::STRING,
        gtk4::glib::Type::STRING,
        gtk4::glib::Type::STRING,
    ]);

    let tree_view = gtk4::TreeView::with_model(&model);

    let add_column = |title: &str, col_idx: i32| -> gtk4::TreeViewColumn {
        let renderer = gtk4::CellRendererText::new();
        let column = gtk4::TreeViewColumn::new();
        column.set_title(title);
        column.pack_start(&renderer, true);
        column.add_attribute(&renderer, "text", col_idx);
        column.set_resizable(true);
        column
    };

    tree_view.append_column(&add_column(i18n.t("history.time"), 0));
    tree_view.append_column(&add_column(i18n.t("history.text"), 1));
    tree_view.append_column(&add_column(i18n.t("history.status"), 2));
    tree_view.append_column(&add_column(i18n.t("history.mode"), 3));
    tree_view.append_column(&add_column(i18n.t("history.duration"), 4));

    populate_gtk_model(&model, history.list());

    scrolled.set_child(Some(&tree_view));
    vbox.append(&scrolled);

    let btn_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let export_json_btn = gtk4::Button::with_label(i18n.t("export.json_btn"));
    let export_csv_btn = gtk4::Button::with_label(i18n.t("export.csv_btn"));
    let export_txt_btn = gtk4::Button::with_label(i18n.t("export.txt_btn"));
    let copy_btn = gtk4::Button::with_label(i18n.t("history.copy"));
    let delete_btn = gtk4::Button::with_label(i18n.t("dict.delete"));
    btn_bar.append(&export_json_btn);
    btn_bar.append(&export_csv_btn);
    btn_bar.append(&export_txt_btn);
    btn_bar.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
    btn_bar.append(&copy_btn);
    btn_bar.append(&delete_btn);
    vbox.append(&btn_bar);

    let selection = tree_view.selection();
    let model_for_copy = model.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some((_model, iter)) = selection.selected() {
            let text: String = model_for_copy.get(&iter, 1);
            let display = gtk4::gdk::Display::default().unwrap();
            let clipboard = display.clipboard();
            clipboard.set_text(&text);
        }
    });

    let model_for_search = model.clone();
    let history_entries: Vec<crate::history::HistoryEntry> = history.list().to_vec();
    search_btn.connect_clicked(move |_| {
        let query = search_entry.text().to_string();
        model_for_search.clear();
        if query.is_empty() {
            populate_gtk_model(&model_for_search, &history_entries);
        } else {
            let filtered: Vec<crate::history::HistoryEntry> = history_entries
                .iter()
                .filter(|e| e.text.contains(&query) || e.status.contains(&query))
                .cloned()
                .collect();
            populate_gtk_model(&model_for_search, &filtered);
        }
    });

    vbox
}
