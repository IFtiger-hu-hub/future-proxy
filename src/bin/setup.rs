#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    println!("Future Proxy Windows Setup Wizard is only built for Windows targets.");
}

#[cfg(windows)]
mod win_setup {
    use eframe::egui::{self, Color32, FontId, Pos2, Rect, Rounding, Sense, Stroke, Vec2, vec2};
    use std::fs;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const VERSION: &str = env!("CARGO_PKG_VERSION");

    // Embedded payload files from Future-Proxy bundle
    static BIN_FUTURE_PROXY: &[u8] = include_bytes!("../../Future-Proxy/future-proxy.exe");
    static BIN_MIHOMO_CORE: &[u8] = include_bytes!("../../Future-Proxy/future-proxy-core.exe");
    static BIN_WINTUN_DLL: &[u8] = include_bytes!("../../Future-Proxy/wintun.dll");
    static DATA_COUNTRY_MMDB: &[u8] = include_bytes!("../../Future-Proxy/country.mmdb");
    static DATA_GEOSITE_DAT: &[u8] = include_bytes!("../../Future-Proxy/geosite.dat");
    static DATA_CONFIG_YAML: &[u8] = include_bytes!("../../Future-Proxy/config.yaml");

    #[derive(Clone, PartialEq)]
    enum WizardStep {
        Configure,
        Installing,
        Done,
        Error(String),
    }

    struct InstallProgress {
        fraction: f32,
        message: String,
        step: WizardStep,
    }

    pub struct SetupWizardApp {
        install_dir: String,
        opt_desktop_shortcut: bool,
        opt_start_menu: bool,
        opt_autostart: bool,
        opt_launch_after: bool,
        progress: Arc<Mutex<InstallProgress>>,
    }

    impl SetupWizardApp {
        pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
            setup_fonts(&cc.egui_ctx);

            let default_dir = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
                .join("Programs")
                .join("Future-Proxy");

            Self {
                install_dir: default_dir.to_string_lossy().to_string(),
                opt_desktop_shortcut: true,
                opt_start_menu: true,
                opt_autostart: true,
                opt_launch_after: true,
                progress: Arc::new(Mutex::new(InstallProgress {
                    fraction: 0.0,
                    message: "准备就绪".to_string(),
                    step: WizardStep::Configure,
                })),
            }
        }

