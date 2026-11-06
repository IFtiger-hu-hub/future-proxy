#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    println!("Future Proxy Windows Setup Wizard is only built for Windows targets.");
}

#[cfg(windows)]
mod win_setup {
    use eframe::egui::{
        self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Rounding, Sense, Stroke,
        TextureHandle, TextureOptions, Vec2, vec2,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const VERSION: &str = env!("CARGO_PKG_VERSION");

    // Embedded payload files
    static BIN_FUTURE_PROXY: &[u8] = include_bytes!("../../Future-Proxy/future-proxy.exe");
    static BIN_MIHOMO_CORE: &[u8] = include_bytes!("../../Future-Proxy/future-proxy-core.exe");
    static BIN_WINTUN_DLL: &[u8] = include_bytes!("../../Future-Proxy/wintun.dll");
    static DATA_COUNTRY_MMDB: &[u8] = include_bytes!("../../Future-Proxy/country.mmdb");
    static DATA_GEOSITE_DAT: &[u8] = include_bytes!("../../Future-Proxy/geosite.dat");
    static DATA_CONFIG_YAML: &[u8] = include_bytes!("../../Future-Proxy/config.yaml");
    static DATA_APP_ICO: &[u8] = include_bytes!("../../packaging/future-proxy.ico");
    static ICON_128_RGBA: &[u8] = include_bytes!("../../packaging/icon_128.rgba");

    // Win32 FFI for Native Folder Browser Dialog (SHBrowseForFolderW), Disk Space, and Shell Icon Cache Refresh
    #[repr(C)]
    struct BROWSEINFOW {
        hwnd_owner: isize,
        pidl_root: *const u8,
        psz_display_name: *mut u16,
        lpsz_title: *const u16,
        ul_flags: u32,
        lpfn: usize,
        lparam: isize,
        i_image: i32,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHBrowseForFolderW(lpbi: *mut BROWSEINFOW) -> *mut u8;
        fn SHGetPathFromIDListW(pidl: *const u8, psz_path: *mut u16) -> i32;
        fn SHChangeNotify(w_event_id: i32, u_flags: u32, dw_item1: *const u8, dw_item2: *const u8);
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *mut u8, dw_co_init: u32) -> i32;
        fn CoTaskMemFree(pv: *mut u8);
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lp_directory_name: *const u16,
            lp_free_bytes_available_to_caller: *mut u64,
            lp_total_number_of_bytes: *mut u64,
            lp_total_number_of_free_bytes: *mut u64,
        ) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Open native Windows Folder Picker dialog (`SHBrowseForFolderW`)
    fn browse_for_folder(title: &str) -> Option<String> {
        unsafe {
            let _ = CoInitializeEx(std::ptr::null_mut(), 0x2); // COINIT_APARTMENTTHREADED
            let mut display_buf = [0u16; 260];
            let title_w = to_wide(title);
            // BIF_RETURNONLYFSDIRS (0x0001) | BIF_NEWDIALOGSTYLE (0x0040) | BIF_EDITBOX (0x0010)
            let mut bi = BROWSEINFOW {
                hwnd_owner: 0,
                pidl_root: std::ptr::null(),
                psz_display_name: display_buf.as_mut_ptr(),
                lpsz_title: title_w.as_ptr(),
                ul_flags: 0x0001 | 0x0040 | 0x0010,
                lpfn: 0,
                lparam: 0,
                i_image: 0,
            };
            let pidl = SHBrowseForFolderW(&mut bi);
            if pidl.is_null() {
                return None;
            }
            let mut path_buf = [0u16; 520];
            let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
            CoTaskMemFree(pidl);
            if ok != 0 {
                let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
                let picked = OsString::from_wide(&path_buf[..len])
                    .to_string_lossy()
                    .to_string();
                let final_path = PathBuf::from(&picked);
                let with_sub = if final_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.eq_ignore_ascii_case("Future-Proxy"))
                    .unwrap_or(false)
                {
                    final_path
                } else {
                    final_path.join("Future-Proxy")
                };
                return Some(with_sub.to_string_lossy().to_string());
            }
            None
        }
    }

    /// Query available and total disk space in GB for the drive of `dir_path`
    fn query_disk_space_gb(dir_path: &str) -> Option<(f64, f64)> {
        let root: String = if dir_path.len() >= 3 && dir_path.as_bytes()[1] == b':' {
            format!("{}\\", &dir_path[..2])
        } else {
            r"C:\".to_string()
        };
        unsafe {
            let root_w = to_wide(&root);
            let mut free_avail: u64 = 0;
            let mut total_bytes: u64 = 0;
            let mut total_free: u64 = 0;
            if GetDiskFreeSpaceExW(
                root_w.as_ptr(),
                &mut free_avail,
                &mut total_bytes,
                &mut total_free,
            ) != 0
            {
                let gb = 1024.0 * 1024.0 * 1024.0;
                return Some((free_avail as f64 / gb, total_bytes as f64 / gb));
            }
        }
        None
    }

    fn refresh_shell_icons() {
        unsafe {
            // SHCNE_ASSOCCHANGED = 0x08000000, SHCNF_IDLIST = 0x0000
            SHChangeNotify(0x0800_0000, 0x0000, std::ptr::null(), std::ptr::null());
        }
        let _ = Command::new("ie4uinit.exe")
            .arg("-show")
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    #[derive(Clone, PartialEq)]
    enum WizardPage {
        Welcome,
        SelectDir,
        SelectTasks,
        Ready,
        Installing,
        Finished,
        Error(String),
    }

    struct InstallState {
        page: WizardPage,
        fraction: f32,
        current_file: String,
        logs: Vec<String>,
    }

    pub struct SetupWizardApp {
        install_dir: String,
        opt_desktop_shortcut: bool,
        opt_start_menu: bool,
        opt_autostart: bool,
        opt_keep_userdata: bool,
        opt_launch_after: bool,
        brand_tex: Option<TextureHandle>,
        state: Arc<Mutex<InstallState>>,
    }

    impl SetupWizardApp {
        pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
            setup_fonts(&cc.egui_ctx);

            let img = ColorImage::from_rgba_unmultiplied([128, 128], ICON_128_RGBA);
            let brand_tex = Some(cc.egui_ctx.load_texture(
                "future_proxy_brand_icon",
                img,
                TextureOptions::LINEAR,
            ));

            let default_dir = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
                .join("Programs")
                .join("Future-Proxy");

            Self {
                install_dir: default_dir.to_string_lossy().to_string(),
                opt_desktop_shortcut: true,
                opt_start_menu: true,
                opt_autostart: true,
                opt_keep_userdata: true,
                opt_launch_after: true,
                brand_tex,
                state: Arc::new(Mutex::new(InstallState {
                    page: WizardPage::Welcome,
                    fraction: 0.0,
                    current_file: String::new(),
                    logs: Vec::new(),
                })),
            }
        }

        fn start_install(&mut self, ctx: egui::Context) {
            let install_dir = PathBuf::from(self.install_dir.trim());
            let opt_desktop = self.opt_desktop_shortcut;
            let opt_start_menu = self.opt_start_menu;
            let opt_autostart = self.opt_autostart;
            let opt_keep_userdata = self.opt_keep_userdata;
            let state = self.state.clone();

            if let Ok(mut st) = state.lock() {
                st.page = WizardPage::Installing;
                st.fraction = 0.04;
                st.current_file = "正在准备安装环境...".to_string();
                st.logs.clear();
            }

            std::thread::spawn(move || {
                let step_log = |frac: f32, cur: &str, done_log: Option<String>| {
                    if let Ok(mut st) = state.lock() {
                        st.fraction = frac;
                        st.current_file = cur.to_string();
                        if let Some(l) = done_log {
                            st.logs.push(l);
                        }
                    }
                    ctx.request_repaint();
                };

                step_log(0.10, "正在检测并关闭运行中的旧版本进程...", None);
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "future-proxy.exe"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "future-proxy-core.exe"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "mihomo.exe"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
                std::thread::sleep(std::time::Duration::from_millis(400));

                step_log(
                    0.20,
                    &format!("创建目录: {}", install_dir.display()),
                    Some("✓ 已停止旧版本残留进程".to_string()),
                );
                if let Err(e) = fs::create_dir_all(&install_dir) {
                    if let Ok(mut st) = state.lock() {
                        st.page = WizardPage::Error(format!(
                            "无法创建目标安装目录 ({}): {}",
                            install_dir.display(),
                            e
                        ));
                    }
                    ctx.request_repaint();
                    return;
                }

                let exe_path = install_dir.join("future-proxy.exe");
                step_log(
                    0.35,
                    &format!("正在释放: {}", exe_path.display()),
                    Some(format!("✓ 已创建安装目录 {}", install_dir.display())),
                );
                if let Err(e) = fs::write(&exe_path, BIN_FUTURE_PROXY) {
                    if let Ok(mut st) = state.lock() {
                        st.page = WizardPage::Error(format!("释放 future-proxy.exe 失败: {}", e));
                    }
                    ctx.request_repaint();
                    return;
                }

                // Write standalone multi-resolution .ico file into install_dir as well
                let ico_path = install_dir.join("future-proxy.ico");
                let _ = fs::write(&ico_path, DATA_APP_ICO);

                let core_path = install_dir.join("future-proxy-core.exe");
                step_log(
                    0.55,
                    &format!("正在释放: {}", core_path.display()),
                    Some("✓ 已释放 future-proxy.exe 与多分辨率图标资源".to_string()),
                );
                if let Err(e) = fs::write(&core_path, BIN_MIHOMO_CORE) {
                    if let Ok(mut st) = state.lock() {
                        st.page =
                            WizardPage::Error(format!("释放 future-proxy-core.exe 失败: {}", e));
                    }
                    ctx.request_repaint();
                    return;
                }
                let _ = fs::write(install_dir.join("wintun.dll"), BIN_WINTUN_DLL);

                step_log(
                    0.72,
                    "正在释放 GeoIP (country.mmdb) 与 GeoSite (geosite.dat)...",
                    Some("✓ 已释放 Mihomo 核心与 Wintun 虚拟网卡组件".to_string()),
                );
                let _ = fs::write(install_dir.join("country.mmdb"), DATA_COUNTRY_MMDB);
                let _ = fs::write(install_dir.join("geosite.dat"), DATA_GEOSITE_DAT);
                let _ = fs::write(install_dir.join("config.yaml"), DATA_CONFIG_YAML);

                // Seed %APPDATA%\future-proxy
                let appdata_dir = dirs::data_dir()
                    .unwrap_or_else(|| install_dir.clone())
                    .join("future-proxy");
                let _ = fs::create_dir_all(&appdata_dir);
                let _ = fs::create_dir_all(appdata_dir.join("profiles"));
                let _ = fs::write(appdata_dir.join("country.mmdb"), DATA_COUNTRY_MMDB);
                let _ = fs::write(appdata_dir.join("geosite.dat"), DATA_GEOSITE_DAT);
                let _ = fs::write(appdata_dir.join("wintun.dll"), BIN_WINTUN_DLL);
                if !opt_keep_userdata || !appdata_dir.join("config.yaml").exists() {
                    let _ = fs::write(appdata_dir.join("config.yaml"), DATA_CONFIG_YAML);
                }

                // Sync profiles.json
                let meta_path = appdata_dir.join("profiles.json");
                let mut meta_val: serde_json::Value = if opt_keep_userdata {
                    fs::read_to_string(&meta_path)
                        .ok()
                        .and_then(|txt| serde_json::from_str(&txt).ok())
                        .unwrap_or_else(|| {
                            serde_json::json!({
                                "profiles": [],
                                "active_index": null,
                                "sys_proxy_enabled": true
                            })
                        })
                } else {
                    serde_json::json!({
                        "profiles": [],
                        "active_index": null,
                        "sys_proxy_enabled": true
                    })
                };
                if let Some(obj) = meta_val.as_object_mut() {
                    obj.insert("autostart".to_string(), serde_json::Value::Bool(opt_autostart));
                    obj.insert("keep_core_on_exit".to_string(), serde_json::Value::Bool(true));
                }
                if let Ok(json_str) = serde_json::to_string_pretty(&meta_val) {
                    let _ = fs::write(&meta_path, json_str);
                }

                step_log(
                    0.86,
                    "正在生成 Windows 桌面与开始菜单快捷方式...",
                    Some("✓ 已配置 GeoIP / GeoSite 路由规则数据库".to_string()),
                );
                let exe_str = exe_path.to_string_lossy().to_string();
                let ico_str = ico_path.to_string_lossy().to_string();
                let dir_str = install_dir.to_string_lossy().to_string();

                create_shell_shortcuts(opt_desktop, opt_start_menu, &exe_str, &ico_str, &dir_str);

                step_log(
                    0.95,
                    "正在注册开机自启动与控制面板卸载程序...",
                    Some("✓ 已生成快捷方式并刷新 Windows 桌面图标缓存".to_string()),
                );
                configure_autostart(opt_autostart, &exe_str, &dir_str);
                register_uninstaller(&exe_str, &ico_str, &dir_str);
                refresh_shell_icons();

                step_log(
                    1.0,
                    "安装完成！",
                    Some("✓ 已完成注册表与开机自启动配置".to_string()),
                );
                std::thread::sleep(std::time::Duration::from_millis(300));
                if let Ok(mut st) = state.lock() {
                    st.page = WizardPage::Finished;
                }
                ctx.request_repaint();
            });
        }
    }

    /// Create Desktop and Start Menu `.lnk` shortcuts via UTF-16LE VBScript (`wscript.exe`)
    /// so Chinese user paths / OneDrive Desktop redirection and `.ico` binding work 100% reliably.
    fn create_shell_shortcuts(
        make_desktop: bool,
        make_start_menu: bool,
        target_exe: &str,
        ico_file: &str,
        work_dir: &str,
    ) {
        if !make_desktop && !make_start_menu {
            return;
        }
        let esc = |s: &str| s.replace('"', "\"\"");
        let mut vbs = String::from("On Error Resume Next\r\nSet ws = CreateObject(\"WScript.Shell\")\r\nSet fso = CreateObject(\"Scripting.FileSystemObject\")\r\n");

        if make_desktop {
            vbs.push_str(&format!(
                "desk = ws.SpecialFolders(\"Desktop\")\r\n\
                 lnk1 = desk & \"\\Future Proxy.lnk\"\r\n\
                 If fso.FileExists(lnk1) Then fso.DeleteFile lnk1, True\r\n\
                 Set sc1 = ws.CreateShortcut(lnk1)\r\n\
                 sc1.TargetPath = \"{}\"\r\n\
                 sc1.WorkingDirectory = \"{}\"\r\n\
                 sc1.Description = \"Future Proxy - 新一代 Mihomo 桌面代理客户端\"\r\n\
                 sc1.IconLocation = \"{},0\"\r\n\
                 sc1.Save\r\n",
                esc(target_exe),
                esc(work_dir),
                esc(ico_file),
            ));
        }

        if make_start_menu {
            vbs.push_str(&format!(
                "progs = ws.SpecialFolders(\"Programs\")\r\n\
                 lnk2 = progs & \"\\Future Proxy.lnk\"\r\n\
                 If fso.FileExists(lnk2) Then fso.DeleteFile lnk2, True\r\n\
                 Set sc2 = ws.CreateShortcut(lnk2)\r\n\
                 sc2.TargetPath = \"{}\"\r\n\
                 sc2.WorkingDirectory = \"{}\"\r\n\
                 sc2.Description = \"Future Proxy - 新一代 Mihomo 桌面代理客户端\"\r\n\
                 sc2.IconLocation = \"{},0\"\r\n\
                 sc2.Save\r\n",
                esc(target_exe),
                esc(work_dir),
                esc(ico_file),
            ));
        }

        let tmp_vbs = std::env::temp_dir().join("fp_create_shortcuts.vbs");
        // Write with UTF-16LE BOM so WScript.exe handles any Unicode/Chinese path natively
        let mut utf16_bytes: Vec<u8> = vec![0xFF, 0xFE];
        for u in vbs.encode_utf16() {
            utf16_bytes.extend_from_slice(&u.to_le_bytes());
        }
        if fs::write(&tmp_vbs, &utf16_bytes).is_ok() {
            let _ = Command::new("wscript.exe")
                .args(["//B", "//Nologo"])
                .arg(&tmp_vbs)
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            let _ = fs::remove_file(&tmp_vbs);
        }
    }

    fn configure_autostart(enable: bool, exe_str: &str, work_dir: &str) {
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", "FutureProxyAutostart", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        let startup_dir = dirs::data_dir().map(|d| {
            d.join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Startup")
        });

        if enable {
            if let Some(ref sdir) = startup_dir {
                let _ = fs::create_dir_all(sdir);
                let vbs_path = sdir.join("FutureProxy.vbs");
                let vbs_content = format!(
                    "Set ws = CreateObject(\"WScript.Shell\")\r\nws.CurrentDirectory = \"{}\"\r\nws.Run \"\"\"{}\"\" --tray\", 0, False\r\n",
                    work_dir.replace('"', "\"\""),
                    exe_str.replace('"', "\"\"")
                );
                let mut utf16_bytes: Vec<u8> = vec![0xFF, 0xFE];
                for u in vbs_content.encode_utf16() {
                    utf16_bytes.extend_from_slice(&u.to_le_bytes());
                }
                let _ = fs::write(vbs_path, utf16_bytes);
            }
            let run_val = format!("\"{}\" --tray", exe_str);
            let _ = Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    "FutureProxy",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &run_val,
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        } else {
            if let Some(ref sdir) = startup_dir {
                let _ = fs::remove_file(sdir.join("FutureProxy.vbs"));
                let _ = fs::remove_file(sdir.join("FutureProxy.lnk"));
            }
            let _ = Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    "FutureProxy",
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }

    fn register_uninstaller(exe_str: &str, ico_str: &str, work_dir: &str) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FutureProxy";
        let uninstall_cmd = format!(
            "cmd.exe /c taskkill /F /IM future-proxy.exe & taskkill /F /IM future-proxy-core.exe & rmdir /S /Q \"{}\"",
            work_dir
        );
        let entries = [
            ("DisplayName", "Future Proxy"),
            ("DisplayVersion", VERSION),
            ("Publisher", "Future Proxy Team"),
            ("InstallLocation", work_dir),
            ("DisplayIcon", ico_str),
            ("UninstallString", &uninstall_cmd),
        ];
        for (k, v) in entries {
            let _ = Command::new("reg")
                .args(["add", key, "/v", k, "/t", "REG_SZ", "/d", v, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }

    fn setup_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simhei.ttf",
        ];
        for path in candidates {
            if let Ok(bytes) = fs::read(path) {
                fonts.font_data.insert(
                    "cjk".to_owned(),
                    egui::FontData::from_owned(bytes).into(),
                );
                if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    vec.insert(0, "cjk".to_owned());
                }
                if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                    vec.push("cjk".to_owned());
                }
                break;
            }
        }
        ctx.set_fonts(fonts);
    }

    impl eframe::App for SetupWizardApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            let mut visuals = egui::Visuals::light();
            visuals.panel_fill = Color32::WHITE;
            ctx.set_visuals(visuals);

            let (page, frac, cur_file, logs) = {
                let st = self.state.lock().unwrap();
                (
                    st.page.clone(),
                    st.fraction,
                    st.current_file.clone(),
                    st.logs.clone(),
                )
            };

            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::WHITE))
                .show(ctx, |ui| {
                    let full = ui.max_rect();
                    let footer_h = 56.0;
                    let main_rect =
                        Rect::from_min_max(full.min, Pos2::new(full.right(), full.bottom() - footer_h));
                    let footer_rect =
                        Rect::from_min_max(Pos2::new(full.left(), full.bottom() - footer_h), full.max);

                    // Draw Bottom Wizard Button Bar (Inno Setup / FlClash Modern style)
                    ui.painter()
                        .rect_filled(footer_rect, 0.0, Color32::from_rgb(241, 245, 249));
                    ui.painter().line_segment(
                        [
                            Pos2::new(footer_rect.left(), footer_rect.top()),
                            Pos2::new(footer_rect.right(), footer_rect.top()),
                        ],
                        Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
                    );
                    ui.painter().text(
                        Pos2::new(footer_rect.left() + 18.0, footer_rect.center().y),
                        Align2::LEFT_CENTER,
                        format!("Future Proxy Setup v{}", VERSION),
                        FontId::proportional(11.5),
                        Color32::from_rgb(148, 163, 184),
                    );

                    match page {
                        WizardPage::Welcome => {
                            self.render_sidebar_and_welcome(ui, main_rect);
                            let btn_y = footer_rect.center().y - 16.0;
                            let next_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 204.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            if draw_wizard_btn(ui, next_r, "下一步(N) >", true, true) {
                                self.state.lock().unwrap().page = WizardPage::SelectDir;
                            }
                            if draw_wizard_btn(ui, cancel_r, "取消", false, true) {
                                std::process::exit(0);
                            }
                        }
                        WizardPage::SelectDir => {
                            self.render_inner_header(
                                ui,
                                main_rect,
                                "选择安装位置",
                                "您想将 Future Proxy 安装在什么地方？",
                            );
                            let body = Rect::from_min_max(
                                Pos2::new(main_rect.left() + 32.0, main_rect.top() + 86.0),
                                Pos2::new(main_rect.right() - 32.0, main_rect.bottom() - 16.0),
                            );
                            self.render_select_dir_body(ui, body);

                            let btn_y = footer_rect.center().y - 16.0;
                            let back_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 302.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let next_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 204.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            if draw_wizard_btn(ui, back_r, "< 上一步(B)", false, true) {
                                self.state.lock().unwrap().page = WizardPage::Welcome;
                            }
                            if draw_wizard_btn(
                                ui,
                                next_r,
                                "下一步(N) >",
                                true,
                                !self.install_dir.trim().is_empty(),
                            ) {
                                self.state.lock().unwrap().page = WizardPage::SelectTasks;
                            }
                            if draw_wizard_btn(ui, cancel_r, "取消", false, true) {
                                std::process::exit(0);
                            }
                        }
                        WizardPage::SelectTasks => {
                            self.render_inner_header(
                                ui,
                                main_rect,
                                "选择附加任务",
                                "您想要安装程序执行哪些附加任务？",
                            );
                            let body = Rect::from_min_max(
                                Pos2::new(main_rect.left() + 32.0, main_rect.top() + 86.0),
                                Pos2::new(main_rect.right() - 32.0, main_rect.bottom() - 16.0),
                            );
                            self.render_select_tasks_body(ui, body);

                            let btn_y = footer_rect.center().y - 16.0;
                            let back_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 302.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let next_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 204.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            if draw_wizard_btn(ui, back_r, "< 上一步(B)", false, true) {
                                self.state.lock().unwrap().page = WizardPage::SelectDir;
                            }
                            if draw_wizard_btn(ui, next_r, "下一步(N) >", true, true) {
                                self.state.lock().unwrap().page = WizardPage::Ready;
                            }
                            if draw_wizard_btn(ui, cancel_r, "取消", false, true) {
                                std::process::exit(0);
                            }
                        }
                        WizardPage::Ready => {
                            self.render_inner_header(
                                ui,
                                main_rect,
                                "准备安装",
                                "安装程序已准备好开始在您的电脑上安装 Future Proxy。",
                            );
                            let body = Rect::from_min_max(
                                Pos2::new(main_rect.left() + 32.0, main_rect.top() + 86.0),
                                Pos2::new(main_rect.right() - 32.0, main_rect.bottom() - 16.0),
                            );
                            self.render_ready_body(ui, body);

                            let btn_y = footer_rect.center().y - 16.0;
                            let back_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 302.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let inst_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 204.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            if draw_wizard_btn(ui, back_r, "< 上一步(B)", false, true) {
                                self.state.lock().unwrap().page = WizardPage::SelectTasks;
                            }
                            if draw_wizard_btn(ui, inst_r, "安装(I)", true, true) {
                                self.start_install(ctx.clone());
                            }
                            if draw_wizard_btn(ui, cancel_r, "取消", false, true) {
                                std::process::exit(0);
                            }
                        }
                        WizardPage::Installing => {
                            self.render_inner_header(
                                ui,
                                main_rect,
                                "正在安装",
                                "请稍候，安装程序正在您的电脑上安装 Future Proxy。",
                            );
                            let body = Rect::from_min_max(
                                Pos2::new(main_rect.left() + 32.0, main_rect.top() + 86.0),
                                Pos2::new(main_rect.right() - 32.0, main_rect.bottom() - 16.0),
                            );
                            self.render_installing_body(ui, body, frac, &cur_file, &logs);

                            let btn_y = footer_rect.center().y - 16.0;
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            let _ = draw_wizard_btn(ui, cancel_r, "取消", false, false);
                        }
                        WizardPage::Finished => {
                            self.render_sidebar_and_finished(ui, main_rect);
                            let btn_y = footer_rect.center().y - 16.0;
                            let done_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 110.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            if draw_wizard_btn(ui, done_r, "完成(F)", true, true) {
                                if self.opt_launch_after {
                                    let install_dir = PathBuf::from(self.install_dir.trim());
                                    let exe_path = install_dir.join("future-proxy.exe");
                                    let _ = Command::new(&exe_path)
                                        .current_dir(&install_dir)
                                        .spawn();
                                }
                                std::process::exit(0);
                            }
                        }
                        WizardPage::Error(err) => {
                            self.render_inner_header(
                                ui,
                                main_rect,
                                "安装未完成",
                                "在写入安装文件时遇到问题，请检查路径权限或重试。",
                            );
                            let body = Rect::from_min_max(
                                Pos2::new(main_rect.left() + 32.0, main_rect.top() + 86.0),
                                Pos2::new(main_rect.right() - 32.0, main_rect.bottom() - 16.0),
                            );
                            let p = ui.painter();
                            let err_box =
                                Rect::from_min_size(body.min, vec2(body.width(), 120.0));
                            p.rect_filled(
                                err_box,
                                Rounding::same(8.0),
                                Color32::from_rgb(254, 242, 242),
                            );
                            p.rect_stroke(
                                err_box,
                                Rounding::same(8.0),
                                Stroke::new(1.0_f32, Color32::from_rgb(252, 165, 165)),
                            );
                            p.text(
                                Pos2::new(err_box.left() + 16.0, err_box.top() + 24.0),
                                Align2::LEFT_CENTER,
                                "错误详情：",
                                FontId::proportional(13.5),
                                Color32::from_rgb(185, 28, 28),
                            );
                            p.text(
                                Pos2::new(err_box.left() + 16.0, err_box.top() + 54.0),
                                Align2::LEFT_CENTER,
                                &err,
                                FontId::proportional(12.5),
                                Color32::from_rgb(127, 29, 29),
                            );

                            let btn_y = footer_rect.center().y - 16.0;
                            let back_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 204.0, btn_y),
                                vec2(92.0, 32.0),
                            );
                            let cancel_r = Rect::from_min_size(
                                Pos2::new(footer_rect.right() - 102.0, btn_y),
                                vec2(84.0, 32.0),
                            );
                            if draw_wizard_btn(ui, back_r, "< 返回重试", true, true) {
                                self.state.lock().unwrap().page = WizardPage::SelectDir;
                            }
                            if draw_wizard_btn(ui, cancel_r, "退出", false, true) {
                                std::process::exit(0);
                            }
                        }
                    }
                });
        }
    }

    impl SetupWizardApp {
        /// Render the Inno Setup Modern top header (`MainPanel`) for inner wizard pages
        fn render_inner_header(
            &self,
            ui: &mut egui::Ui,
            main_rect: Rect,
            title: &str,
            subtitle: &str,
        ) {
            let header_h = 68.0;
            let header_rect =
                Rect::from_min_size(main_rect.min, vec2(main_rect.width(), header_h));
            let p = ui.painter();
            p.rect_filled(header_rect, 0.0, Color32::from_rgb(248, 250, 252));
            p.line_segment(
                [
                    Pos2::new(header_rect.left(), header_rect.bottom()),
                    Pos2::new(header_rect.right(), header_rect.bottom()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240)),
            );

            p.text(
                Pos2::new(header_rect.left() + 26.0, header_rect.top() + 23.0),
                Align2::LEFT_CENTER,
                title,
                FontId::proportional(15.5),
                Color32::from_rgb(15, 23, 42),
            );
            p.text(
                Pos2::new(header_rect.left() + 38.0, header_rect.top() + 46.0),
                Align2::LEFT_CENTER,
                subtitle,
                FontId::proportional(12.5),
                Color32::from_rgb(71, 85, 105),
            );

            if let Some(ref tex) = self.brand_tex {
                let icon_rect = Rect::from_min_size(
                    Pos2::new(header_rect.right() - 64.0, header_rect.top() + 12.0),
                    vec2(44.0, 44.0),
                );
                p.image(
                    tex.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }

        fn render_left_graphic_banner(&self, ui: &mut egui::Ui, banner_rect: Rect, finished: bool) {
            let p = ui.painter();
            p.rect_filled(banner_rect, 0.0, Color32::from_rgb(15, 23, 42));

            // Decorative subtle radial rings
            let ring_c = Pos2::new(banner_rect.center().x, banner_rect.top() + 108.0);
            p.circle_filled(
                ring_c,
                74.0,
                Color32::from_rgba_unmultiplied(10, 132, 255, 20),
            );
            p.circle_filled(
                ring_c,
                54.0,
                Color32::from_rgba_unmultiplied(10, 132, 255, 34),
            );

            if let Some(ref tex) = self.brand_tex {
                let icon_rect =
                    Rect::from_center_size(ring_c, vec2(80.0, 80.0));
                p.image(
                    tex.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            p.text(
                Pos2::new(banner_rect.center().x, banner_rect.top() + 178.0),
                Align2::CENTER_CENTER,
                "Future Proxy",
                FontId::proportional(17.5),
                Color32::WHITE,
            );

            let badge_rect = Rect::from_center_size(
                Pos2::new(banner_rect.center().x, banner_rect.top() + 206.0),
                vec2(96.0, 22.0),
            );
            p.rect_filled(
                badge_rect,
                Rounding::same(11.0),
                Color32::from_rgba_unmultiplied(10, 132, 255, 45),
            );
            p.text(
                badge_rect.center(),
                Align2::CENTER_CENTER,
                if finished {
                    "✓ 安装就绪"
                } else {
                    "Windows x64"
                },
                FontId::proportional(11.5),
                Color32::from_rgb(147, 197, 253),
            );

            // Stepper overview in left banner
            let steps = ["1. 欢迎向导", "2. 安装位置", "3. 附加任务", "4. 准备就绪", "5. 完成安装"];
            for (i, s) in steps.iter().enumerate() {
                let y = banner_rect.top() + 252.0 + (i as f32) * 24.0;
                let active = (!finished && i == 0) || (finished && i == 4);
                let col = if active {
                    Color32::from_rgb(96, 165, 250)
                } else {
                    Color32::from_rgb(100, 116, 139)
                };
                p.text(
                    Pos2::new(banner_rect.left() + 34.0, y),
                    Align2::LEFT_CENTER,
                    *s,
                    FontId::proportional(12.0),
                    col,
                );
            }

            p.line_segment(
                [
                    Pos2::new(banner_rect.right(), banner_rect.top()),
                    Pos2::new(banner_rect.right(), banner_rect.bottom()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240)),
            );
        }

        fn render_sidebar_and_welcome(&self, ui: &mut egui::Ui, main_rect: Rect) {
            let side_w = 184.0;
            let banner_rect =
                Rect::from_min_size(main_rect.min, vec2(side_w, main_rect.height()));
            self.render_left_graphic_banner(ui, banner_rect, false);

            let content = Rect::from_min_max(
                Pos2::new(banner_rect.right() + 28.0, main_rect.top() + 28.0),
                Pos2::new(main_rect.right() - 28.0, main_rect.bottom() - 20.0),
            );
            let p = ui.painter();
            p.text(
                Pos2::new(content.left(), content.top() + 12.0),
                Align2::LEFT_CENTER,
                "欢迎使用 Future Proxy 安装向导",
                FontId::proportional(20.0),
                Color32::from_rgb(15, 23, 42),
            );
            p.text(
                Pos2::new(content.left(), content.top() + 44.0),
                Align2::LEFT_CENTER,
                format!("此向导将在您的电脑上安装 Future Proxy v{} (64-bit)。", VERSION),
                FontId::proportional(13.0),
                Color32::from_rgb(51, 65, 85),
            );

            let features = [
                (
                    "内置 Mihomo 高性能代理核心",
                    "自动配置 GeoIP / GeoSite 路由分流数据库与 Wintun 虚拟网卡驱动",
                ),
                (
                    "原生桌面快捷方式与系统托盘常驻",
                    "内置多分辨率高清图标，支持一键开关系统代理、TUN 与节点延迟测速",
                ),
                (
                    "智能开机自启与代理自动恢复",
                    "登录 Windows 时静默运行后台托盘，自动恢复上次开启的系统代理与订阅",
                ),
            ];

            for (i, (ft, desc)) in features.iter().enumerate() {
                let card_y = content.top() + 74.0 + (i as f32) * 68.0;
                let card_r =
                    Rect::from_min_size(Pos2::new(content.left(), card_y), vec2(content.width(), 58.0));
                p.rect_filled(card_r, Rounding::same(8.0), Color32::from_rgb(248, 250, 252));
                p.rect_stroke(
                    card_r,
                    Rounding::same(8.0),
                    Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240)),
                );
                let dot_c = Pos2::new(card_r.left() + 20.0, card_r.center().y);
                p.circle_filled(
                    dot_c,
                    10.0,
                    Color32::from_rgba_unmultiplied(10, 132, 255, 28),
                );
                p.circle_filled(dot_c, 4.5, Color32::from_rgb(10, 132, 255));

                p.text(
                    Pos2::new(card_r.left() + 40.0, card_r.top() + 19.0),
                    Align2::LEFT_CENTER,
                    *ft,
                    FontId::proportional(13.0),
                    Color32::from_rgb(15, 23, 42),
                );
                p.text(
                    Pos2::new(card_r.left() + 40.0, card_r.top() + 39.0),
                    Align2::LEFT_CENTER,
                    *desc,
                    FontId::proportional(11.8),
                    Color32::from_rgb(100, 116, 139),
                );
            }

            p.text(
                Pos2::new(content.left(), content.bottom() - 14.0),
                Align2::LEFT_CENTER,
                "点击「下一步(N) >」继续，或点击「取消」退出安装程序。",
                FontId::proportional(12.5),
                Color32::from_rgb(71, 85, 105),
            );
        }

        fn render_select_dir_body(&mut self, ui: &mut egui::Ui, body: Rect) {
            let p = ui.painter();
            p.text(
                Pos2::new(body.left(), body.top() + 8.0),
                Align2::LEFT_CENTER,
                "安装程序会将 Future Proxy 安装在以下文件夹中。",
                FontId::proportional(13.0),
                Color32::from_rgb(30, 41, 59),
            );
            p.text(
                Pos2::new(body.left(), body.top() + 30.0),
                Align2::LEFT_CENTER,
                "若要继续，请点击「下一步」。如果您想选择其他文件夹，请点击「浏览(B)...」。",
                FontId::proportional(12.5),
                Color32::from_rgb(100, 116, 139),
            );

            // Directory input box + Browse button
            let row_y = body.top() + 56.0;
            let browse_w = 96.0;
            let input_w = body.width() - browse_w - 10.0;
            let input_rect = Rect::from_min_size(Pos2::new(body.left(), row_y), vec2(input_w, 34.0));
            let browse_rect = Rect::from_min_size(
                Pos2::new(input_rect.right() + 10.0, row_y),
                vec2(browse_w, 34.0),
            );

            ui.painter()
                .rect_filled(input_rect, Rounding::same(6.0), Color32::from_rgb(248, 250, 252));
            ui.painter().rect_stroke(
                input_rect,
                Rounding::same(6.0),
                Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
            );

            let text_inner = input_rect.shrink2(vec2(10.0, 5.0));
            let mut tui = ui.new_child(egui::UiBuilder::new().max_rect(text_inner));
            tui.add(
                egui::TextEdit::singleline(&mut self.install_dir)
                    .frame(false)
                    .font(FontId::proportional(13.0))
                    .desired_width(text_inner.width()),
            );

            if draw_wizard_btn(ui, browse_rect, "浏览(B)...", false, true) {
                if let Some(chosen) = browse_for_folder("请选择 Future Proxy 安装目录：") {
                    self.install_dir = chosen;
                }
            }

            // Quick path presets
            let preset_y = row_y + 46.0;
            let local_preset = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
                .join("Programs")
                .join("Future-Proxy")
                .to_string_lossy()
                .to_string();
            let c_prog = r"C:\Program Files\Future-Proxy".to_string();
            let d_prog = r"D:\Program Files\Future-Proxy".to_string();

            let p1_r = Rect::from_min_size(Pos2::new(body.left(), preset_y), vec2(168.0, 26.0));
            if draw_chip_btn(ui, p1_r, "推荐：当前用户目录", self.install_dir == local_preset) {
                self.install_dir = local_preset;
            }
            let p2_r =
                Rect::from_min_size(Pos2::new(p1_r.right() + 8.0, preset_y), vec2(175.0, 26.0));
            if draw_chip_btn(ui, p2_r, r"C:\Program Files\Future-Proxy", self.install_dir == c_prog)
            {
                self.install_dir = c_prog;
            }
            if Path::new(r"D:\").exists() {
                let p3_r = Rect::from_min_size(
                    Pos2::new(p2_r.right() + 8.0, preset_y),
                    vec2(175.0, 26.0),
                );
                if draw_chip_btn(
                    ui,
                    p3_r,
                    r"D:\Program Files\Future-Proxy",
                    self.install_dir == d_prog,
                ) {
                    self.install_dir = d_prog;
                }
            }

            // Disk space card at bottom of body
            let disk_card = Rect::from_min_size(
                Pos2::new(body.left(), body.bottom() - 82.0),
                vec2(body.width(), 74.0),
            );
            let p = ui.painter();
            p.rect_filled(disk_card, Rounding::same(8.0), Color32::from_rgb(248, 250, 252));
            p.rect_stroke(
                disk_card,
                Rounding::same(8.0),
                Stroke::new(1.0_f32, Color32::from_rgb(226, 232, 240)),
            );

            p.text(
                Pos2::new(disk_card.left() + 16.0, disk_card.top() + 22.0),
                Align2::LEFT_CENTER,
                "所需磁盘空间：至少需要 64.5 MB",
                FontId::proportional(12.5),
                Color32::from_rgb(30, 41, 59),
            );

            let (free_gb, total_gb) =
                query_disk_space_gb(&self.install_dir).unwrap_or((120.0, 500.0));
            p.text(
                Pos2::new(disk_card.right() - 16.0, disk_card.top() + 22.0),
                Align2::RIGHT_CENTER,
                format!("目标磁盘可用空间：{:.1} GB / 共 {:.0} GB", free_gb, total_gb),
                FontId::proportional(12.5),
                Color32::from_rgb(10, 132, 255),
            );

            let bar_r = Rect::from_min_max(
                Pos2::new(disk_card.left() + 16.0, disk_card.top() + 44.0),
                Pos2::new(disk_card.right() - 16.0, disk_card.top() + 54.0),
            );
            p.rect_filled(bar_r, Rounding::same(5.0), Color32::from_rgb(226, 232, 240));
            let used_ratio = if total_gb > 0.0 {
                ((total_gb - free_gb) / total_gb).clamp(0.05, 0.95) as f32
            } else {
                0.35
            };
            let used_r =
                Rect::from_min_size(bar_r.min, vec2(bar_r.width() * used_ratio, bar_r.height()));
            p.rect_filled(used_r, Rounding::same(5.0), Color32::from_rgb(10, 132, 255));
        }

        fn render_select_tasks_body(&mut self, ui: &mut egui::Ui, body: Rect) {
            ui.painter().text(
                Pos2::new(body.left(), body.top() + 8.0),
                Align2::LEFT_CENTER,
                "请选择安装 Future Proxy 时想要执行的附加任务，然后点击「下一步」：",
                FontId::proportional(13.0),
                Color32::from_rgb(30, 41, 59),
            );

            ui.painter().text(
                Pos2::new(body.left(), body.top() + 38.0),
                Align2::LEFT_CENTER,
                "附加图标：",
                FontId::proportional(12.5),
                Color32::from_rgb(100, 116, 139),
            );

            let r1 = Rect::from_min_size(
                Pos2::new(body.left(), body.top() + 54.0),
                vec2(body.width(), 44.0),
            );
            if draw_custom_checkbox_row(
                ui,
                r1,
                self.opt_desktop_shortcut,
                "创建桌面快捷方式(D)",
                "在 Windows 桌面上生成 Future Proxy 高清图标快捷方式",
            ) {
                self.opt_desktop_shortcut = !self.opt_desktop_shortcut;
            }

            let r2 = Rect::from_min_size(
                Pos2::new(body.left(), body.top() + 104.0),
                vec2(body.width(), 44.0),
            );
            if draw_custom_checkbox_row(
                ui,
                r2,
                self.opt_start_menu,
                "创建「开始菜单」快捷方式(S)",
                "将 Future Proxy 添加到 Windows 开始菜单程序列表",
            ) {
                self.opt_start_menu = !self.opt_start_menu;
            }

            ui.painter().text(
                Pos2::new(body.left(), body.top() + 168.0),
                Align2::LEFT_CENTER,
                "系统与启动项：",
                FontId::proportional(12.5),
                Color32::from_rgb(100, 116, 139),
            );

            let r3 = Rect::from_min_size(
                Pos2::new(body.left(), body.top() + 184.0),
                vec2(body.width(), 44.0),
            );
            if draw_custom_checkbox_row(
                ui,
                r3,
                self.opt_autostart,
                "开机自动启动 Future Proxy (推荐)",
                "登录 Windows 时静默运行后台托盘，并自动恢复系统代理状态",
            ) {
                self.opt_autostart = !self.opt_autostart;
            }

            let r4 = Rect::from_min_size(
                Pos2::new(body.left(), body.top() + 234.0),
                vec2(body.width(), 44.0),
            );
            if draw_custom_checkbox_row(
                ui,
                r4,
                self.opt_keep_userdata,
                "保留已有订阅与节点配置",
                "覆盖升级时不重置 %APPDATA%\\future-proxy 下已导入的机场订阅",
            ) {
                self.opt_keep_userdata = !self.opt_keep_userdata;
            }
        }

        fn render_ready_body(&self, ui: &mut egui::Ui, body: Rect) {
            let p = ui.painter();
            p.text(
                Pos2::new(body.left(), body.top() + 8.0),
                Align2::LEFT_CENTER,
                "点击「安装(I)」继续此安装程序。如果您想要回顾或更改任何设置，请点击「< 上一步(B)」。",
                FontId::proportional(12.8),
                Color32::from_rgb(30, 41, 59),
            );

            let memo_rect = Rect::from_min_max(
                Pos2::new(body.left(), body.top() + 34.0),
                Pos2::new(body.right(), body.bottom() - 8.0),
            );
            p.rect_filled(memo_rect, Rounding::same(6.0), Color32::from_rgb(248, 250, 252));
            p.rect_stroke(
                memo_rect,
                Rounding::same(6.0),
                Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
            );

            let mut lines = vec![
                ("目标安装位置：".to_string(), true),
                (format!("      {}", self.install_dir), false),
                ("".to_string(), false),
                ("安装类型与组件：".to_string(), true),
                (
                    "      完整安装 (Future Proxy GUI + Mihomo 核心 + Wintun 驱动 + GeoIP/GeoSite)".to_string(),
                    false,
                ),
                ("".to_string(), false),
                ("所选附加任务：".to_string(), true),
            ];
            if self.opt_desktop_shortcut {
                lines.push(("      • 创建桌面快捷方式 (Future Proxy.lnk)".to_string(), false));
            }
            if self.opt_start_menu {
                lines.push(("      • 创建开始菜单快捷方式".to_string(), false));
            }
            if self.opt_autostart {
                lines.push((
                    "      • 启用开机自动启动 (Startup 启动目录 + 注册表 Run 双重守护)".to_string(),
                    false,
                ));
            }
            if self.opt_keep_userdata {
                lines.push(("      • 保留已有用户订阅配置".to_string(), false));
            }

            for (i, (txt, bold)) in lines.iter().enumerate() {
                if txt.is_empty() {
                    continue;
                }
                let y = memo_rect.top() + 20.0 + (i as f32) * 19.5;
                p.text(
                    Pos2::new(memo_rect.left() + 16.0, y),
                    Align2::LEFT_CENTER,
                    txt,
                    FontId::proportional(if *bold { 12.8 } else { 12.3 }),
                    if *bold {
                        Color32::from_rgb(15, 23, 42)
                    } else {
                        Color32::from_rgb(71, 85, 105)
                    },
                );
            }
        }

        fn render_installing_body(
            &self,
            ui: &mut egui::Ui,
            body: Rect,
            frac: f32,
            cur_file: &str,
            logs: &[String],
        ) {
            let p = ui.painter();
            p.text(
                Pos2::new(body.left(), body.top() + 8.0),
                Align2::LEFT_CENTER,
                cur_file,
                FontId::proportional(12.8),
                Color32::from_rgb(30, 41, 59),
            );
            p.text(
                Pos2::new(body.right(), body.top() + 8.0),
                Align2::RIGHT_CENTER,
                format!("{:.0}%", frac * 100.0),
                FontId::proportional(12.8),
                Color32::from_rgb(10, 132, 255),
            );

            let bar_rect = Rect::from_min_size(
                Pos2::new(body.left(), body.top() + 26.0),
                vec2(body.width(), 16.0),
            );
            p.rect_filled(bar_rect, Rounding::same(4.0), Color32::from_rgb(226, 232, 240));
            let fill_w = (bar_rect.width() * frac.clamp(0.0, 1.0)).max(8.0);
            p.rect_filled(
                Rect::from_min_size(bar_rect.min, vec2(fill_w, bar_rect.height())),
                Rounding::same(4.0),
                Color32::from_rgb(10, 132, 255),
            );

            let log_box = Rect::from_min_max(
                Pos2::new(body.left(), bar_rect.bottom() + 16.0),
                Pos2::new(body.right(), body.bottom() - 8.0),
            );
            p.rect_filled(log_box, Rounding::same(6.0), Color32::from_rgb(248, 250, 252));
            p.rect_stroke(
                log_box,
                Rounding::same(6.0),
                Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
            );

            for (i, line) in logs.iter().enumerate() {
                let y = log_box.top() + 20.0 + (i as f32) * 23.0;
                p.text(
                    Pos2::new(log_box.left() + 14.0, y),
                    Align2::LEFT_CENTER,
                    line,
                    FontId::proportional(12.3),
                    Color32::from_rgb(22, 101, 52),
                );
            }
        }

        fn render_sidebar_and_finished(&mut self, ui: &mut egui::Ui, main_rect: Rect) {
            let side_w = 184.0;
            let banner_rect =
                Rect::from_min_size(main_rect.min, vec2(side_w, main_rect.height()));
            self.render_left_graphic_banner(ui, banner_rect, true);

            let content = Rect::from_min_max(
                Pos2::new(banner_rect.right() + 28.0, main_rect.top() + 28.0),
                Pos2::new(main_rect.right() - 28.0, main_rect.bottom() - 20.0),
            );
            ui.painter().text(
                Pos2::new(content.left(), content.top() + 12.0),
                Align2::LEFT_CENTER,
                "Future Proxy 安装向导完成",
                FontId::proportional(20.0),
                Color32::from_rgb(15, 23, 42),
            );
            ui.painter().text(
                Pos2::new(content.left(), content.top() + 44.0),
                Align2::LEFT_CENTER,
                format!("安装程序已在您的电脑上完成安装 Future Proxy v{}。", VERSION),
                FontId::proportional(13.0),
                Color32::from_rgb(51, 65, 85),
            );
            ui.painter().text(
                Pos2::new(content.left(), content.top() + 68.0),
                Align2::LEFT_CENTER,
                "您可以通过桌面或「开始菜单」中的快捷方式随时启动此应用程序。",
                FontId::proportional(12.5),
                Color32::from_rgb(100, 116, 139),
            );

            let info_box = Rect::from_min_size(
                Pos2::new(content.left(), content.top() + 98.0),
                vec2(content.width(), 74.0),
            );
            ui.painter()
                .rect_filled(info_box, Rounding::same(8.0), Color32::from_rgb(240, 253, 244));
            ui.painter().rect_stroke(
                info_box,
                Rounding::same(8.0),
                Stroke::new(1.0_f32, Color32::from_rgb(187, 247, 208)),
            );
            ui.painter().text(
                Pos2::new(info_box.left() + 14.0, info_box.top() + 24.0),
                Align2::LEFT_CENTER,
                format!("✓ 安装位置: {}", self.install_dir),
                FontId::proportional(12.3),
                Color32::from_rgb(22, 101, 52),
            );
            ui.painter().text(
                Pos2::new(info_box.left() + 14.0, info_box.top() + 48.0),
                Align2::LEFT_CENTER,
                if self.opt_autostart {
                    "✓ 开机自启动与桌面快捷方式图标均已配置就绪"
                } else {
                    "✓ 桌面快捷方式图标与 Mihomo 内核已配置就绪"
                },
                FontId::proportional(12.3),
                Color32::from_rgb(21, 128, 61),
            );

            let launch_r = Rect::from_min_size(
                Pos2::new(content.left(), content.top() + 190.0),
                vec2(content.width(), 46.0),
            );
            if draw_custom_checkbox_row(
                ui,
                launch_r,
                self.opt_launch_after,
                "立即运行 Future Proxy",
                "点击「完成(F)」退出向导后立即打开 Future Proxy 主界面",
            ) {
                self.opt_launch_after = !self.opt_launch_after;
            }

            ui.painter().text(
                Pos2::new(content.left(), content.bottom() - 14.0),
                Align2::LEFT_CENTER,
                "点击「完成(F)」退出安装程序。",
                FontId::proportional(12.5),
                Color32::from_rgb(71, 85, 105),
            );
        }
    }

    fn draw_custom_checkbox_row(
        ui: &mut egui::Ui,
        rect: Rect,
        checked: bool,
        title: &str,
        subtitle: &str,
    ) -> bool {
        let resp = ui.allocate_rect(rect, Sense::click());
        let hovered = resp.hovered();
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let bg = if checked {
            if hovered {
                Color32::from_rgb(239, 246, 255)
            } else {
                Color32::from_rgb(248, 250, 252)
            }
        } else if hovered {
            Color32::from_rgb(248, 250, 252)
        } else {
            Color32::WHITE
        };
        let border = if checked {
            Color32::from_rgba_unmultiplied(10, 132, 255, 110)
        } else if hovered {
            Color32::from_rgb(203, 213, 225)
        } else {
            Color32::from_rgb(226, 232, 240)
        };

        let p = ui.painter();
        p.rect_filled(rect, Rounding::same(8.0), bg);
        p.rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, border));

        let box_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 22.0, rect.center().y),
            vec2(18.0, 18.0),
        );
        if checked {
            p.rect_filled(box_rect, Rounding::same(5.0), Color32::from_rgb(10, 132, 255));
            let c = box_rect.center();
            let p1 = Pos2::new(c.x - 4.2, c.y + 0.2);
            let p2 = Pos2::new(c.x - 1.0, c.y + 3.4);
            let p3 = Pos2::new(c.x + 4.5, c.y - 3.2);
            let stroke = Stroke::new(2.0_f32, Color32::WHITE);
            p.line_segment([p1, p2], stroke);
            p.line_segment([p2, p3], stroke);
        } else {
            p.rect_filled(box_rect, Rounding::same(5.0), Color32::WHITE);
            p.rect_stroke(
                box_rect,
                Rounding::same(5.0),
                Stroke::new(1.5_f32, Color32::from_rgb(148, 163, 184)),
            );
        }

        p.text(
            Pos2::new(box_rect.right() + 12.0, rect.top() + 14.0),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(13.0),
            Color32::from_rgb(15, 23, 42),
        );
        p.text(
            Pos2::new(box_rect.right() + 12.0, rect.top() + 31.0),
            Align2::LEFT_CENTER,
            subtitle,
            FontId::proportional(11.5),
            Color32::from_rgb(100, 116, 139),
        );

        resp.clicked()
    }

    fn draw_chip_btn(ui: &mut egui::Ui, rect: Rect, label: &str, active: bool) -> bool {
        let resp = ui.allocate_rect(rect, Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let bg = if active {
            Color32::from_rgba_unmultiplied(10, 132, 255, 24)
        } else if resp.hovered() {
            Color32::from_rgb(241, 245, 249)
        } else {
            Color32::WHITE
        };
        let border = if active {
            Color32::from_rgb(10, 132, 255)
        } else {
            Color32::from_rgb(203, 213, 225)
        };
        let fg = if active {
            Color32::from_rgb(10, 132, 255)
        } else {
            Color32::from_rgb(71, 85, 105)
        };
        ui.painter().rect_filled(rect, Rounding::same(5.0), bg);
        ui.painter()
            .rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0_f32, border));
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.5),
            fg,
        );
        resp.clicked()
    }

    fn draw_wizard_btn(
        ui: &mut egui::Ui,
        rect: Rect,
        label: &str,
        primary: bool,
        enabled: bool,
    ) -> bool {
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let resp = ui.allocate_rect(rect, sense);
        let hovered = enabled && resp.hovered();
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let bg = if !enabled {
            Color32::from_rgb(226, 232, 240)
        } else if primary {
            if hovered {
                Color32::from_rgb(0, 112, 235)
            } else {
                Color32::from_rgb(10, 132, 255)
            }
        } else if hovered {
            Color32::from_rgb(248, 250, 252)
        } else {
            Color32::WHITE
        };
        let fg = if !enabled {
            Color32::from_rgb(148, 163, 184)
        } else if primary {
            Color32::WHITE
        } else {
            Color32::from_rgb(30, 41, 59)
        };
        ui.painter().rect_filled(rect, Rounding::same(6.0), bg);
        if !primary {
            ui.painter().rect_stroke(
                rect,
                Rounding::same(6.0),
                Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
            );
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(12.8),
            fg,
        );
        enabled && resp.clicked()
    }

    pub fn run_setup() -> eframe::Result<()> {
        let icon = egui::IconData {
            rgba: ICON_128_RGBA.to_vec(),
            width: 128,
            height: 128,
        };

        let viewport = egui::ViewportBuilder::default()
            .with_title(format!("安装 - Future Proxy v{}", VERSION))
            .with_inner_size(Vec2::new(628.0, 436.0))
            .with_min_inner_size(Vec2::new(628.0, 436.0))
            .with_max_inner_size(Vec2::new(628.0, 436.0))
            .with_resizable(false)
            .with_icon(Arc::new(icon));

        let options = eframe::NativeOptions {
            viewport,
            centered: true,
            ..Default::default()
        };

        eframe::run_native(
            "Future Proxy Setup",
            options,
            Box::new(|cc| Ok(Box::new(SetupWizardApp::new(cc)))),
        )
    }
}

#[cfg(windows)]
fn main() -> eframe::Result<()> {
    win_setup::run_setup()
}
