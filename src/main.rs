use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_SECRET: &str = "future-proxy-secret";
const API_URL: &str = "http://127.0.0.1:9090";

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
    group_name: String,
    current_selected: String,
    search_keyword: String,
    selected_region: String,
    upload_rate: u64,
    download_rate: u64,
    is_core_alive: bool,
    is_system_proxy_on: bool,
    is_testing_delay: bool,
    show_import_dialog: bool,
    import_name: String,
    import_url: String,
    status_msg: String,
    last_poll: Instant,
}

impl AppState {
    fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/home/hu-hub/.config"))
            .join("future-proxy");
        let profiles_dir = config_dir.join("profiles");
        let _ = std::fs::create_dir_all(&profiles_dir);

        // 确保地理数据库文件存在
        ensure_geo_assets(&config_dir);

        let mut meta = ProfilesMeta {
            active: "default".to_string(),
            subscriptions: Vec::new(),
        };

        // 读取 profiles.json
        let json_path = config_dir.join("profiles.json");
        if let Ok(content) = std::fs::read_to_string(&json_path) {
            if let Ok(parsed) = serde_json::from_str::<ProfilesMeta>(&content) {
                meta = parsed;
            }
        }

        // 自动发现已有的旧配置（例如 ~/.config/mihomo-party/profile.yaml 或 ~/.config/mihomo/profiles.json）
        if meta.subscriptions.is_empty() {
            auto_discover_subscriptions(&mut meta);
        }

        // 检测当前 KDE 系统代理状态
        let is_sys_proxy = check_kde_proxy_enabled();

        Self {
            config_dir,
            profiles: meta,
            nodes: Vec::new(),
            group_name: "节点选择".to_string(),
            current_selected: "".to_string(),
            search_keyword: "".to_string(),
            selected_region: "全部".to_string(),
            upload_rate: 0,
            download_rate: 0,
            is_core_alive: false,
            is_system_proxy_on: is_sys_proxy,
            is_testing_delay: false,
            show_import_dialog: false,
            import_name: "".to_string(),
            import_url: "".to_string(),
            status_msg: "就绪".to_string(),
            last_poll: Instant::now() - Duration::from_secs(5),
        }
    }

    fn save_profiles(&self) {
        let json_path = self.config_dir.join("profiles.json");
        if let Ok(content) = serde_json::to_string_pretty(&self.profiles) {
            let _ = std::fs::write(json_path, content);
        }
    }
}

fn ensure_geo_assets(config_dir: &PathBuf) {
    let mmdb = config_dir.join("country.mmdb");
    let geosite = config_dir.join("geosite.dat");

    if !mmdb.exists() {
        for src in &[
            "/usr/share/future-proxy/country.mmdb",
            "/home/hu-hub/.config/mihomo/country.mmdb",
            "/home/hu-hub/.config/mihomo-party/work/country.mmdb",
        ] {
            if std::path::Path::new(src).exists() {
                let _ = std::fs::copy(src, &mmdb);
                break;
            }
        }
    }

    if !geosite.exists() {
        for src in &[
            "/usr/share/future-proxy/geosite.dat",
            "/home/hu-hub/.config/mihomo/geosite.dat",
            "/home/hu-hub/.config/mihomo-party/work/geosite.dat",
        ] {
            if std::path::Path::new(src).exists() {
                let _ = std::fs::copy(src, &geosite);
                break;
            }
        }
    }
}

fn auto_discover_subscriptions(meta: &mut ProfilesMeta) {
    let party_profile = dirs::config_dir()
        .unwrap_or_default()
        .join("mihomo-party/profile.yaml");
    if party_profile.exists() {
        if let Ok(content) = std::fs::read_to_string(&party_profile) {
            let re_name = regex::Regex::new(r"name:\s*(.+)$").unwrap();
            let re_url = regex::Regex::new(r"url:\s*(https?://[^\s]+)").unwrap();
            let mut current_name = String::new();
            for line in content.lines() {
                if let Some(cap) = re_name.captures(line) {
                    current_name = cap[1].trim().to_string();
                }
                if let Some(cap) = re_url.captures(line) {
                    let url = cap[1].trim().to_string();
                    if !current_name.is_empty() {
                        meta.subscriptions.push(Subscription {
                            id: format!("sub_{}", meta.subscriptions.len() + 1),
                            name: current_name.clone(),
                            url,
                            updated_at: "自动读取".to_string(),
                        });
                        current_name.clear();
                    }
                }
            }
        }
    }
}