        fn start_install(&mut self, ctx: egui::Context) {
            let install_dir = PathBuf::from(self.install_dir.trim());
            let opt_desktop = self.opt_desktop_shortcut;
            let opt_start_menu = self.opt_start_menu;
            let opt_autostart = self.opt_autostart;
            let progress = self.progress.clone();

            if let Ok(mut p) = progress.lock() {
                p.step = WizardStep::Installing;
                p.fraction = 0.05;
                p.message = "正在准备安装环境...".to_string();
            }

            std::thread::spawn(move || {
                let update = |frac: f32, msg: &str| {
                    if let Ok(mut p) = progress.lock() {
                        p.fraction = frac;
                        p.message = msg.to_string();
                    }
                    ctx.request_repaint();
                };

                update(0.12, "正在停止运行中的 Future Proxy 与内核进程...");
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
                std::thread::sleep(std::time::Duration::from_millis(450));

                update(0.25, "正在创建安装目录...");
                if let Err(e) = fs::create_dir_all(&install_dir) {
                    if let Ok(mut p) = progress.lock() {
                        p.step = WizardStep::Error(format!("无法创建安装目录 ({}): {}", install_dir.display(), e));
                    }
                    ctx.request_repaint();
                    return;
                }

                update(0.40, "正在释放主程序 future-proxy.exe...");
                let exe_path = install_dir.join("future-proxy.exe");
                if let Err(e) = fs::write(&exe_path, BIN_FUTURE_PROXY) {
                    if let Ok(mut p) = progress.lock() {
                        p.step = WizardStep::Error(format!("写入 future-proxy.exe 失败: {}", e));
                    }
                    ctx.request_repaint();
                    return;
                }

                update(0.60, "正在释放 Mihomo 内核与 Wintun 驱动组件...");
                if let Err(e) = fs::write(install_dir.join("future-proxy-core.exe"), BIN_MIHOMO_CORE) {
                    if let Ok(mut p) = progress.lock() {
                        p.step = WizardStep::Error(format!("写入 future-proxy-core.exe 失败: {}", e));
                    }
                    ctx.request_repaint();
                    return;
                }
                let _ = fs::write(install_dir.join("wintun.dll"), BIN_WINTUN_DLL);

                update(0.75, "正在写入 GeoIP / GeoSite 路由规则数据库...");
                let _ = fs::write(install_dir.join("country.mmdb"), DATA_COUNTRY_MMDB);
                let _ = fs::write(install_dir.join("geosite.dat"), DATA_GEOSITE_DAT);
                let _ = fs::write(install_dir.join("config.yaml"), DATA_CONFIG_YAML);

                // Also seed %APPDATA%\future-proxy so first launch is instant
                let appdata_dir = dirs::data_dir()
                    .unwrap_or_else(|| install_dir.clone())
                    .join("future-proxy");
                let _ = fs::create_dir_all(&appdata_dir);
                let _ = fs::create_dir_all(appdata_dir.join("profiles"));
                let _ = fs::write(appdata_dir.join("country.mmdb"), DATA_COUNTRY_MMDB);
                let _ = fs::write(appdata_dir.join("geosite.dat"), DATA_GEOSITE_DAT);
                let _ = fs::write(appdata_dir.join("wintun.dll"), BIN_WINTUN_DLL);
                if !appdata_dir.join("config.yaml").exists() {
                    let _ = fs::write(appdata_dir.join("config.yaml"), DATA_CONFIG_YAML);
                }

                // Update profiles.json to sync autostart preference while preserving existing profiles
                let meta_path = appdata_dir.join("profiles.json");
                let mut meta_val: serde_json::Value = if let Ok(txt) = fs::read_to_string(&meta_path) {
                    serde_json::from_str(&txt).unwrap_or_else(|_| serde_json::json!({}))
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

                update(0.88, "正在配置快捷方式与开机自启动...");
                let exe_str = exe_path.to_string_lossy().to_string();
                let dir_str = install_dir.to_string_lossy().to_string();

                if opt_desktop {
                    if let Some(desktop) = dirs::desktop_dir() {
                        let lnk = desktop.join("Future Proxy.lnk");
                        create_shortcut(&lnk.to_string_lossy(), &exe_str, &dir_str, "");
                    }
                }

                if opt_start_menu {
                    if let Some(roaming) = dirs::data_dir() {
                        let start_programs = roaming
                            .join("Microsoft")
                            .join("Windows")
                            .join("Start Menu")
                            .join("Programs");
                        let _ = fs::create_dir_all(&start_programs);
                        let lnk = start_programs.join("Future Proxy.lnk");
                        create_shortcut(&lnk.to_string_lossy(), &exe_str, &dir_str, "");
                    }
                }

                configure_autostart(opt_autostart, &exe_str, &dir_str);
                register_uninstaller(&exe_str, &dir_str);

                update(1.0, "Future Proxy 安装完成！");
                std::thread::sleep(std::time::Duration::from_millis(250));
                if let Ok(mut p) = progress.lock() {
                    p.step = WizardStep::Done;
                }
                ctx.request_repaint();
            });
        }
    }

