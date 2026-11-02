#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Condvar;

#[cfg(unix)]
use ksni::blocking::TrayMethods;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(unix)]
type IpcListener = UnixListener;
#[cfg(windows)]
type IpcListener = std::net::TcpListener;

#[cfg(windows)]
const WIN_IPC_ADDR: &str = "127.0.0.1:19091";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Path for the single-instance Unix socket (under XDG_RUNTIME_DIR when available).
#[cfg(unix)]
fn ui_sock_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::runtime_dir())
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("future-proxy-{}", nix_uid_fallback()))
        });
    let _ = std::fs::create_dir_all(&dir);
    dir.join("future-proxy-ui.sock")
}

#[cfg(unix)]
fn nix_uid_fallback() -> u32 {
    // Avoid depending on the libc crate; read from /proc.
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Become the sole UI/tray process, or hand off to the existing one.
/// - `activate_if_running`: desktop/dock launch should SHOW the existing instance.
/// - `--tray` relaunch should NOT activate (avoids opening GUI just because a tray already exists).
#[cfg(unix)]
fn take_single_instance(activate_if_running: bool) -> Option<IpcListener> {
    let path = ui_sock_path();
    // Live peer?
    if let Ok(mut stream) = UnixStream::connect(&path) {
        if activate_if_running {
            let _ = stream.write_all(b"SHOW\n");
        }
        return None;
    }
    // Stale sock file from a crashed process — remove then bind.
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Some(listener)
        }
        Err(_) => {
            // Lost a bind race: peer came up between connect and bind.
            if let Ok(mut stream) = UnixStream::connect(&path) {
                if activate_if_running {
                    let _ = stream.write_all(b"SHOW\n");
                }
                return None;
            }
            // Last resort — try bind once more; otherwise run without IPC (degraded).
            let _ = std::fs::remove_file(&path);
            UnixListener::bind(&path).ok()
        }
    }
}

#[cfg(windows)]
fn take_single_instance(activate_if_running: bool) -> Option<IpcListener> {
    use std::net::{SocketAddr, TcpListener, TcpStream};
    let addr: SocketAddr = WIN_IPC_ADDR.parse().unwrap();
    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(180)) {
        if activate_if_running {
            let _ = stream.write_all(b"SHOW\n");
        }
        return None;
    }
    match TcpListener::bind(addr) {
        Ok(listener) => Some(listener),
        Err(_) => {
            if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(180)) {
                if activate_if_running {
                    let _ = stream.write_all(b"SHOW\n");
                }
            }
            None
        }
    }
}

