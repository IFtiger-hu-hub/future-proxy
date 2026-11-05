use eframe::egui;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::config_patch::MIXED_PORT;

#[cfg(unix)]
use ksni::blocking::TrayMethods;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(unix)]
pub type IpcListener = UnixListener;
#[cfg(windows)]
pub type IpcListener = std::net::TcpListener;

#[cfg(windows)]
pub const WIN_IPC_ADDR: &str = "127.0.0.1:19091";
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Path for the single-instance Unix socket (under XDG_RUNTIME_DIR when available).
#[cfg(unix)]
pub fn ui_sock_path() -> PathBuf {
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
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Become the sole UI/tray process, or hand off to the existing one.
#[cfg(unix)]
pub fn take_single_instance(activate_if_running: bool) -> Option<IpcListener> {
    let path = ui_sock_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        if activate_if_running {
            let _ = stream.write_all(b"SHOW\n");
        }
        return None;
    }
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Some(listener)
        }
        Err(_) => {
            if let Ok(mut stream) = UnixStream::connect(&path) {
                if activate_if_running {
                    let _ = stream.write_all(b"SHOW\n");
                }
                return None;
            }
            let _ = std::fs::remove_file(&path);
            UnixListener::bind(&path).ok()
        }
    }
}

#[cfg(windows)]
pub fn take_single_instance(activate_if_running: bool) -> Option<IpcListener> {
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
pub fn spawn_ipc_listener(
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

pub fn release_instance(listener_slot: &Mutex<Option<IpcListener>>) {
    *listener_slot.lock().unwrap() = None;
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(ui_sock_path());
    }
    std::thread::sleep(Duration::from_millis(50));
}

pub struct TrayFlags {
    pub show: AtomicBool,
    pub quit: AtomicBool,
}

/// Wake the main loop (window closed) and/or egui (window open).
pub fn tray_notify(_flags: &TrayFlags, wake: &Condvar, egui_ctx: &Mutex<Option<egui::Context>>) {
    wake.notify_all();
    if let Ok(guard) = egui_ctx.lock() {
        if let Some(ctx) = guard.as_ref() {
            ctx.request_repaint();
        }
    }
}

pub fn request_repaint_slot(egui_ctx: &Mutex<Option<egui::Context>>) {
    if let Ok(guard) = egui_ctx.lock() {
        if let Some(ctx) = guard.as_ref() {
            ctx.request_repaint();
        }
    }
}

/// Generate a crisp 32x32 RGBA icon (Cupertino Blue rounded square with white lightning bolt).
pub fn build_brand_rgba_icon() -> (Vec<u8>, u32, u32) {
    let w = 32usize;
    let h = 32usize;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let qx = (fx - 16.0).abs() - 7.0;
            let qy = (fy - 16.0).abs() - 7.0;
            let dist = (qx.max(0.0).hypot(qy.max(0.0)) + qx.min(qy).max(0.0)) - 7.0;
            if dist <= 0.5 {
                let alpha = ((0.5 - dist).clamp(0.0, 1.0) * 255.0) as u8;
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
pub struct ProxyTray {
    pub flags: Arc<TrayFlags>,
    pub wake: Arc<Condvar>,
    pub egui_ctx: Arc<Mutex<Option<egui::Context>>>,
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
pub type TrayGuard = Option<ksni::blocking::Handle<ProxyTray>>;
#[cfg(windows)]
pub type TrayGuard = ();

#[cfg(unix)]
pub fn spawn_system_tray(
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
pub mod win_os {
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

    const HKEY_CURRENT_USER: isize = -2147483647i32 as isize;
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
        fn DestroyMenu(h_menu: hmenu_t) -> i32;
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

    type hmenu_t = isize;

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
            bgra[i] = rgba[i + 2];
            bgra[i + 1] = rgba[i + 1];
            bgra[i + 2] = rgba[i];
            bgra[i + 3] = rgba[i + 3];
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
                -3isize,
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
                if s == 2 { 0 } else { s }
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
pub fn spawn_system_tray(
    flags: Arc<TrayFlags>,
    wake: Arc<Condvar>,
    egui_ctx: Arc<Mutex<Option<egui::Context>>>,
) -> TrayGuard {
    win_os::spawn_tray(flags, wake, egui_ctx);
}

#[cfg(unix)]
pub fn check_kde_proxy_enabled() -> bool {
    Command::new("kreadconfig6")
        .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn check_kde_proxy_enabled() -> bool {
    win_os::check_system_proxy()
}

#[cfg(unix)]
pub fn set_kde_proxy(enable: bool) {
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
pub fn set_kde_proxy(enable: bool) {
    win_os::set_system_proxy(enable, MIXED_PORT);
}

#[cfg(unix)]
pub fn systemd_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "future-proxy.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn systemd_active() -> bool {
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
pub fn set_autostart(enable: bool) -> Result<(), String> {
    if enable {
        let _ = Command::new("systemctl")
            .args(["--user", "reset-failed", "future-proxy.service"])
            .status();
    }
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
pub fn set_autostart(enable: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let config_dir = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("future-proxy");
    let core_bin = crate::core_api::CoreManager::core_bin();

    if enable {
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

/// 主动通知操作系统回收当前进程闲置堆页（Linux 调用 glibc malloc_trim，Windows 调用 EmptyWorkingSet）
pub fn trim_process_memory() {
    #[cfg(target_os = "linux")]
    {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe {
            let _ = malloc_trim(0);
        }
    }
    #[cfg(windows)]
    {
        win_os::trim_working_set();
    }
}
