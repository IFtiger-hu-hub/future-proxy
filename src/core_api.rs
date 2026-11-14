use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::config_patch::{
    format_proto_name, looks_like_clash_config, match_flag, API_HOST, DEFAULT_SECRET, MIXED_PORT,
};
use crate::platform::systemd_active;
use crate::state::{LiveConnItem, ProxyNode};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn api(path: &str) -> String {
    format!("http://{}{}", API_HOST, path)
}

pub fn auth() -> String {
    format!("Bearer {}", DEFAULT_SECRET)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryTelemetry {
    /// GUI 私有堆内存 (RssAnon, KB)
    pub gui_kb: u64,
    /// GUI 含共享显存/字体的总驻留集 (VmRSS, KB)
    pub gui_rss_kb: u64,
    /// Mihomo 内核实际占用内存 (Inuse / RssAnon, KB)
    pub core_kb: u64,
    /// Mihomo 内核总驻留集 (VmRSS, KB)
    pub core_rss_kb: u64,
    /// 宿主系统已用物理内存 (GB)
    pub sys_used_gb: f32,
    /// 宿主系统总物理内存 (GB)
    pub sys_total_gb: f32,
}

#[cfg(unix)]
fn parse_proc_status_kb(pid: &str) -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string(format!("/proc/{}/status", pid)) else {
        return (0, 0);
    };
    let mut rss_kb = 0u64;
    let mut anon_kb = 0u64;
    for line in content.lines() {
        if line.starts_with("VmRSS:") {
            rss_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("RssAnon:") {
            anon_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
    }
    if anon_kb == 0 {
        anon_kb = rss_kb;
    }
    (anon_kb, rss_kb)
}

#[cfg(unix)]
fn read_sys_mem_gb() -> (f32, f32) {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return (0.0, 0.0);
    };
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = line
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
        }
        if total_kb > 0 && avail_kb > 0 {
            break;
        }
    }
    if total_kb == 0 {
        return (0.0, 0.0);
    }
    let used_kb = total_kb.saturating_sub(avail_kb);
    let total_gb = total_kb as f32 / (1024.0 * 1024.0);
    let used_gb = used_kb as f32 / (1024.0 * 1024.0);
    (used_gb, total_gb)
}

#[cfg(unix)]
fn read_core_mem_kb(child_pid: Option<u32>) -> (u64, u64) {
    if let Some(p) = child_pid {
        let (anon, rss) = parse_proc_status_kb(&p.to_string());
        if rss > 0 {
            return (anon, rss);
        }
    }
    if let Ok(out) = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "--property=MainPID",
            "--value",
            "future-proxy.service",
        ])
        .output()
    {
        let pid_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !pid_str.is_empty() && pid_str != "0" {
            let (anon, rss) = parse_proc_status_kb(&pid_str);
            if rss > 0 {
                return (anon, rss);
            }
        }
    }
    if let Ok(out) = Command::new("pgrep").args(["-n", "future-proxy-co"]).output() {
        let pid_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !pid_str.is_empty() {
            let (anon, rss) = parse_proc_status_kb(&pid_str);
            if rss > 0 {
                return (anon, rss);
            }
        }
    }
    (0, 0)
}

pub fn collect_memory_telemetry(core_pid: Option<u32>) -> MemoryTelemetry {
    #[cfg(unix)]
    {
        let (gui_kb, gui_rss_kb) = parse_proc_status_kb("self");
        let (core_kb, core_rss_kb) = read_core_mem_kb(core_pid);
        let (sys_used_gb, sys_total_gb) = read_sys_mem_gb();
        MemoryTelemetry {
            gui_kb,
            gui_rss_kb,
            core_kb,
            core_rss_kb,
            sys_used_gb,
            sys_total_gb,
        }
    }
    #[cfg(windows)]
    {
        let (gui_kb, gui_rss_kb) = crate::platform::win_os::read_process_mem_kb(None);
        let (core_kb, core_rss_kb) = read_core_rss_kb_win(core_pid);
        let (sys_used_gb, sys_total_gb) = crate::platform::win_os::read_sys_mem_gb();
        MemoryTelemetry {
            gui_kb,
            gui_rss_kb,
            core_kb,
            core_rss_kb,
            sys_used_gb,
            sys_total_gb,
        }
    }
}