/// Accept SHOW commands from second Exec launches; sets the same flags as tray Activate.
fn spawn_ipc_listener(
    listener_slot: Arc<Mutex<Option<IpcListener>>>,
    flags: Arc<TrayFlags>,
    wake: Arc<Condvar>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
) {
    std::thread::spawn(move || {
        loop {
            let accept_result = {
                let guard = listener_slot.lock().unwrap();
                let Some(ref listener) = *guard else {
                    break;
                };
                let _ = listener.set_nonblocking(true);
                listener.accept()
            };
            match accept_result {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 64];
                    if let Ok(n) = stream.read(&mut buf) {
                        let msg = String::from_utf8_lossy(&buf[..n]);
                        if msg.contains("SHOW") {
                            flags.show.store(true, Ordering::SeqCst);
                            tray_notify(&flags, &wake, &egui_ctx);
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
    });
}

fn release_instance(listener_slot: &Mutex<Option<IpcListener>>) {
    *listener_slot.lock().unwrap() = None;
    #[cfg(unix)]
    {
        // Unlink sock path so the replacement process can bind immediately.
        let _ = std::fs::remove_file(ui_sock_path());
    }
    // Brief pause so the accept thread notices the cleared slot.
    std::thread::sleep(Duration::from_millis(50));
}

struct TrayFlags {
    show: AtomicBool,
    quit: AtomicBool,
}

/// Wake the main loop (window closed) and/or egui (window open).
fn tray_notify(_flags: &TrayFlags, wake: &Condvar, egui_ctx: &Mutex<Option<egui::Context>>) {
    wake.notify_all();
    if let Ok(guard) = egui_ctx.lock() {
        if let Some(ctx) = guard.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Generate a crisp 32x32 RGBA icon (Cupertino Blue rounded square with white lightning bolt).
fn build_brand_rgba_icon() -> (Vec<u8>, u32, u32) {
    let w = 32usize;
    let h = 32usize;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            // Signed distance to rounded rect [2..30, 2..30] with radius 7.0
            let qx = (fx - 16.0).abs() - 7.0;
            let qy = (fy - 16.0).abs() - 7.0;
            let dist = (qx.max(0.0).hypot(qy.max(0.0)) + qx.min(qy).max(0.0)) - 7.0;
            if dist <= 0.5 {
                let alpha = ((0.5 - dist).clamp(0.0, 1.0) * 255.0) as u8;
                // Check if inside lightning bolt polygon
                let in_upper = fy >= 6.5 && fy <= 17.5 && {
                    let t = (fy - 6.5) / 11.0;
                    let left_x = 18.5 - t * 8.5;
                    let right_x = 18.5 - t * 2.5;
                    fx >= left_x && fx <= right_x
                };
                let in_lower = fy >= 14.5 && fy <= 25.5 && {
                    let t = (fy - 14.5) / 11.0;
                    let left_x = 16.0 - t * 2.5;
                    let right_x = 22.0 - t * 8.5;
                    fx >= left_x && fx <= right_x
                };
                let idx = (y * w + x) * 4;
                if in_upper || in_lower {
                    rgba[idx] = 255;
                    rgba[idx + 1] = 255;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = alpha;
                } else {
                    rgba[idx] = 10;
                    rgba[idx + 1] = 132;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = alpha;
                }
            }
        }
    }
    (rgba, w as u32, h as u32)
}

#[cfg(unix)]
struct ProxyTray {
    flags: Arc<TrayFlags>,
    wake: Arc<Condvar>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
}

#[cfg(unix)]
impl ksni::Tray for ProxyTray {
    fn id(&self) -> String {
        "future-proxy".into()
    }
    fn icon_name(&self) -> String {
        "future-proxy".into()
    }
    fn title(&self) -> String {
        "Future Proxy".into()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "future-proxy".into(),
            title: "Future Proxy".into(),
            description: "点击打开界面".into(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.flags.show.store(true, Ordering::SeqCst);
        tray_notify(&self.flags, &self.wake, &self.egui_ctx);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "打开界面".into(),
                activate: Box::new(|this: &mut Self| {
                    this.flags.show.store(true, Ordering::SeqCst);
                    tray_notify(&this.flags, &this.wake, &this.egui_ctx);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "退出".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    this.flags.quit.store(true, Ordering::SeqCst);
                    tray_notify(&this.flags, &this.wake, &this.egui_ctx);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(unix)]
type TrayGuard = Option<ksni::blocking::Handle<ProxyTray>>;
#[cfg(windows)]
type TrayGuard = ();

#[cfg(unix)]
fn spawn_system_tray(
    flags: Arc<TrayFlags>,
    wake: Arc<Condvar>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
) -> TrayGuard {
    ProxyTray {
        flags,
        wake,
        egui_ctx,
    }
    .spawn()
    .map_err(|e| eprintln!("托盘启动失败: {e}"))
    .ok()
}

#[cfg(windows)]
mod win_os {
    use super::*;

    const WM_USER: u32 = 0x0400;
    const WM_TRAYICON: u32 = WM_USER + 1;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const NIM_ADD: u32 = 0x0000_0000;
    const NIM_DELETE: u32 = 0x0000_0002;
    const NIF_MESSAGE: u32 = 0x0000_0001;
    const NIF_ICON: u32 = 0x0000_0002;
    const NIF_TIP: u32 = 0x0000_0004;
    const MF_STRING: u32 = 0x0000_0000;
    const MF_SEPARATOR: u32 = 0x0000_0800;
    const TPM_RIGHTBUTTON: u32 = 0x0002;
    const TPM_NONOTIFY: u32 = 0x0080;
    const TPM_RETURNCMD: u32 = 0x0100;
    const PM_REMOVE: u32 = 0x0001;
    const GWLP_USERDATA: i32 = -21;

    const HKEY_CURRENT_USER: isize = -2147483647i32 as isize; // 0x80000001
    const KEY_READ: u32 = 0x20019;
    const KEY_WRITE: u32 = 0x20006;
    const REG_SZ: u32 = 1;
    const REG_DWORD: u32 = 4;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;
    const INTERNET_OPTION_REFRESH: u32 = 37;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct MSG {
        hwnd: isize,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: POINT,
    }

    #[repr(C)]
    struct WNDCLASSW {
        style: u32,
        lpfn_wnd_proc: Option<unsafe extern "system" fn(isize, u32, usize, isize) -> isize>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: isize,
        h_icon: isize,
        h_cursor: isize,
        hbr_background: isize,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
    }

    #[repr(C)]
    struct NOTIFYICONDATAW {
        cb_size: u32,
        hwnd: isize,
        u_id: u32,
        u_flags: u32,
        u_callback_message: u32,
        h_icon: isize,
        sz_tip: [u16; 128],
        dw_state: u32,
        dw_state_mask: u32,
        sz_info: [u16; 256],
        u_timeout_or_version: u32,
        sz_info_title: [u16; 64],
        dw_info_flags: u32,
        guid_item: [u32; 4],
        h_balloon_icon: isize,
    }

    #[repr(C)]
    struct PROCESS_MEMORY_COUNTERS {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(lp_wnd_class: *const WNDCLASSW) -> u16;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: isize,
            h_menu: isize,
            h_instance: isize,
            lp_param: *const std::ffi::c_void,
        ) -> isize;
        fn DefWindowProcW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn SetWindowLongPtrW(hwnd: isize, n_index: i32, dw_new_long: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: isize, n_index: i32) -> isize;
        fn PeekMessageW(lp_msg: *mut MSG, hwnd: isize, w_msg_filter_min: u32, w_msg_filter_max: u32, w_remove_msg: u32) -> i32;
        fn TranslateMessage(lp_msg: *const MSG) -> i32;
        fn DispatchMessageW(lp_msg: *const MSG) -> isize;
        fn GetCursorPos(lp_point: *mut POINT) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
        fn CreatePopupMenu() -> isize;
        fn AppendMenuW(h_menu: isize, u_flags: u32, u_id_new_item: usize, lp_new_item: *const u16) -> i32;
        fn TrackPopupMenu(
            h_menu: isize,
            u_flags: u32,
            x: i32,
            y: i32,
            n_reserved: i32,
            hwnd: isize,
            prc_rect: *const std::ffi::c_void,
        ) -> i32;
        fn DestroyMenu(h_menu: isize) -> i32;
        fn CreateIcon(
            h_instance: isize,
            n_width: i32,
            n_height: i32,
            c_planes: u8,
            c_bits_pixel: u8,
            lpb_and_bits: *const u8,
            lpb_xor_bits: *const u8,
        ) -> isize;
        fn DestroyIcon(h_icon: isize) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(dw_message: u32, lp_data: *const NOTIFYICONDATAW) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> isize;
        fn GetCurrentProcess() -> isize;
        fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> isize;
        fn CloseHandle(h_object: isize) -> i32;
        fn K32GetProcessMemoryInfo(process: isize, ppsmem_counters: *mut PROCESS_MEMORY_COUNTERS, cb: u32) -> i32;
        fn K32EmptyWorkingSet(h_process: isize) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(h_key: isize, lp_sub_key: *const u16, ul_options: u32, sam_desired: u32, phk_result: *mut isize) -> i32;
        fn RegQueryValueExW(
            h_key: isize,
            lp_value_name: *const u16,
            lp_reserved: *const u32,
            lp_type: *mut u32,
            lp_data: *mut u8,
            lpcb_data: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            h_key: isize,
            lp_value_name: *const u16,
            reserved: u32,
            dw_type: u32,
            lp_data: *const u8,
            cb_data: u32,
        ) -> i32;
        fn RegDeleteValueW(h_key: isize, lp_value_name: *const u16) -> i32;
        fn RegCloseKey(h_key: isize) -> i32;
    }

    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(h_internet: isize, dw_option: u32, lp_buffer: *const std::ffi::c_void, dw_buffer_length: u32) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct WinTrayContext {
        flags: Arc<TrayFlags>,
        wake: Arc<Condvar>,
        egui_ctx: Arc<Mutex<Option<egui::Context>>>,
    }

    unsafe extern "system" fn tray_wnd_proc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
        if msg == WM_TRAYICON {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WinTrayContext;
            if !ptr.is_null() {
                let ctx = &*ptr;
                let event = (lparam as u32) & 0xFFFF;
                if event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
                    ctx.flags.show.store(true, Ordering::SeqCst);
                    tray_notify(&ctx.flags, &ctx.wake, &ctx.egui_ctx);
                } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    SetForegroundWindow(hwnd);
                    let hmenu = CreatePopupMenu();
                    if hmenu != 0 {
                        let open_lbl = to_wide("打开 Future Proxy");
                        let quit_lbl = to_wide("退出");
                        AppendMenuW(hmenu, MF_STRING, 1001, open_lbl.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(hmenu, MF_STRING, 1002, quit_lbl.as_ptr());
                        let cmd = TrackPopupMenu(
                            hmenu,
                            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        DestroyMenu(hmenu);
                        if cmd == 1001 {
                            ctx.flags.show.store(true, Ordering::SeqCst);
                            tray_notify(&ctx.flags, &ctx.wake, &ctx.egui_ctx);
                        } else if cmd == 1002 {
                            ctx.flags.quit.store(true, Ordering::SeqCst);
                            tray_notify(&ctx.flags, &ctx.wake, &ctx.egui_ctx);
                        }
                    }
                }
            }
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    unsafe fn create_brand_hicon() -> isize {
        let (rgba, w, h) = build_brand_rgba_icon();
        let mut bgra = vec![0u8; rgba.len()];
        for i in (0..rgba.len()).step_by(4) {
            bgra[i] = rgba[i + 2];     // B
            bgra[i + 1] = rgba[i + 1]; // G
            bgra[i + 2] = rgba[i];     // R
            bgra[i + 3] = rgba[i + 3]; // A
        }
        let and_mask = vec![0u8; ((w * h) / 8) as usize];
        CreateIcon(0, w as i32, h as i32, 1, 32, and_mask.as_ptr(), bgra.as_ptr())
    }

    pub fn spawn_tray(
        flags: Arc<TrayFlags>,
        wake: Arc<Condvar>,
        egui_ctx: Arc<Mutex<Option<egui::Context>>>,
    ) {
        std::thread::spawn(move || unsafe {
            let h_instance = GetModuleHandleW(std::ptr::null());
            let class_name = to_wide("FutureProxyTrayClass");
            let wc = WNDCLASSW {
                style: 0,
                lpfn_wnd_proc: Some(tray_wnd_proc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance,
                h_icon: 0,
                h_cursor: 0,
                hbr_background: 0,
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: class_name.as_ptr(),
            };
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                -3isize, // HWND_MESSAGE
                0,
                h_instance,
                std::ptr::null(),
            );
            if hwnd == 0 {
                return;
            }

            let ctx = Box::new(WinTrayContext {
                flags: Arc::clone(&flags),
                wake,
                egui_ctx,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&*ctx as *const WinTrayContext) as isize);

            let h_icon = create_brand_hicon();
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cb_size = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hwnd = hwnd;
            nid.u_id = 1;
            nid.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.u_callback_message = WM_TRAYICON;
            nid.h_icon = h_icon;
            let tip = to_wide("Future Proxy (点击打开界面)");
            for (i, &ch) in tip.iter().take(127).enumerate() {
                nid.sz_tip[i] = ch;
            }
            Shell_NotifyIconW(NIM_ADD, &nid);

            let mut msg: MSG = std::mem::zeroed();
            while !flags.quit.load(Ordering::SeqCst) {
                while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            Shell_NotifyIconW(NIM_DELETE, &nid);
            if h_icon != 0 {
                DestroyIcon(h_icon);
            }
        });
    }

    pub fn check_system_proxy() -> bool {
        unsafe {
            let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
            let mut hkey: isize = 0;
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
                return false;
            }
            let val_name = to_wide("ProxyEnable");
            let mut data: u32 = 0;
            let mut data_len: u32 = std::mem::size_of::<u32>() as u32;
            let mut reg_type: u32 = 0;
            let status = RegQueryValueExW(
                hkey,
                val_name.as_ptr(),
                std::ptr::null(),
                &mut reg_type,
                (&mut data as *mut u32) as *mut u8,
                &mut data_len,
            );
            RegCloseKey(hkey);
            status == 0 && data == 1
        }
    }

    pub fn set_system_proxy(enable: bool, port: u16) {
        unsafe {
            let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
            let mut hkey: isize = 0;
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_WRITE, &mut hkey) != 0 {
                return;
            }
            let enable_name = to_wide("ProxyEnable");
            let enable_val: u32 = if enable { 1 } else { 0 };
            RegSetValueExW(
                hkey,
                enable_name.as_ptr(),
                0,
                REG_DWORD,
                (&enable_val as *const u32) as *const u8,
                std::mem::size_of::<u32>() as u32,
            );
            if enable {
                let server_name = to_wide("ProxyServer");
                let server_val = to_wide(&format!("127.0.0.1:{}", port));
                RegSetValueExW(
                    hkey,
                    server_name.as_ptr(),
                    0,
                    REG_SZ,
                    server_val.as_ptr() as *const u8,
                    (server_val.len() * 2) as u32,
                );
                let override_name = to_wide("ProxyOverride");
                let override_val = to_wide("localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>");
                RegSetValueExW(
                    hkey,
                    override_name.as_ptr(),
                    0,
                    REG_SZ,
                    override_val.as_ptr() as *const u8,
                    (override_val.len() * 2) as u32,
                );
            }
            RegCloseKey(hkey);
            InternetSetOptionW(0, INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null(), 0);
            InternetSetOptionW(0, INTERNET_OPTION_REFRESH, std::ptr::null(), 0);
        }
    }

    pub fn set_registry_run_autostart(enable: bool, cmd_str: &str) -> Result<(), String> {
        unsafe {
            let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let mut hkey: isize = 0;
            let rc = RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_WRITE, &mut hkey);
            if rc != 0 {
                return Err(format!("无法打开注册表启动项 ({})", rc));
            }
            let val_name = to_wide("FutureProxyCore");
            let status = if enable {
                let wide_val = to_wide(cmd_str);
                RegSetValueExW(
                    hkey,
                    val_name.as_ptr(),
                    0,
                    REG_SZ,
                    wide_val.as_ptr() as *const u8,
                    (wide_val.len() * 2) as u32,
                )
            } else {
                let s = RegDeleteValueW(hkey, val_name.as_ptr());
                if s == 2 { 0 } else { s } // ERROR_FILE_NOT_FOUND is OK
            };
            RegCloseKey(hkey);
            if status == 0 {
                Ok(())
            } else {
                Err(format!("写入注册表启动项失败 ({})", status))
            }
        }
    }

    pub fn check_registry_run_autostart() -> bool {
        unsafe {
            let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let mut hkey: isize = 0;
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
                return false;
            }
            let val_name = to_wide("FutureProxyCore");
            let mut data_len: u32 = 0;
            let status = RegQueryValueExW(
                hkey,
                val_name.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut data_len,
            );
            RegCloseKey(hkey);
            status == 0 && data_len > 0
        }
    }

    pub fn read_process_rss_kb(pid: Option<u32>) -> u64 {
        unsafe {
            let handle = match pid {
                None => GetCurrentProcess(),
                Some(p) => {
                    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, p);
                    if h == 0 {
                        return 0;
                    }
                    h
                }
            };
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ok = K32GetProcessMemoryInfo(handle, &mut pmc, pmc.cb);
            if pid.is_some() {
                CloseHandle(handle);
            }
            if ok != 0 {
                (pmc.working_set_size / 1024) as u64
            } else {
                0
            }
        }
    }

    pub fn trim_working_set() {
        unsafe {
            let _ = K32EmptyWorkingSet(GetCurrentProcess());
        }
    }
}

#[cfg(windows)]
fn spawn_system_tray(
    flags: Arc<TrayFlags>,
    wake: Arc<Condvar>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
) -> TrayGuard {
    win_os::spawn_tray(flags, wake, egui_ctx);
}

fn request_repaint_slot(egui_ctx: &Mutex<Option<egui::Context>>) {
    if let Ok(guard) = egui_ctx.lock() {
        if let Some(ctx) = guard.as_ref() {
            ctx.request_repaint();
        }
    }
}


const DEFAULT_SECRET: &str = "future-proxy-secret";
const API_HOST: &str = "127.0.0.1:9090";
const MIXED_PORT: u16 = 7890;
const PATCH_BEGIN: &str = "# === Future Proxy Auto Injected Config ===";
const PATCH_END: &str = "# === Future Proxy Injected End ===";

fn api(path: &str) -> String {
    format!("http://{}{}", API_HOST, path)
}

fn auth() -> String {
    format!("Bearer {}", DEFAULT_SECRET)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Subscription {
    id: String,
    name: String,
    url: String,
    updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ProfilesMeta {
    active: String,
    subscriptions: Vec<Subscription>,
    /// 是否启用 TUN（默认关闭，使用系统代理）
    #[serde(default)]
    tun: bool,
    /// 关闭窗口后是否保留内核在后台运行
    #[serde(default)]
    keep_core_on_exit: bool,
    /// 锁定某个地区自动选节点（None 为关闭）
    #[serde(default)]
    auto_region: Option<String>,
    /// 开机自启内核（systemd --user）
    #[serde(default)]
    autostart: bool,
}

#[derive(Clone, Debug)]
struct ProxyNode {
    name: String,
    flag: String,
    delay: i32,
    is_selected: bool,
}

struct AppState {
    config_dir: PathBuf,
    profiles: ProfilesMeta,
    nodes: Vec<ProxyNode>,
    /// 可手动选择的策略组（Selector）
    groups: Vec<String>,
    group_name: String,
    group_picked_by_user: bool,
    current_selected: String,
    search_keyword: String,
    selected_region: String,
    upload_rate: u64,
    download_rate: u64,
    is_core_alive: bool,
    core_error: String,
    is_system_proxy_on: bool,
    is_testing_delay: bool,
    is_updating_sub: bool,
    show_import_dialog: bool,
    show_settings_dialog: bool,
    import_name: String,
    import_url: String,
    status_msg: String,
    mem_gui_kb: u64,
    mem_core_kb: u64,
    /// 立即触发一次地区自动选择检查
    auto_kick: bool,
    open_popup: Option<String>,
    popup_anchor: egui::Pos2,
    popup_width: f32,
}

impl AppState {
    fn new() -> Self {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("future-proxy");
        let _ = std::fs::create_dir_all(config_dir.join("profiles"));

        ensure_geo_assets(&config_dir);

        let mut meta = ProfilesMeta {
            active: "default".to_string(),
            subscriptions: Vec::new(),
            tun: false,
            keep_core_on_exit: true,
            auto_region: None,
            autostart: false,
        };

        let json_path = config_dir.join("profiles.json");
        if let Ok(content) = std::fs::read_to_string(&json_path) {
            if let Ok(parsed) = serde_json::from_str::<ProfilesMeta>(&content) {
                meta = parsed;
            }
        }

        let mut state = Self {
            config_dir,
            profiles: meta,
            nodes: Vec::new(),
            groups: Vec::new(),
            group_name: String::new(),
            group_picked_by_user: false,
            current_selected: String::new(),
            search_keyword: String::new(),
            selected_region: "全部".to_string(),
            upload_rate: 0,
            download_rate: 0,
            is_core_alive: false,
            core_error: String::new(),
            is_system_proxy_on: check_kde_proxy_enabled(),
            is_testing_delay: false,
            is_updating_sub: false,
            show_import_dialog: false,
            show_settings_dialog: false,
            import_name: String::new(),
            import_url: String::new(),
            status_msg: "就绪".to_string(),
            mem_gui_kb: 0,
            mem_core_kb: 0,
            auto_kick: true,
            open_popup: None,
            popup_anchor: egui::Pos2::ZERO,
            popup_width: 180.0,
        };

        // 首次运行：从 clash-party 迁移订阅（连同已下载的配置文件）
        if state.profiles.subscriptions.is_empty() {
            let n = migrate_party_subscriptions(&state.config_dir, &mut state.profiles);
            if n > 0 {
                state.status_msg = format!("已从 clash-party 导入 {} 个订阅", n);
            }
        }

        // 当前激活订阅不存在时，自动激活第一个有配置文件的订阅
        let active_ok = state
            .profiles
            .subscriptions
            .iter()
            .any(|s| s.id == state.profiles.active && state.sub_file(&s.id).exists());
        if !active_ok {
            if let Some(first) = state
                .profiles
                .subscriptions
                .iter()
                .find(|s| state.sub_file(&s.id).exists())
            {
                state.profiles.active = first.id.clone();
            }
        }

        // 每次启动都根据原始订阅重新生成 config.yaml，保证注入项（端口/TUN）是最新的
        if let Err(e) = state.write_active_config() {
            state.status_msg = e;
        }
        state.save_profiles();
        state
    }

    fn sub_file(&self, id: &str) -> PathBuf {
        self.config_dir.join("profiles").join(format!("{}.yaml", id))
    }

    fn active_config(&self) -> PathBuf {
        self.config_dir.join("config.yaml")
    }

    /// 用激活订阅的原始内容生成 config.yaml
    fn write_active_config(&self) -> Result<(), String> {
        let raw = std::fs::read_to_string(self.sub_file(&self.profiles.active))
            .unwrap_or_else(|_| "proxies: []\nrules:\n  - MATCH,DIRECT\n".to_string());
        std::fs::write(self.active_config(), patch_subscription(&raw, self.profiles.tun))
            .map_err(|e| format!("✗ 写入配置失败: {}", e))
    }

    fn save_profiles(&self) {
        let json_path = self.config_dir.join("profiles.json");
        if let Ok(content) = serde_json::to_string_pretty(&self.profiles) {
            let _ = std::fs::write(json_path, content);
        }
    }
}

fn ensure_geo_assets(config_dir: &Path) {
    let user_cfg = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    let asset_files: &[&str] = &[
        "country.mmdb",
        "geosite.dat",
        #[cfg(windows)]
        "wintun.dll",
    ];

    for &file in asset_files {
        let dst = config_dir.join(file);
        if dst.exists() {
            continue;
        }
        let mut candidates = Vec::new();
        if let Some(ref ed) = exe_dir {
            candidates.push(ed.join(file));
            candidates.push(ed.join("resources").join(file));
        }
        candidates.push(PathBuf::from("/usr/share/future-proxy").join(file));
        candidates.push(user_cfg.join("mihomo").join(file));
        candidates.push(user_cfg.join("mihomo-party/work").join(file));

        if let Some(src) = candidates.iter().find(|p| p.exists()) {
            let _ = std::fs::copy(src, &dst);
        }
    }
}

/// 读取 ~/.config/mihomo-party/profile.yaml 中的远程订阅，并复制其已下载的配置
fn migrate_party_subscriptions(config_dir: &Path, meta: &mut ProfilesMeta) -> usize {
    let party_dir = dirs::config_dir().unwrap_or_default().join("mihomo-party");
    let Ok(content) = std::fs::read_to_string(party_dir.join("profile.yaml")) else {
        return 0;
    };
    let re_id = regex::Regex::new(r"^\s*-\s*id:\s*(\S+)").unwrap();
    let re_name = regex::Regex::new(r"^\s+name:\s*(.+)$").unwrap();
    let re_url = regex::Regex::new(r"^\s+url:\s*(https?://\S+)").unwrap();

    let mut items: Vec<(String, String, String)> = Vec::new();
    let (mut id, mut name, mut url) = (String::new(), String::new(), String::new());
    let flush = |id: &mut String, name: &mut String, url: &mut String, items: &mut Vec<_>| {
        if !id.is_empty() && !url.is_empty() {
            let n = if name.is_empty() { id.clone() } else { name.clone() };
            items.push((id.clone(), n, url.clone()));
        }
        id.clear();
        name.clear();
        url.clear();
    };
    for line in content.lines() {
        if let Some(c) = re_id.captures(line) {
            flush(&mut id, &mut name, &mut url, &mut items);
            id = c[1].trim().to_string();
        } else if let Some(c) = re_name.captures(line) {
            if name.is_empty() {
                name = c[1].trim().trim_matches(|ch| ch == '"' || ch == '\'').to_string();
            }
        } else if let Some(c) = re_url.captures(line) {
            if url.is_empty() {
                url = c[1].trim().to_string();
            }
        }
    }
    flush(&mut id, &mut name, &mut url, &mut items);

    let mut count = 0;
    for (pid, pname, purl) in items {
        let new_id = format!("party_{}", pid);
        let src = party_dir.join("profiles").join(format!("{}.yaml", pid));
        let dst = config_dir.join("profiles").join(format!("{}.yaml", new_id));
        let updated_at = if src.exists() && std::fs::copy(&src, &dst).is_ok() {
            "从 clash-party 迁移".to_string()
        } else {
            "未下载，请点更新".to_string()
        };
        meta.subscriptions.push(Subscription {
            id: new_id,
            name: pname,
            url: purl,
            updated_at,
        });
        count += 1;
    }
    count
}

#[cfg(unix)]
fn check_kde_proxy_enabled() -> bool {
    Command::new("kreadconfig6")
        .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

#[cfg(windows)]
fn check_kde_proxy_enabled() -> bool {
    win_os::check_system_proxy()
}

#[cfg(unix)]
fn set_kde_proxy(enable: bool) {
    let write = |key: &str, val: &str| {
        let _ = Command::new("kwriteconfig6")
            .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, val])
            .status();
    };
    if enable {
        let http = format!("http://127.0.0.1:{}", MIXED_PORT);
        let socks = format!("socks://127.0.0.1:{}", MIXED_PORT);
        write("httpProxy", &http);
        write("httpsProxy", &http);
        write("socksProxy", &socks);
        write("NoProxyFor", "localhost,127.0.0.1,::1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12");
        write("ProxyType", "1");
    } else {
        write("ProxyType", "0");
    }
    // 通知 KIO 重新读取代理设置，让已打开的程序立即生效
    let _ = Command::new("dbus-send")
        .args([
            "--type=signal",
            "/KIO/Scheduler",
            "org.kde.KIO.Scheduler.reparseSlaveConfiguration",
            "string:",
        ])
        .status();
}

#[cfg(windows)]
fn set_kde_proxy(enable: bool) {
    win_os::set_system_proxy(enable, MIXED_PORT);
}

#[cfg(unix)]
fn systemd_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "future-proxy.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn systemd_active() -> bool {
    use std::os::windows::process::CommandExt;
    let task_ok = Command::new("schtasks")
        .args(["/Query", "/TN", "FutureProxyCore"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    task_ok || win_os::check_registry_run_autostart()
}

#[cfg(unix)]
fn set_autostart(enable: bool) -> Result<(), String> {
    let out = if enable {
        Command::new("systemctl")
            .args(["--user", "enable", "--now", "future-proxy.service"])
            .output()
    } else {
        Command::new("systemctl")
            .args(["--user", "disable", "future-proxy.service"])
            .output()
    }
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn set_autostart(enable: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let config_dir = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("future-proxy");
    let core_bin = CoreManager::core_bin();

    if enable {
        // 优先尝试创建最高权限计划任务（支持后台静默运行内核与免 UAC 开启 TUN）
        let tr_cmd = format!("\"{}\" -d \"{}\"", core_bin.display(), config_dir.display());
        let sch = Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                "FutureProxyCore",
                "/TR",
                &tr_cmd,
                "/SC",
                "ONLOGON",
                "/RL",
                "HIGHEST",
                "/F",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(out) = sch {
            if out.status.success() {
                let _ = Command::new("schtasks")
                    .args(["/Run", "/TN", "FutureProxyCore"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
        }
        // 同时写入当前用户注册表 Run 启动项作为无管理员权限时的可靠保底
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("future-proxy.exe"));
        let run_cmd = format!("\"{}\" --tray", exe.display());
        win_os::set_registry_run_autostart(true, &run_cmd)
    } else {
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", "FutureProxyCore", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        win_os::set_registry_run_autostart(false, "")
    }
}

fn match_flag(name: &str) -> &'static str {
    if name.contains("非港台") {
        return "AUTO";
    }
    if name.contains("直接连接") || name.eq_ignore_ascii_case("direct") {
        return "DIR";
    }
    if name.contains("自动选择") {
        return "AUTO";
    }
    if name.contains("故障转移") {
        return "FAIL";
    }
    let lower = name.to_lowercase();
    if lower.contains("港") || lower.contains("hk") || lower.contains("hong kong") {
        "HK"
    } else if lower.contains("台") || lower.contains("tw") || lower.contains("taiwan") {
        "TW"
    } else if lower.contains("日") || lower.contains("jp") || lower.contains("japan") {
        "JP"
    } else if lower.contains("美") || lower.contains("us") || lower.contains("united states") {
        "US"
    } else if lower.contains("新") || lower.contains("sg") || lower.contains("singapore") || lower.contains("狮城") {
        "SG"
    } else if lower.contains("韩") || lower.contains("kr") || lower.contains("korea") {
        "KR"
    } else if lower.contains("英") || lower.contains("uk") || lower.contains("britain") {
        "UK"
    } else if lower.contains("德") || lower.contains("de") || lower.contains("germany") {
        "DE"
    } else if lower.contains("瑞") || lower.contains("ch") || lower.contains("swiss") {
        "CH"
    } else if lower.contains("法") || lower.contains("fr") || lower.contains("france") {
        "FR"
    } else if lower.contains("加") || lower.contains("ca") || lower.contains("canada") {
        "CA"
    } else {
        "NET"
    }
}

const SMART_GROUPS_DEF: &str = r#"  - name: "⚡ 🌍 非港台-自动优选"
    type: url-test
    url: "http://www.gstatic.com/generate_204"
    interval: 180
    tolerance: 50
    include-all: true
    exclude-filter: "(?i)香港|HK|Hong Kong|台湾|TW|Taiwan|澳门|MO|中国|CN|China"
  - name: "⚡ 🇸🇬 新加坡-自动最优"
    type: url-test
    url: "http://www.gstatic.com/generate_204"
    interval: 180
    tolerance: 50
    include-all: true
    filter: "(?i)新加坡|狮城|SG|Singapore"
  - name: "⚡ 🇯🇵 日本-自动最优"
    type: url-test
    url: "http://www.gstatic.com/generate_204"
    interval: 180
    tolerance: 50
    include-all: true
    filter: "(?i)日本|东京|大阪|埼玉|JP|Japan"
  - name: "⚡ 🇺🇸 美国-自动最优"
    type: url-test
    url: "http://www.gstatic.com/generate_204"
    interval: 180
    tolerance: 50
    include-all: true
    filter: "(?i)美国|波特兰|洛杉矶|西雅图|芝加哥|US|United States"
  - name: "⚡ 🇭🇰 香港-自动最优"
    type: url-test
    url: "http://www.gstatic.com/generate_204"
    interval: 180
    tolerance: 50
    include-all: true
    filter: "(?i)香港|HK|Hong Kong""#;

const SMART_PROXY_REFS: &str = r#"      - "⚡ 🌍 非港台-自动优选"
      - "⚡ 🇸🇬 新加坡-自动最优"
      - "⚡ 🇯🇵 日本-自动最优"
      - "⚡ 🇺🇸 美国-自动最优"
      - "⚡ 🇭🇰 香港-自动最优""#;

fn inject_smart_groups(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len() + 40);
    let mut in_proxy_groups = false;
    let mut inserted_defs = false;
    let mut inserted_proxies = false;

    for line in lines {
        out.push(line.to_string());
        let trimmed = line.trim();

        if trimmed == "proxy-groups:" && !inserted_defs {
            out.push(SMART_GROUPS_DEF.to_string());
            inserted_defs = true;
            in_proxy_groups = true;
        } else if in_proxy_groups && trimmed == "proxies:" && !inserted_proxies {
            out.push(SMART_PROXY_REFS.to_string());
            inserted_proxies = true;
        }
    }

    out
}

/// 去掉订阅中与本客户端冲突的顶层字段，并注入统一的端口/控制器/TUN/DNS 配置
fn patch_subscription(raw_yaml: &str, tun: bool) -> String {
    let override_keys = [
        "external-controller",
        "external-controller-unix",
        "secret",
        "external-ui",
        "tun",
        "dns",
        "log-level",
        "ipv6",
        "mode",
        "port",
        "socks-port",
        "mixed-port",
        "redir-port",
        "tproxy-port",
        "allow-lan",
        "bind-address",
        "sniffer",
        "profile",
        "unified-delay",
        "find-process-mode",
    ];
    let re_key = regex::Regex::new(r"^([a-zA-Z0-9_-]+)\s*:").unwrap();
    let mut filtered = Vec::new();
    let mut skipping = false;
    let mut in_old_patch = false;

    for line in raw_yaml.lines() {
        // 兼容旧版本：剔除之前注入过的头部
        if line.starts_with(PATCH_BEGIN) {
            in_old_patch = true;
            continue;
        }
        if in_old_patch {
            if line.starts_with(PATCH_END) || line.starts_with("# ========") {
                in_old_patch = false;
            }
            continue;
        }
        if let Some(caps) = re_key.captures(line) {
            skipping = override_keys.contains(&&caps[1]);
        }
        if !skipping {
            filtered.push(line);
        }
    }

    let header = format!(
        r#"{begin}
mixed-port: {port}
allow-lan: false
external-controller: {api}
secret: "{secret}"
external-ui: ui
log-level: warning
ipv6: true
mode: rule
unified-delay: true
find-process-mode: off
profile:
  store-selected: true
  store-fake-ip: true

tun:
  enable: {tun}
  stack: gvisor
  device: Mihomo
  mtu: 1500
  strict-route: false
  auto-route: true
  auto-redirect: false
  auto-detect-interface: true
  dns-hijack:
    - "any:53"

sniffer:
  enable: true
  parse-pure-ip: true
  force-dns-mapping: true
  override-destination: false
  sniff:
    HTTP:
      ports: [80, 443]
    TLS:
      ports: [443]
  skip-domain:
    - "+.push.apple.com"

dns:
  enable: true
  ipv6: true
  listen: 127.0.0.1:1053
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - "*.lan"
    - "*.local"
    - "*.localhost"
    - "*.localdomain"
    - "*.home.arpa"
    - "+.pool.ntp.org"
    - "time.*.com"
    - "ntp.*.com"
    - "stun.*.*"
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://doh.pub/dns-query
    - https://dns.alidns.com/dns-query
    - 223.5.5.5
  proxy-server-nameserver:
    - https://doh.pub/dns-query
    - 223.5.5.5
    - 119.29.29.29
{end}
"#,
        begin = PATCH_BEGIN,
        end = PATCH_END,
        port = MIXED_PORT,
        api = API_HOST,
        secret = DEFAULT_SECRET,
        tun = tun,
    );

    let injected = inject_smart_groups(&filtered);
    format!("{}\n{}\n", header, injected.join("\n"))
}

fn looks_like_clash_config(body: &str) -> bool {
    body.lines().any(|l| l.starts_with("proxies:") || l.starts_with("proxy-providers:"))
}

#[cfg(unix)]
fn read_rss_kb(pid: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{}/status", pid))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

#[cfg(unix)]
fn read_self_rss_kb() -> u64 {
    read_rss_kb("self")
}

#[cfg(windows)]
fn read_self_rss_kb() -> u64 {
    win_os::read_process_rss_kb(None)
}

#[cfg(unix)]
fn read_core_rss_kb(child_pid: Option<u32>) -> u64 {
    if let Some(p) = child_pid {
        let kb = read_rss_kb(&p.to_string());
        if kb > 0 {
            return kb;
        }
    }
    if let Ok(out) = Command::new("systemctl")
        .args(["--user", "show", "--property=MainPID", "--value", "future-proxy.service"])
        .output()
    {
        let pid_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !pid_str.is_empty() && pid_str != "0" {
            return read_rss_kb(&pid_str);
        }
    }
    0
}

#[cfg(windows)]
fn read_core_rss_kb(child_pid: Option<u32>) -> u64 {
    if let Some(pid) = child_pid {
        let kb = win_os::read_process_rss_kb(Some(pid));
        if kb > 0 {
            return kb;
        }
    }
    use std::os::windows::process::CommandExt;
    for img in ["future-proxy-core.exe", "mihomo.exe"] {
        if let Ok(out) = Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {}", img), "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split("\",\"").collect();
                if parts.len() >= 2 {
                    if let Ok(pid) = parts[1].trim_matches('"').parse::<u32>() {
                        let kb = win_os::read_process_rss_kb(Some(pid));
                        if kb > 0 {
                            return kb;
                        }
                    }
                }
            }
        }
    }
    0
}

fn reload_core(config: &Path) -> Result<(), String> {
    let body = serde_json::json!({ "path": config.to_string_lossy() });
    match ureq::put(&api("/configs?force=true"))
        .set("Authorization", &auth())
        .timeout(Duration::from_secs(10))
        .send_json(body)
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, resp)) => {
            let msg = resp.into_string().unwrap_or_default();
            Err(format!("内核拒绝配置 ({}): {}", code, msg.trim()))
        }
        Err(e) => Err(format!("内核未响应: {}", e)),
    }
}

/// 内核进程管理：只由本程序拉起一次，失败时退避，不再每秒无限重启
struct CoreManager {
    child: Option<Child>,
    last_spawn: Option<Instant>,
    failures: u32,
}

impl CoreManager {
    fn core_bin() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                #[cfg(windows)]
                for name in ["future-proxy-core.exe", "mihomo.exe", "clash-meta.exe"] {
                    let candidate = dir.join(name);
                    if candidate.exists() {
                        return candidate;
                    }
                }
                #[cfg(unix)]
                for name in ["future-proxy-core", "mihomo-core", "mihomo"] {
                    let candidate = dir.join(name);
                    if candidate.exists() {
                        return candidate;
                    }
                }
            }
        }
        #[cfg(unix)]
        {
            if Path::new("/usr/bin/future-proxy-core").exists() {
                return PathBuf::from("/usr/bin/future-proxy-core");
            }
            if Path::new("/usr/bin/mihomo-core").exists() {
                return PathBuf::from("/usr/bin/mihomo-core");
            }
            PathBuf::from("future-proxy-core")
        }
        #[cfg(windows)]
        {
            PathBuf::from("future-proxy-core.exe")
        }
    }

    /// 返回内核错误提示（为空表示正常或正在启动）
    fn ensure_running(&mut self, config_dir: &Path, alive: bool, prefer_systemd: bool) -> String {
        if alive {
            self.failures = 0;
            return String::new();
        }
        // 自启服务已启用时，优先让后台服务拉起，避免和 GUI 各起一份
        if prefer_systemd {
            #[cfg(unix)]
            {
                if systemd_active() {
                    return "systemd 已托管内核，但控制接口暂无响应".to_string();
                }
                let _ = Command::new("systemctl")
                    .args(["--user", "start", "future-proxy.service"])
                    .status();
                if self.last_spawn.map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false) {
                    return String::new();
                }
                self.last_spawn = Some(Instant::now());
                if systemd_active() {
                    return String::new();
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("schtasks")
                    .args(["/Run", "/TN", "FutureProxyCore"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        // 自己拉起的进程已退出：记录失败
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    self.failures += 1;
                }
                Ok(None) => {
                    // 仍在启动中，给它 5 秒
                    if self.last_spawn.map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false) {
                        return String::new();
                    }
                    return "内核进程在运行但控制接口无响应，可能 9090 端口被占用".to_string();
                }
                Err(_) => self.child = None,
            }
        }
        // 退避：1 次失败后每 10 秒重试，3 次后停止自动重试
        if self.failures >= 3 {
            return format!("内核启动失败，详见 {}", config_dir.join("core.log").display());
        }
        if let Some(t) = self.last_spawn {
            if t.elapsed() < Duration::from_secs(10) && self.failures > 0 {
                return format!("内核启动失败，正在重试，详见 {}", config_dir.join("core.log").display());
            }
        }
        let log = std::fs::File::create(config_dir.join("core.log")).ok();
        let bin = Self::core_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("-d").arg(config_dir).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(f) = log {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f).stderr(f2);
            }
        }
        self.last_spawn = Some(Instant::now());
        match cmd.spawn() {
            Ok(c) => {
                self.child = Some(c);
                String::new()
            }
            Err(e) => {
                self.failures += 1;
                format!("无法启动内核 {}: {}", bin.display(), e)
            }
        }
    }

    fn reset(&mut self) {
        self.failures = 0;
        self.last_spawn = None;
    }

    fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    fn stop(&mut self) {
        // 后台服务托管时不要杀，否则会跟服务抢
        if systemd_active() {
            self.child = None;
            return;
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

const REGIONS: [&str; 8] = [
    "全部",
    "非港台",
    "新加坡",
    "日本",
    "美国",
    "香港",
    "台湾",
    "其他",
];

fn region_match(region: &str, name: &str) -> bool {
    let lower = name.to_lowercase();
    let is = |a: &str, b: &str| lower.contains(a) || lower.contains(b);
    if region == "全部" {
        true
    } else if region.contains("非港台") {
        if name.contains("非港台") {
            return true;
        }
        let is_hk = lower.contains("港") || lower.contains("hk") || lower.contains("hong kong");
        let is_tw = lower.contains("台") || lower.contains("tw") || lower.contains("taiwan");
        let is_mo = lower.contains("澳") || lower.contains("macau") || lower.contains("macao");
        let is_cn = lower.contains("中国") || lower.contains("国内") || lower.contains("china");
        let is_smart_group = name.starts_with("⚡") || name == "自动选择" || name == "故障转移" || name == "🚀直接连接";
        !is_hk && !is_tw && !is_mo && !is_cn && !is_smart_group
    } else if region.contains("香港") {
        is("港", "hk")
    } else if region.contains("日本") {
        is("日", "jp")
    } else if region.contains("美国") {
        is("美", "us")
    } else if region.contains("新加坡") {
        is("新", "sg")
    } else if region.contains("台湾") {
        is("台", "tw")
    } else {
        !(is("港", "hk") || is("日", "jp") || is("美", "us") || is("新", "sg") || is("台", "tw") || name.contains("非港台"))
    }
}

fn region_smart_group(region: &str) -> Option<&'static str> {
    match region {
        r if r.contains("非港台") => Some("⚡ 🌍 非港台-自动优选"),
        r if r.contains("新加坡") => Some("⚡ 🇸🇬 新加坡-自动最优"),
        r if r.contains("日本") => Some("⚡ 🇯🇵 日本-自动最优"),
        r if r.contains("美国") => Some("⚡ 🇺🇸 美国-自动最优"),
        r if r.contains("香港") => Some("⚡ 🇭🇰 香港-自动最优"),
        _ => None,
    }
}

static RE_MULT_SUFFIX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static RE_MULT_ANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

/// 解析节点名称中的倍率标签（如 "香港—F2 3倍率" -> ("香港—F2", Some("3x"))）
fn parse_node_display(name: &str) -> (String, Option<String>) {
    let re_suffix = RE_MULT_SUFFIX.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:[\[(（\s_-]*|\s+)(\d+(?:\.\d+)?)\s*(?:倍率|倍|x|X)(?:[\])）\s_-]*|\s*)$").unwrap()
    });
    if let Some(caps) = re_suffix.captures(name) {
        if let Some(m) = caps.get(1) {
            let mult = format!("{}x", m.as_str());
            let clean = re_suffix.replace(name, "").trim().to_string();
            return (clean, Some(mult));
        }
    }
    let re_any = RE_MULT_ANY.get_or_init(|| {
        regex::Regex::new(r"(?i)\s*(\d+(?:\.\d+)?)\s*(?:倍率|倍)\s*").unwrap()
    });
    if let Some(caps) = re_any.captures(name) {
        if let Some(m) = caps.get(1) {
            let mult = format!("{}x", m.as_str());
            let clean = re_any.replace(name, " ").trim().to_string();
            return (clean, Some(mult));
        }
    }
    (name.to_string(), None)
}