fn check_kde_proxy_enabled() -> bool {
    let output = std::process::Command::new("kreadconfig6")
        .args(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType"])
        .output();
    if let Ok(out) = output {
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return val == "1";
    }
    false
}

fn set_kde_proxy(enable: bool) {
    if enable {
        let _ = std::process::Command::new("kwriteconfig6")
            .args(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", "1"])
            .status();
        let _ = std::process::Command::new("kwriteconfig6")
            .args(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpProxy", "http://127.0.0.1:7890"])
            .status();
        let _ = std::process::Command::new("kwriteconfig6")
            .args(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "httpsProxy", "http://127.0.0.1:7890"])
            .status();
    } else {
        let _ = std::process::Command::new("kwriteconfig6")
            .args(&["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", "0"])
            .status();
    }
}

fn match_flag(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("港") || lower.contains("hk") || lower.contains("hong kong") {
        "🇭🇰"
    } else if lower.contains("台") || lower.contains("tw") || lower.contains("taiwan") {
        "🇹🇼"
    } else if lower.contains("日") || lower.contains("jp") || lower.contains("japan") {
        "🇯🇵"
    } else if lower.contains("美") || lower.contains("us") || lower.contains("united states") {
        "🇺🇸"
    } else if lower.contains("新") || lower.contains("sg") || lower.contains("singapore") {
        "🇸🇬"
    } else if lower.contains("韩") || lower.contains("kr") || lower.contains("korea") {
        "🇰🇷"
    } else if lower.contains("英") || lower.contains("uk") || lower.contains("britain") {
        "🇬🇧"
    } else if lower.contains("德") || lower.contains("de") || lower.contains("germany") {
        "🇩🇪"
    } else {
        "🌐"
    }
}

fn patch_subscription(raw_yaml: &str) -> String {
    let override_keys = [
        "external-controller",
        "secret",
        "external-ui",
        "tun",
        "dns",
        "log-level",
        "ipv6",
        "mode",
    ];
    let mut filtered = Vec::new();
    let mut skipping = false;
    let re_key = regex::Regex::new(r"^([a-zA-Z0-9_-]+)\s*:").unwrap();

    for line in raw_yaml.lines() {
        if let Some(caps) = re_key.captures(line) {
            let key = &caps[1];
            skipping = override_keys.contains(&key);
        }
        if !skipping {
            filtered.push(line);
        }
    }

    let header = format!(r#"# === Future Proxy Auto Injected Config ===
external-controller: 127.0.0.1:9090
secret: "{}"
external-ui: ui
log-level: warning
ipv6: false
mode: rule

tun:
  enable: true
  stack: mixed
  dns-hijack:
    - "any:53"
  auto-route: true
  auto-detect-interface: true

dns:
  enable: true
  listen: 127.0.0.1:1053
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  nameserver:
    - 223.5.5.5
    - 119.29.29.29
# =========================================
"#, DEFAULT_SECRET);

    format!("{}\n{}\n", header, filtered.join("\n"))
}

pub struct FutureProxyApp {
    state: Arc<Mutex<AppState>>,
}

impl FutureProxyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);

        let state = Arc::new(Mutex::new(AppState::new()));

        let state_clone = Arc::clone(&state);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1000));
            let mut app = state_clone.lock().unwrap();

            // 检查内核是否存活，若未运行则自动拉起
            let alive = ureq::get(&format!("{}/version", API_URL))
                .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                .timeout(Duration::from_millis(400))
                .call()
                .is_ok();

            app.is_core_alive = alive;
            if !alive {
                let cfg_file = app.config_dir.join("config.yaml");
                if !cfg_file.exists() {
                    let default_cfg = patch_subscription("mode: rule\nproxies: []\nrules:\n  - MATCH,DIRECT\n");
                    let _ = std::fs::write(&cfg_file, default_cfg);
                }
                // 优先使用 /usr/bin/future-proxy-core，次选 mihomo-core
                let core_bin = if std::path::Path::new("/usr/bin/future-proxy-core").exists() {
                    "/usr/bin/future-proxy-core"
                } else if std::path::Path::new("/usr/bin/mihomo-core").exists() {
                    "/usr/bin/mihomo-core"
                } else {
                    "future-proxy-core"
                };

                let _ = std::process::Command::new(core_bin)
                    .arg("-d")
                    .arg(&app.config_dir)
                    .spawn();
            }

            // 轮询流量
            if let Ok(resp) = ureq::get(&format!("{}/traffic", API_URL))
                .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                .timeout(Duration::from_millis(600))
                .call()
            {
                if let Ok(val) = resp.into_json::<serde_json::Value>() {
                    app.upload_rate = val.get("up").and_then(|v| v.as_u64()).unwrap_or(0);
                    app.download_rate = val.get("down").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }

            // 定期刷新节点列表
            if app.last_poll.elapsed() > Duration::from_secs(3) {
                app.last_poll = Instant::now();
                Self::refresh_nodes_internal(&mut app);
            }
        });

        Self { state }
    }

    fn refresh_nodes_internal(app: &mut AppState) {
        let resp = ureq::get(&format!("{}/proxies", API_URL))
            .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
            .timeout(Duration::from_millis(1500))
            .call();

        if let Ok(res) = resp {
            if let Ok(json) = res.into_json::<serde_json::Value>() {
                if let Some(proxies) = json.get("proxies").and_then(|p| p.as_object()) {
                    let mut found_group = None;
                    for name in &["节点选择", "PROXY", "Proxy", "节点", "Select", "GLOBAL", "Auto"] {
                        if let Some(group) = proxies.get(*name) {
                            if group.get("all").and_then(|a| a.as_array()).is_some() {
                                found_group = Some((name.to_string(), group));
                                break;
                            }
                        }
                    }

                    if let Some((grp_name, group)) = found_group {
                        app.group_name = grp_name;
                        app.current_selected = group
                            .get("now")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();

                        if let Some(arr) = group.get("all").and_then(|a| a.as_array()) {
                            let mut new_nodes = Vec::new();
                            for item in arr {
                                if let Some(n_name) = item.as_str() {
                                    if n_name == "DIRECT" || n_name == "REJECT" || n_name == "GLOBAL" {
                                        continue;
                                    }
                                    if n_name.contains("流量") || n_name.contains("到期") || n_name.contains("官网") || n_name.contains("通知") || n_name.contains("重置") {
                                        continue;
                                    }
                                    let mut delay = -1;
                                    if let Some(p_obj) = proxies.get(n_name) {
                                        if let Some(hist) = p_obj.get("history").and_then(|h| h.as_array()) {
                                            if let Some(last) = hist.last() {
                                                delay = last.get("delay").and_then(|d| d.as_i64()).unwrap_or(-1) as i32;
                                            }
                                        }
                                    }
                                    new_nodes.push(ProxyNode {
                                        name: n_name.to_string(),
                                        flag: match_flag(n_name).to_string(),
                                        delay,
                                        is_selected: n_name == app.current_selected,
                                    });
                                }
                            }
                            app.nodes = new_nodes;
                        }
                    }
                }
            }
        }
    }

    fn select_node(&self, node_name: String) {
        let state_clone = Arc::clone(&self.state);
        std::thread::spawn(move || {
            let group = {
                let app = state_clone.lock().unwrap();
                app.group_name.clone()
            };
            let enc_group: String = url::form_urlencoded::byte_serialize(group.as_bytes()).collect();
            let body = serde_json::json!({ "name": node_name });
            let _ = ureq::put(&format!("{}/proxies/{}", API_URL, enc_group))
                .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                .set("Content-Type", "application/json")
                .send_json(body);

            let mut app = state_clone.lock().unwrap();
            app.current_selected = node_name.clone();
            for n in &mut app.nodes {
                n.is_selected = n.name == node_name;
            }
        });
    }

    fn ping_all(&self) {
        let state_clone = Arc::clone(&self.state);
        {
            let mut app = state_clone.lock().unwrap();
            app.is_testing_delay = true;
            for n in &mut app.nodes {
                n.delay = -1;
            }
        }

        std::thread::spawn(move || {
            let nodes_to_test = {
                let app = state_clone.lock().unwrap();
                app.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
            };

            let results = Arc::new(Mutex::new(HashMap::new()));
            let mut handles = Vec::new();

            for name in nodes_to_test {
                let res_arc = Arc::clone(&results);
                handles.push(std::thread::spawn(move || {
                    let enc_name: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
                    let url = format!("{}/proxies/{}/delay?timeout=3000&url=http://www.gstatic.com/generate_204", API_URL, enc_name);
                    let delay = if let Ok(resp) = ureq::get(&url)
                        .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                        .timeout(Duration::from_millis(3500))
                        .call()
                    {
                        if let Ok(json) = resp.into_json::<serde_json::Value>() {
                            json.get("delay").and_then(|d| d.as_i64()).unwrap_or(0) as i32
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    res_arc.lock().unwrap().insert(name, delay);
                }));
            }

            for h in handles {
                let _ = h.join();
            }

            let mut app = state_clone.lock().unwrap();
            let final_res = results.lock().unwrap();
            for n in &mut app.nodes {
                if let Some(d) = final_res.get(&n.name) {
                    n.delay = *d;
                }
            }
            app.is_testing_delay = false;
        });
    }

    fn import_sub_action(&self, name: String, url: String) {
        let state_clone = Arc::clone(&self.state);
        std::thread::spawn(move || {
            {
                let mut app = state_clone.lock().unwrap();
                app.status_msg = "正在下载订阅并应用补丁...".to_string();
            }

            let resp = ureq::get(&url)
                .set("User-Agent", "clash.meta; mihomo")
                .timeout(Duration::from_secs(15))
                .call();

            match resp {
                Ok(res) => {
                    if let Ok(body) = res.into_string() {
                        let patched = patch_subscription(&body);
                        let sub_id = format!("sub_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
                        
                        let mut app = state_clone.lock().unwrap();
                        let sub_file = app.config_dir.join("profiles").join(format!("{}.yaml", sub_id));
                        let active_file = app.config_dir.join("config.yaml");

                        let _ = std::fs::write(&sub_file, &patched);
                        let _ = std::fs::write(&active_file, &patched);

                        app.profiles.subscriptions.push(Subscription {
                            id: sub_id.clone(),
                            name: name.clone(),
                            url,
                            updated_at: "刚刚".to_string(),
                        });
                        app.profiles.active = sub_id;
                        app.save_profiles();

                        let reload_body = serde_json::json!({ "path": active_file.to_str().unwrap() });
                        let _ = ureq::put(&format!("{}/configs?force=true", API_URL))
                            .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                            .set("Content-Type", "application/json")
                            .send_json(reload_body);

                        app.status_msg = format!("✓ 订阅 [{}] 导入并切换成功！", name);
                        app.show_import_dialog = false;
                    }
                }
                Err(e) => {
                    let mut app = state_clone.lock().unwrap();
                    app.status_msg = format!("✗ 订阅下载失败: {}", e);
                }
            }
        });
    }

    fn switch_sub_action(&self, sub_id: String) {
        let state_clone = Arc::clone(&self.state);
        std::thread::spawn(move || {
            let mut app = state_clone.lock().unwrap();
            let sub_file = app.config_dir.join("profiles").join(format!("{}.yaml", sub_id));
            let active_file = app.config_dir.join("config.yaml");

            if sub_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&sub_file) {
                    let _ = std::fs::write(&active_file, content);
                    app.profiles.active = sub_id;
                    app.save_profiles();

                    let reload_body = serde_json::json!({ "path": active_file.to_str().unwrap() });
                    let _ = ureq::put(&format!("{}/configs?force=true", API_URL))
                        .set("Authorization", &format!("Bearer {}", DEFAULT_SECRET))
                        .set("Content-Type", "application/json")
                        .send_json(reload_body);

                    app.status_msg = "已切换并平滑重载订阅！".to_string();
                }
            }
        });
    }
}