#[cfg(windows)]
fn read_core_rss_kb_win(child_pid: Option<u32>) -> (u64, u64) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CACHED_CORE_PID: AtomicU32 = AtomicU32::new(0);

    if let Some(pid) = child_pid {
        let (priv_kb, rss_kb) = crate::platform::win_os::read_process_mem_kb(Some(pid));
        if rss_kb > 0 {
            CACHED_CORE_PID.store(pid, Ordering::Relaxed);
            return (priv_kb, rss_kb);
        }
    }
    let cached = CACHED_CORE_PID.load(Ordering::Relaxed);
    if cached > 0 {
        let (priv_kb, rss_kb) = crate::platform::win_os::read_process_mem_kb(Some(cached));
        if rss_kb > 0 {
            return (priv_kb, rss_kb);
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
                        let (priv_kb, rss_kb) =
                            crate::platform::win_os::read_process_mem_kb(Some(pid));
                        if rss_kb > 0 {
                            CACHED_CORE_PID.store(pid, Ordering::Relaxed);
                            return (priv_kb, rss_kb);
                        }
                    }
                }
            }
        }
    }
    (0, 0)
}

pub fn is_http_url(source: &str) -> bool {
    let s = source.trim().trim_matches(|c| c == '"' || c == '\'');
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

pub fn normalize_local_path(source: &str) -> PathBuf {
    let mut s = source.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    if let Some(rest) = s.strip_prefix("file:///") {
        #[cfg(windows)]
        {
            s = rest.replace('/', "\\");
        }
        #[cfg(unix)]
        {
            s = format!("/{}", rest);
        }
    } else if let Some(rest) = s.strip_prefix("file://") {
        s = rest.to_string();
    } else if s.starts_with("~/") || s.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            s = home.join(&s[2..]).to_string_lossy().to_string();
        }
    }
    PathBuf::from(s)
}

fn build_sub_agent(
    #[allow(unused_variables)] accept_invalid_certs: bool,
    use_local_proxy: bool,
) -> Result<ureq::Agent, String> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(20))
        .timeout(Duration::from_secs(25));
    #[cfg(windows)]
    {
        let tls = if accept_invalid_certs {
            native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
                .build()
        } else {
            native_tls::TlsConnector::new()
        }
        .map_err(|e| format!("初始化系统 TLS 失败: {}", e))?;
        builder = builder.tls_connector(std::sync::Arc::new(tls));
    }
    if use_local_proxy {
        if let Ok(proxy) = ureq::Proxy::new(&format!("http://127.0.0.1:{}", MIXED_PORT)) {
            builder = builder.proxy(proxy);
        }
    }
    Ok(builder.build())
}