/// 净化节点/策略组标题：剥离乱码 Emoji，将杂乱破折号规范为 Apple 风格间隔号 " · "
fn format_cupertino_title(raw: &str) -> String {
    let mut buf = String::with_capacity(raw.len());
    let mut prev_sep = false;
    for ch in raw.chars() {
        if ch == '-' || ch == '—' || ch == '–' {
            if !prev_sep && !buf.trim().is_empty() {
                buf.push_str(" · ");
                prev_sep = true;
            }
            continue;
        }
        let keep = ch.is_ascii_alphanumeric()
            || matches!(ch, ' ' | '_' | '.' | '+' | '(' | ')' | '/' | ':')
            || ('\u{4E00}'..='\u{9FFF}').contains(&ch);
        if keep {
            if ch != ' ' {
                prev_sep = false;
            }
            buf.push(ch);
        }
    }
    let mut out = String::new();
    for word in buf.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.trim_matches(|c: char| c == '·' || c == ' ').to_string()
}

// ============================================================================
// Apple Cupertino Design System (macOS Sequoia / iOS 18 Dark Mode)
// ============================================================================

const CUP_BG_WINDOW: egui::Color32 = egui::Color32::from_rgb(18, 18, 20);      // #121214
const CUP_BG_GROUPED: egui::Color32 = egui::Color32::from_rgb(27, 27, 31);     // #1B1B1F
const CUP_BG_SECONDARY: egui::Color32 = egui::Color32::from_rgb(35, 35, 40);   // #232328
const CUP_BG_TERTIARY: egui::Color32 = egui::Color32::from_rgb(46, 46, 52);    // #2E2E34
const CUP_BG_ELEVATED: egui::Color32 = egui::Color32::from_rgb(58, 58, 66);    // #3A3A42
const CUP_HAIRLINE: egui::Color32 = egui::Color32::from_rgb(40, 40, 46);       // #28282E 柔和暗色微边框（绝不发白刺眼）
const CUP_HAIRLINE_BRIGHT: egui::Color32 = egui::Color32::from_rgb(58, 58, 66);// #3A3A42 悬停微高光边框
const CUP_DIVIDER: egui::Color32 = egui::Color32::from_rgb(30, 30, 35);        // #1E1E23 极细分割线

const CUP_LABEL_PRIMARY: egui::Color32 = egui::Color32::from_rgb(242, 242, 247);   // #F2F2F7
const CUP_LABEL_SECONDARY: egui::Color32 = egui::Color32::from_rgb(152, 152, 159); // #98989F
const CUP_LABEL_TERTIARY: egui::Color32 = egui::Color32::from_rgb(99, 99, 104);    // #636368

const CUP_BLUE: egui::Color32 = egui::Color32::from_rgb(10, 132, 255);     // #0A84FF
const CUP_GREEN: egui::Color32 = egui::Color32::from_rgb(48, 209, 88);     // #30D158
const CUP_INDIGO: egui::Color32 = egui::Color32::from_rgb(94, 92, 230);    // #5E5CE6
const CUP_ORANGE: egui::Color32 = egui::Color32::from_rgb(255, 159, 10);   // #FF9F0A
const CUP_RED: egui::Color32 = egui::Color32::from_rgb(255, 69, 58);       // #FF453A
const CUP_TEAL: egui::Color32 = egui::Color32::from_rgb(100, 210, 255);    // #64D2FF

fn cup_font_reg(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

fn cup_font_med(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("Medium".into()))
}

fn cup_font_mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Monospace)
}

fn apply_cupertino_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 5.0,
        handle_min_length: 28.0,
        bar_inner_margin: 4.0,
        bar_outer_margin: 2.0,
        floating: true,
        ..Default::default()
    };

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CUP_BG_WINDOW;
    visuals.window_fill = CUP_BG_GROUPED;
    visuals.extreme_bg_color = CUP_BG_GROUPED;
    visuals.faint_bg_color = CUP_BG_GROUPED;
    visuals.widgets.noninteractive.bg_fill = CUP_BG_GROUPED;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, CUP_HAIRLINE);
    visuals.widgets.noninteractive.fg_stroke.color = CUP_LABEL_PRIMARY;
    visuals.widgets.inactive.bg_fill = CUP_BG_SECONDARY;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = CUP_LABEL_PRIMARY;
    visuals.widgets.hovered.bg_fill = CUP_BG_TERTIARY;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
    visuals.widgets.active.bg_fill = CUP_BLUE;
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;
    visuals.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(10, 132, 255, 80);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, CUP_BLUE);
    style.visuals = visuals;
    ctx.set_style(style);
}