impl eframe::App for FutureProxyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut action_switch_sub = None;
        let mut action_ping = false;
        let mut action_select_node = None;
        let mut action_import = None;
        let mut toggle_sys_proxy = None;

        {
            let mut app = self.state.lock().unwrap();

            // 1. 顶部状态栏
            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("🚀 Future Proxy");

                    if app.is_core_alive {
                        ui.colored_label(egui::Color32::from_rgb(166, 227, 161), "● 运行中 [TUN全局已就绪]");
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(243, 139, 168), "○ 启动中...");
                    }

                    // 系统代理快捷开关
                    let sys_proxy_text = if app.is_system_proxy_on { "🌐 系统代理: [已开启]" } else { "🌐 系统代理: [已关闭]" };
                    let sys_proxy_btn = ui.selectable_label(app.is_system_proxy_on, sys_proxy_text);
                    if sys_proxy_btn.clicked() {
                        toggle_sys_proxy = Some(!app.is_system_proxy_on);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let up_str = format_speed(app.upload_rate);
                        let down_str = format_speed(app.download_rate);
                        ui.label(format!("↑ {}  ↓ {}", up_str, down_str));
                    });
                });
                ui.add_space(8.0);

                // 2. 订阅工具栏
                ui.horizontal(|ui| {
                    ui.label("当前订阅:");

                    let active_name = app.profiles.subscriptions
                        .iter()
                        .find(|s| s.id == app.profiles.active)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "默认配置".to_string());

                    egui::ComboBox::from_id_salt("sub_combo")
                        .selected_text(format!("✈️ {}", active_name))
                        .show_ui(ui, |ui| {
                            for sub in &app.profiles.subscriptions {
                                if ui.selectable_label(sub.id == app.profiles.active, &sub.name).clicked() {
                                    action_switch_sub = Some(sub.id.clone());
                                }
                            }
                        });

                    if ui.button("➕ 导入新订阅").clicked() {
                        app.show_import_dialog = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ping_btn_text = if app.is_testing_delay { "⚡ 测速中..." } else { "⚡ 全部测速" };
                        if ui.add_enabled(!app.is_testing_delay, egui::Button::new(ping_btn_text)).clicked() {
                            action_ping = true;
                        }
                    });
                });
                ui.add_space(8.0);

                // 3. 搜索与地区筛选
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.search_keyword).hint_text("🔍 搜索节点 (输入关键词例如: 香港 / 01 / IPLC)..."));
                });
                ui.add_space(4.0);

                ui.horizontal_wrapped(|ui| {
                    for reg in &["全部", "🇭🇰 香港", "🇯🇵 日本", "🇺🇸 美国", "🇸🇬 新加坡", "🇹🇼 台湾", "其他"] {
                        let is_active = app.selected_region == *reg;
                        if ui.selectable_label(is_active, *reg).clicked() {
                            app.selected_region = reg.to_string();
                        }
                    }
                });
                ui.add_space(6.0);
            });

            // 底部状态栏
            egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(format!("状态: {}", app.status_msg));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("常驻内存: 约 50MB (纯原生机器码)");
                    });
                });
                ui.add_space(4.0);
            });

            // 中间主区域：节点卡片列表
            egui::CentralPanel::default().show(ctx, |ui| {
                let keyword = app.search_keyword.to_lowercase();
                let region = app.selected_region.clone();

                let filtered_nodes: Vec<ProxyNode> = app
                    .nodes
                    .iter()
                    .filter(|n| {
                        let lower = n.name.to_lowercase();
                        let match_kw = if keyword.is_empty() { true } else { lower.contains(&keyword) };
                        let match_reg = if region == "全部" {
                            true
                        } else if region.contains("香港") {
                            lower.contains("港") || lower.contains("hk")
                        } else if region.contains("日本") {
                            lower.contains("日") || lower.contains("jp")
                        } else if region.contains("美国") {
                            lower.contains("美") || lower.contains("us")
                        } else if region.contains("新加坡") {
                            lower.contains("新") || lower.contains("sg")
                        } else if region.contains("台湾") {
                            lower.contains("台") || lower.contains("tw")
                        } else {
                            !(lower.contains("港") || lower.contains("hk") || lower.contains("日") || lower.contains("jp") || lower.contains("美") || lower.contains("us") || lower.contains("新") || lower.contains("sg") || lower.contains("台") || lower.contains("tw"))
                        };
                        match_kw && match_reg
                    })
                    .cloned()
                    .collect();

                ui.label(format!("显示 {} / {} 个节点 (当前组: {})", filtered_nodes.len(), app.nodes.len(), app.group_name));
                ui.separator();

                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    for node in filtered_nodes {
                        let is_sel = node.is_selected;
                        let frame = egui::Frame::none()
                            .fill(if is_sel { egui::Color32::from_rgb(30, 30, 46) } else { egui::Color32::from_rgb(24, 24, 37) })
                            .stroke(if is_sel { egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(137, 180, 250)) } else { egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(49, 50, 68)) })
                            .rounding(6.0)
                            .inner_margin(8.0);

                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(&node.flag);
                                ui.label(egui::RichText::new(&node.name).strong());

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if is_sel {
                                        ui.colored_label(egui::Color32::from_rgb(137, 180, 250), "✓ 使用中");
                                    } else {
                                        if ui.small_button("选择").clicked() {
                                            action_select_node = Some(node.name.clone());
                                        }
                                    }

                                    if node.delay > 0 {
                                        let col = if node.delay < 150 {
                                            egui::Color32::from_rgb(166, 227, 161)
                                        } else if node.delay < 350 {
                                            egui::Color32::from_rgb(249, 226, 175)
                                        } else {
                                            egui::Color32::from_rgb(243, 139, 168)
                                        };
                                        ui.colored_label(col, format!("{} ms", node.delay));
                                    } else if node.delay == 0 {
                                        ui.colored_label(egui::Color32::from_rgb(243, 139, 168), "超时");
                                    } else {
                                        ui.colored_label(egui::Color32::GRAY, "测速");
                                    }
                                });
                            });
                        });
                        ui.add_space(4.0);
                    }
                });
            });

            // 导入新订阅弹窗
            if app.show_import_dialog {
                let mut do_import = false;

                egui::Window::new("📥 导入新订阅")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.set_width(420.0);
                        ui.add_space(8.0);
                        ui.label("订阅名称 (别名):");
                        ui.add(egui::TextEdit::singleline(&mut app.import_name).hint_text("例如: 主机场 / 备用"));
                        ui.add_space(8.0);
                        ui.label("订阅链接 (URL):");
                        ui.add(egui::TextEdit::singleline(&mut app.import_url).hint_text("请粘贴 Clash / Mihomo 订阅链接..."));
                        ui.add_space(14.0);

                        ui.horizontal(|ui| {
                            if ui.button("取消").clicked() {
                                app.show_import_dialog = false;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("📥 下载并导入").clicked() {
                                    if !app.import_url.trim().is_empty() {
                                        let mut name = app.import_name.trim().to_string();
                                        if name.is_empty() {
                                            name = format!("机场_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
                                        }
                                        action_import = Some((name, app.import_url.trim().to_string()));
                                        do_import = true;
                                    }
                                }
                            });
                        });
                    });

                if do_import {
                    app.show_import_dialog = false;
                }
            }
        }

        if let Some(enable) = toggle_sys_proxy {
            set_kde_proxy(enable);
            let mut app = self.state.lock().unwrap();
            app.is_system_proxy_on = enable;
            app.status_msg = if enable { "KDE 系统代理已开启 (127.0.0.1:7890)".to_string() } else { "KDE 系统代理已关闭 (直连)".to_string() };
        }
        if let Some(sub_id) = action_switch_sub {
            self.switch_sub_action(sub_id);
        }
        if action_ping {
            self.ping_all();
        }
        if let Some(node_name) = action_select_node {
            self.select_node(node_name);
        }
        if let Some((name, url)) = action_import {
            self.import_sub_action(name, url);
        }

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        "/usr/share/fonts/truetype/SourceHanSansCN-Normal.otf",
        "/usr/share/fonts/truetype/wqy-zenhei.ttc",
        "/home/hu-hub/.local/share/fonts/win/msyh.ttf",
    ];

    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk_font".to_owned(),
                egui::FontData::from_owned(data),
            );
            if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                vec.insert(0, "cjk_font".to_owned());
            }
            if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                vec.push("cjk_font".to_owned());
            }
            break;
        }
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

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 720.0])
            .with_min_inner_size([540.0, 500.0])
            .with_title("Future Proxy"),
        ..Default::default()
    };

    eframe::run_native(
        "future-proxy",
        native_options,
        Box::new(|cc| Ok(Box::new(FutureProxyApp::new(cc)))),
    )
}