fn do_http_get_sub(agent: &ureq::Agent, url: &str) -> Result<(String, String), String> {
    let resp = agent
        .get(url)
        .set(
            "User-Agent",
            "FlClash/v0.8.86 clash-verge/v1.7.7 clash.meta mihomo/1.18.9",
        )
        .call()
        .map_err(|e| format!("{}", e))?;
    let userinfo = resp
        .header("subscription-userinfo")
        .or_else(|| resp.header("Subscription-Userinfo"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let body = resp
        .into_string()
        .map_err(|e| format!("读取响应内容失败: {}", e))?;
    Ok((body.trim_start_matches('\u{feff}').to_string(), userinfo))
}

/// 获取订阅配置内容：同时支持 HTTP/HTTPS 远程订阅链接与本地 `.yaml` / `.yml` 文件路径
pub fn fetch_subscription_content(
    source: &str,
    fallback_local: Option<&Path>,
) -> Result<(String, String), String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("✗ 订阅链接或 YAML 文件路径不能为空".to_string());
    }

    if !is_http_url(trimmed) {
        let local_path = normalize_local_path(trimmed);
        let bytes = std::fs::read(&local_path)
            .or_else(|e| {
                if let Some(fb) = fallback_local {
                    if fb.exists() {
                        return std::fs::read(fb);
                    }
                }
                Err(e)
            })
            .map_err(|e| format!("✗ 读取本地 YAML 文件失败 ({}): {}", local_path.display(), e))?;
        let body = String::from_utf8_lossy(&bytes)
            .trim_start_matches('\u{feff}')
            .to_string();
        if looks_like_clash_config(&body) {
            return Ok((body, String::new()));
        }
        return Err(
            "✗ 所选文件不是有效的 Clash / Mihomo YAML 配置（需包含 proxies 或 proxy-providers）"
                .to_string(),
        );
    }

    let url = trimmed.trim_matches(|c| c == '"' || c == '\'');
    let try_fetch = |target_url: &str| -> Result<(String, String), String> {
        let agent = build_sub_agent(false, false)?;
        match do_http_get_sub(&agent, target_url) {
            Ok(ok) => Ok(ok),
            Err(first_err) => {
                #[cfg(windows)]
                if let Ok(agent_insecure) = build_sub_agent(true, false) {
                    if let Ok(ok) = do_http_get_sub(&agent_insecure, target_url) {
                        return Ok(ok);
                    }
                }
                if let Ok(agent_proxy) = build_sub_agent(true, true) {
                    if let Ok(ok) = do_http_get_sub(&agent_proxy, target_url) {
                        return Ok(ok);
                    }
                }
                Err(format!("✗ 订阅下载失败: {}", first_err))
            }
        }
    };

    let (body, userinfo) = try_fetch(url)?;
    if looks_like_clash_config(&body) {
        return Ok((body, userinfo));
    }

    // 部分机场链接未带 flag=clash 时默认下发 Base64 节点列表，自动追加 flag=clash 重试一次
    if !url.to_ascii_lowercase().contains("flag=") {
        let sep = if url.contains('?') { "&" } else { "?" };
        let clash_url = format!("{}{}flag=clash", url, sep);
        if let Ok((body2, userinfo2)) = try_fetch(&clash_url) {
            if looks_like_clash_config(&body2) {
                return Ok((
                    body2,
                    if userinfo2.is_empty() {
                        userinfo
                    } else {
                        userinfo2
                    },
                ));
            }
        }
    }

    Err("✗ 订阅内容不是 Clash/Mihomo YAML 格式，请检查订阅链接或直接导入本地 .yaml 文件".to_string())
}