#[derive(Clone, Copy)]
enum CupIcon {
    Refresh,
    Plus,
    Bolt,
    Sliders,
}

#[derive(Clone, Copy)]
enum CupBtnStyle {
    Filled(egui::Color32),
    Tinted(egui::Color32),
    Secondary,
}

fn draw_sf_globe(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.3_f32, col);
    painter.circle_stroke(center, r, s);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(r, 0.0)], s);
    painter.line_segment([center + egui::vec2(0.0, -r), center + egui::vec2(0.0, r)], s);
    let s_thin = egui::Stroke::new(1.0_f32, col);
    painter.line_segment(
        [center + egui::vec2(-r * 0.84, -r * 0.45), center + egui::vec2(r * 0.84, -r * 0.45)],
        s_thin,
    );
    painter.line_segment(
        [center + egui::vec2(-r * 0.84, r * 0.45), center + egui::vec2(r * 0.84, r * 0.45)],
        s_thin,
    );
}

fn draw_sf_bolt(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let t1 = vec![
        center + egui::vec2(r * 0.15, -r * 0.95),
        center + egui::vec2(-r * 0.65, r * 0.10),
        center + egui::vec2(r * 0.08, r * 0.10),
    ];
    let t2 = vec![
        center + egui::vec2(-r * 0.08, -r * 0.10),
        center + egui::vec2(r * 0.65, -r * 0.10),
        center + egui::vec2(-r * 0.15, r * 0.95),
    ];
    painter.add(egui::Shape::convex_polygon(t1, col, egui::Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(t2, col, egui::Stroke::NONE));
}

fn draw_sf_refresh(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let steps = 14;
    let start_ang = -std::f32::consts::FRAC_PI_4;
    let sweep = std::f32::consts::PI * 1.55;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let a = start_ang + sweep * (i as f32 / steps as f32);
        pts.push(center + egui::vec2(a.cos() * r, a.sin() * r));
    }
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.4_f32, col)));
    let tip = center + egui::vec2(start_ang.cos() * r, start_ang.sin() * r);
    let s = egui::Stroke::new(1.4_f32, col);
    painter.line_segment([tip, tip + egui::vec2(-3.2, -0.8)], s);
    painter.line_segment([tip, tip + egui::vec2(0.5, 3.2)], s);
}

fn draw_sf_plus(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.5_f32, col);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(r, 0.0)], s);
    painter.line_segment([center + egui::vec2(0.0, -r), center + egui::vec2(0.0, r)], s);
}

fn draw_sf_search(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let c = center + egui::vec2(-1.2, -1.2);
    let cr = r * 0.72;
    painter.circle_stroke(c, cr, egui::Stroke::new(1.4_f32, col));
    let p1 = c + egui::vec2(cr * 0.707, cr * 0.707);
    let p2 = center + egui::vec2(r * 0.85, r * 0.85);
    painter.line_segment([p1, p2], egui::Stroke::new(1.5_f32, col));
}

fn draw_sf_sliders(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32, bg: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    for (dy, kx) in [(-4.2_f32, -2.0_f32), (0.0_f32, 2.2_f32), (4.2_f32, -1.0_f32)] {
        let y = center.y + dy;
        painter.line_segment([egui::pos2(center.x - r, y), egui::pos2(center.x + r, y)], s);
        let kc = egui::pos2(center.x + kx, y);
        painter.circle_filled(kc, 2.0, bg);
        painter.circle_stroke(kc, 1.8, egui::Stroke::new(1.25_f32, col));
    }
}

fn draw_sf_chevron_up_down(painter: &egui::Painter, center: egui::Pos2, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    painter.line_segment([center + egui::vec2(-2.8, -1.2), center + egui::vec2(0.0, -3.8)], s);
    painter.line_segment([center + egui::vec2(0.0, -3.8), center + egui::vec2(2.8, -1.2)], s);
    painter.line_segment([center + egui::vec2(-2.8, 1.2), center + egui::vec2(0.0, 3.8)], s);
    painter.line_segment([center + egui::vec2(0.0, 3.8), center + egui::vec2(2.8, 1.2)], s);
}

fn draw_sf_checkmark(painter: &egui::Painter, center: egui::Pos2, scale: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.7_f32 * scale, col);
    let p1 = center + egui::vec2(-3.8 * scale, 0.2 * scale);
    let p2 = center + egui::vec2(-1.0 * scale, 3.0 * scale);
    let p3 = center + egui::vec2(4.0 * scale, -2.8 * scale);
    painter.line_segment([p1, p2], s);
    painter.line_segment([p2, p3], s);
}

fn draw_sf_xmark(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.4_f32, col);
    painter.line_segment([center + egui::vec2(-r, -r), center + egui::vec2(r, r)], s);
    painter.line_segment([center + egui::vec2(r, -r), center + egui::vec2(-r, r)], s);
}

/// 纯正 iOS / macOS UISwitch 绘制器
fn paint_cupertino_switch(painter: &egui::Painter, rect: egui::Rect, on: bool) {
    let r = rect.height() * 0.5;
    let track_col = if on {
        CUP_GREEN
    } else {
        egui::Color32::from_rgb(56, 56, 62)
    };
    painter.rect_filled(rect, r, track_col);
    let knob_r = r - 2.0;
    let knob_x = if on { rect.right() - r } else { rect.left() + r };
    let knob_center = egui::pos2(knob_x, rect.center().y);
    painter.circle_filled(
        knob_center + egui::vec2(0.0, 1.0),
        knob_r,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 60),
    );
    painter.circle_filled(knob_center, knob_r, egui::Color32::WHITE);
}

fn cupertino_switch(ui: &mut egui::Ui, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        paint_cupertino_switch(ui.painter(), rect.shrink(0.5), on);
    }
    resp.clicked()
}

/// 紧凑型 macOS Control Pill Switch（内嵌圆形彩色 SF 图标 + 标题 + iOS Switch，高度 30px）
fn cupertino_toolbar_switch(
    ui: &mut egui::Ui,
    title: &str,
    on: bool,
    accent: egui::Color32,
    is_bolt: bool,
) -> bool {
    let g = ui.fonts(|f| f.layout_no_wrap(title.to_string(), cup_font_med(12.0), CUP_LABEL_PRIMARY));
    let w = g.size().x + 82.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let draw_r = rect.shrink(0.5);
        let bg = if resp.hovered() {
            CUP_BG_TERTIARY
        } else {
            CUP_BG_GROUPED
        };
        let border = if on {
            egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90)
        } else {
            CUP_HAIRLINE
        };
        painter.rect(draw_r, 8.0, bg, egui::Stroke::new(1.0_f32, border));

        // 左侧圆形徽章 (半径 9.5)
        let ic_c = egui::pos2(draw_r.left() + 15.0, draw_r.center().y);
        let ic_bg = if on { accent } else { CUP_BG_SECONDARY };
        let ic_fg = if on { egui::Color32::WHITE } else { CUP_LABEL_SECONDARY };
        painter.circle_filled(ic_c, 9.5, ic_bg);
        if is_bolt {
            draw_sf_bolt(painter, ic_c, 5.0, ic_fg);
        } else {
            draw_sf_globe(painter, ic_c, 5.0, ic_fg);
        }

        // 中间文字
        painter.galley(
            egui::pos2(draw_r.left() + 30.0, draw_r.center().y - g.size().y * 0.5),
            g,
            CUP_LABEL_PRIMARY,
        );

        // 右侧紧凑 iOS Switch (32x18)
        let sw_rect = egui::Rect::from_center_size(
            egui::pos2(draw_r.right() - 22.0, draw_r.center().y),
            egui::vec2(32.0, 18.0),
        );
        paint_cupertino_switch(painter, sw_rect, on);
    }
    resp.clicked()
}

/// 纯正 Cupertino 胶囊/圆角按钮（使用父级 painter + shrink(0.5) 防止任何边缘截断）
fn cupertino_button(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<CupIcon>,
    style: CupBtnStyle,
    enabled: bool,
) -> bool {
    let text_galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), cup_font_med(11.5), egui::Color32::WHITE));
    let icon_pad = if icon.is_some() { 16.0 } else { 0.0 };
    let w = (text_galley.size().x + icon_pad + 20.0).max(52.0);
    let h = 28.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let draw_r = rect.shrink(0.5);
        let hovered = enabled && resp.hovered();
        let active = enabled && resp.is_pointer_button_down_on();

        let (bg, fg, stroke) = match style {
            CupBtnStyle::Filled(c) => {
                let fill = if !enabled {
                    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 80)
                } else if active {
                    egui::Color32::from_rgb(c.r().saturating_sub(20), c.g().saturating_sub(20), c.b().saturating_sub(20))
                } else if hovered {
                    egui::Color32::from_rgb(c.r().saturating_add(15), c.g().saturating_add(15), c.b().saturating_add(15))
                } else {
                    c
                };
                let txt_c = if enabled { egui::Color32::WHITE } else { CUP_LABEL_SECONDARY };
                (fill, txt_c, egui::Stroke::NONE)
            }
            CupBtnStyle::Tinted(c) => {
                let alpha = if !enabled { 18 } else if active { 70 } else if hovered { 52 } else { 34 };
                let fill = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
                let border = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), if hovered { 100 } else { 65 });
                let txt_c = if enabled { c } else { CUP_LABEL_TERTIARY };
                (fill, txt_c, egui::Stroke::new(1.0_f32, border))
            }
            CupBtnStyle::Secondary => {
                let fill = if !enabled {
                    CUP_BG_GROUPED
                } else if active {
                    CUP_BG_ELEVATED
                } else if hovered {
                    CUP_BG_TERTIARY
                } else {
                    CUP_BG_SECONDARY
                };
                let border = if hovered { CUP_HAIRLINE_BRIGHT } else { CUP_HAIRLINE };
                let txt_c = if enabled { CUP_LABEL_PRIMARY } else { CUP_LABEL_TERTIARY };
                (fill, txt_c, egui::Stroke::new(1.0_f32, border))
            }
        };

        painter.rect(draw_r, 7.5, bg, stroke);

        let content_w = text_galley.size().x + icon_pad;
        let start_x = draw_r.center().x - content_w * 0.5;
        if let Some(ic) = icon {
            let ic_center = egui::pos2(start_x + 5.5, draw_r.center().y);
            match ic {
                CupIcon::Refresh => draw_sf_refresh(painter, ic_center, 4.8, fg),
                CupIcon::Plus => draw_sf_plus(painter, ic_center, 4.6, fg),
                CupIcon::Bolt => draw_sf_bolt(painter, ic_center, 5.2, fg),
                CupIcon::Sliders => draw_sf_sliders(painter, ic_center, 5.5, fg, bg),
            }
        }
        painter.text(
            egui::pos2(start_x + icon_pad, draw_r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            cup_font_med(11.5),
            fg,
        );
    }
    enabled && resp.clicked()
}

/// 纯正 Cupertino Pop-up 选择器触发胶囊（无边缘截断）
fn cupertino_popup_trigger(
    ui: &mut egui::Ui,
    prefix: &str,
    value: &str,
    width: f32,
    is_open: bool,
) -> (egui::Rect, bool) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let draw_r = rect.shrink(0.5);
        let bg = if is_open || resp.hovered() {
            CUP_BG_TERTIARY
        } else {
            CUP_BG_SECONDARY
        };
        let stroke = if is_open {
            egui::Stroke::new(1.2_f32, CUP_BLUE)
        } else if resp.hovered() {
            egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT)
        } else {
            egui::Stroke::new(1.0_f32, CUP_HAIRLINE)
        };
        painter.rect(draw_r, 7.5, bg, stroke);

        let prefix_galley = ui.fonts(|f| f.layout_no_wrap(prefix.to_string(), cup_font_reg(11.0), CUP_LABEL_SECONDARY));
        let pw = prefix_galley.size().x;
        painter.galley(
            egui::pos2(draw_r.left() + 9.0, draw_r.center().y - prefix_galley.size().y * 0.5),
            prefix_galley,
            CUP_LABEL_SECONDARY,
        );

        let max_val_w = (width - pw - 32.0).max(20.0);
        let mut job = egui::text::LayoutJob::single_section(
            value.to_string(),
            egui::TextFormat {
                font_id: cup_font_med(11.5),
                color: CUP_LABEL_PRIMARY,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping {
            max_width: max_val_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let val_galley = ui.fonts(|f| f.layout_job(job));
        painter.galley(
            egui::pos2(draw_r.left() + 14.0 + pw, draw_r.center().y - val_galley.size().y * 0.5),
            val_galley,
            CUP_LABEL_PRIMARY,
        );

        draw_sf_chevron_up_down(painter, egui::pos2(draw_r.right() - 10.0, draw_r.center().y), CUP_LABEL_SECONDARY);
    }
    (rect, resp.clicked())
}

/// 纯正 Cupertino Segmented Control（无边框截断）
fn cupertino_segmented_control(ui: &mut egui::Ui, items: &[&str], selected: &mut String) {
    let seg_h = 28.0;
    let pad = 2.5;
    let mut item_widths = Vec::with_capacity(items.len());
    for &it in items {
        let g = ui.fonts(|f| f.layout_no_wrap(it.to_string(), cup_font_med(11.5), egui::Color32::WHITE));
        item_widths.push((g.size().x + 18.0).max(42.0));
    }
    let total_w: f32 = item_widths.iter().sum::<f32>() + pad * 2.0;
    let (track_rect, _) = ui.allocate_exact_size(egui::vec2(total_w, seg_h), egui::Sense::hover());

    let painter = ui.painter();
    let draw_track = track_rect.shrink(0.5);
    painter.rect(draw_track, 7.5, CUP_BG_GROUPED, egui::Stroke::new(1.0_f32, CUP_HAIRLINE));

    let is_match = |item: &str, sel: &str| -> bool {
        item == sel || (item != "全部" && sel.contains(item))
    };

    let active_idx = items.iter().position(|&it| is_match(it, selected)).unwrap_or(0);

    let mut cur_x = track_rect.left() + pad;
    for (idx, &it) in items.iter().enumerate() {
        let w = item_widths[idx];
        let seg_rect = egui::Rect::from_min_size(
            egui::pos2(cur_x, track_rect.top() + pad),
            egui::vec2(w, seg_h - pad * 2.0),
        );
        let id = ui.id().with(("cup_seg", idx));
        let resp = ui.interact(seg_rect, id, egui::Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let is_active = idx == active_idx;
        if is_active {
            painter.rect_filled(
                seg_rect.translate(egui::vec2(0.0, 1.0)),
                5.5,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 70),
            );
            painter.rect(
                seg_rect,
                5.5,
                CUP_BG_ELEVATED,
                egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT),
            );
        } else if resp.hovered() {
            painter.rect_filled(seg_rect, 5.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 10));
        }

        if idx > 0 && idx != active_idx && (idx - 1) != active_idx {
            painter.line_segment(
                [
                    egui::pos2(cur_x, track_rect.center().y - 5.5),
                    egui::pos2(cur_x, track_rect.center().y + 5.5),
                ],
                egui::Stroke::new(1.0_f32, CUP_HAIRLINE),
            );
        }

        let fg = if is_active {
            CUP_LABEL_PRIMARY
        } else if resp.hovered() {
            egui::Color32::from_rgb(205, 205, 210)
        } else {
            CUP_LABEL_SECONDARY
        };
        let font = if is_active { cup_font_med(11.5) } else { cup_font_reg(11.5) };
        painter.text(seg_rect.center(), egui::Align2::CENTER_CENTER, it, font, fg);

        if resp.clicked() {
            *selected = it.to_string();
        }
        cur_x += w;
    }
}

/// 纯正 Cupertino Search Field（无边缘截断）
fn cupertino_search_field(ui: &mut egui::Ui, keyword: &mut String, width: f32) {
    let h = 28.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let painter = ui.painter().clone();
    let draw_r = rect.shrink(0.5);
    painter.rect(draw_r, 7.5, CUP_BG_GROUPED, egui::Stroke::new(1.0_f32, CUP_HAIRLINE));

    draw_sf_search(
        &painter,
        egui::pos2(draw_r.left() + 13.0, draw_r.center().y),
        5.0,
        CUP_LABEL_SECONDARY,
    );

    let has_clear = !keyword.is_empty();
    let right_pad = if has_clear { 26.0 } else { 8.0 };
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(draw_r.left() + 24.0, draw_r.top() + 2.5),
        egui::pos2(draw_r.right() - right_pad, draw_r.bottom() - 2.5),
    );

    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
    let edit_resp = child_ui.add(
        egui::TextEdit::singleline(keyword)
            .frame(false)
            .font(cup_font_reg(11.5))
            .hint_text(egui::RichText::new("搜索节点名称、地区或关键字…").color(CUP_LABEL_TERTIARY).size(11.5))
            .desired_width(edit_rect.width()),
    );
    if edit_resp.has_focus() {
        painter.rect_stroke(draw_r, 7.5, egui::Stroke::new(1.2_f32, CUP_BLUE));
    }

    if has_clear {
        let clear_rect = egui::Rect::from_center_size(
            egui::pos2(draw_r.right() - 13.0, draw_r.center().y),
            egui::vec2(16.0, 16.0),
        );
        let clear_resp = ui.interact(clear_rect, ui.id().with("cup_search_clear"), egui::Sense::click());
        if clear_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let circle_col = if clear_resp.hovered() { CUP_LABEL_SECONDARY } else { CUP_LABEL_TERTIARY };
        painter.circle_filled(clear_rect.center(), 6.5, circle_col);
        draw_sf_xmark(&painter, clear_rect.center(), 2.5, CUP_BG_GROUPED);
        if clear_resp.clicked() {
            keyword.clear();
        }
    }
}

