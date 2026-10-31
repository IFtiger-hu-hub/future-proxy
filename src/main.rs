use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicBool, Ordering};
use ksni::blocking::TrayMethods;

struct TrayFlags {
    show: AtomicBool,
    quit: AtomicBool,
}

struct ProxyTray {
    flags: Arc<TrayFlags>,
    ctx: egui::Context,
}

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
        self.ctx.request_repaint();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "打开界面".into(),
                activate: Box::new(|this: &mut Self| {
                    this.flags.show.store(true, Ordering::SeqCst);
                    this.ctx.request_repaint();
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
                    this.ctx.request_repaint();
                }),
                ..Default::default()
            }
            .into(),
        ]
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
    import_name: String,
    import_url: String,
    status_msg: String,
    mem_gui_kb: u64,
    mem_core_kb: u64,
    /// 立即触发一次地区自动选择检查
    auto_kick: bool,
}

impl AppState {
    fn new() -> Self {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/home/hu-hub/.config"))
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
            import_name: String::new(),
            import_url: String::new(),
            status_msg: "就绪".to_string(),
            mem_gui_kb: 0,
            mem_core_kb: 0,
            auto_kick: true,
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
    for (file, sources) in [
        (
            "country.mmdb",
            [
                "/usr/share/future-proxy/country.mmdb",
                "/home/hu-hub/.config/mihomo/country.mmdb",
                "/home/hu-hub/.config/mihomo-party/work/country.mmdb",
            ],
        ),
        (
            "geosite.dat",
            [
                "/usr/share/future-proxy/geosite.dat",
                "/home/hu-hub/.config/mihomo/geosite.dat",
                "/home/hu-hub/.config/mihomo-party/work/geosite.dat",
            ],
        ),
    ] {
        let dst = config_dir.join(file);
        if dst.exists() {
            continue;
        }
        if let Some(src) = sources.iter().find(|s| Path::new(s).exists()) {
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

fn check_kde_proxy_enabled() -> bool {
    Command::new("kreadconfig6")
        .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

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

fn systemd_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "future-proxy.service"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

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

    format!("{}\n{}\n", header, filtered.join("\n"))
}

fn looks_like_clash_config(body: &str) -> bool {
    body.lines().any(|l| l.starts_with("proxies:") || l.starts_with("proxy-providers:"))
}

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
    fn core_bin() -> &'static str {
        if Path::new("/usr/bin/future-proxy-core").exists() {
            "/usr/bin/future-proxy-core"
        } else if Path::new("/usr/bin/mihomo-core").exists() {
            "/usr/bin/mihomo-core"
        } else {
            "future-proxy-core"
        }
    }

    /// 返回内核错误提示（为空表示正常或正在启动）
    fn ensure_running(&mut self, config_dir: &Path, alive: bool, prefer_systemd: bool) -> String {
        if alive {
            self.failures = 0;
            return String::new();
        }
        // 自启服务已启用时，优先让 systemd 拉起，避免和 GUI 各起一份
        if prefer_systemd {
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
            // 给 systemd 一点时间；仍失败再回落到直接 spawn
            if systemd_active() {
                return String::new();
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
        let mut cmd = Command::new(Self::core_bin());
        cmd.arg("-d").arg(config_dir).stdin(Stdio::null());
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
                format!("无法启动内核 {}: {}", Self::core_bin(), e)
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
        // systemd 托管时不要杀，否则会跟服务抢
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

const REGIONS: [&str; 7] = ["全部", "🇭🇰 香港", "🇯🇵 日本", "🇺🇸 美国", "🇸🇬 新加坡", "🇹🇼 台湾", "其他"];

fn region_match(region: &str, name: &str) -> bool {
    let lower = name.to_lowercase();
    let is = |a: &str, b: &str| lower.contains(a) || lower.contains(b);
    if region == "全部" {
        true
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
        !(is("港", "hk") || is("日", "jp") || is("美", "us") || is("新", "sg") || is("台", "tw"))
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
    _tray: Option<ksni::blocking::Handle<ProxyTray>>,
    force_quit: bool,
}

impl FutureProxyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);

        let state = Arc::new(Mutex::new(AppState::new()));
        {
            // 启动时按配置同步 systemd 自启，并尽量走服务拉内核
            let mut app = state.lock().unwrap();
            if app.profiles.autostart {
                match set_autostart(true) {
                    Ok(()) => app.status_msg = "内核由 systemd 托管".to_string(),
                    Err(e) => app.status_msg = format!("自启服务异常: {}（将回退直接启动内核）", e),
                }
            }
        }
        let core = Arc::new(Mutex::new(CoreManager { child: None, last_spawn: None, failures: 0 }));
        let ctx = cc.egui_ctx.clone();

        // 监控线程：网络请求一律在锁外进行，只在写结果时短暂加锁
        {
            let state = Arc::clone(&state);
            let core = Arc::clone(&core);
            let ctx = ctx.clone();
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
                        let core_kb = core_pid.map(|p| read_rss_kb(&p.to_string())).unwrap_or(0);
                        Some((read_rss_kb("self"), core_kb))
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
                            // 测速中保留本地测速结果，避免被覆盖
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
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_millis(if alive { 1000 } else { 500 }));
                }
            });
        }

        // 流量线程：/traffic 是持续推送的流，按行读取
        {
            let state = Arc::clone(&state);
            let ctx = ctx.clone();
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
                                ctx.request_repaint();
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            });
        }

        // 地区自动选择：当前节点超时或不在锁定地区时，切到该地区延迟最低的节点
        {
            let state = Arc::clone(&state);
            let ctx = ctx.clone();
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
                    last = Instant::now();
                    state.lock().unwrap().auto_kick = false;

                    let candidates: Vec<String> = names.into_iter().filter(|n| region_match(&region, n)).collect();
                    if candidates.is_empty() {
                        state.lock().unwrap().status_msg = format!("自动选择: {} 没有可用节点", region);
                        ctx.request_repaint();
                        continue;
                    }
                    // 当前节点在该地区且可用，就不打扰
                    if candidates.contains(&current) {
                        let d = test_delay(&current);
                        if d > 0 {
                            let mut app = state.lock().unwrap();
                            if let Some(n) = app.nodes.iter_mut().find(|n| n.name == current) {
                                n.delay = d;
                            }
                            drop(app);
                            ctx.request_repaint();
                            continue;
                        }
                    }
                    let results = test_delays(&candidates);
                    let best = results.iter().filter(|(_, d)| **d > 0).min_by_key(|(_, d)| **d).map(|(n, d)| (n.clone(), *d));
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
                                    app.status_msg = format!("自动选择: 已切到 {} ({} ms)", name, d);
                                }
                                Err(e) => app.status_msg = format!("✗ 自动切换失败: {}", e),
                            }
                        }
                        None => app.status_msg = format!("自动选择: {} 的节点全部超时，保持当前节点", region),
                    }
                    ctx.request_repaint();
                }
            });
        }

        let tray_flags = Arc::new(TrayFlags {
            show: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        });
        let tray_handle = ProxyTray {
            flags: Arc::clone(&tray_flags),
            ctx: ctx.clone(),
        }
        .spawn()
        .map_err(|e| eprintln!("托盘启动失败: {e}"))
        .ok();

        Self {
            state,
            core,
            ctx,
            tray_flags,
            _tray: tray_handle,
            force_quit: false,
        }
    }

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
    Command::new("date")
        .arg("+%m-%d %H:%M")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "刚刚".to_string())
}