pub fn reload_core(config: &Path) -> Result<(), String> {
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
pub struct CoreManager {
    pub child: Option<Child>,
    pub last_spawn: Option<Instant>,
    pub failures: u32,
}

impl CoreManager {
    pub fn core_bin() -> PathBuf {
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
    pub fn ensure_running(&mut self, config_dir: &Path, alive: bool, prefer_systemd: bool) -> String {
        if crate::platform::is_system_shutting_down() {
            return String::new();
        }
        if alive {
            self.failures = 0;
            return String::new();
        }
        // 自启服务已启用时，优先尝试唤醒后台服务；若服务未能启动则平滑回退到直接拉起子进程
        #[allow(unused_variables)]
        if prefer_systemd {
            #[cfg(unix)]
            {
                if systemd_active() {
                    return "systemd 已托管内核，但控制接口暂无响应".to_string();
                }
                let _ = Command::new("systemctl")
                    .args(["--user", "reset-failed", "future-proxy.service"])
                    .status();
                let _ = Command::new("systemctl")
                    .args(["--user", "start", "future-proxy.service"])
                    .status();
                if systemd_active() {
                    self.last_spawn = Some(Instant::now());
                    return String::new();
                }
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
                    if self
                        .last_spawn
                        .map(|t| t.elapsed() < Duration::from_secs(5))
                        .unwrap_or(false)
                    {
                        return String::new();
                    }
                    return "内核进程在运行但控制接口无响应，可能 9090 端口被占用".to_string();
                }
                Err(_) => self.child = None,
            }
        }
        // 退避：1 次失败后每 10 秒重试，3 次后停止自动重试
        if self.failures >= 3 {
            return format!(
                "内核启动失败，详见 {}",
                config_dir.join("core.log").display()
            );
        }
        if let Some(t) = self.last_spawn {
            if t.elapsed() < Duration::from_secs(10) && self.failures > 0 {
                return format!(
                    "内核启动失败，正在重试，详见 {}",
                    config_dir.join("core.log").display()
                );
            }
        }
        let log = std::fs::File::create(config_dir.join("core.log")).ok();
        let bin = Self::core_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("-d")
            .arg(config_dir)
            .current_dir(config_dir)
            .stdin(Stdio::null());
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

    pub fn reset(&mut self) {
        self.failures = 0;
        self.last_spawn = None;
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    pub fn stop(&mut self) {
        let shutting_down = crate::platform::is_system_shutting_down();
        if !shutting_down && systemd_active() {
            self.child = None;
            return;
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            if !shutting_down {
                let _ = c.wait();
            }
        }
        #[cfg(windows)]
        if !shutting_down {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "future-proxy-core.exe", "/T"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

pub fn enc_path(name: &str) -> String {
    let enc: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    enc.replace('+', "%20")
}

/// 测试单个节点延迟，失败/超时返回 0
pub fn test_delay(name: &str) -> i32 {
    test_delay_with_url(name, "https://www.gstatic.com/generate_204")
}

/// 指定测试目标 URL 探测单个节点/策略组的真实 HTTP 延迟
pub fn test_delay_with_url(name: &str, target_url: &str) -> i32 {
    let url = api(&format!(
        "/proxies/{}/delay?timeout=3500&url={}",
        enc_path(name),
        target_url
    ));
    ureq::get(&url)
        .set("Authorization", &auth())
        .timeout(Duration::from_millis(4000))
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|j| j.get("delay").and_then(|d| d.as_i64()))
        .unwrap_or(0) as i32
}

/// 并发（16 个一批）测试一组节点
pub fn test_delays(names: &[String]) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for chunk in names.chunks(16) {
        let handles: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|n| {
                std::thread::spawn(move || {
                    let d = test_delay(&n);
                    (n, d)
                })
            })
            .collect();
        out.extend(handles.into_iter().filter_map(|h| h.join().ok()));
    }
    out
}

pub fn put_select(group: &str, name: &str) -> Result<(), String> {
    ureq::put(&api(&format!("/proxies/{}", enc_path(group))))
        .set("Authorization", &auth())
        .timeout(Duration::from_secs(3))
        .send_json(serde_json::json!({ "name": name }))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 切换节点或模式后主动清理旧连接，使新节点立即生效（FlClash closeConnections 机制）
pub fn close_all_connections() {
    let _ = ureq::delete(&api("/connections"))
        .set("Authorization", &auth())
        .timeout(Duration::from_millis(1200))
        .call();
}

/// 热更新内核出站模式（rule / global / direct）
pub fn patch_core_mode(mode: &str) -> Result<(), String> {
    ureq::request("PATCH", &api("/configs"))
        .set("Authorization", &auth())
        .timeout(Duration::from_secs(3))
        .send_json(serde_json::json!({ "mode": mode }))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 热更新内核局域网共享 (allow-lan)
pub fn patch_allow_lan(allow: bool) -> Result<(), String> {
    ureq::request("PATCH", &api("/configs"))
        .set("Authorization", &auth())
        .timeout(Duration::from_secs(3))
        .send_json(serde_json::json!({ "allow-lan": allow }))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub struct ProxiesSnapshot {
    pub groups: Vec<String>,
    pub group_summaries: Vec<(String, String)>,
    pub group_name: String,
    pub current: String,
    pub resolved_node: String,
    pub resolved_delay: i32,
    pub nodes: Vec<ProxyNode>,
}

pub fn fetch_proxies(preferred_group: &str, user_picked: bool) -> Option<ProxiesSnapshot> {
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
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let is_selector = |name: &str| {
        proxies
            .get(name)
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "Selector")
            .unwrap_or(false)
    };
    let is_summary_group = |name: &str| {
        proxies
            .get(name)
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| matches!(t, "Selector" | "URLTest" | "Fallback"))
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
        return Some(ProxiesSnapshot {
            groups,
            group_summaries: Vec::new(),
            group_name: String::new(),
            current: String::new(),
            resolved_node: String::new(),
            resolved_delay: -1,
            nodes: Vec::new(),
        });
    }

    // 收集仪表盘「策略组」卡片的展示列表（包含 Selector 及关键 URLTest/Fallback 组）
    let mut summary_names: Vec<String> = groups.clone();
    for name in &order {
        if name != "GLOBAL" && is_summary_group(name) && !summary_names.contains(name) {
            summary_names.push(name.clone());
        }
    }
    let group_summaries: Vec<(String, String)> = summary_names
        .into_iter()
        .map(|g| {
            let now = proxies
                .get(&g)
                .and_then(|p| p.get("now"))
                .and_then(|n| n.as_str())
                .unwrap_or("DIRECT")
                .to_string();
            (g, now)
        })
        .collect();

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
    let current = group
        .get("now")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    // 递归追溯当前选中策略组背后的真实落地节点及延迟（例如 `节点选择 -> 自动选择 -> Singapore 03`）
    let mut resolved_node = current.clone();
    for _ in 0..4 {
        if let Some(next_now) = proxies
            .get(&resolved_node)
            .and_then(|p| p.get("now"))
            .and_then(|n| n.as_str())
        {
            if !next_now.is_empty() && next_now != resolved_node {
                resolved_node = next_now.to_string();
                continue;
            }
        }
        break;
    }
    let resolved_delay = proxies
        .get(&resolved_node)
        .or_else(|| proxies.get(&current))
        .and_then(|p| p.get("history"))
        .and_then(|h| h.as_array())
        .and_then(|h| h.last())
        .and_then(|l| l.get("delay"))
        .and_then(|d| d.as_i64())
        .map(|d| d as i32)
        .unwrap_or(-1);

    let mut nodes = Vec::new();
    if let Some(arr) = group.get("all").and_then(|a| a.as_array()) {
        for n_name in arr.iter().filter_map(|v| v.as_str()) {
            if n_name == "REJECT" || n_name == "GLOBAL" || n_name.starts_with("⚡ ") {
                continue;
            }
            let p_obj = proxies.get(n_name);
            let proto = p_obj
                .and_then(|p| p.get("type"))
                .and_then(|t| t.as_str())
                .map(format_proto_name)
                .unwrap_or_else(|| "Proxy".to_string());
            let delay = p_obj
                .and_then(|p| p.get("history"))
                .and_then(|h| h.as_array())
                .and_then(|h| h.last())
                .and_then(|l| l.get("delay"))
                .and_then(|d| d.as_i64())
                .map(|d| d as i32)
                .unwrap_or(-1);
            nodes.push(ProxyNode {
                name: n_name.to_string(),
                proto,
                flag: match_flag(n_name).to_string(),
                delay,
                is_selected: n_name == current,
            });
        }
    }
    Some(ProxiesSnapshot {
        groups,
        group_summaries,
        group_name,
        current,
        resolved_node,
        resolved_delay,
        nodes,
    })
}

pub fn fetch_connections() -> Option<(u64, u64, usize, Vec<LiveConnItem>)> {
    let json: serde_json::Value = ureq::get(&api("/connections"))
        .set("Authorization", &auth())
        .timeout(Duration::from_millis(1200))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let tot_up = json
        .get("uploadTotal")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tot_dn = json
        .get("downloadTotal")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let arr = json.get("connections").and_then(|v| v.as_array())?;
    let total_count = arr.len();
    let mut items = Vec::with_capacity(total_count.min(16));
    for c in arr.iter().take(16) {
        let meta = c.get("metadata");
        let proc_name = meta
            .and_then(|m| m.get("process"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                meta.and_then(|m| m.get("processPath"))
                    .and_then(|v| v.as_str())
                    .and_then(|p| p.rsplit(['/', '\\']).next())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                meta.and_then(|m| m.get("type"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("HTTP/TCP")
            .to_string();
        let host = meta
            .and_then(|m| m.get("host"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| meta.and_then(|m| m.get("destinationIP")).and_then(|v| v.as_str()))
            .unwrap_or("unknown");
        let port = meta
            .and_then(|m| m.get("destinationPort"))
            .and_then(|v| v.as_str())
            .unwrap_or("443");
        let target = format!("{}:{}", host, port);
        let exit = c
            .get("chains")
            .and_then(|ch| ch.as_array())
            .and_then(|ch| ch.first())
            .and_then(|v| v.as_str())
            .unwrap_or("DIRECT")
            .to_string();
        let dl = c.get("download").and_then(|v| v.as_u64()).unwrap_or(0);
        let ul = c.get("upload").and_then(|v| v.as_u64()).unwrap_or(0);
        items.push(LiveConnItem {
            source: proc_name,
            target,
            exit,
            download: dl,
            upload: ul,
        });
    }
    Some((tot_up, tot_dn, total_count, items))
}