fn enc_path(name: &str) -> String {
    let enc: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    enc.replace('+', "%20")
}

/// 测试单个节点延迟，失败/超时返回 0
fn test_delay(name: &str) -> i32 {
    let url = api(&format!(
        "/proxies/{}/delay?timeout=3000&url=http://www.gstatic.com/generate_204",
        enc_path(name)
    ));
    ureq::get(&url)
        .set("Authorization", &auth())
        .timeout(Duration::from_millis(3500))
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|j| j.get("delay").and_then(|d| d.as_i64()))
        .unwrap_or(0) as i32
}

/// 并发（16 个一批）测试一组节点
fn test_delays(names: &[String]) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for chunk in names.chunks(16) {
        let handles: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|n| std::thread::spawn(move || { let d = test_delay(&n); (n, d) }))
            .collect();
        out.extend(handles.into_iter().filter_map(|h| h.join().ok()));
    }
    out
}

fn put_select(group: &str, name: &str) -> Result<(), String> {
    ureq::put(&api(&format!("/proxies/{}", enc_path(group))))
        .set("Authorization", &auth())
        .timeout(Duration::from_secs(3))
        .send_json(serde_json::json!({ "name": name }))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

struct ProxiesSnapshot {
    groups: Vec<String>,
    group_name: String,
    current: String,
    nodes: Vec<ProxyNode>,
}

fn fetch_proxies(preferred_group: &str, user_picked: bool) -> Option<ProxiesSnapshot> {
    let json: serde_json::Value = ureq::get(&api("/proxies"))
        .set("Authorization", &auth())
        .timeout(Duration::from_millis(1500))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let proxies = json.get("proxies")?.as_object()?;

    // 优先使用 GLOBAL 的顺序列出 Selector 组，保持与订阅一致
    let mut groups: Vec<String> = Vec::new();
    let order: Vec<String> = proxies
        .get("GLOBAL")
        .and_then(|g| g.get("all"))
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let is_selector = |name: &str| {
        proxies
            .get(name)
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "Selector")
            .unwrap_or(false)
    };
    for name in &order {
        if name != "GLOBAL" && is_selector(name) {
            groups.push(name.clone());
        }
    }
    for name in proxies.keys() {
        if name != "GLOBAL" && is_selector(name) && !groups.contains(name) {
            groups.push(name.clone());
        }
    }
    if groups.is_empty() {
        return Some(ProxiesSnapshot { groups, group_name: String::new(), current: String::new(), nodes: Vec::new() });
    }

    let group_name = if user_picked && groups.iter().any(|g| g == preferred_group) {
        preferred_group.to_string()
    } else {
        ["节点选择", "🚀 节点选择", "PROXY", "Proxy", "节点", "Select"]
            .iter()
            .map(|s| s.to_string())
            .find(|n| groups.contains(n))
            .or_else(|| groups.iter().find(|g| g.contains("节点选择")).cloned())
            .unwrap_or_else(|| groups[0].clone())
    };

    let group = proxies.get(&group_name)?;
    let current = group.get("now").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let mut nodes = Vec::new();
    if let Some(arr) = group.get("all").and_then(|a| a.as_array()) {
        for n_name in arr.iter().filter_map(|v| v.as_str()) {
            if n_name == "REJECT" || n_name == "GLOBAL" {
                continue;
            }
            if ["流量", "到期", "官网", "通知", "重置"].iter().any(|k| n_name.contains(k)) {
                continue;
            }
            let delay = proxies
                .get(n_name)
                .and_then(|p| p.get("history"))
                .and_then(|h| h.as_array())
                .and_then(|h| h.last())
                .and_then(|l| l.get("delay"))
                .and_then(|d| d.as_i64())
                .map(|d| d as i32)
                .unwrap_or(-1);
            nodes.push(ProxyNode {
                name: n_name.to_string(),
                flag: match_flag(n_name).to_string(),
                delay,
                is_selected: n_name == current,
            });
        }
    }
    Some(ProxiesSnapshot { groups, group_name, current, nodes })
}

pub struct FutureProxyApp {
    state: Arc<Mutex<AppState>>,
    core: Arc<Mutex<CoreManager>>,
    ctx: egui::Context,
    tray_flags: Arc<TrayFlags>,
    egui_ctx_slot: Arc<Mutex<Option<egui::Context>>>,
}

/// Process-wide state: survives egui window close so tray can reopen the UI.
struct AppRuntime {
    state: Arc<Mutex<AppState>>,
    core: Arc<Mutex<CoreManager>>,
    tray_flags: Arc<TrayFlags>,
    egui_ctx_slot: Arc<Mutex<Option<egui::Context>>>,
    _tray: TrayGuard,
    // Kept so Condvar/tray wake stays alive for tray handlers.
    _wake: Arc<Condvar>,
    _wake_mutex: Arc<Mutex<()>>,
    /// Single-instance IPC listener; dropping/clearing releases the lock for handoff.
    listener_slot: Arc<Mutex<Option<IpcListener>>>,
}

impl AppRuntime {
    fn start(listener: Option<IpcListener>) -> Self {
        let state = Arc::new(Mutex::new(AppState::new()));
        {
            let mut app = state.lock().unwrap();
            if app.profiles.autostart {
                match set_autostart(true) {
                    Ok(()) => {
                        #[cfg(unix)]
                        {
                            app.status_msg = "内核由 systemd 托管".to_string();
                        }
                        #[cfg(windows)]
                        {
                            app.status_msg = "内核由系统后台服务托管".to_string();
                        }
                    }
                    Err(e) => app.status_msg = format!("自启服务异常: {}（将回退直接启动内核）", e),
                }
            }
        }
        let core = Arc::new(Mutex::new(CoreManager {
            child: None,
            last_spawn: None,
            failures: 0,
        }));
        let tray_flags = Arc::new(TrayFlags {
            show: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        });
        let wake = Arc::new(Condvar::new());
        let wake_mutex = Arc::new(Mutex::new(()));
        let egui_ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));

        // Background monitors (once per process). Repaint only if a window is open.
        {
            let state = Arc::clone(&state);
            let core = Arc::clone(&core);
            let egui_ctx_slot = Arc::clone(&egui_ctx_slot);
            std::thread::spawn(move || {
                let mut last_nodes = Instant::now() - Duration::from_secs(10);
                let mut last_mem = Instant::now() - Duration::from_secs(10);
                loop {
                    let alive = ureq::get(&api("/version"))
                        .set("Authorization", &auth())
                        .timeout(Duration::from_millis(400))
                        .call()
                        .is_ok();
                    let config_dir = state.lock().unwrap().config_dir.clone();
                    let prefer_systemd = state.lock().unwrap().profiles.autostart;
                    let err = core.lock().unwrap().ensure_running(&config_dir, alive, prefer_systemd);

                    let snapshot = if alive && last_nodes.elapsed() > Duration::from_secs(3) {
                        last_nodes = Instant::now();
                        let (g, picked) = {
                            let app = state.lock().unwrap();
                            (app.group_name.clone(), app.group_picked_by_user)
                        };
                        fetch_proxies(&g, picked)
                    } else {
                        None
                    };

                    let mem = if last_mem.elapsed() > Duration::from_secs(3) {
                        last_mem = Instant::now();
                        let core_pid = core.lock().unwrap().pid();
                        let core_kb = read_core_rss_kb(core_pid);
                        Some((read_self_rss_kb(), core_kb))
                    } else {
                        None
                    };

                    let mut changed = false;
                    {
                        let mut app = state.lock().unwrap();
                        if app.is_core_alive != alive || app.core_error != err {
                            app.is_core_alive = alive;
                            app.core_error = err;
                            changed = true;
                        }
                        if let Some(s) = snapshot {
                            app.groups = s.groups;
                            app.group_name = s.group_name;
                            app.current_selected = s.current;
                            if !app.is_testing_delay {
                                app.nodes = s.nodes;
                            }
                            changed = true;
                        }
                        if let Some((g, c)) = mem {
                            app.mem_gui_kb = g;
                            app.mem_core_kb = c;
                            changed = true;
                        }
                    }
                    if changed {
                        request_repaint_slot(&egui_ctx_slot);
                    }
                    std::thread::sleep(Duration::from_millis(if alive { 1000 } else { 500 }));
                }
            });
        }

        {
            let state = Arc::clone(&state);
            let egui_ctx_slot = Arc::clone(&egui_ctx_slot);
            std::thread::spawn(move || loop {
                if let Ok(resp) = ureq::get(&api("/traffic"))
                    .set("Authorization", &auth())
                    .call()
                {
                    let reader = std::io::BufReader::new(resp.into_reader());
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                            let up = val.get("up").and_then(|v| v.as_u64()).unwrap_or(0);
                            let down = val.get("down").and_then(|v| v.as_u64()).unwrap_or(0);
                            let mut app = state.lock().unwrap();
                            if app.upload_rate != up || app.download_rate != down {
                                app.upload_rate = up;
                                app.download_rate = down;
                                drop(app);
                                request_repaint_slot(&egui_ctx_slot);
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            });
        }

        {
            let state = Arc::clone(&state);
            let egui_ctx_slot = Arc::clone(&egui_ctx_slot);
            std::thread::spawn(move || {
                let mut last = Instant::now() - Duration::from_secs(60);
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let (region, group, names, current, kick, alive) = {
                        let app = state.lock().unwrap();
                        (
                            app.profiles.auto_region.clone(),
                            app.group_name.clone(),
                            app.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>(),
                            app.current_selected.clone(),
                            app.auto_kick,
                            app.is_core_alive,
                        )
                    };
                    let Some(region) = region else { continue };
                    if !alive || names.is_empty() || (!kick && last.elapsed() < Duration::from_secs(30)) {
                        continue;
                    }
                    if current.starts_with("⚡") {
                        // 当前已由内核原生智能策略组接管（Mihomo 在底层毫秒级健康检查自愈），无需前端覆盖重选
                        continue;
                    }
                    last = Instant::now();
                    state.lock().unwrap().auto_kick = false;

                    let candidates: Vec<String> =
                        names.into_iter().filter(|n| region_match(&region, n)).collect();
                    if candidates.is_empty() {
                        state.lock().unwrap().status_msg =
                            format!("自动选择: {} 没有可用节点", region);
                        request_repaint_slot(&egui_ctx_slot);
                        continue;
                    }
                    if candidates.contains(&current) {
                        let d = test_delay(&current);
                        if d > 0 {
                            let mut app = state.lock().unwrap();
                            if let Some(n) = app.nodes.iter_mut().find(|n| n.name == current) {
                                n.delay = d;
                            }
                            drop(app);
                            request_repaint_slot(&egui_ctx_slot);
                            continue;
                        }
                    }
                    let results = test_delays(&candidates);
                    let best = results
                        .iter()
                        .filter(|(_, d)| **d > 0)
                        .min_by_key(|(_, d)| **d)
                        .map(|(n, d)| (n.clone(), *d));
                    let mut app = state.lock().unwrap();
                    for n in &mut app.nodes {
                        if let Some(d) = results.get(&n.name) {
                            n.delay = *d;
                        }
                    }
                    match best {
                        Some((name, d)) => {
                            drop(app);
                            let res = put_select(&group, &name);
                            let mut app = state.lock().unwrap();
                            match res {
                                Ok(()) => {
                                    app.current_selected = name.clone();
                                    for n in &mut app.nodes {
                                        n.is_selected = n.name == name;
                                    }
                                    app.status_msg =
                                        format!("自动选择: 已切到 {} ({} ms)", name, d);
                                }
                                Err(e) => app.status_msg = format!("✗ 自动切换失败: {}", e),
                            }
                        }
                        None => {
                            app.status_msg = format!(
                                "自动选择: {} 的节点全部超时，保持当前节点",
                                region
                            )
                        }
                    }
                    request_repaint_slot(&egui_ctx_slot);
                }
            });
        }

        let tray_handle = spawn_system_tray(
            Arc::clone(&tray_flags),
            Arc::clone(&wake),
            Arc::clone(&egui_ctx_slot),
        );

        let listener_slot = Arc::new(Mutex::new(listener));
        spawn_ipc_listener(
            Arc::clone(&listener_slot),
            Arc::clone(&tray_flags),
            Arc::clone(&wake),
            Arc::clone(&egui_ctx_slot),
        );

        Self {
            state,
            core,
            tray_flags,
            egui_ctx_slot,
            _tray: tray_handle,
            _wake: wake,
            _wake_mutex: wake_mutex,
            listener_slot,
        }
    }

}


impl FutureProxyApp {
    fn set_status(&self, msg: String) {

        self.state.lock().unwrap().status_msg = msg;
        self.ctx.request_repaint();
    }

    fn select_node(&self, node_name: String) {
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let group = state.lock().unwrap().group_name.clone();
            let res = put_select(&group, &node_name);

            let mut app = state.lock().unwrap();
            match res {
                Ok(_) => {
                    app.current_selected = node_name.clone();
                    for n in &mut app.nodes {
                        n.is_selected = n.name == node_name;
                    }
                    app.status_msg = format!("已切换到 {}", node_name);
                }
                Err(e) => app.status_msg = format!("✗ 切换节点失败: {}", e),
            }
            drop(app);
            ctx.request_repaint();
        });
    }

    fn ping_all(&self) {
        let names = {
            let mut app = self.state.lock().unwrap();
            app.is_testing_delay = true;
            for n in &mut app.nodes {
                n.delay = -1;
            }
            app.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
        };
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // 分批测速，每批结果立即刷新到界面
            for chunk in names.chunks(16) {
                let results = test_delays(chunk);
                let mut app = state.lock().unwrap();
                for n in &mut app.nodes {
                    if let Some(d) = results.get(&n.name) {
                        n.delay = *d;
                    }
                }
                app.nodes.sort_by_key(|n| match n.delay {
                    d if d > 0 => (0, d),
                    -1 => (1, 0),
                    _ => (2, 0),
                });
                drop(app);
                ctx.request_repaint();
            }
            state.lock().unwrap().is_testing_delay = false;
            ctx.request_repaint();
        });
    }

    /// 下载订阅（新导入或更新已有），保存原始内容，若为激活订阅则重载内核
    fn download_sub(&self, existing_id: Option<String>, name: String, url: String) {
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        {
            let mut app = state.lock().unwrap();
            app.is_updating_sub = true;
            app.status_msg = format!("正在下载订阅 [{}]...", name);
        }
        std::thread::spawn(move || {
            let result = ureq::get(&url)
                .set("User-Agent", "clash.meta; mihomo")
                .timeout(Duration::from_secs(20))
                .call()
                .map_err(|e| format!("✗ 订阅下载失败: {}", e))
                .and_then(|r| r.into_string().map_err(|e| format!("✗ 读取订阅失败: {}", e)))
                .and_then(|body| {
                    if looks_like_clash_config(&body) {
                        Ok(body)
                    } else {
                        Err("✗ 订阅内容不是 Clash/Mihomo 格式，请检查订阅链接".to_string())
                    }
                });

            let mut app = state.lock().unwrap();
            app.is_updating_sub = false;
            match result {
                Ok(body) => {
                    let now = chrono_like_now();
                    let id = existing_id.unwrap_or_else(|| {
                        format!(
                            "sub_{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs()
                        )
                    });
                    if let Err(e) = std::fs::write(app.sub_file(&id), &body) {
                        app.status_msg = format!("✗ 保存订阅失败: {}", e);
                    } else {
                        if let Some(s) = app.profiles.subscriptions.iter_mut().find(|s| s.id == id) {
                            s.updated_at = now;
                        } else {
                            app.profiles.subscriptions.push(Subscription {
                                id: id.clone(),
                                name: name.clone(),
                                url,
                                updated_at: now,
                            });
                            app.profiles.active = id.clone();
                        }
                        app.save_profiles();
                        if app.profiles.active == id {
                            let msg = match app.write_active_config() {
                                Ok(()) => {
                                    let cfg = app.active_config();
                                    drop(app);
                                    match reload_core(&cfg) {
                                        Ok(()) => format!("✓ 订阅 [{}] 已更新并生效", name),
                                        Err(e) => format!("✗ {}", e),
                                    }
                                }
                                Err(e) => e,
                            };
                            let mut app = state.lock().unwrap();
                            app.status_msg = msg;
                            app.group_picked_by_user = false;
                        } else {
                            app.status_msg = format!("✓ 订阅 [{}] 已更新", name);
                        }
                    }
                }
                Err(e) => app.status_msg = e,
            }
            ctx.request_repaint();
        });
    }

    /// 切换订阅或 TUN 开关后，重新生成配置并重载内核
    fn apply_config(&self, msg_ok: &'static str) {
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = {
                let app = state.lock().unwrap();
                app.save_profiles();
                app.write_active_config().map(|_| app.active_config())
            };
            let msg = match res.and_then(|cfg| reload_core(&cfg).map_err(|e| format!("✗ {}", e))) {
                Ok(()) => msg_ok.to_string(),
                Err(e) => e,
            };
            let mut app = state.lock().unwrap();
            app.status_msg = msg;
            app.group_picked_by_user = false;
            drop(app);
            ctx.request_repaint();
        });
    }
}

fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + 8 * 3600; // UTC+8
    let days = secs.div_euclid(86400);
    let day_secs = secs.rem_euclid(86400);
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:02}-{:02} {:02}:{:02}", m, d, hour, min)
}

