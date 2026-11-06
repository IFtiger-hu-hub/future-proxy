use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config_patch::{
    detect_intranet_ip, extract_yaml_traffic_hints, patch_subscription_with_mode,
};
use crate::platform::check_kde_proxy_enabled;

fn default_mode() -> String {
    "rule".to_string()
}

fn default_sort_mode() -> String {
    "delay".to_string()
}

fn default_card_size() -> String {
    "compact".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub updated_at: String,
    /// 已上传流量（字节，来自 Subscription-Userinfo 响应头）
    #[serde(default)]
    pub upload: u64,
    /// 已下载流量（字节，来自 Subscription-Userinfo 响应头）
    #[serde(default)]
    pub download: u64,
    /// 总配额流量（字节，来自 Subscription-Userinfo 响应头）
    #[serde(default)]
    pub total: u64,
    /// 到期 Unix 时间戳（秒，0 表示未提供或长期有效）
    #[serde(default)]
    pub expire: u64,
    /// 从订阅 YAML 信息节点解析的兜底剩余流量（如 "31.59 GB"）
    #[serde(default)]
    pub remaining_hint: String,
    /// 从订阅 YAML 信息节点解析的兜底到期时间（如 "2027-11-04" / "长期有效"）
    #[serde(default)]
    pub expire_hint: String,
}

#[derive(Clone, Debug)]
pub struct LiveConnItem {
    pub source: String,
    pub target: String,
    pub exit: String,
    pub download: u64,
    pub upload: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProfilesMeta {
    pub active: String,
    pub subscriptions: Vec<Subscription>,
    /// 是否启用 TUN（默认关闭，使用系统代理）
    #[serde(default)]
    pub tun: bool,
    /// 是否开启局域网共享 Allow LAN
    #[serde(default)]
    pub allow_lan: bool,
    /// 关闭窗口后是否保留内核在后台运行
    #[serde(default)]
    pub keep_core_on_exit: bool,
    /// 锁定某个地区自动选节点（None 为关闭）
    #[serde(default)]
    pub auto_region: Option<String>,
    /// 开机自启内核（systemd --user）
    #[serde(default)]
    pub autostart: bool,
    /// 出站模式：rule / global / direct
    #[serde(default = "default_mode")]
    pub mode: String,
    /// 节点排序方式：default (原序) / delay (按延迟) / name (按名称)
    #[serde(default = "default_sort_mode")]
    pub sort_mode: String,
    /// 卡片尺寸：compact / standard / loose
    #[serde(default = "default_card_size")]
    pub card_size: String,
    /// 隐藏超时节点
    #[serde(default)]
    pub hide_timeout: bool,
    /// 切换节点时自动断开旧连接（FlClash 同款 closeConnections）
    #[serde(default = "default_true")]
    pub close_connections: bool,
    /// 深色模式（默认 false 使用 FlClash 清爽亮色 Material You 风格）
    #[serde(default)]
    pub dark_mode: bool,
}

#[derive(Clone, Debug)]
pub struct ProxyNode {
    pub name: String,
    pub proto: String,
    pub flag: String,
    pub delay: i32,
    pub is_selected: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Dashboard,
    Proxies,
    Profiles,
    Connections,
    Settings,
}

#[allow(dead_code)]
pub struct AppState {
    pub config_dir: PathBuf,
    pub profiles: ProfilesMeta,
    pub nodes: Vec<ProxyNode>,
    /// 可手动选择的策略组（Selector）
    pub groups: Vec<String>,
    pub group_name: String,
    pub group_picked_by_user: bool,
    pub current_selected: String,
    pub resolved_node: String,
    pub resolved_delay: i32,
    pub probe_google_ms: i32,
    pub probe_cf_ms: i32,
    pub is_probing_status: bool,
    pub search_keyword: String,
    pub search_expanded: bool,
    pub selected_region: String,
    pub upload_rate: u64,
    pub download_rate: u64,
    pub started_at: Instant,
    pub upload_history: VecDeque<u64>,
    pub download_history: VecDeque<u64>,
    pub peak_upload: u64,
    pub peak_download: u64,
    pub total_upload: u64,
    pub total_download: u64,
    pub intranet_ip: String,
    pub hide_ip: bool,
    pub live_connections: Vec<LiveConnItem>,
    pub dash_conn_tab: usize,
    pub group_summaries: Vec<(String, String)>,
    pub active_connections: usize,
    pub is_core_alive: bool,
    pub core_error: String,
    pub is_system_proxy_on: bool,
    pub is_testing_delay: bool,
    pub ping_done: usize,
    pub ping_total: usize,
    pub last_ping_done_at: Option<Instant>,
    pub is_updating_sub: bool,
    pub show_import_dialog: bool,
    pub show_settings_dialog: bool,
    pub import_name: String,
    pub import_url: String,
    pub status_msg: String,
    /// GUI 私有堆内存 (RssAnon, KB)
    pub mem_gui_kb: u64,
    /// GUI 含共享显存/字体的总驻留集 (VmRSS, KB)
    pub mem_gui_rss_kb: u64,
    /// Mihomo 内核实际占用内存 (Inuse / RssAnon, KB)
    pub mem_core_kb: u64,
    /// Mihomo 内核总驻留集 (VmRSS, KB)
    pub mem_core_rss_kb: u64,
    /// 宿主系统已用物理内存 (GB)
    pub mem_sys_used_gb: f32,
    /// 宿主系统总物理内存 (GB)
    pub mem_sys_total_gb: f32,
    /// 内存占用历史趋势 (KB)
    pub mem_history: VecDeque<u64>,
    /// 立即触发一次地区自动选择检查
    pub auto_kick: bool,
    pub open_popup: Option<String>,
    pub more_submenu: Option<String>,
    pub popup_anchor: egui::Pos2,
    pub popup_width: f32,
    pub popup_open_time: f64,
    pub nav_tab: NavTab,
    pub sidebar_collapsed: bool,
    pub scroll_to_selected: bool,
}

impl AppState {
    pub fn new() -> Self {
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
            allow_lan: false,
            keep_core_on_exit: true,
            auto_region: None,
            autostart: false,
            mode: "rule".to_string(),
            sort_mode: "delay".to_string(),
            card_size: "compact".to_string(),
            hide_timeout: false,
            close_connections: true,
            dark_mode: false,
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
            resolved_node: String::new(),
            resolved_delay: -1,
            probe_google_ms: -1,
            probe_cf_ms: -1,
            is_probing_status: false,
            search_keyword: String::new(),
            search_expanded: false,
            selected_region: "全部".to_string(),
            upload_rate: 0,
            download_rate: 0,
            started_at: Instant::now(),
            upload_history: VecDeque::from(vec![0; 28]),
            download_history: VecDeque::from(vec![0; 28]),
            peak_upload: 0,
            peak_download: 0,
            total_upload: 0,
            total_download: 0,
            intranet_ip: detect_intranet_ip(),
            hide_ip: false,
            live_connections: Vec::new(),
            dash_conn_tab: 0,
            group_summaries: Vec::new(),
            active_connections: 0,
            is_core_alive: false,
            core_error: String::new(),
            is_system_proxy_on: check_kde_proxy_enabled(),
            is_testing_delay: false,
            ping_done: 0,
            ping_total: 0,
            last_ping_done_at: None,
            is_updating_sub: false,
            show_import_dialog: false,
            show_settings_dialog: false,
            import_name: String::new(),
            import_url: String::new(),
            status_msg: "就绪".to_string(),
            mem_gui_kb: 0,
            mem_gui_rss_kb: 0,
            mem_core_kb: 0,
            mem_core_rss_kb: 0,
            mem_sys_used_gb: 0.0,
            mem_sys_total_gb: 0.0,
            mem_history: VecDeque::from(vec![0; 24]),
            auto_kick: true,
            open_popup: None,
            more_submenu: None,
            popup_anchor: egui::Pos2::ZERO,
            popup_width: 148.0,
            popup_open_time: 0.0,
            nav_tab: NavTab::Dashboard,
            sidebar_collapsed: false,
            scroll_to_selected: false,
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

        // 从本地 YAML 配置文件补全流量/到期兜底信息
        let cfg_dir_clone = state.config_dir.clone();
        for sub in &mut state.profiles.subscriptions {
            if sub.remaining_hint.is_empty() || sub.expire_hint.is_empty() {
                let p = cfg_dir_clone.join("profiles").join(format!("{}.yaml", sub.id));
                if let Ok(raw) = std::fs::read_to_string(p) {
                    let (rem, exp) = extract_yaml_traffic_hints(&raw);
                    if sub.remaining_hint.is_empty() && !rem.is_empty() {
                        sub.remaining_hint = rem;
                    }
                    if sub.expire_hint.is_empty() && !exp.is_empty() {
                        sub.expire_hint = exp;
                    }
                }
            }
        }

        // 每次启动都根据原始订阅重新生成 config.yaml，保证注入项（端口/TUN/DNS防污染）是最新的
        if let Err(e) = state.write_active_config() {
            state.status_msg = e;
        }
        // Windows 便携版若移动过目录，启动时自动修复注册表/计划任务中的开机自启路径
        #[cfg(windows)]
        {
            if state.profiles.autostart {
                let _ = crate::platform::set_autostart(true);
            } else if crate::platform::check_autostart_enabled() {
                state.profiles.autostart = true;
            }
        }
        state.save_profiles();
        state
    }