    fn create_shortcut(lnk_path: &str, target_exe: &str, work_dir: &str, args: &str) {
        let ps_script = format!(
            "$s=(New-Object -COM WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Arguments='{}';$s.IconLocation='{},0';$s.Save()",
            lnk_path.replace('\'', "''"),
            target_exe.replace('\'', "''"),
            work_dir.replace('\'', "''"),
            args.replace('\'', "''"),
            target_exe.replace('\'', "''"),
        );
        let _ = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &ps_script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    fn configure_autostart(enable: bool, exe_str: &str, work_dir: &str) {
        // Remove any legacy broken schtasks entry
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
            // 1. Write Startup VBS launcher so working directory and hidden tray startup are guaranteed
            if let Some(ref sdir) = startup_dir {
                let _ = fs::create_dir_all(sdir);
                let vbs_path = sdir.join("FutureProxy.vbs");
                let vbs_content = format!(
                    "Set ws = CreateObject(\"WScript.Shell\")\r\nws.CurrentDirectory = \"{}\"\r\nws.Run \"\"\"{}\"\" --tray\", 0, False\r\n",
                    work_dir.replace('"', "\"\""),
                    exe_str.replace('"', "\"\"")
                );
                let _ = fs::write(vbs_path, vbs_content);
            }
            // 2. Write HKCU\Software\Microsoft\Windows\CurrentVersion\Run
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

    fn register_uninstaller(exe_str: &str, work_dir: &str) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\FutureProxy";
        let entries = [
            ("DisplayName", "Future Proxy"),
            ("DisplayVersion", VERSION),
            ("Publisher", "Future Proxy Team"),
            ("InstallLocation", work_dir),
            ("DisplayIcon", exe_str),
            ("UninstallString", &format!("cmd.exe /c taskkill /F /IM future-proxy.exe & taskkill /F /IM future-proxy-core.exe & rmdir /S /Q \"{}\"", work_dir)),
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
            visuals.panel_fill = Color32::from_rgb(245, 247, 250);
            ctx.set_visuals(visuals);

            let (step, frac, msg) = {
                let p = self.progress.lock().unwrap();
                (p.step.clone(), p.fraction, p.message.clone())
            };

            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::from_rgb(245, 247, 250)))
                .show(ctx, |ui| {
                    let full_rect = ui.max_rect();

                    // Top dark header banner
                    let banner_h = 96.0;
                    let banner_rect = Rect::from_min_size(full_rect.min, vec2(full_rect.width(), banner_h));
                    let p = ui.painter();
                    p.rect_filled(banner_rect, 0.0, Color32::from_rgb(15, 23, 42));

                    // Subtle accent circle glow in header right
                    p.circle_filled(
                        Pos2::new(banner_rect.right() - 48.0, banner_rect.center().y),
                        76.0,
                        Color32::from_rgba_unmultiplied(10, 132, 255, 28),
                    );
                    p.circle_filled(
                        Pos2::new(banner_rect.right() - 48.0, banner_rect.center().y),
                        44.0,
                        Color32::from_rgba_unmultiplied(10, 132, 255, 45),
                    );

                    // Brand icon badge
                    let icon_rect = Rect::from_min_size(
                        Pos2::new(banner_rect.left() + 24.0, banner_rect.top() + 22.0),
                        vec2(48.0, 48.0),
                    );
                    p.rect_filled(icon_rect, Rounding::same(12.0), Color32::from_rgb(10, 132, 255));
                    // Lightning bolt inside icon
                    let ic = icon_rect.center();
                    let bolt = vec![
                        Pos2::new(ic.x + 2.5, ic.y - 13.0),
                        Pos2::new(ic.x - 8.0, ic.y + 1.5),
                        Pos2::new(ic.x - 0.5, ic.y + 1.5),
                        Pos2::new(ic.x - 2.5, ic.y + 13.0),
                        Pos2::new(ic.x + 8.0, ic.y - 1.5),
                        Pos2::new(ic.x + 0.5, ic.y - 1.5),
                    ];
                    p.add(egui::Shape::convex_polygon(bolt, Color32::WHITE, Stroke::NONE));

                    p.text(
                        Pos2::new(icon_rect.right() + 16.0, icon_rect.top() + 14.0),
                        egui::Align2::LEFT_CENTER,
                        format!("Future Proxy v{} 安装向导", VERSION),
                        FontId::proportional(19.0),
                        Color32::WHITE,
                    );
                    p.text(
                        Pos2::new(icon_rect.right() + 16.0, icon_rect.top() + 36.0),
                        egui::Align2::LEFT_CENTER,
                        "新一代轻量级 Mihomo 桌面代理客户端 · 自动配置内核与开机自启",
                        FontId::proportional(12.5),
                        Color32::from_rgb(148, 163, 184),
                    );

                    // Main content area
                    let body_rect = Rect::from_min_max(
                        Pos2::new(full_rect.left() + 24.0, banner_rect.bottom() + 18.0),
                        Pos2::new(full_rect.right() - 24.0, full_rect.bottom() - 20.0),
                    );
                    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));

                    match step {
                        WizardStep::Configure => {
                            // Card container
                            let card_rect = Rect::from_min_size(
                                body_rect.min,
                                vec2(body_rect.width(), body_rect.height() - 54.0),
                            );
                            body_ui.painter().rect_filled(card_rect, Rounding::same(14.0), Color32::WHITE);
                            body_ui.painter().rect_stroke(
                                card_rect,
                                Rounding::same(14.0),
                                Stroke::new(1.0, Color32::from_rgb(226, 232, 240)),
                            );

                            let inner = card_rect.shrink(18.0);
                            let mut cui = body_ui.new_child(egui::UiBuilder::new().max_rect(inner));
                            cui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("安装位置 (Installation Directory)")
                                        .size(13.0)
                                        .strong()
                                        .color(Color32::from_rgb(30, 41, 59)),
                                );
                                ui.add_space(6.0);
                                ui.add_sized(
                                    [inner.width(), 30.0],
                                    egui::TextEdit::singleline(&mut self.install_dir)
                                        .font(FontId::proportional(13.0)),
                                );
                                ui.add_space(14.0);
                                ui.separator();
                                ui.add_space(10.0);

                                ui.label(
                                    egui::RichText::new("安装与启动选项")
                                        .size(13.0)
                                        .strong()
                                        .color(Color32::from_rgb(30, 41, 59)),
                                );
                                ui.add_space(6.0);
                                ui.checkbox(
                                    &mut self.opt_desktop_shortcut,
                                    egui::RichText::new("创建桌面快捷方式 (Future Proxy)").size(13.0),
                                );
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut self.opt_start_menu,
                                    egui::RichText::new("创建「开始菜单」程序快捷方式").size(13.0),
                                );
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut self.opt_autostart,
                                    egui::RichText::new("开机自动启动 (登录 Windows 时静默运行托盘并恢复系统代理)")
                                        .size(13.0)
                                        .color(Color32::from_rgb(10, 132, 255)),
                                );
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut self.opt_launch_after,
                                    egui::RichText::new("安装完成后立即启动 Future Proxy").size(13.0),
                                );
                            });

                            // Footer buttons
                            let btn_y = body_rect.bottom() - 38.0;
                            let cancel_rect = Rect::from_min_size(Pos2::new(body_rect.left(), btn_y), vec2(96.0, 36.0));
                            if draw_pill_btn(&mut body_ui, cancel_rect, "取消退出", false) {
                                std::process::exit(0);
                            }

                            let install_rect = Rect::from_min_size(
                                Pos2::new(body_rect.right() - 148.0, btn_y),
                                vec2(148.0, 36.0),
                            );
                            if draw_pill_btn(&mut body_ui, install_rect, "立即安装", true) {
                                self.start_install(ctx.clone());
                            }
                        }
                        WizardStep::Installing => {
                            let card_rect = Rect::from_min_size(
                                body_rect.min,
                                vec2(body_rect.width(), body_rect.height() - 54.0),
                            );
                            let p = body_ui.painter();
                            p.rect_filled(card_rect, Rounding::same(14.0), Color32::WHITE);
                            p.rect_stroke(
                                card_rect,
                                Rounding::same(14.0),
                                Stroke::new(1.0, Color32::from_rgb(226, 232, 240)),
                            );

                            let center = card_rect.center();
                            p.text(
                                Pos2::new(center.x, center.y - 34.0),
                                egui::Align2::CENTER_CENTER,
                                "正在安装 Future Proxy...",
                                FontId::proportional(16.5),
                                Color32::from_rgb(15, 23, 42),
                            );
                            p.text(
                                Pos2::new(center.x, center.y - 6.0),
                                egui::Align2::CENTER_CENTER,
                                &msg,
                                FontId::proportional(13.0),
                                Color32::from_rgb(100, 116, 139),
                            );

                            let bar_w = card_rect.width() - 72.0;
                            let bar_rect = Rect::from_center_size(
                                Pos2::new(center.x, center.y + 28.0),
                                vec2(bar_w, 10.0),
                            );
                            p.rect_filled(bar_rect, Rounding::same(5.0), Color32::from_rgb(226, 232, 240));
                            let fill_w = (bar_w * frac.clamp(0.0, 1.0)).max(10.0);
                            let fill_rect = Rect::from_min_size(bar_rect.min, vec2(fill_w, 10.0));
                            p.rect_filled(fill_rect, Rounding::same(5.0), Color32::from_rgb(10, 132, 255));

                            p.text(
                                Pos2::new(center.x, center.y + 52.0),
                                egui::Align2::CENTER_CENTER,
                                format!("{:.0}%", frac * 100.0),
                                FontId::proportional(12.5),
                                Color32::from_rgb(10, 132, 255),
                            );
                        }
                        WizardStep::Done => {
                            let card_rect = Rect::from_min_size(
                                body_rect.min,
                                vec2(body_rect.width(), body_rect.height() - 54.0),
                            );
                            let p = body_ui.painter();
                            p.rect_filled(card_rect, Rounding::same(14.0), Color32::WHITE);
                            p.rect_stroke(
                                card_rect,
                                Rounding::same(14.0),
                                Stroke::new(1.0, Color32::from_rgb(226, 232, 240)),
                            );

                            let center = card_rect.center();
                            p.circle_filled(
                                Pos2::new(center.x, center.y - 44.0),
                                24.0,
                                Color32::from_rgba_unmultiplied(52, 199, 89, 32),
                            );
                            p.circle_filled(
                                Pos2::new(center.x, center.y - 44.0),
                                17.0,
                                Color32::from_rgb(52, 199, 89),
                            );
                            p.text(
                                Pos2::new(center.x, center.y - 45.0),
                                egui::Align2::CENTER_CENTER,
                                "✓",
                                FontId::proportional(18.0),
                                Color32::WHITE,
                            );

                            p.text(
                                Pos2::new(center.x, center.y - 4.0),
                                egui::Align2::CENTER_CENTER,
                                format!("Future Proxy v{} 安装成功！", VERSION),
                                FontId::proportional(17.0),
                                Color32::from_rgb(15, 23, 42),
                            );
                            p.text(
                                Pos2::new(center.x, center.y + 24.0),
                                egui::Align2::CENTER_CENTER,
                                format!("安装路径: {}", self.install_dir),
                                FontId::proportional(12.5),
                                Color32::from_rgb(100, 116, 139),
                            );
                            let auto_tip = if self.opt_autostart {
                                "✓ 已启用开机自动启动 (Windows 启动目录 + 注册表双重守护)"
                            } else {
                                "可在软件「工具」页面随时开启开机自启动"
                            };
                            p.text(
                                Pos2::new(center.x, center.y + 48.0),
                                egui::Align2::CENTER_CENTER,
                                auto_tip,
                                FontId::proportional(12.5),
                                Color32::from_rgb(34, 197, 94),
                            );

                            let btn_y = body_rect.bottom() - 38.0;
                            let btn_label = if self.opt_launch_after {
                                "完成并启动 Future Proxy"
                            } else {
                                "完成"
                            };
                            let done_rect = Rect::from_min_size(
                                Pos2::new(body_rect.right() - 188.0, btn_y),
                                vec2(188.0, 36.0),
                            );
                            if draw_pill_btn(&mut body_ui, done_rect, btn_label, true) {
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
                        WizardStep::Error(err_msg) => {
                            let card_rect = Rect::from_min_size(
                                body_rect.min,
                                vec2(body_rect.width(), body_rect.height() - 54.0),
                            );
                            let p = body_ui.painter();
                            p.rect_filled(card_rect, Rounding::same(14.0), Color32::WHITE);
                            p.text(
                                card_rect.center() - vec2(0.0, 16.0),
                                egui::Align2::CENTER_CENTER,
                                "安装过程中出现错误",
                                FontId::proportional(16.0),
                                Color32::from_rgb(239, 68, 68),
                            );
                            p.text(
                                card_rect.center() + vec2(0.0, 14.0),
                                egui::Align2::CENTER_CENTER,
                                &err_msg,
                                FontId::proportional(12.5),
                                Color32::from_rgb(71, 85, 105),
                            );

                            let btn_y = body_rect.bottom() - 38.0;
                            let back_rect = Rect::from_min_size(
                                Pos2::new(body_rect.right() - 120.0, btn_y),
                                vec2(120.0, 36.0),
                            );
                            if draw_pill_btn(&mut body_ui, back_rect, "返回重试", true) {
                                if let Ok(mut pr) = self.progress.lock() {
                                    pr.step = WizardStep::Configure;
                                }
                            }
                        }
                    }
                });
        }
    }

    fn draw_pill_btn(ui: &mut egui::Ui, rect: Rect, label: &str, primary: bool) -> bool {
        let resp = ui.allocate_rect(rect, Sense::click());
        let hovered = resp.hovered();
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let bg = if primary {
            if hovered {
                Color32::from_rgb(0, 112, 235)
            } else {
                Color32::from_rgb(10, 132, 255)
            }
        } else if hovered {
            Color32::from_rgb(226, 232, 240)
        } else {
            Color32::WHITE
        };
        let fg = if primary {
            Color32::WHITE
        } else {
            Color32::from_rgb(71, 85, 105)
        };
        ui.painter().rect_filled(rect, Rounding::same(18.0), bg);
        if !primary {
            ui.painter().rect_stroke(
                rect,
                Rounding::same(18.0),
                Stroke::new(1.0, Color32::from_rgb(203, 213, 225)),
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.5),
            fg,
        );
        resp.clicked()
    }

    pub fn run_setup() -> eframe::Result<()> {
        // Generate 64x64 blue rounded icon
        let mut rgba = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let idx = (y * 64 + x) * 4;
                rgba[idx] = 10;
                rgba[idx + 1] = 132;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
        let icon = egui::IconData {
            rgba,
            width: 64,
            height: 64,
        };

        let viewport = egui::ViewportBuilder::default()
            .with_title(format!("Future Proxy v{} 安装向导", VERSION))
            .with_inner_size(Vec2::new(560.0, 410.0))
            .with_min_inner_size(Vec2::new(560.0, 410.0))
            .with_max_inner_size(Vec2::new(560.0, 410.0))
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