impl eframe::App for FutureProxyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray "退出": close the window; main() will see quit and exit the process.
        if self.tray_flags.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Tray "打开界面" while window already open: just focus (no Visible — unsupported on Wayland).
        if self.tray_flags.show.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        apply_cupertino_theme(ctx);

        let mut action_switch_sub = None;
        let mut action_update_sub = None;
        let mut action_ping = false;
        let mut action_select_node = None;
        let mut action_import = None;
        let mut toggle_sys_proxy = None;
        let mut toggle_tun = None;
        let mut action_retry_core = false;

        {
            let mut app = self.state.lock().unwrap();

            // === Top Panel: Cupertino Compact Navigation & Control Toolbar ===
            egui::TopBottomPanel::top("cup_top_panel")
                .frame(
                    egui::Frame::none()
                        .fill(CUP_BG_WINDOW)
                        .inner_margin(egui::Margin {
                            left: 16.0,
                            right: 16.0,
                            top: 12.0,
                            bottom: 8.0,
                        }),
                )
                .show(ctx, |ui| {
                    // --- Row 1: Cupertino Header + Telemetry + System Proxy / TUN Toolbar Switches ---
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;

                        // Apple 风格圆角品牌图标徽章 (24x24)
                        let (logo_rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        let lp = ui.painter();
                        lp.rect_filled(logo_rect, 6.0, CUP_BLUE);
                        draw_sf_bolt(&lp, logo_rect.center(), 7.0, egui::Color32::WHITE);

                        // 主标题
                        let title_g = ui.fonts(|f| {
                            f.layout_no_wrap("Future Proxy".to_string(), cup_font_med(15.0), CUP_LABEL_PRIMARY)
                        });
                        let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
                        ui.painter().galley(t_rect.min, title_g, CUP_LABEL_PRIMARY);

                        // 运行状态 Cupertino Tinted Pill
                        if app.is_core_alive {
                            let mode_txt = if app.profiles.tun {
                                "TUN 增强模式"
                            } else if app.is_system_proxy_on {
                                "系统代理已启用"
                            } else {
                                "内核就绪"
                            };
                            let dot_col = if app.profiles.tun || app.is_system_proxy_on {
                                CUP_GREEN
                            } else {
                                CUP_TEAL
                            };
                            let pill_bg = egui::Color32::from_rgba_unmultiplied(dot_col.r(), dot_col.g(), dot_col.b(), 28);
                            let pill_border = egui::Color32::from_rgba_unmultiplied(dot_col.r(), dot_col.g(), dot_col.b(), 72);
                            let g = ui.fonts(|f| f.layout_no_wrap(mode_txt.to_string(), cup_font_med(11.0), dot_col));
                            let pw = g.size().x + 22.0;
                            let (p_rect, _) = ui.allocate_exact_size(egui::vec2(pw, 22.0), egui::Sense::hover());
                            let pp = ui.painter();
                            pp.rect(p_rect.shrink(0.5), 11.0, pill_bg, egui::Stroke::new(1.0_f32, pill_border));
                            pp.circle_filled(egui::pos2(p_rect.left() + 9.5, p_rect.center().y), 3.0, dot_col);
                            pp.galley(
                                egui::pos2(p_rect.left() + 16.0, p_rect.center().y - g.size().y * 0.5),
                                g,
                                dot_col,
                            );
                        } else if app.core_error.is_empty() {
                            let g = ui.fonts(|f| f.layout_no_wrap("内核启动中…".to_string(), cup_font_med(11.0), CUP_ORANGE));
                            let (p_rect, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 18.0, 22.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                p_rect,
                                11.0,
                                egui::Color32::from_rgba_unmultiplied(255, 159, 10, 32),
                            );
                            ui.painter().galley(
                                egui::pos2(p_rect.left() + 9.0, p_rect.center().y - g.size().y * 0.5),
                                g,
                                CUP_ORANGE,
                            );
                        } else {
                            let g = ui.fonts(|f| f.layout_no_wrap("内核未运行".to_string(), cup_font_med(11.0), CUP_RED));
                            let (p_rect, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 18.0, 22.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                p_rect,
                                11.0,
                                egui::Color32::from_rgba_unmultiplied(255, 69, 58, 32),
                            );
                            ui.painter().galley(
                                egui::pos2(p_rect.left() + 9.0, p_rect.center().y - g.size().y * 0.5),
                                g,
                                CUP_RED,
                            );
                            if cupertino_button(ui, "重试", None, CupBtnStyle::Tinted(CUP_BLUE), true) {
                                action_retry_core = true;
                            }
                        }

                        // 右侧：偏好设置 + 速率胶囊 + TUN 模式开关 + 系统代理开关
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 7.0;
                            if cupertino_button(ui, "偏好设置", Some(CupIcon::Sliders), CupBtnStyle::Secondary, true) {
                                app.show_settings_dialog = true;
                                app.open_popup = None;
                            }

                            // Cupertino Telemetry Pill
                            let (sp_rect, _) = ui.allocate_exact_size(egui::vec2(158.0, 28.0), egui::Sense::hover());
                            let sp = ui.painter();
                            sp.rect(sp_rect.shrink(0.5), 7.5, CUP_BG_GROUPED, egui::Stroke::new(1.0_f32, CUP_HAIRLINE));
                            let mid_x = sp_rect.center().x;
                            sp.line_segment(
                                [egui::pos2(mid_x, sp_rect.top() + 6.0), egui::pos2(mid_x, sp_rect.bottom() - 6.0)],
                                egui::Stroke::new(1.0_f32, CUP_HAIRLINE),
                            );

                            let down_str = format!("↓ {}", format_speed(app.download_rate));
                            let up_str = format!("↑ {}", format_speed(app.upload_rate));
                            sp.text(
                                egui::pos2((sp_rect.left() + mid_x) * 0.5, sp_rect.center().y),
                                egui::Align2::CENTER_CENTER,
                                down_str,
                                cup_font_mono(10.5),
                                CUP_GREEN,
                            );
                            sp.text(
                                egui::pos2((mid_x + sp_rect.right()) * 0.5, sp_rect.center().y),
                                egui::Align2::CENTER_CENTER,
                                up_str,
                                cup_font_mono(10.5),
                                CUP_TEAL,
                            );

                            let tun_on = app.profiles.tun;
                            if cupertino_toolbar_switch(ui, "TUN 模式", tun_on, CUP_INDIGO, true) {
                                toggle_tun = Some(!tun_on);
                            }

                            let sys_on = app.is_system_proxy_on;
                            if cupertino_toolbar_switch(ui, "系统代理", sys_on, CUP_BLUE, false) {
                                toggle_sys_proxy = Some(!sys_on);
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // --- Row 2: 订阅管理 + 策略组选择 + 搜索框 + 全部测速 (一体成型紧凑工具行) ---
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;

                        let active = app.profiles.subscriptions.iter().find(|s| s.id == app.profiles.active).cloned();
                        let active_name = active
                            .as_ref()
                            .map(|s| format_cupertino_title(&s.name))
                            .unwrap_or_else(|| "未选择".to_string());
                        let is_sub_open = app.open_popup.as_deref() == Some("sub");
                        let sub_w = 185.0;
                        let (sub_rect, sub_clicked) = cupertino_popup_trigger(ui, "订阅", &active_name, sub_w, is_sub_open);
                        if sub_clicked {
                            if is_sub_open {
                                app.open_popup = None;
                            } else {
                                app.open_popup = Some("sub".to_string());
                                app.popup_anchor = sub_rect.left_bottom();
                                app.popup_width = 220.0;
                            }
                        }

                        if let Some(s) = active {
                            let upd_txt = if app.is_updating_sub { "更新中" } else { "更新" };
                            if cupertino_button(ui, upd_txt, Some(CupIcon::Refresh), CupBtnStyle::Secondary, !app.is_updating_sub) {
                                action_update_sub = Some(s);
                            }
                        }

                        if cupertino_button(ui, "添加", Some(CupIcon::Plus), CupBtnStyle::Secondary, true) {
                            app.show_import_dialog = true;
                            app.open_popup = None;
                        }

                        if app.groups.len() > 1 {
                            let g_clean = format_cupertino_title(&app.group_name);
                            let is_grp_open = app.open_popup.as_deref() == Some("group");
                            let (grp_rect, grp_clicked) = cupertino_popup_trigger(ui, "策略", &g_clean, 132.0, is_grp_open);
                            if grp_clicked {
                                if is_grp_open {
                                    app.open_popup = None;
                                } else {
                                    app.open_popup = Some("group".to_string());
                                    app.popup_anchor = grp_rect.left_bottom();
                                    app.popup_width = 170.0;
                                }
                            }
                        }

                        let ping_txt = if app.is_testing_delay { "测速中…" } else { "全部测速" };
                        let ping_reserve = 88.0;
                        let search_w = (ui.available_width() - ping_reserve - 7.0).max(120.0);
                        cupertino_search_field(ui, &mut app.search_keyword, search_w);

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if cupertino_button(
                                ui,
                                ping_txt,
                                Some(CupIcon::Bolt),
                                CupBtnStyle::Tinted(CUP_BLUE),
                                !app.is_testing_delay && app.is_core_alive,
                            ) {
                                action_ping = true;
                            }
                        });
                    });

                    ui.add_space(7.0);

                    // --- Row 3: Cupertino Segmented Control + 右侧智能自愈与当前节点摘要 ---
                    ui.horizontal(|ui| {
                        cupertino_segmented_control(ui, &REGIONS, &mut app.selected_region);

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            let region = app.selected_region.clone();
                            if let Some(target_name) = region_smart_group(&region) {
                                let is_active = app.current_selected == target_name;
                                let (label, style) = if is_active {
                                    ("智能自愈中", CupBtnStyle::Tinted(CUP_GREEN))
                                } else {
                                    ("开启本区自愈", CupBtnStyle::Tinted(CUP_BLUE))
                                };
                                if cupertino_button(ui, label, Some(CupIcon::Bolt), style, true) && !is_active {
                                    let group = app.group_name.clone();
                                    match put_select(&group, target_name) {
                                        Ok(()) => {
                                            app.current_selected = target_name.to_string();
                                            for n in &mut app.nodes {
                                                n.is_selected = n.name == target_name;
                                            }
                                            app.profiles.auto_region = Some(region.clone());
                                            app.save_profiles();
                                            app.status_msg = format!("已启用 {} 内核智能自愈", format_cupertino_title(&region));
                                        }
                                        Err(e) => {
                                            app.status_msg = format!("切换失败: {}", e);
                                        }
                                    }
                                }
                            } else if let Some(r) = app.profiles.auto_region.clone() {
                                let clean_r = format_cupertino_title(&r);
                                if cupertino_button(
                                    ui,
                                    &format!("{}自愈 · 关闭", clean_r),
                                    Some(CupIcon::Bolt),
                                    CupBtnStyle::Tinted(CUP_GREEN),
                                    true,
                                ) {
                                    app.profiles.auto_region = None;
                                    app.save_profiles();
                                }
                            }

                            if !app.current_selected.is_empty() {
                                let (c_clean, _) = parse_node_display(&app.current_selected);
                                let cur_str = format!("当前: {}", format_cupertino_title(&c_clean));
                                let cg = ui.fonts(|f| f.layout_no_wrap(cur_str, cup_font_med(11.0), CUP_LABEL_SECONDARY));
                                let (c_rect, _) = ui.allocate_exact_size(cg.size(), egui::Sense::hover());
                                ui.painter().galley(c_rect.min, cg, CUP_LABEL_SECONDARY);
                            }
                        });
                    });

                    // 顶栏底部柔和分隔线
                    let div_y = ui.max_rect().bottom() + 7.0;
                    ui.painter().line_segment(
                        [egui::pos2(ui.max_rect().left() - 16.0, div_y), egui::pos2(ui.max_rect().right() + 16.0, div_y)],
                        egui::Stroke::new(1.0_f32, CUP_DIVIDER),
                    );
                });

            // === Bottom Panel: Cupertino Hairline Status Bar ===
            egui::TopBottomPanel::bottom("cup_bottom_panel")
                .frame(
                    egui::Frame::none()
                        .fill(CUP_BG_WINDOW)
                        .inner_margin(egui::Margin {
                            left: 16.0,
                            right: 16.0,
                            top: 5.0,
                            bottom: 6.0,
                        }),
                )
                .show(ctx, |ui| {
                    let top_y = ui.max_rect().top() - 5.0;
                    ui.painter().line_segment(
                        [egui::pos2(ui.max_rect().left() - 16.0, top_y), egui::pos2(ui.max_rect().right() + 16.0, top_y)],
                        egui::Stroke::new(1.0_f32, CUP_DIVIDER),
                    );
                    ui.horizontal(|ui| {
                        let raw_msg = if !app.core_error.is_empty() {
                            app.core_error.clone()
                        } else {
                            app.status_msg.clone()
                        };
                        let clean_msg = raw_msg.trim_start_matches(['✓', '✗', ' ']);
                        let dot_c = if !app.core_error.is_empty() || raw_msg.starts_with('✗') {
                            CUP_RED
                        } else {
                            CUP_GREEN
                        };
                        let (d_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
                        ui.painter().circle_filled(d_rect.center(), 2.8, dot_c);
                        let g = ui.fonts(|f| f.layout_no_wrap(clean_msg.to_string(), cup_font_reg(11.0), CUP_LABEL_SECONDARY));
                        let (l_rect, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
                        ui.painter().galley(l_rect.min, g, CUP_LABEL_SECONDARY);

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mem_core_str = if app.mem_core_kb > 0 {
                                format!("{:.0} MB", app.mem_core_kb as f64 / 1024.0)
                            } else if systemd_active() {
                                if cfg!(windows) {
                                    "后台守护".to_string()
                                } else {
                                    "systemd 守护".to_string()
                                }
                            } else {
                                "-".to_string()
                            };
                            let mem_txt = format!(
                                "共 {} 节点  ·  界面 {:.0} MB  ·  内核 {}",
                                app.nodes.len(),
                                app.mem_gui_kb as f64 / 1024.0,
                                mem_core_str
                            );
                            let mg = ui.fonts(|f| f.layout_no_wrap(mem_txt, cup_font_reg(11.0), CUP_LABEL_TERTIARY));
                            let (m_rect, _) = ui.allocate_exact_size(mg.size(), egui::Sense::hover());
                            ui.painter().galley(m_rect.min, mg, CUP_LABEL_TERTIARY);
                        });
                    });
                });

            // === Central Panel: High-Density Cupertino Node Cards Grid ===
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(CUP_BG_WINDOW)
                        .inner_margin(egui::Margin {
                            left: 16.0,
                            right: 16.0,
                            top: 8.0,
                            bottom: 6.0,
                        }),
                )
                .show(ctx, |ui| {
                    let keyword = app.search_keyword.to_lowercase();
                    let region = app.selected_region.clone();

                    let mut filtered_nodes: Vec<ProxyNode> = app
                        .nodes
                        .iter()
                        .filter(|n| {
                            let match_kw = keyword.is_empty() || n.name.to_lowercase().contains(&keyword);
                            match_kw && region_match(&region, &n.name)
                        })
                        .cloned()
                        .collect();

                    filtered_nodes.sort_by_key(|n| {
                        let is_smart = n.name.starts_with("⚡")
                            || n.name == "自动选择"
                            || n.name == "故障转移"
                            || n.name == "🚀直接连接";
                        let type_rank = if is_smart { 0 } else { 1 };
                        let delay_rank = match n.delay {
                            d if d > 0 => (0, d),
                            -1 => (1, 0),
                            _ => (2, 0),
                        };
                        (type_rank, delay_rank)
                    });

                    if app.profiles.subscriptions.is_empty() {
                        let (empty_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 110.0), egui::Sense::hover());
                        ui.painter().rect(
                            empty_rect.shrink(1.0),
                            10.0,
                            CUP_BG_GROUPED,
                            egui::Stroke::new(1.0_f32, CUP_HAIRLINE),
                        );
                        ui.painter().text(
                            empty_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "尚未添加任何订阅，请点击上方「添加」导入配置链接",
                            cup_font_reg(12.5),
                            CUP_LABEL_SECONDARY,
                        );
                    } else if app.nodes.is_empty() && app.is_core_alive {
                        let (empty_rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::hover());
                        ui.painter().text(
                            empty_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "正在同步节点列表…",
                            cup_font_reg(12.5),
                            CUP_LABEL_SECONDARY,
                        );
                    }

                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        let gap = 7.0;
                        let avail = ui.available_width();
                        // 紧凑高密度列宽：最小 172px，常规窗口稳定 4 列或 5 列，单屏可一览 36~45 个节点
                        let cols = ((avail + gap) / (172.0 + gap)).floor().max(1.0) as usize;
                        let card_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                        let card_h = 46.0;

                        for row in filtered_nodes.chunks(cols) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for node in row {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, card_h), egui::Sense::click());
                                    if resp.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    let is_sel = node.is_selected;
                                    let is_smart = node.name.starts_with("⚡")
                                        || node.name == "自动选择"
                                        || node.name == "故障转移";

                                    let fill = if is_sel {
                                        egui::Color32::from_rgb(17, 38, 68)
                                    } else if is_smart {
                                        if resp.hovered() {
                                            egui::Color32::from_rgb(36, 36, 50)
                                        } else {
                                            egui::Color32::from_rgb(27, 27, 37)
                                        }
                                    } else if resp.hovered() {
                                        CUP_BG_SECONDARY
                                    } else {
                                        CUP_BG_GROUPED
                                    };

                                    let stroke = if is_sel {
                                        egui::Stroke::new(1.5_f32, CUP_BLUE)
                                    } else if is_smart {
                                        if resp.hovered() {
                                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(94, 92, 230))
                                        } else {
                                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(56, 55, 92))
                                        }
                                    } else if resp.hovered() {
                                        egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT)
                                    } else {
                                        egui::Stroke::new(1.0_f32, CUP_HAIRLINE)
                                    };

                                    // 关键修复：使用父级 ui.painter() + rect.shrink(1.0) 绘制完整闭合边框，绝不裁剪四条直边
                                    let painter = ui.painter();
                                    let draw_rect = rect.shrink(1.0);
                                    painter.rect(draw_rect, 8.5, fill, stroke);

                                    // --- 第一行：紧凑地区代码胶囊 + 完整节点名 + 倍率微胶囊 ---
                                    let badge_code = if node.flag.is_empty() {
                                        match_flag(&node.name)
                                    } else {
                                        node.flag.as_str()
                                    };
                                    let (badge_bg, badge_fg) = if is_sel {
                                        (CUP_BLUE, egui::Color32::WHITE)
                                    } else if is_smart {
                                        (CUP_INDIGO, egui::Color32::WHITE)
                                    } else if badge_code == "DIR" {
                                        (egui::Color32::from_rgba_unmultiplied(48, 209, 88, 38), CUP_GREEN)
                                    } else {
                                        (CUP_BG_TERTIARY, egui::Color32::from_rgb(200, 200, 208))
                                    };

                                    let top_row_y = draw_rect.top() + 13.5;
                                    let badge_rect = egui::Rect::from_center_size(
                                        egui::pos2(draw_rect.left() + 21.5, top_row_y),
                                        egui::vec2(27.0, 15.0),
                                    );
                                    painter.rect_filled(badge_rect, 4.0, badge_bg);
                                    painter.text(
                                        badge_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        badge_code,
                                        cup_font_med(9.0),
                                        badge_fg,
                                    );

                                    let (raw_clean, rate_opt) = parse_node_display(&node.name);
                                    let display_title = format_cupertino_title(&raw_clean);
                                    let title_col = if is_sel {
                                        egui::Color32::WHITE
                                    } else {
                                        CUP_LABEL_PRIMARY
                                    };

                                    let title_x = 40.5;
                                    let right_pad = if rate_opt.is_some() { 34.0 } else { 8.0 };
                                    let mut job = egui::text::LayoutJob::single_section(
                                        display_title,
                                        egui::TextFormat {
                                            font_id: cup_font_med(11.5),
                                            color: title_col,
                                            ..Default::default()
                                        },
                                    );
                                    job.wrap = egui::text::TextWrapping {
                                        max_width: (draw_rect.width() - title_x - right_pad).max(24.0),
                                        max_rows: 1,
                                        break_anywhere: true,
                                        overflow_character: Some('…'),
                                    };
                                    let galley = ui.fonts(|f| f.layout_job(job));
                                    painter.galley(
                                        egui::pos2(draw_rect.left() + title_x, top_row_y - galley.size().y * 0.5),
                                        galley,
                                        title_col,
                                    );

                                    // 右上角 Apple Orange 倍率微胶囊
                                    if let Some(rate) = rate_opt {
                                        let r_rect = egui::Rect::from_center_size(
                                            egui::pos2(draw_rect.right() - 18.0, top_row_y),
                                            egui::vec2(23.0, 14.5),
                                        );
                                        painter.rect(
                                            r_rect,
                                            3.5,
                                            egui::Color32::from_rgba_unmultiplied(255, 159, 10, 32),
                                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(135, 88, 20)),
                                        );
                                        painter.text(
                                            r_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            rate,
                                            cup_font_med(9.0),
                                            CUP_ORANGE,
                                        );
                                    }

                                    // --- 第二行：信号圆点 + 等宽延迟 + 右下角连接状态 ---
                                    let (dtxt, dot_col) = if node.delay > 0 {
                                        let c = if node.delay < 150 {
                                            CUP_GREEN
                                        } else if node.delay < 350 {
                                            CUP_ORANGE
                                        } else {
                                            CUP_RED
                                        };
                                        (format!("{} ms", node.delay), c)
                                    } else if node.delay == 0 {
                                        ("超时".to_string(), CUP_RED)
                                    } else {
                                        ("未测速".to_string(), CUP_LABEL_TERTIARY)
                                    };

                                    let bottom_y = draw_rect.bottom() - 11.5;
                                    let dot_pos = egui::pos2(draw_rect.left() + 11.5, bottom_y);
                                    painter.circle_filled(dot_pos, 2.6, dot_col);
                                    painter.text(
                                        egui::pos2(dot_pos.x + 6.5, bottom_y),
                                        egui::Align2::LEFT_CENTER,
                                        dtxt,
                                        cup_font_mono(10.0),
                                        dot_col,
                                    );

                                    if is_sel {
                                        let chk_c = egui::pos2(draw_rect.right() - 13.5, bottom_y);
                                        painter.circle_filled(chk_c, 6.2, CUP_BLUE);
                                        draw_sf_checkmark(&painter, chk_c, 0.72, egui::Color32::WHITE);
                                        painter.text(
                                            egui::pos2(chk_c.x - 9.0, bottom_y),
                                            egui::Align2::RIGHT_CENTER,
                                            "已连接",
                                            cup_font_med(9.8),
                                            CUP_BLUE,
                                        );
                                    } else if is_smart {
                                        painter.text(
                                            egui::pos2(draw_rect.right() - 8.5, bottom_y),
                                            egui::Align2::RIGHT_CENTER,
                                            "自动容灾",
                                            cup_font_reg(9.8),
                                            CUP_TEAL,
                                        );
                                    }

                                    if !is_sel && resp.clicked() {
                                        action_select_node = Some(node.name.clone());
                                        app.open_popup = None;
                                    }
                                }
                            });
                            ui.add_space(gap);
                        }
                    });
                });

            // === Top-level Cupertino Popover Menu (订阅切换 / 策略组切换) ===
            if let Some(kind) = app.open_popup.clone() {
                let mut close_pop = false;
                let anchor = app.popup_anchor + egui::vec2(0.0, 6.0);
                let pop_w = app.popup_width;

                egui::Area::new(egui::Id::new("cup_popover_menu"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(anchor)
                    .show(ctx, |ui| {
                        let items: Vec<(String, String, bool)> = if kind == "sub" {
                            app.profiles
                                .subscriptions
                                .iter()
                                .map(|s| {
                                    let lbl = format!("{} ({})", format_cupertino_title(&s.name), s.updated_at);
                                    (s.id.clone(), lbl, s.id == app.profiles.active)
                                })
                                .collect()
                        } else {
                            app.groups
                                .iter()
                                .map(|g| (g.clone(), format_cupertino_title(g), *g == app.group_name))
                                .collect()
                        };

                        let row_h = 30.0;
                        let pad = 5.0;
                        let total_h = (items.len().max(1) as f32) * row_h + pad * 2.0;
                        let (menu_rect, _) = ui.allocate_exact_size(egui::vec2(pop_w, total_h), egui::Sense::hover());
                        let mp = ui.painter();

                        // Popover 柔和投影与毛玻璃深色面板
                        mp.rect_filled(
                            menu_rect.translate(egui::vec2(0.0, 4.0)).expand(2.0),
                            11.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
                        );
                        mp.rect(
                            menu_rect.shrink(0.5),
                            10.0,
                            egui::Color32::from_rgb(38, 38, 43),
                            egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT),
                        );

                        for (idx, (id_val, label, is_cur)) in items.iter().enumerate() {
                            let r_rect = egui::Rect::from_min_size(
                                egui::pos2(menu_rect.left() + pad, menu_rect.top() + pad + idx as f32 * row_h),
                                egui::vec2(pop_w - pad * 2.0, row_h),
                            );
                            let r_resp = ui.interact(r_rect, ui.id().with(("pop_row", idx)), egui::Sense::click());
                            if r_resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                mp.rect_filled(r_rect, 6.5, CUP_BLUE);
                            }
                            let fg = if r_resp.hovered() || *is_cur {
                                egui::Color32::WHITE
                            } else {
                                CUP_LABEL_PRIMARY
                            };
                            if *is_cur {
                                let chk_col = if r_resp.hovered() { egui::Color32::WHITE } else { CUP_BLUE };
                                draw_sf_checkmark(&mp, egui::pos2(r_rect.left() + 12.0, r_rect.center().y), 0.9, chk_col);
                            }
                            mp.text(
                                egui::pos2(r_rect.left() + 26.0, r_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                label,
                                cup_font_med(12.0),
                                fg,
                            );
                            if r_resp.clicked() {
                                if kind == "sub" {
                                    if *id_val != app.profiles.active {
                                        action_switch_sub = Some(id_val.clone());
                                    }
                                } else if *id_val != app.group_name {
                                    app.group_name = id_val.clone();
                                    app.group_picked_by_user = true;
                                    app.nodes.clear();
                                }
                                close_pop = true;
                            }
                        }

                        if ctx.input(|i| i.pointer.any_pressed()) {
                            let ptr = ctx.input(|i| i.pointer.interact_pos());
                            if let Some(p) = ptr {
                                if !menu_rect.contains(p) && p.y > anchor.y {
                                    close_pop = true;
                                }
                            }
                        }
                    });

                if close_pop || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.open_popup = None;
                }
            }

            // === Cupertino Modal Sheet: 添加新订阅 ===
            if app.show_import_dialog {
                let mut close = false;
                let screen_rect = ctx.screen_rect();
                egui::Area::new(egui::Id::new("cup_import_sheet"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(screen_rect.min)
                    .show(ctx, |ui| {
                        let (_, bg_resp) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                        ui.painter().rect_filled(
                            screen_rect,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 145),
                        );

                        let sheet_w = 430.0;
                        let sheet_h = 250.0;
                        let sheet_rect = egui::Rect::from_center_size(screen_rect.center(), egui::vec2(sheet_w, sheet_h));
                        // 拦截 Sheet 内部点击防止穿透到遮罩
                        let _ = ui.interact(sheet_rect, ui.id().with("import_sheet_body"), egui::Sense::click());

                        let sp = ui.painter();
                        sp.rect_filled(
                            sheet_rect.translate(egui::vec2(0.0, 8.0)).expand(4.0),
                            18.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 140),
                        );
                        sp.rect(
                            sheet_rect,
                            16.0,
                            CUP_BG_GROUPED,
                            egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT),
                        );

                        let inner = sheet_rect.shrink(20.0);
                        let mut s_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                        s_ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let g = ui.fonts(|f| f.layout_no_wrap("添加新订阅".to_string(), cup_font_med(15.0), CUP_LABEL_PRIMARY));
                                let (r, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
                                ui.painter().galley(r.min, g, CUP_LABEL_PRIMARY);
                            });
                            ui.add_space(12.0);

                            // Inset Grouped Form Container
                            let form_w = inner.width();
                            let (f_rect, _) = ui.allocate_exact_size(egui::vec2(form_w, 96.0), egui::Sense::hover());
                            let fp = ui.painter().clone();
                            fp.rect(f_rect.shrink(0.5), 10.0, CUP_BG_SECONDARY, egui::Stroke::new(1.0_f32, CUP_HAIRLINE));
                            fp.line_segment(
                                [egui::pos2(f_rect.left() + 12.0, f_rect.center().y), egui::pos2(f_rect.right() - 1.0, f_rect.center().y)],
                                egui::Stroke::new(1.0_f32, CUP_HAIRLINE),
                            );

                            // Row 1: 名称
                            let r1 = egui::Rect::from_min_max(f_rect.left_top(), egui::pos2(f_rect.right(), f_rect.center().y));
                            fp.text(
                                egui::pos2(r1.left() + 14.0, r1.center().y),
                                egui::Align2::LEFT_CENTER,
                                "订阅名称",
                                cup_font_med(12.5),
                                CUP_LABEL_PRIMARY,
                            );
                            let e1_rect = egui::Rect::from_min_max(
                                egui::pos2(r1.left() + 84.0, r1.top() + 12.0),
                                egui::pos2(r1.right() - 12.0, r1.bottom() - 12.0),
                            );
                            let mut e1_ui = ui.new_child(egui::UiBuilder::new().max_rect(e1_rect));
                            e1_ui.add(
                                egui::TextEdit::singleline(&mut app.import_name)
                                    .frame(false)
                                    .font(cup_font_reg(12.5))
                                    .hint_text(egui::RichText::new("例如：主力节点 / 备用机场").color(CUP_LABEL_TERTIARY).size(12.0))
                                    .desired_width(e1_rect.width()),
                            );

                            // Row 2: 链接
                            let r2 = egui::Rect::from_min_max(egui::pos2(f_rect.left(), f_rect.center().y), f_rect.right_bottom());
                            fp.text(
                                egui::pos2(r2.left() + 14.0, r2.center().y),
                                egui::Align2::LEFT_CENTER,
                                "订阅地址",
                                cup_font_med(12.5),
                                CUP_LABEL_PRIMARY,
                            );
                            let e2_rect = egui::Rect::from_min_max(
                                egui::pos2(r2.left() + 84.0, r2.top() + 12.0),
                                egui::pos2(r2.right() - 12.0, r2.bottom() - 12.0),
                            );
                            let mut e2_ui = ui.new_child(egui::UiBuilder::new().max_rect(e2_rect));
                            e2_ui.add(
                                egui::TextEdit::singleline(&mut app.import_url)
                                    .frame(false)
                                    .font(cup_font_reg(12.5))
                                    .hint_text(egui::RichText::new("https://… (Clash / Mihomo 订阅链接)").color(CUP_LABEL_TERTIARY).size(12.0))
                                    .desired_width(e2_rect.width()),
                            );

                            ui.add_space(18.0);
                            ui.horizontal(|ui| {
                                if cupertino_button(ui, "取消", None, CupBtnStyle::Secondary, true) {
                                    close = true;
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let can_submit = !app.import_url.trim().is_empty();
                                    if cupertino_button(ui, "下载并添加", None, CupBtnStyle::Filled(CUP_BLUE), can_submit) {
                                        let mut name = app.import_name.trim().to_string();
                                        if name.is_empty() {
                                            name = "新订阅".to_string();
                                        }
                                        action_import = Some((name, app.import_url.trim().to_string()));
                                        close = true;
                                    }
                                });
                            });
                        });

                        if bg_resp.clicked() && !sheet_rect.contains(bg_resp.interact_pointer_pos().unwrap_or_default()) {
                            close = true;
                        }
                    });
                if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.show_import_dialog = false;
                    app.import_name.clear();
                    app.import_url.clear();
                }
            }

            // === Cupertino Modal Sheet: 系统偏好设置 ===
            if app.show_settings_dialog {
                let mut close = false;
                let screen_rect = ctx.screen_rect();
                egui::Area::new(egui::Id::new("cup_settings_sheet"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(screen_rect.min)
                    .show(ctx, |ui| {
                        let (_, bg_resp) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
                        ui.painter().rect_filled(
                            screen_rect,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 145),
                        );

                        let sheet_w = 430.0;
                        let sheet_h = 264.0;
                        let sheet_rect = egui::Rect::from_center_size(screen_rect.center(), egui::vec2(sheet_w, sheet_h));
                        let _ = ui.interact(sheet_rect, ui.id().with("settings_sheet_body"), egui::Sense::click());

                        let sp = ui.painter();
                        sp.rect_filled(
                            sheet_rect.translate(egui::vec2(0.0, 8.0)).expand(4.0),
                            18.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 140),
                        );
                        sp.rect(
                            sheet_rect,
                            16.0,
                            CUP_BG_GROUPED,
                            egui::Stroke::new(1.0_f32, CUP_HAIRLINE_BRIGHT),
                        );

                        let inner = sheet_rect.shrink(20.0);
                        let mut s_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                        s_ui.vertical(|ui| {
                            let g = ui.fonts(|f| f.layout_no_wrap("系统偏好设置".to_string(), cup_font_med(15.0), CUP_LABEL_PRIMARY));
                            let (r, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
                            ui.painter().galley(r.min, g, CUP_LABEL_PRIMARY);
                            ui.add_space(12.0);

                            // Inset Grouped TableView Card
                            let card_w = inner.width();
                            let (c_rect, _) = ui.allocate_exact_size(egui::vec2(card_w, 116.0), egui::Sense::hover());
                            let cp = ui.painter().clone();
                            cp.rect(c_rect.shrink(0.5), 11.0, CUP_BG_SECONDARY, egui::Stroke::new(1.0_f32, CUP_HAIRLINE));
                            cp.line_segment(
                                [egui::pos2(c_rect.left() + 14.0, c_rect.center().y), egui::pos2(c_rect.right() - 1.0, c_rect.center().y)],
                                egui::Stroke::new(1.0_f32, CUP_HAIRLINE),
                            );

                            // Row 1: 关闭窗口后常驻托盘
                            let row1 = egui::Rect::from_min_max(c_rect.left_top(), egui::pos2(c_rect.right(), c_rect.center().y));
                            cp.text(
                                egui::pos2(row1.left() + 14.0, row1.center().y - 8.0),
                                egui::Align2::LEFT_CENTER,
                                "后台常驻运行",
                                cup_font_med(13.0),
                                CUP_LABEL_PRIMARY,
                            );
                            cp.text(
                                egui::pos2(row1.left() + 14.0, row1.center().y + 9.0),
                                egui::Align2::LEFT_CENTER,
                                "关闭主窗口后保持代理与托盘常驻",
                                cup_font_reg(11.0),
                                CUP_LABEL_SECONDARY,
                            );
                            let sw1_rect = egui::Rect::from_center_size(
                                egui::pos2(row1.right() - 35.0, row1.center().y),
                                egui::vec2(42.0, 24.0),
                            );
                            let mut sw1_ui = ui.new_child(egui::UiBuilder::new().max_rect(sw1_rect));
                            if cupertino_switch(&mut sw1_ui, app.profiles.keep_core_on_exit) {
                                app.profiles.keep_core_on_exit = !app.profiles.keep_core_on_exit;
                                app.save_profiles();
                            }

                            // Row 2: 开机自启内核
                            let row2 = egui::Rect::from_min_max(egui::pos2(c_rect.left(), c_rect.center().y), c_rect.right_bottom());
                            cp.text(
                                egui::pos2(row2.left() + 14.0, row2.center().y - 8.0),
                                egui::Align2::LEFT_CENTER,
                                "开机自动启动",
                                cup_font_med(13.0),
                                CUP_LABEL_PRIMARY,
                            );
                            let auto_desc = if cfg!(windows) {
                                "登录 Windows 时由系统后台任务静默守护运行"
                            } else {
                                "登录桌面时由 systemd 用户服务守护运行"
                            };
                            cp.text(
                                egui::pos2(row2.left() + 14.0, row2.center().y + 9.0),
                                egui::Align2::LEFT_CENTER,
                                auto_desc,
                                cup_font_reg(11.0),
                                CUP_LABEL_SECONDARY,
                            );
                            let sw2_rect = egui::Rect::from_center_size(
                                egui::pos2(row2.right() - 35.0, row2.center().y),
                                egui::vec2(42.0, 24.0),
                            );
                            let mut sw2_ui = ui.new_child(egui::UiBuilder::new().max_rect(sw2_rect));
                            let cur_auto = app.profiles.autostart;
                            if cupertino_switch(&mut sw2_ui, cur_auto) {
                                let next_auto = !cur_auto;
                                match set_autostart(next_auto) {
                                    Ok(()) => {
                                        app.profiles.autostart = next_auto;
                                        if next_auto {
                                            app.profiles.keep_core_on_exit = true;
                                        }
                                        app.save_profiles();
                                        app.status_msg = if next_auto {
                                            if cfg!(windows) {
                                                "已开启开机自启 (Windows 后台任务与启动项)".to_string()
                                            } else {
                                                "已开启开机自启 (systemd 用户服务)".to_string()
                                            }
                                        } else {
                                            "已关闭开机自启".to_string()
                                        };
                                    }
                                    Err(e) => {
                                        app.status_msg = format!("设置自启失败: {}", e);
                                    }
                                }
                            }

                            ui.add_space(18.0);
                            ui.horizontal(|ui| {
                                if cupertino_button(ui, "隐藏窗口到托盘", None, CupBtnStyle::Tinted(CUP_BLUE), true) {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if cupertino_button(ui, "完成", None, CupBtnStyle::Filled(CUP_BLUE), true) {
                                        close = true;
                                    }
                                });
                            });
                        });

                        if bg_resp.clicked() && !sheet_rect.contains(bg_resp.interact_pointer_pos().unwrap_or_default()) {
                            close = true;
                        }
                    });
                if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.show_settings_dialog = false;
                }
            }
        }

        if let Some(enable) = toggle_sys_proxy {
            set_kde_proxy(enable);
            let on = check_kde_proxy_enabled();
            let mut app = self.state.lock().unwrap();
            app.is_system_proxy_on = on;
            app.status_msg = if on == enable {
                if on {
                    format!("系统代理已开启 (127.0.0.1:{})", MIXED_PORT)
                } else {
                    "系统代理已关闭".to_string()
                }
            } else {
                "✗ 修改系统代理失败".to_string()
            };
        }
        if let Some(on) = toggle_tun {
            self.state.lock().unwrap().profiles.tun = on;
            self.apply_config(if on { "✓ TUN 已开启" } else { "✓ TUN 已关闭" });
        }
        if action_retry_core {
            self.core.lock().unwrap().reset();
            self.set_status("正在重新启动内核...".to_string());
        }
        if let Some(sub_id) = action_switch_sub {
            self.state.lock().unwrap().profiles.active = sub_id;
            self.apply_config("✓ 已切换订阅");
        }
        if let Some(s) = action_update_sub {
            self.download_sub(Some(s.id), s.name, s.url);
        }
        if action_ping {
            self.ping_all();
        }
        if let Some(node_name) = action_select_node {
            {
                let mut app = self.state.lock().unwrap();
                if let Some(r) = app.profiles.auto_region.clone() {
                    if !region_match(&r, &node_name) {
                        app.profiles.auto_region = None;
                        app.save_profiles();
                    }
                }
            }
            self.select_node(node_name);
        }
        if let Some((name, url)) = action_import {
            self.download_sub(None, name, url);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Window-only teardown. Core/proxy lifecycle is decided in main() on real process quit.
        *self.egui_ctx_slot.lock().unwrap() = None;
        self.state.lock().unwrap().save_profiles();
    }
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut reg_candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            let fonts_dir = PathBuf::from(windir).join("Fonts");
            reg_candidates.push(fonts_dir.join("msyh.ttc"));
            reg_candidates.push(fonts_dir.join("msyh.ttf"));
            reg_candidates.push(fonts_dir.join("simhei.ttf"));
        }
        reg_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyh.ttc"));
        reg_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyh.ttf"));
        reg_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\simhei.ttf"));
    }
    reg_candidates.extend([
        PathBuf::from("/usr/share/fonts/truetype/SourceHanSansCN-Regular.otf"),
        PathBuf::from("/usr/share/fonts/truetype/NotoSansCJKsc-VF.otf"),
        PathBuf::from("/usr/share/fonts/truetype/SourceHanSansCN-Normal.otf"),
        PathBuf::from("/usr/share/fonts/truetype/wqy-zenhei.ttc"),
    ]);
    if let Some(data_dir) = dirs::data_local_dir() {
        reg_candidates.push(data_dir.join("fonts/win/msyh.ttf"));
    }

    for path in &reg_candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert("cjk_reg".to_owned(), egui::FontData::from_owned(data));
            if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                vec.insert(0, "cjk_reg".to_owned());
            }
            if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                vec.push("cjk_reg".to_owned());
            }
            break;
        }
    }

    let mut med_candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            let fonts_dir = PathBuf::from(windir).join("Fonts");
            med_candidates.push(fonts_dir.join("msyhbd.ttc"));
            med_candidates.push(fonts_dir.join("msyhbd.ttf"));
            med_candidates.push(fonts_dir.join("msyh.ttc"));
        }
        med_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyhbd.ttc"));
        med_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyh.ttc"));
    }
    med_candidates.extend([
        PathBuf::from("/usr/share/fonts/truetype/SourceHanSansCN-Medium.otf"),
        PathBuf::from("/usr/share/fonts/truetype/SourceHanSansCN-Bold.otf"),
        PathBuf::from("/usr/share/fonts/truetype/SourceHanSansCN-Regular.otf"),
    ]);
    for path in &med_candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert("cjk_med".to_owned(), egui::FontData::from_owned(data));
            let mut fam = vec!["cjk_med".to_owned()];
            if let Some(prop) = fonts.families.get(&egui::FontFamily::Proportional) {
                fam.extend(prop.iter().cloned());
            }
            fonts.families.insert(egui::FontFamily::Name("Medium".into()), fam);
            break;
        }
    }

    if !fonts.families.contains_key(&egui::FontFamily::Name("Medium".into())) {
        let fallback = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        fonts.families.insert(egui::FontFamily::Name("Medium".into()), fallback);
    }

    ctx.set_fonts(fonts);
}

fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec < 1024 {
        format!("{} B/s", bytes_per_sec)
    } else if bytes_per_sec < 1024 * 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    }
}

fn native_options() -> eframe::NativeOptions {
    let (rgba, width, height) = build_brand_rgba_icon();
    let icon = egui::IconData { rgba, width, height };
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 740.0])
            .with_min_inner_size([640.0, 520.0])
            .with_icon(Arc::new(icon))
            .with_title("Future Proxy"),
        // Required so closing the window returns from run_native (tray can reopen UI).
        run_and_return: true,
        ..Default::default()
    }
}

/// Cloneable handles so eframe::run_native's 'static factory can rebuild the window.
#[derive(Clone)]
struct AppRuntimeView {
    state: Arc<Mutex<AppState>>,
    core: Arc<Mutex<CoreManager>>,
    tray_flags: Arc<TrayFlags>,
    egui_ctx_slot: Arc<Mutex<Option<egui::Context>>>,
}

impl AppRuntimeView {
    fn into_app(self, cc: &eframe::CreationContext<'_>) -> FutureProxyApp {
        setup_custom_fonts(&cc.egui_ctx);
        apply_cupertino_theme(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        *self.egui_ctx_slot.lock().unwrap() = Some(ctx.clone());
        self.tray_flags.show.store(false, Ordering::SeqCst);
        FutureProxyApp {
            state: self.state,
            core: self.core,
            ctx,
            tray_flags: self.tray_flags,
            egui_ctx_slot: self.egui_ctx_slot,
        }
    }
}

fn run_gui(rt: &AppRuntime) -> eframe::Result<()> {
    let boot = AppRuntimeView {
        state: Arc::clone(&rt.state),
        core: Arc::clone(&rt.core),
        tray_flags: Arc::clone(&rt.tray_flags),
        egui_ctx_slot: Arc::clone(&rt.egui_ctx_slot),
    };
    eframe::run_native(
        "future-proxy",
        native_options(),
        Box::new(move |cc| Ok(Box::new(boot.clone().into_app(cc)))),
    )
}

fn shutdown_core_if_needed(rt: &AppRuntime) {
    let app = rt.state.lock().unwrap();
    app.save_profiles();
    let keep = app.profiles.keep_core_on_exit || app.profiles.autostart || systemd_active();
    if !keep {
        if app.is_system_proxy_on {
            set_kde_proxy(false);
        }
        drop(app);
        rt.core.lock().unwrap().stop();
    }
}

#[cfg(unix)]
fn relaunch_tray_and_exit(listener_slot: &Mutex<Option<IpcListener>>) -> ! {
    // Release single-instance sock BEFORE spawning, or the child --tray will see us
    // still holding the name, send no SHOW, and exit — leaving the user with zero tray.
    release_instance(listener_slot);
    // Drop Wayland/GL by exiting this process; a fresh --tray process keeps the icon.
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .arg("--tray")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    std::process::exit(0);
}

#[cfg(unix)]
fn tray_only_main() {
    // No eframe / no Wayland window — avoids zombie "没有响应" surfaces after GUI close.
    // If a UI/tray already owns the instance sock, exit quietly (do not SHOW).
    let Some(listener) = take_single_instance(false) else {
        return;
    };

    let flags = Arc::new(TrayFlags {
        show: AtomicBool::new(false),
        quit: AtomicBool::new(false),
    });
    let wake = Arc::new(Condvar::new());
    let wake_mutex = Arc::new(Mutex::new(()));
    let egui_ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let listener_slot = Arc::new(Mutex::new(Some(listener)));
    spawn_ipc_listener(
        Arc::clone(&listener_slot),
        Arc::clone(&flags),
        Arc::clone(&wake),
        Arc::clone(&egui_ctx_slot),
    );
    let _tray = spawn_system_tray(
        Arc::clone(&flags),
        Arc::clone(&wake),
        Arc::clone(&egui_ctx_slot),
    );

    loop {
        let guard = wake_mutex.lock().unwrap();
        let mut guard = guard;
        loop {
            if flags.quit.load(Ordering::SeqCst) {
                // Real quit from tray menu.
                let keep = {
                    // Read profiles without full AppRuntime.
                    let config_dir = dirs::config_dir()
                        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
                        .unwrap_or_else(|| PathBuf::from(".config"))
                        .join("future-proxy");
                    let path = config_dir.join("profiles.json");
                    std::fs::read_to_string(path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<ProfilesMeta>(&s).ok())
                        .map(|p| p.keep_core_on_exit || p.autostart || systemd_active())
                        .unwrap_or(true)
                };
                if !keep {
                    let _ = Command::new("systemctl")
                        .args(["--user", "stop", "future-proxy.service"])
                        .status();
                    set_kde_proxy(false);
                }
                release_instance(&listener_slot);
                return;
            }
            if flags.show.swap(false, Ordering::SeqCst) {
                break;
            }
            let (g, _) = wake.wait_timeout(guard, Duration::from_millis(500)).unwrap();
            guard = g;
        }
        drop(guard);
        // Hand off to one GUI process: release sock first, then spawn, then exit tray.
        release_instance(&listener_slot);
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        return;
    }
}

fn main() -> eframe::Result<()> {
    let start_in_tray = std::env::args().any(|a| a == "--tray");

    #[cfg(unix)]
    if start_in_tray {
        tray_only_main();
        return Ok(());
    }

    // Desktop/dock Exec: if tray or GUI already running, ask it to SHOW and exit.
    let Some(listener) = take_single_instance(!start_in_tray) else {
        return Ok(());
    };

    let rt = AppRuntime::start(Some(listener));

    #[cfg(windows)]
    let mut skip_initial_gui = start_in_tray;

    loop {
        if rt.tray_flags.quit.load(Ordering::SeqCst) {
            break;
        }

        #[cfg(windows)]
        let should_open_gui = if skip_initial_gui {
            skip_initial_gui = false;
            false
        } else {
            true
        };
        #[cfg(unix)]
        let should_open_gui = true;

        if should_open_gui {
            match run_gui(&rt) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Future Proxy GUI error: {e}");
                }
            }
            *rt.egui_ctx_slot.lock().unwrap() = None;
        }