    pub fn sub_file(&self, id: &str) -> PathBuf {
        self.config_dir.join("profiles").join(format!("{}.yaml", id))
    }

    pub fn active_config(&self) -> PathBuf {
        self.config_dir.join("config.yaml")
    }

    /// 用激活订阅的原始内容生成 config.yaml
    pub fn write_active_config(&self) -> Result<(), String> {
        let raw = std::fs::read_to_string(self.sub_file(&self.profiles.active))
            .unwrap_or_else(|_| "proxies: []\nrules:\n  - MATCH,DIRECT\n".to_string());
        let mode = if self.profiles.mode.is_empty() {
            "rule"
        } else {
            self.profiles.mode.as_str()
        };
        std::fs::write(
            self.active_config(),
            patch_subscription_with_mode(&raw, self.profiles.tun, mode),
        )
        .map_err(|e| format!("✗ 写入配置失败: {}", e))
    }

    pub fn save_profiles(&self) {
        let json_path = self.config_dir.join("profiles.json");
        if let Ok(content) = serde_json::to_string_pretty(&self.profiles) {
            let _ = std::fs::write(json_path, content);
        }
    }
}

pub fn ensure_geo_assets(config_dir: &Path) {
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
pub fn migrate_party_subscriptions(config_dir: &Path, meta: &mut ProfilesMeta) -> usize {
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
        let (remaining_hint, expire_hint) = std::fs::read_to_string(&dst)
            .map(|raw| extract_yaml_traffic_hints(&raw))
            .unwrap_or_default();
        meta.subscriptions.push(Subscription {
            id: new_id,
            name: pname,
            url: purl,
            updated_at,
            upload: 0,
            download: 0,
            total: 0,
            expire: 0,
            remaining_hint,
            expire_hint,
        });
        count += 1;
    }
    count
}