impl eframe::App for FutureProxyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.tray_flags.quit.swap(false, Ordering::SeqCst) {
            self.force_quit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.tray_flags.show.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

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

            egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("🚀 Future Proxy");

                    if app.is_core_alive {
                        let mode = if app.profiles.tun { "TUN" } else { "系统代理模式" };
                        ui.colored_label(egui::Color32::from_rgb(166, 227, 161), format!("● 运行中 [{}]", mode));
                    } else if app.core_error.is_empty() {
                        ui.colored_label(egui::Color32::from_rgb(249, 226, 175), "○ 内核启动中...");
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(243, 139, 168), "✗ 内核未运行");
                        if ui.small_button("重试").clicked() {
                            action_retry_core = true;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("↑ {}  ↓ {}", format_speed(app.upload_rate), format_speed(app.download_rate)));
                    });
                });
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    let sys_on = app.is_system_proxy_on;
                    if ui.selectable_label(sys_on, if sys_on { "🌐 系统代理: 开" } else { "🌐 系统代理: 关" }).clicked() {
                        toggle_sys_proxy = Some(!sys_on);
                    }
                    let tun_on = app.profiles.tun;
                    if ui
                        .selectable_label(tun_on, if tun_on { "🛡 TUN: 开" } else { "🛡 TUN: 关" })
                        .on_hover_text("接管全部流量（包括不走系统代理的程序）。不要和 clash-party 的 TUN 同时开。")
                        .clicked()
                    {
                        toggle_tun = Some(!tun_on);
                    }
                    if ui
                        .checkbox(&mut app.profiles.keep_core_on_exit, "关闭窗口后保持代理")
                        .changed()
                    {
                        app.save_profiles();
                    }
                    let mut auto = app.profiles.autostart;
                    if ui
                        .checkbox(&mut auto, "开机自启内核")
                        .on_hover_text("登录后由 systemd 用户服务拉起内核并打开系统代理，界面可不开")
                        .changed()
                    {
                        match set_autostart(auto) {
                            Ok(()) => {
                                app.profiles.autostart = auto;
                                if auto {
                                    app.profiles.keep_core_on_exit = true;
                                }
                                app.save_profiles();
                                app.status_msg = if auto {
                                    "✓ 已开启开机自启（systemd 用户服务）".to_string()
                                } else {
                                    "✓ 已关闭开机自启（当前内核仍在运行）".to_string()
                                };
                            }
                            Err(e) => {
                                app.status_msg = format!("✗ 设置自启失败: {}", e);
                            }
                        }
                    }
                });
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("当前订阅:");
                    let active = app.profiles.subscriptions.iter().find(|s| s.id == app.profiles.active).cloned();
                    let active_name = active.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "无".to_string());

                    egui::ComboBox::from_id_salt("sub_combo")
                        .selected_text(format!("✈️ {}", active_name))
                        .show_ui(ui, |ui| {
                            for sub in &app.profiles.subscriptions {
                                let label = format!("{}  ({})", sub.name, sub.updated_at);
                                if ui.selectable_label(sub.id == app.profiles.active, label).clicked()
                                    && sub.id != app.profiles.active
                                {
                                    action_switch_sub = Some(sub.id.clone());
                                }
                            }
                        });

                    if let Some(s) = active {
                        if ui
                            .add_enabled(!app.is_updating_sub, egui::Button::new(if app.is_updating_sub { "更新中..." } else { "🔄 更新" }))
                            .clicked()
                        {
                            action_update_sub = Some(s);
                        }
                    }
                    if ui.button("➕ 导入").clicked() {
                        app.show_import_dialog = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let txt = if app.is_testing_delay { "⚡ 测速中..." } else { "⚡ 全部测速" };
                        if ui.add_enabled(!app.is_testing_delay && app.is_core_alive, egui::Button::new(txt)).clicked() {
                            action_ping = true;
                        }
                    });
                });
                ui.add_space(6.0);

                if app.groups.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.label("策略组:");
                        let mut picked = None;
                        egui::ComboBox::from_id_salt("group_combo")
                            .selected_text(app.group_name.clone())
                            .show_ui(ui, |ui| {
                                for g in &app.groups {
                                    if ui.selectable_label(*g == app.group_name, g).clicked() {
                                        picked = Some(g.clone());
                                    }
                                }
                            });
                        if let Some(g) = picked {
                            app.group_name = g;
                            app.group_picked_by_user = true;
                            app.nodes.clear();
                        }
                    });
                    ui.add_space(4.0);
                }

                ui.add(
                    egui::TextEdit::singleline(&mut app.search_keyword)
                        .hint_text("🔍 搜索节点 (例如: 香港 / 01 / IPLC)...")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(4.0);

                ui.horizontal_wrapped(|ui| {
                    for reg in REGIONS {
                        let is_active = app.selected_region == reg;
                        let label = if app.profiles.auto_region.as_deref() == Some(reg) { format!("{} 🔁", reg) } else { reg.to_string() };
                        if ui.selectable_label(is_active, label).clicked() {
                            app.selected_region = reg.to_string();
                        }
                    }
                    let region = app.selected_region.clone();
                    if region != "全部" {
                        ui.separator();
                        let on = app.profiles.auto_region.as_deref() == Some(region.as_str());
                        let txt = if on { "🔁 自动选该地区: 开" } else { "🔁 在该地区自动选" };
                        if ui
                            .selectable_label(on, txt)
                            .on_hover_text("当前节点超时或不在该地区时，自动切到该地区延迟最低的节点（每 30 秒检查一次）")
                            .clicked()
                        {
                            app.profiles.auto_region = if on { None } else { Some(region.clone()) };
                            app.auto_kick = true;
                            app.status_msg = if on { "已关闭地区自动选择".to_string() } else { format!("已开启: 在 {} 内自动选择最快节点...", region) };
                            app.save_profiles();
                        }
                    } else if let Some(r) = app.profiles.auto_region.clone() {
                        ui.separator();
                        ui.label(format!("🔁 自动: {}", r));
                        if ui.small_button("关闭").clicked() {
                            app.profiles.auto_region = None;
                            app.save_profiles();
                        }
                    }
                });
                ui.add_space(6.0);
            });

            egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let msg = if !app.core_error.is_empty() { app.core_error.clone() } else { app.status_msg.clone() };
                    ui.label(format!("状态: {}", msg));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!(
                            "内存: 界面 {:.0}MB · 内核 {:.0}MB",
                            app.mem_gui_kb as f64 / 1024.0,
                            app.mem_core_kb as f64 / 1024.0
                        ));
                    });
                });
                ui.add_space(4.0);
            });

            egui::CentralPanel::default().show(ctx, |ui| {
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
                // 有延迟的按升序；未测(-1)其次；超时(0)最后
                filtered_nodes.sort_by_key(|n| match n.delay {
                    d if d > 0 => (0, d),
                    -1 => (1, 0),
                    _ => (2, 0),
                });

                if app.profiles.subscriptions.is_empty() {
                    ui.label("还没有订阅，点击上方「➕ 导入」添加。");
                } else if app.nodes.is_empty() && app.is_core_alive {
                    ui.label("当前策略组没有可选节点，或正在加载...");
                }
                ui.label(format!("显示 {} / {} 个节点 (当前组: {})", filtered_nodes.len(), app.nodes.len(), app.group_name));
                ui.separator();

                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    // 紧凑多列网格：按窗口宽度自动决定列数
                    let gap = 6.0;
                    let avail = ui.available_width();
                    let cols = ((avail + gap) / (190.0 + gap)).floor().max(1.0) as usize;
                    let card_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
                    let card_h = 44.0;
                    for row in filtered_nodes.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for node in row {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(card_w, card_h), egui::Sense::click());
                                let is_sel = node.is_selected;
                                let fill = if is_sel {
                                    egui::Color32::from_rgb(36, 39, 58)
                                } else if resp.hovered() {
                                    egui::Color32::from_rgb(32, 32, 48)
                                } else {
                                    egui::Color32::from_rgb(24, 24, 37)
                                };
                                let stroke = if is_sel {
                                    egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(137, 180, 250))
                                } else {
                                    egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(49, 50, 68))
                                };
                                let painter = ui.painter_at(rect);
                                painter.rect(rect.shrink(0.5), 6.0, fill, stroke);

                                let text_col = if is_sel { egui::Color32::from_rgb(137, 180, 250) } else { egui::Color32::from_rgb(205, 214, 244) };
                                let mut job = egui::text::LayoutJob::single_section(
                                    format!("{} {}", node.flag, node.name),
                                    egui::TextFormat { font_id: egui::FontId::proportional(13.0), color: text_col, ..Default::default() },
                                );
                                job.wrap = egui::text::TextWrapping {
                                    max_width: card_w - 16.0,
                                    max_rows: 1,
                                    break_anywhere: true,
                                    overflow_character: Some('…'),
                                };
                                let galley = ui.fonts(|f| f.layout_job(job));
                                painter.galley(rect.left_top() + egui::vec2(8.0, 6.0), galley, text_col);

                                let (dtxt, dcol) = if node.delay > 0 {
                                    let c = if node.delay < 150 {
                                        egui::Color32::from_rgb(166, 227, 161)
                                    } else if node.delay < 350 {
                                        egui::Color32::from_rgb(249, 226, 175)
                                    } else {
                                        egui::Color32::from_rgb(243, 139, 168)
                                    };
                                    (format!("{} ms", node.delay), c)
                                } else if node.delay == 0 {
                                    ("超时".to_string(), egui::Color32::from_rgb(243, 139, 168))
                                } else {
                                    ("-".to_string(), egui::Color32::GRAY)
                                };
                                painter.text(
                                    rect.left_bottom() + egui::vec2(8.0, -6.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    dtxt,
                                    egui::FontId::proportional(11.0),
                                    dcol,
                                );
                                if is_sel {
                                    painter.text(
                                        rect.right_bottom() + egui::vec2(-8.0, -6.0),
                                        egui::Align2::RIGHT_BOTTOM,
                                        "✓ 使用中",
                                        egui::FontId::proportional(11.0),
                                        egui::Color32::from_rgb(137, 180, 250),
                                    );
                                }
                                let resp = resp.on_hover_text(&node.name);
                                if !is_sel && resp.clicked() {
                                    action_select_node = Some(node.name.clone());
                                }
                            }
                        });
                        ui.add_space(gap);
                    }
                });
            });

            if app.show_import_dialog {
                let mut close = false;
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
                                close = true;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("📥 下载并导入").clicked() && !app.import_url.trim().is_empty() {
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
                if close {
                    app.show_import_dialog = false;
                    app.import_name.clear();
                    app.import_url.clear();
                }
            }
        }

        if let Some(enable) = toggle_sys_proxy {
            set_kde_proxy(enable);
            let on = check_kde_proxy_enabled();
            let mut app = self.state.lock().unwrap();
            app.is_system_proxy_on = on;
            app.status_msg = if on == enable {
                if on { format!("KDE 系统代理已开启 (127.0.0.1:{})", MIXED_PORT) } else { "KDE 系统代理已关闭".to_string() }
            } else {
                "✗ 修改 KDE 系统代理失败（kwriteconfig6 不可用？）".to_string()
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
        let app = self.state.lock().unwrap();
        app.save_profiles();
        let keep = app.profiles.keep_core_on_exit || app.profiles.autostart || systemd_active();
        if !keep {
            if app.is_system_proxy_on {
                set_kde_proxy(false);
            }
            drop(app);
            self.core.lock().unwrap().stop();
        }
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
            fonts.font_data.insert("cjk_font".to_owned(), egui::FontData::from_owned(data));
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