        if rt.tray_flags.quit.load(Ordering::SeqCst) {
            break;
        }

        rt.state.lock().unwrap().save_profiles();

        #[cfg(unix)]
        {
            // Window closed (X or in-UI button). Hand off to a clean --tray process so the
            // Wayland/GL connection is fully torn down — otherwise KDE keeps a frozen
            // "没有响应" zombie window that ignores further clicks.
            relaunch_tray_and_exit(&rt.listener_slot);
        }

        #[cfg(windows)]
        {
            // On Windows, keep the tray icon alive in the same process, trim WorkingSet
            // memory (~5-8 MB in background), and sleep until user clicks the tray icon.
            win_os::trim_working_set();
            let guard = rt._wake_mutex.lock().unwrap();
            let mut guard = guard;
            loop {
                if rt.tray_flags.quit.load(Ordering::SeqCst) {
                    break;
                }
                if rt.tray_flags.show.swap(false, Ordering::SeqCst) {
                    break;
                }
                let (g, _) = rt._wake.wait_timeout(guard, Duration::from_millis(500)).unwrap();
                guard = g;
            }
        }
    }

    release_instance(&rt.listener_slot);
    shutdown_core_if_needed(&rt);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_match_non_hktw() {
        assert!(region_match("🌍 非港台", "⚡ 🌍 非港台-自动优选"));
        assert!(region_match("🌍 非港台", "新加坡1—F0"));
        assert!(region_match("🌍 非港台", "日本原生GMO—F1"));
        assert!(region_match("🌍 非港台", "美国1—F1"));
        assert!(!region_match("🌍 非港台", "香港—F1"));
        assert!(!region_match("🌍 非港台", "台湾1—F2 3倍率"));
        assert!(!region_match("🌍 非港台", "自动选择"));
    }

    #[test]
    fn test_parse_node_display() {
        let (n1, r1) = parse_node_display("香港—F2 3倍率");
        assert_eq!(n1, "香港—F2");
        assert_eq!(r1, Some("3x".to_string()));

        let (n2, r2) = parse_node_display("台湾1—F2 3倍率");
        assert_eq!(n2, "台湾1—F2");
        assert_eq!(r2, Some("3x".to_string()));

        let (n3, r3) = parse_node_display("新加坡1—F1");
        assert_eq!(n3, "新加坡1—F1");
        assert_eq!(r3, None);

        let (n4, r4) = parse_node_display("美国1—F2 1.5x");
        assert_eq!(n4, "美国1—F2");
        assert_eq!(r4, Some("1.5x".to_string()));
    }

    #[test]
    fn test_smart_groups_injection() {
        let raw = r#"proxies:
  - name: "香港 01"
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: test
  - name: "新加坡 01"
    type: ss
    server: 1.1.1.1
    port: 8388
    cipher: aes-128-gcm
    password: test
proxy-groups:
  - name: 🔰国外流量
    type: select
    proxies:
      - 自动选择
      - 香港 01
      - 新加坡 01
  - name: 自动选择
    type: url-test
    proxies:
      - 香港 01
      - 新加坡 01
rules:
  - MATCH,🔰国外流量
"#;
        let patched = patch_subscription(raw, false);
        assert!(patched.contains("⚡ 🌍 非港台-自动优选"));
        assert!(patched.contains("⚡ 🇸🇬 新加坡-自动最优"));
        assert!(patched.contains("exclude-filter: \"(?i)香港|HK|Hong Kong|台湾|TW|Taiwan|澳门|MO|中国|CN|China\""));

        let tmp_path = std::env::temp_dir().join("test_fp_smart.yaml");
        let _ = std::fs::write(&tmp_path, &patched);
        let out = Command::new("/usr/bin/mihomo-core")
            .args(["-t", "-f", tmp_path.to_str().unwrap()])
            .output();
        if let Ok(res) = out {
            assert!(res.status.success(), "mihomo configuration syntax check failed: {}", String::from_utf8_lossy(&res.stderr));
        }
        let _ = std::fs::remove_file(tmp_path);
    }
}
