#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

mod config_patch;
mod core_api;
mod platform;
mod state;
mod theme;
mod ui_dashboard;
mod ui_proxies;
mod ui_tabs;

use crate::config_patch::{
    chrono_like_now, detect_intranet_ip, extract_yaml_traffic_hints, is_info_node,
    parse_subscription_userinfo, region_match, MIXED_PORT,
};
use crate::core_api::{
    api, auth, close_all_connections, collect_memory_telemetry, fetch_connections, fetch_proxies,
    fetch_subscription_content, is_http_url, patch_allow_lan, patch_core_mode, put_select,
    reload_core, test_delay, test_delay_with_url, test_delays, CoreManager,
};
use crate::platform::{
    build_brand_rgba_icon, check_kde_proxy_enabled, release_instance, request_repaint_slot,
    set_autostart, set_kde_proxy, spawn_ipc_listener, spawn_system_tray, systemd_active,
    take_single_instance, trim_process_memory, IpcListener, TrayFlags, TrayGuard,
};
#[cfg(unix)]
use crate::state::ProfilesMeta;
use crate::state::{AppState, NavTab, Subscription};
use crate::theme::*;

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
    _wake: Arc<Condvar>,
    _wake_mutex: Arc<Mutex<()>>,
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
                let mut last_conns = Instant::now() - Duration::from_secs(10);
                let mut initial_reloaded = false;
                loop {
                    let alive = ureq::get(&api("/version"))
                        .set("Authorization", &auth())
                        .timeout(Duration::from_millis(400))
                        .call()
                        .is_ok();
                    let config_dir = state.lock().unwrap().config_dir.clone();
                    let prefer_systemd = state.lock().unwrap().profiles.autostart;
                    let err =
                        core.lock()
                            .unwrap()
                            .ensure_running(&config_dir, alive, prefer_systemd);
                    if alive && !initial_reloaded {
                        initial_reloaded = true;
                        let _ = reload_core(&config_dir.join("config.yaml"));
                    }

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

                    let conn_snap = if alive && last_conns.elapsed() > Duration::from_secs(2) {
                        last_conns = Instant::now();
                        fetch_connections()
                    } else {
                        None
                    };

                    let mem = if last_mem.elapsed() > Duration::from_secs(3) {
                        last_mem = Instant::now();
                        let core_pid = core.lock().unwrap().pid();
                        Some(collect_memory_telemetry(core_pid))
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
                        if let Some(mut s) = snapshot {
                            app.groups = s.groups;
                            app.group_summaries = s.group_summaries;
                            app.group_name = s.group_name;
                            app.current_selected = s.current;
                            app.resolved_node = s.resolved_node;
                            if s.resolved_delay > 0 || app.resolved_delay < 0 {
                                app.resolved_delay = s.resolved_delay;
                            }
                            if !app.is_testing_delay {
                                // 保留本地已测出的延迟数据，防止被后台轮询重置闪烁
                                let old_delays: HashMap<String, i32> = app
                                    .nodes
                                    .iter()
                                    .filter(|n| n.delay >= 0)
                                    .map(|n| (n.name.clone(), n.delay))
                                    .collect();
                                for n in &mut s.nodes {
                                    if n.delay < 0 {
                                        if let Some(&old_d) = old_delays.get(&n.name) {
                                            n.delay = old_d;
                                        }
                                    }
                                }
                                app.nodes = s.nodes;
                            }
                            changed = true;
                        }
                        if let Some((tot_up, tot_dn, cnt, items)) = conn_snap {
                            app.total_upload = tot_up;
                            app.total_download = tot_dn;
                            app.active_connections = cnt;
                            app.live_connections = items;
                            changed = true;
                        }
                        if let Some(m) = mem {
                            app.mem_gui_kb = m.gui_kb;
                            app.mem_gui_rss_kb = m.gui_rss_kb;
                            app.mem_core_kb = m.core_kb;
                            app.mem_core_rss_kb = m.core_rss_kb;
                            app.mem_sys_used_gb = m.sys_used_gb;
                            app.mem_sys_total_gb = m.sys_total_gb;
                            if app.intranet_ip.starts_with("198.18.") || app.intranet_ip == "127.0.0.1" {
                                app.intranet_ip = detect_intranet_ip();
                            }
                            let total_kb = m.gui_kb.saturating_add(m.core_kb);
                            if app.mem_history.len() >= 24 {
                                app.mem_history.pop_front();
                            }
                            app.mem_history.push_back(total_kb);
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
                            app.upload_rate = up;
                            app.download_rate = down;
                            if up > app.peak_upload {
                                app.peak_upload = up;
                            }
                            if down > app.peak_download {
                                app.peak_download = down;
                            }
                            if app.upload_history.len() >= 28 {
                                app.upload_history.pop_front();
                            }
                            app.upload_history.push_back(up);
                            if app.download_history.len() >= 28 {
                                app.download_history.pop_front();
                            }
                            app.download_history.push_back(down);
                            drop(app);
                            request_repaint_slot(&egui_ctx_slot);
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
                    let (region, group, names, current, kick, alive, close_conn) = {
                        let app = state.lock().unwrap();
                        (
                            app.profiles.auto_region.clone(),
                            app.group_name.clone(),
                            app.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>(),
                            app.current_selected.clone(),
                            app.auto_kick,
                            app.is_core_alive,
                            app.profiles.close_connections,
                        )
                    };
                    let Some(region) = region else { continue };
                    if !alive
                        || names.is_empty()
                        || (!kick && last.elapsed() < Duration::from_secs(30))
                    {
                        continue;
                    }
                    if current.starts_with("⚡") {
                        continue;
                    }
                    last = Instant::now();
                    state.lock().unwrap().auto_kick = false;

                    let candidates: Vec<String> = names
                        .into_iter()
                        .filter(|n| !is_info_node(n) && region_match(&region, n))
                        .collect();
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
                            if res.is_ok() && close_conn {
                                close_all_connections();
                            }
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

        // 启动后后台静默拉取各订阅的 Subscription-Userinfo 流量配额响应头（不阻塞界面）
        {
            let state = Arc::clone(&state);
            let egui_ctx_slot = Arc::clone(&egui_ctx_slot);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(800));
                let pending: Vec<(String, String)> = {
                    let app = state.lock().unwrap();
                    app.profiles
                        .subscriptions
                        .iter()
                        .filter(|s| s.total == 0 && !s.url.trim().is_empty())
                        .map(|s| (s.id.clone(), s.url.clone()))
                        .collect()
                };
                for (sub_id, url) in pending {
                    if let Ok(resp) = ureq::get(&url)
                        .set("User-Agent", "FlClash/v0.8.86 clash-verge clash.meta mihomo")
                        .timeout(Duration::from_secs(8))
                        .call()
                    {
                        let hdr = resp
                            .header("subscription-userinfo")
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let (up, down, tot, exp) = parse_subscription_userinfo(&hdr);
                        let body_hints = resp
                            .into_string()
                            .ok()
                            .map(|b| extract_yaml_traffic_hints(&b))
                            .unwrap_or_default();
                        if tot > 0 || !body_hints.0.is_empty() || !body_hints.1.is_empty() {
                            let mut app = state.lock().unwrap();
                            if let Some(sub) = app
                                .profiles
                                .subscriptions
                                .iter_mut()
                                .find(|s| s.id == sub_id)
                            {
                                if tot > 0 {
                                    sub.upload = up;
                                    sub.download = down;
                                    sub.total = tot;
                                    sub.expire = exp;
                                }
                                if !body_hints.0.is_empty() {
                                    sub.remaining_hint = body_hints.0;
                                }
                                if !body_hints.1.is_empty() {
                                    sub.expire_hint = body_hints.1;
                                }
                            }
                            app.save_profiles();
                            drop(app);
                            request_repaint_slot(&egui_ctx_slot);
                        }
                    }
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
        let mut app = self.state.lock().unwrap();
        app.status_msg = msg;
        app.last_action_done_at = Some(Instant::now());
        drop(app);
        self.ctx.request_repaint();
    }

    fn select_node(&self, node_name: String) {
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let (group, close_conn) = {
                let app = state.lock().unwrap();
                (app.group_name.clone(), app.profiles.close_connections)
            };
            let res = put_select(&group, &node_name);
            if res.is_ok() && close_conn {
                close_all_connections();
            }

            let mut app = state.lock().unwrap();
            match res {
                Ok(_) => {
                    app.current_selected = node_name.clone();
                    for n in &mut app.nodes {
                        n.is_selected = n.name == node_name;
                    }
                    app.status_msg = format!("✓ 已切换到 {}", format_cupertino_title(&node_name));
                }
                Err(e) => app.status_msg = format!("✗ 切换节点失败: {}", e),
            }
            drop(app);
            ctx.request_repaint();
        });
    }

    fn switch_mode(&self, mode: String) {
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let close_conn = {
                let mut app = state.lock().unwrap();
                app.profiles.mode = mode.clone();
                app.save_profiles();
                let _ = app.write_active_config();
                app.profiles.close_connections
            };
            let res = patch_core_mode(&mode);
            if res.is_ok() && close_conn {
                close_all_connections();
            }
            let mode_cn = match mode.as_str() {
                "global" => "全局模式 (Global)",
                "direct" => "直连模式 (Direct)",
                _ => "规则分流 (Rule)",
            };
            let mut app = state.lock().unwrap();
            app.last_action_done_at = Some(Instant::now());
            app.status_msg = match res {
                Ok(()) => format!("✓ 已切换为 {}", mode_cn),
                Err(e) => format!("✗ 模式切换失败: {}", e),
            };
            drop(app);
            ctx.request_repaint();
        });
    }

    fn ping_all(&self) {
        let names = {
            let mut app = self.state.lock().unwrap();
            if app.is_testing_delay {
                return;
            }
            let total = app.nodes.len();
            app.is_testing_delay = true;
            app.ping_done = 0;
            app.ping_total = total;
            app.last_ping_done_at = None;
            app.status_msg = format!("⚡ 正在测速 (0 / {})…", total);
            for n in &mut app.nodes {
                n.delay = -1;
            }
            app.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>()
        };
        self.ctx.request_repaint();
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let total = names.len();
            for chunk in names.chunks(8) {
                let results = test_delays(chunk);
                let mut app = state.lock().unwrap();
                for n in &mut app.nodes {
                    if let Some(d) = results.get(&n.name) {
                        n.delay = *d;
                    }
                }
                app.ping_done = (app.ping_done + chunk.len()).min(total);
                app.status_msg = format!("⚡ 正在测速 ({} / {})…", app.ping_done, total);
                drop(app);
                ctx.request_repaint();
            }
            let elapsed = started.elapsed();
            if elapsed < Duration::from_millis(320) {
                std::thread::sleep(Duration::from_millis(320) - elapsed);
            }
            let mut app = state.lock().unwrap();
            app.is_testing_delay = false;
            app.last_ping_done_at = Some(Instant::now());
            if let Some(best_d) = app
                .nodes
                .iter()
                .find(|n| n.name == app.resolved_node || n.name == app.current_selected)
                .map(|n| n.delay)
                .filter(|&d| d > 0)
            {
                app.resolved_delay = best_d;
            }
            let ok_cnt = app.nodes.iter().filter(|n| n.delay > 0).count();
            let min_d = app
                .nodes
                .iter()
                .map(|n| n.delay)
                .filter(|&d| d > 0)
                .min();
            app.status_msg = match min_d {
                Some(fastest) => format!(
                    "✓ 测速完成：{} / {} 节点可用 · 最快 {} ms",
                    ok_cnt,
                    app.nodes.len(),
                    fastest
                ),
                None => format!("✗ 测速结束：0 / {} 节点可用（连接超时）", app.nodes.len()),
            };
            drop(app);
            ctx.request_repaint();
        });
    }

    /// 实时检测当前激活节点的连通质量与 Google / Cloudflare 延迟
    fn probe_active_node_status(&self) {
        let target = {
            let mut app = self.state.lock().unwrap();
            if app.is_probing_status {
                return;
            }
            app.is_probing_status = true;
            app.last_ping_done_at = None;
            let t = if !app.resolved_node.is_empty() {
                app.resolved_node.clone()
            } else if !app.current_selected.is_empty() {
                app.current_selected.clone()
            } else {
                "GLOBAL".to_string()
            };
            app.status_msg = format!("⚡ 正在检测节点 [{}] 延迟与连通性…", format_cupertino_title(&t));
            t
        };
        self.ctx.request_repaint();
        let state = Arc::clone(&self.state);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let t1 = target.clone();
            let t2 = target.clone();
            let h1 = std::thread::spawn(move || {
                test_delay_with_url(&t1, "https://www.gstatic.com/generate_204")
            });
            let h2 = std::thread::spawn(move || {
                test_delay_with_url(&t2, "http://cp.cloudflare.com/generate_204")
            });
            let g_ms = h1.join().unwrap_or(0);
            let cf_ms = h2.join().unwrap_or(0);
            let elapsed = started.elapsed();
            if elapsed < Duration::from_millis(360) {
                std::thread::sleep(Duration::from_millis(360) - elapsed);
            }
            let best = match (g_ms > 0, cf_ms > 0) {
                (true, true) => g_ms.min(cf_ms),
                (true, false) => g_ms,
                (false, true) => cf_ms,
                (false, false) => 0,
            };
            let mut app = state.lock().unwrap();
            app.is_probing_status = false;
            app.last_ping_done_at = Some(Instant::now());
            app.probe_google_ms = g_ms;
            app.probe_cf_ms = cf_ms;
            app.resolved_delay = best;
            let cur_sel = app.current_selected.clone();
            for n in &mut app.nodes {
                if n.name == target || n.name == cur_sel {
                    n.delay = best;
                }
            }
            app.status_msg = if best > 0 {
                format!(
                    "✓ 测速完成：Google {} ms · Cloudflare {} ms",
                    if g_ms > 0 { g_ms } else { best },
                    if cf_ms > 0 { cf_ms } else { best }
                )
            } else {
                format!(
                    "✗ 测速结束：节点 [{}] 连接超时",
                    format_cupertino_title(&target)
                )
            };
            drop(app);
            ctx.request_repaint();
        });
    }

    /// 一键释放界面与内核闲置内存 (malloc_trim + 清理残留空闲连接)
    fn trim_memory_now(&self) {
        let state = Arc::clone(&self.state);
        let core = Arc::clone(&self.core);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let core_pid = core.lock().unwrap().pid();
            let before = collect_memory_telemetry(core_pid);
            let _ = ureq::post(&api("/configs/geo"))
                .set("Authorization", &auth())
                .timeout(Duration::from_millis(500));
            trim_process_memory();
            std::thread::sleep(Duration::from_millis(120));
            let after = collect_memory_telemetry(core_pid);

            let mut app = state.lock().unwrap();
            app.mem_gui_kb = after.gui_kb;
            app.mem_gui_rss_kb = after.gui_rss_kb;
            app.mem_core_kb = after.core_kb;
            app.mem_core_rss_kb = after.core_rss_kb;
            app.mem_sys_used_gb = after.sys_used_gb;
            app.mem_sys_total_gb = after.sys_total_gb;

            let before_mb = (before.gui_kb + before.core_kb) as f64 / 1024.0;
            let after_mb = (after.gui_kb + after.core_kb) as f64 / 1024.0;
            let freed_mb = (before_mb - after_mb).max(0.0);
            app.last_action_done_at = Some(Instant::now());
            app.status_msg = if freed_mb >= 0.1 {
                format!(
                    "✓ 已释放 {:.1} MB 闲置内存（当前占用 {:.1} MB）",
                    freed_mb, after_mb
                )
            } else {
                format!(
                    "✓ 内存已处于精简状态（界面 {:.1} MB · 内核 {:.1} MB）",
                    after.gui_kb as f64 / 1024.0,
                    after.core_kb as f64 / 1024.0
                )
            };
            drop(app);
            ctx.request_repaint();
        });
    }

    /// 重启 Mihomo 内核并重新加载当前配置
    fn restart_core_now(&self) {
        let state = Arc::clone(&self.state);
        let core = Arc::clone(&self.core);
        let ctx = self.ctx.clone();
        {
            let mut app = state.lock().unwrap();
            if app.is_restarting_core {
                return;
            }
            app.is_restarting_core = true;
            app.last_action_done_at = None;
            app.status_msg = "↻ 正在重启 Mihomo 内核并重载配置…".to_string();
            app.started_at = Instant::now();
        }
        ctx.request_repaint();
        std::thread::spawn(move || {
            let started = Instant::now();
            let (cfg, prefer_sys) = {
                let app = state.lock().unwrap();
                let _ = app.write_active_config();
                (app.active_config(), app.profiles.autostart)
            };
            let cfg_dir = cfg
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            #[cfg(unix)]
            if systemd_active() {
                let _ = Command::new("systemctl")
                    .args(["--user", "restart", "future-proxy.service"])
                    .status();
                std::thread::sleep(Duration::from_millis(450));
            }
            {
                let mut cm = core.lock().unwrap();
                if !systemd_active() {
                    cm.stop();
                }
                cm.reset();
                let _ = cm.ensure_running(&cfg_dir, false, prefer_sys);
            }
            std::thread::sleep(Duration::from_millis(320));
            let _ = reload_core(&cfg);
            close_all_connections();
            trim_process_memory();
            let snap = fetch_proxies("", false);
            let elapsed = started.elapsed();
            if elapsed < Duration::from_millis(480) {
                std::thread::sleep(Duration::from_millis(480) - elapsed);
            }
            let mut app = state.lock().unwrap();
            app.is_restarting_core = false;
            app.last_action_done_at = Some(Instant::now());
            app.intranet_ip = detect_intranet_ip();
            if let Some(s) = snap {
                app.groups = s.groups;
                app.group_summaries = s.group_summaries;
                app.group_name = s.group_name;
                app.current_selected = s.current;
                app.resolved_node = s.resolved_node;
                app.resolved_delay = s.resolved_delay;
                app.nodes = s.nodes;
                app.is_core_alive = true;
            }
            app.status_msg = "✓ Mihomo 内核已重启并重新加载配置".to_string();
            drop(app);
            ctx.request_repaint();
        });
    }

    /// 下载订阅或导入本地 YAML 文件（新导入或更新已有），保存原始内容，若为激活订阅则重载内核并立即刷新节点
    fn download_sub(&self, existing_id: Option<String>, name: String, url: String) {
        let state = Arc::clone(&self.state);
        let core = Arc::clone(&self.core);
        let ctx = self.ctx.clone();
        let is_remote = is_http_url(&url);
        {
            let mut app = state.lock().unwrap();
            app.is_updating_sub = true;
            app.status_msg = if is_remote {
                format!("↻ 正在下载订阅 [{}]...", name)
            } else {
                format!("↻ 正在导入本地 YAML 配置 [{}]...", name)
            };
        }
        ctx.request_repaint();
        std::thread::spawn(move || {
            let fallback_path = {
                let app = state.lock().unwrap();
                existing_id.as_ref().map(|id| app.sub_file(id))
            };
            let result = fetch_subscription_content(&url, fallback_path.as_deref());

            let mut app = state.lock().unwrap();
            app.is_updating_sub = false;
            match result {
                Ok((body, userinfo)) => {
                    let now = if is_remote {
                        chrono_like_now()
                    } else {
                        format!("本地 YAML · {}", chrono_like_now())
                    };
                    let (up, down, tot, exp) = parse_subscription_userinfo(&userinfo);
                    let (rem_hint, exp_hint) = extract_yaml_traffic_hints(&body);
                    let id = existing_id.unwrap_or_else(|| {
                        format!(
                            "sub_{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis()
                        )
                    });
                    if let Err(e) = std::fs::write(app.sub_file(&id), &body) {
                        app.status_msg = format!("✗ 保存订阅失败: {}", e);
                    } else {
                        if let Some(s) =
                            app.profiles.subscriptions.iter_mut().find(|s| s.id == id)
                        {
                            s.updated_at = now;
                            if tot > 0 {
                                s.upload = up;
                                s.download = down;
                                s.total = tot;
                                s.expire = exp;
                            }
                            if !rem_hint.is_empty() {
                                s.remaining_hint = rem_hint;
                            }
                            if !exp_hint.is_empty() {
                                s.expire_hint = exp_hint;
                            }
                        } else {
                            app.profiles.subscriptions.push(Subscription {
                                id: id.clone(),
                                name: name.clone(),
                                url,
                                updated_at: now,
                                upload: up,
                                download: down,
                                total: tot,
                                expire: exp,
                                remaining_hint: rem_hint,
                                expire_hint: exp_hint,
                            });
                            app.profiles.active = id.clone();
                        }
                        app.save_profiles();
                        if app.profiles.active == id {
                            let write_res = app.write_active_config();
                            let cfg = app.active_config();
                            let config_dir = app.config_dir.clone();
                            let prefer_systemd = app.profiles.autostart;
                            drop(app);

                            let msg = match write_res {
                                Ok(()) => {
                                    let mut r = reload_core(&cfg);
                                    if r.is_err() {
                                        let _ = core.lock().unwrap().ensure_running(
                                            &config_dir,
                                            false,
                                            prefer_systemd,
                                        );
                                        std::thread::sleep(Duration::from_millis(450));
                                        r = reload_core(&cfg);
                                    }
                                    match r {
                                        Ok(()) => format!("✓ 订阅 [{}] 已导入并生效", name),
                                        Err(e) => format!("✗ {}", e),
                                    }
                                }
                                Err(e) => e,
                            };
                            let snap = fetch_proxies("", false);
                            let mut app = state.lock().unwrap();
                            app.status_msg = msg;
                            app.group_picked_by_user = false;
                            if let Some(s) = snap {
                                app.groups = s.groups;
                                app.group_summaries = s.group_summaries;
                                app.group_name = s.group_name;
                                app.current_selected = s.current;
                                app.resolved_node = s.resolved_node;
                                app.resolved_delay = s.resolved_delay;
                                app.nodes = s.nodes;
                                app.is_core_alive = true;
                            }
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
            let msg = match res.and_then(|cfg| reload_core(&cfg).map_err(|e| format!("✗ {}", e)))
            {
                Ok(()) => msg_ok.to_string(),
                Err(e) => e,
            };
            let snap = fetch_proxies("", false);
            let mut app = state.lock().unwrap();
            app.last_action_done_at = Some(Instant::now());
            app.status_msg = msg;
            app.group_picked_by_user = false;
            if let Some(s) = snap {
                app.groups = s.groups;
                app.group_summaries = s.group_summaries;
                app.group_name = s.group_name;
                app.current_selected = s.current;
                app.resolved_node = s.resolved_node;
                app.resolved_delay = s.resolved_delay;
                app.nodes = s.nodes;
            }
            drop(app);
            ctx.request_repaint();
        });
    }
}

impl eframe::App for FutureProxyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.tray_flags.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.tray_flags.show.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        let dark_mode = self.state.lock().unwrap().profiles.dark_mode;
        let pal = ThemePalette::new(dark_mode);
        apply_app_theme(ctx, &pal);

        let mut action_switch_sub = None;
        let mut action_update_sub = None;
        let mut action_delete_sub = None;
        let mut action_ping = false;
        let mut action_select_node = None;
        let mut action_import = None;
        let mut toggle_sys_proxy = None;
        let mut toggle_tun = None;
        let mut toggle_allow_lan = None;
        let mut action_retry_core = false;
        let mut action_probe_status = false;
        let mut action_trim_mem = false;
        let mut action_switch_mode = None;

        {
            let mut app = self.state.lock().unwrap();

            // === Left Panel: FlClash Collapsible Navigation Drawer (Responsive Auto-Collapse on < 840px) ===
            let screen_w = ctx.screen_rect().width();
            let auto_narrow = screen_w < 840.0;
            let is_expanded = !app.sidebar_collapsed && !auto_narrow;
            let drawer_t = ctx.animate_bool_with_time(
                egui::Id::new("fl_drawer_collapse"),
                is_expanded,
                0.24,
            );
            let drawer_w = 60.0 + (168.0 - 60.0) * ease_emphasized(drawer_t);

            egui::SidePanel::left("md3_nav_rail")
                .exact_width(drawer_w)
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::none()
                        .fill(pal.bg_sidebar)
                        .inner_margin(egui::Margin {
                            left: 8.0,
                            right: 8.0,
                            top: 12.0,
                            bottom: 12.0,
                        }),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let row_w = ui.available_width();
                        let (hdr_rect, _) =
                            ui.allocate_exact_size(egui::vec2(row_w, 34.0), egui::Sense::hover());
                        let hp = ui.painter().clone();

                        if drawer_w > 110.0 {
                            let logo_r = egui::Rect::from_center_size(
                                egui::pos2(hdr_rect.left() + 18.0, hdr_rect.center().y),
                                egui::vec2(24.0, 24.0),
                            );
                            hp.rect_filled(logo_r, 7.0, pal.primary);
                            draw_cup_icon(&hp, CupIcon::Bolt, logo_r.center(), egui::Color32::WHITE);
                            let txt_alpha = ((drawer_w - 110.0) / 48.0).clamp(0.0, 1.0);
                            let brand_col = egui::Color32::from_rgba_unmultiplied(
                                pal.text_primary.r(),
                                pal.text_primary.g(),
                                pal.text_primary.b(),
                                (txt_alpha * 255.0) as u8,
                            );
                            hp.text(
                                egui::pos2(hdr_rect.left() + 36.0, hdr_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                "Future",
                                cup_font_med(13.5),
                                brand_col,
                            );
                        }

                        let sb_rect = if drawer_w > 110.0 {
                            egui::Rect::from_center_size(
                                egui::pos2(hdr_rect.right() - 16.0, hdr_rect.center().y),
                                egui::vec2(30.0, 30.0),
                            )
                        } else {
                            egui::Rect::from_center_size(hdr_rect.center(), egui::vec2(34.0, 32.0))
                        };
                        let sb_resp =
                            ui.interact(sb_rect, ui.id().with("sb_toggle_btn"), egui::Sense::click());
                        if sb_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            hp.rect_filled(sb_rect, 8.0, pal.surface_hover);
                        }
                        let sb_scale = interact_spring_scale(
                            ctx,
                            sb_resp.id,
                            sb_resp.is_pointer_button_down_on(),
                            sb_resp.hovered(),
                        );
                        let s_r = scale_rect(sb_rect, sb_scale);
                        draw_cup_icon(&hp, CupIcon::SidebarToggle, s_r.center(), pal.text_primary);
                        if sb_resp.clicked() {
                            app.sidebar_collapsed = !app.sidebar_collapsed;
                        }
                    });

                    ui.add_space(12.0);

                    let nav_items = [
                        (NavTab::Dashboard, "仪表盘", CupIcon::Dashboard),
                        (NavTab::Proxies, "代理", CupIcon::Proxy),
                        (NavTab::Profiles, "配置", CupIcon::Doc),
                        (NavTab::Connections, "实时连接", CupIcon::Network),
                        (NavTab::Settings, "工具", CupIcon::Toolbox),
                    ];

                    for (tab, label, icon) in nav_items {
                        let selected = app.nav_tab == tab;
                        let row_w = ui.available_width();
                        let (item_rect, item_resp) =
                            ui.allocate_exact_size(egui::vec2(row_w, 38.0), egui::Sense::click());
                        if item_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }

                        let sel_t =
                            ctx.animate_bool_with_time(item_resp.id.with("nav_sel"), selected, 0.22);
                        let scale = interact_spring_scale(
                            ctx,
                            item_resp.id,
                            item_resp.is_pointer_button_down_on(),
                            item_resp.hovered(),
                        );
                        let draw_r = scale_rect(item_rect, scale);

                        let np = ui.painter();
                        if sel_t > 0.01 {
                            let alpha = (ease_emphasized(sel_t) * 255.0).clamp(0.0, 255.0) as u8;
                            let bg = egui::Color32::from_rgba_unmultiplied(
                                pal.bg_selected.r(),
                                pal.bg_selected.g(),
                                pal.bg_selected.b(),
                                alpha,
                            );
                            np.rect_filled(draw_r, 8.0, bg);

                            let bar_h = 16.0 * spring_overshoot(sel_t);
                            let bar_rect = egui::Rect::from_center_size(
                                egui::pos2(draw_r.left() + 1.8, draw_r.center().y),
                                egui::vec2(3.4, bar_h),
                            );
                            np.rect_filled(bar_rect, 2.0, pal.primary);
                        } else if item_resp.hovered() {
                            np.rect_filled(draw_r, 8.0, pal.surface_hover);
                        }

                        let fg = if selected || item_resp.hovered() {
                            pal.text_primary
                        } else {
                            pal.text_secondary
                        };

                        let icon_pos = egui::pos2(draw_r.left() + 18.0, draw_r.center().y);
                        draw_cup_icon(&np, icon, icon_pos, fg);

                        if drawer_w > 95.0 {
                            let txt_alpha = ((drawer_w - 95.0) / 55.0).clamp(0.0, 1.0);
                            let txt_col = egui::Color32::from_rgba_unmultiplied(
                                fg.r(),
                                fg.g(),
                                fg.b(),
                                (txt_alpha * 255.0) as u8,
                            );
                            np.text(
                                egui::pos2(draw_r.left() + 36.0, draw_r.center().y),
                                egui::Align2::LEFT_CENTER,
                                label,
                                if selected {
                                    cup_font_med(13.0)
                                } else {
                                    cup_font_reg(13.0)
                                },
                                txt_col,
                            );
                        }

                        if item_resp.clicked() {
                            app.nav_tab = tab;
                            app.open_popup = None;
                            app.more_submenu = None;
                        }
                        ui.add_space(4.0);
                    }
                });

            // === Central Panel ===
            let mut hover_tooltip: Option<(egui::Pos2, &'static str)> = None;
            let h_pad = if auto_narrow { 12.0 } else { 18.0 };

            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(pal.bg_window)
                        .inner_margin(egui::Margin {
                            left: h_pad,
                            right: h_pad,
                            top: 16.0,
                            bottom: 12.0,
                        }),
                )
                .show(ctx, |ui| {
                    if !app.core_error.is_empty() {
                        let (err_rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 38.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect(
                            err_rect,
                            10.0,
                            egui::Color32::from_rgba_unmultiplied(
                                pal.red.r(),
                                pal.red.g(),
                                pal.red.b(),
                                20,
                            ),
                            egui::Stroke::new(1.0_f32, pal.red),
                        );
                        ui.painter().text(
                            egui::pos2(err_rect.left() + 14.0, err_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            &app.core_error,
                            cup_font_med(12.0),
                            pal.red,
                        );
                        let btn_r = egui::Rect::from_center_size(
                            egui::pos2(err_rect.right() - 48.0, err_rect.center().y),
                            egui::vec2(76.0, 26.0),
                        );
                        let mut b_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_r));
                        if cupertino_button(
                            &mut b_ui,
                            "重启内核",
                            None,
                            CupBtnStyle::Filled(pal.primary),
                            true,
                            &pal,
                        ) {
                            action_retry_core = true;
                        }
                        ui.add_space(10.0);
                    }

                    match app.nav_tab {
                        NavTab::Proxies => {
                            ui_proxies::render_proxies_tab(
                                ui,
                                ctx,
                                &mut app,
                                &pal,
                                &mut action_ping,
                                &mut action_select_node,
                                &mut hover_tooltip,
                            );
                        }
                        NavTab::Dashboard => {
                            let acts = ui_dashboard::render_dashboard_tab(ui, ctx, &mut app, &pal);
                            if acts.toggle_sys_proxy.is_some() {
                                toggle_sys_proxy = acts.toggle_sys_proxy;
                            }
                            if acts.toggle_tun.is_some() {
                                toggle_tun = acts.toggle_tun;
                            }
                            if acts.toggle_allow_lan.is_some() {
                                toggle_allow_lan = acts.toggle_allow_lan;
                            }
                            if acts.switch_mode.is_some() {
                                action_switch_mode = acts.switch_mode;
                            }
                            if acts.ping_all {
                                action_ping = true;
                            }
                            if acts.test_node_status {
                                action_probe_status = true;
                            }
                            if acts.restart_core {
                                action_retry_core = true;
                            }
                            if acts.trim_memory {
                                action_trim_mem = true;
                            }
                            if acts.update_active_sub {
                                action_update_sub = app
                                    .profiles
                                    .subscriptions
                                    .iter()
                                    .find(|s| s.id == app.profiles.active)
                                    .cloned();
                            }
                        }
                        NavTab::Profiles => {
                            ui_tabs::render_profiles_tab(
                                ui,
                                &mut app,
                                &pal,
                                &mut action_update_sub,
                                &mut action_delete_sub,
                                &mut action_switch_sub,
                                &mut action_import,
                            );
                        }
                        NavTab::Connections => {
                            ui_tabs::render_connections_tab(ui, &mut app, &pal);
                        }
                        NavTab::Settings => {
                            ui_tabs::render_tools_tab(
                                ui,
                                &mut app,
                                &pal,
                                &mut toggle_sys_proxy,
                                &mut toggle_tun,
                                &mut action_trim_mem,
                            );
                        }
                    }
                });

            if let Some((pos, tip_text)) = hover_tooltip {
                egui::Area::new(egui::Id::new("fl_dark_tooltip"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos)
                    .show(ctx, |ui| {
                        let tg = ui.fonts(|f| {
                            f.layout_no_wrap(
                                tip_text.to_string(),
                                cup_font_reg(11.0),
                                egui::Color32::WHITE,
                            )
                        });
                        let tw = tg.size().x + 14.0;
                        let th = 22.0;
                        let r = egui::Rect::from_center_size(pos, egui::vec2(tw, th));
                        ui.painter().rect_filled(
                            r,
                            5.0,
                            egui::Color32::from_rgba_unmultiplied(64, 68, 76, 235),
                        );
                        ui.painter().galley(
                            egui::pos2(
                                r.center().x - tg.size().x * 0.5,
                                r.center().y - tg.size().y * 0.5,
                            ),
                            tg,
                            egui::Color32::WHITE,
                        );
                    });
            }

            ui_proxies::render_more_popover(ctx, &mut app, &pal);
            ui_proxies::render_import_modal(ctx, &mut app, &pal, &mut action_import);
        }

        if let Some(enable) = toggle_sys_proxy {
            set_kde_proxy(enable);
            let on = check_kde_proxy_enabled();
            let mut app = self.state.lock().unwrap();
            app.is_system_proxy_on = on;
            app.profiles.sys_proxy_enabled = on;
            app.save_profiles();
            app.last_action_done_at = Some(Instant::now());
            app.status_msg = if on == enable {
                if on {
                    format!("✓ 系统代理已开启 (127.0.0.1:{})", MIXED_PORT)
                } else {
                    "✓ 系统代理已关闭".to_string()
                }
            } else {
                "✗ 修改系统代理失败".to_string()
            };
        }
        if let Some(on) = toggle_tun {
            self.state.lock().unwrap().profiles.tun = on;
            self.apply_config(if on {
                "✓ TUN 已开启"
            } else {
                "✓ TUN 已关闭"
            });
        }
        if let Some(allow) = toggle_allow_lan {
            {
                let mut app = self.state.lock().unwrap();
                app.profiles.allow_lan = allow;
                app.save_profiles();
            }
            let _ = patch_allow_lan(allow);
            self.set_status(if allow {
                "✓ 局域网共享已开启".to_string()
            } else {
                "✓ 局域网共享已关闭".to_string()
            });
        }
        if let Some(mode) = action_switch_mode {
            self.switch_mode(mode);
        }
        if action_retry_core {
            self.restart_core_now();
        }
        if action_probe_status {
            self.probe_active_node_status();
        }
        if action_trim_mem {
            self.trim_memory_now();
        }
        if let Some(sub_id) = action_switch_sub {
            self.state.lock().unwrap().profiles.active = sub_id;
            self.apply_config("✓ 已切换订阅");
        }
        if let Some(del_id) = action_delete_sub {
            let need_reload = {
                let mut app = self.state.lock().unwrap();
                let _ = std::fs::remove_file(app.sub_file(&del_id));
                app.profiles.subscriptions.retain(|s| s.id != del_id);
                let was_active = app.profiles.active == del_id;
                if was_active {
                    if let Some(first) = app.profiles.subscriptions.first() {
                        app.profiles.active = first.id.clone();
                    }
                }
                app.save_profiles();
                app.status_msg = "✓ 已删除订阅".to_string();
                was_active && !app.profiles.subscriptions.is_empty()
            };
            if need_reload {
                self.apply_config("✓ 已删除原订阅并切换至新订阅");
            }
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
        // 支持直接拖拽本地 .yaml / .yml 文件到窗口任意位置快速导入
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = file.path {
                let path_str = path.to_string_lossy().to_string();
                let name = ui_proxies::infer_sub_name("", &path_str);
                action_import = Some((name, path_str));
                break;
            }
        }

        if let Some((name, url)) = action_import {
            self.download_sub(None, name, url);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        *self.egui_ctx_slot.lock().unwrap() = None;
        self.state.lock().unwrap().save_profiles();
    }
}

fn setup_custom_fonts(ctx: &egui::Context) {
    use std::sync::OnceLock;
    static CACHED_CJK_BYTES: OnceLock<Option<&'static [u8]>> = OnceLock::new();

    let cjk_bytes = CACHED_CJK_BYTES.get_or_init(|| {
        let mut reg_candidates: Vec<PathBuf> = Vec::new();
        #[cfg(windows)]
        {
            if let Some(windir) = std::env::var_os("WINDIR") {
                let fonts_dir = PathBuf::from(windir).join("Fonts");
                reg_candidates.push(fonts_dir.join("msyh.ttf"));
                reg_candidates.push(fonts_dir.join("msyh.ttc"));
                reg_candidates.push(fonts_dir.join("simhei.ttf"));
            }
            reg_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyh.ttf"));
            reg_candidates.push(PathBuf::from("C:\\Windows\\Fonts\\msyh.ttc"));
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
                let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
                return Some(leaked);
            }
        }
        None
    });

    let mut fonts = egui::FontDefinitions::default();
    if let Some(bytes) = *cjk_bytes {
        fonts
            .font_data
            .insert("cjk_reg".to_owned(), egui::FontData::from_static(bytes));
        if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            vec.insert(0, "cjk_reg".to_owned());
        }
        if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            vec.push("cjk_reg".to_owned());
        }
    }

    // 复用同一份 CJK 字体数据作为 Medium 族，避免在 Windows 下重复加载第二个 20MB 的 msyhbd.ttc 字体集导致内存翻倍
    let fallback = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .insert(egui::FontFamily::Name("Medium".into()), fallback);

    ctx.set_fonts(fonts);
}

fn native_options() -> eframe::NativeOptions {
    let (rgba, width, height) = build_brand_rgba_icon();
    let icon = egui::IconData {
        rgba,
        width,
        height,
    };
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060.0, 660.0])
            .with_min_inner_size([620.0, 480.0])
            .with_icon(Arc::new(icon))
            .with_title("Future Proxy"),
        run_and_return: true,
        ..Default::default()
    }
}

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
        let dark = self.state.lock().unwrap().profiles.dark_mode;
        apply_app_theme(&cc.egui_ctx, &ThemePalette::new(dark));
        let ctx = cc.egui_ctx.clone();
        *self.egui_ctx_slot.lock().unwrap() = Some(ctx.clone());
        self.tray_flags.show.store(false, Ordering::SeqCst);
        #[cfg(windows)]
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1200));
            crate::platform::win_os::trim_working_set();
        });
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
    let boot_fallback = boot.clone();
    let res = eframe::run_native(
        "future-proxy",
        native_options(),
        Box::new(move |cc| Ok(Box::new(boot.clone().into_app(cc)))),
    );
    #[cfg(windows)]
    if res.is_err() {
        let mut opts = native_options();
        opts.hardware_acceleration = eframe::HardwareAcceleration::Off;
        return eframe::run_native(
            "future-proxy",
            opts,
            Box::new(move |cc| Ok(Box::new(boot_fallback.clone().into_app(cc)))),
        );
    }
    res
}

fn shutdown_core_if_needed(rt: &AppRuntime) {
    let app = rt.state.lock().unwrap();
    app.save_profiles();
    #[cfg(unix)]
    let keep = app.profiles.keep_core_on_exit || app.profiles.autostart || systemd_active();
    #[cfg(windows)]
    let keep = false;
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
    release_instance(listener_slot);
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
                let keep = {
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
    // 确保开机自启（Windows 默认工作目录为 C:\Windows\System32）时切换回程序所在目录
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::env::set_current_dir(dir);
        }
    }

    #[cfg(windows)]
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Future Proxy 发生运行时错误:\n{info}");
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let _ = std::fs::write(dir.join("future-proxy-error.log"), &msg);
            }
        }
        crate::platform::win_os::show_error_dialog("Future Proxy 启动异常", &msg);
    }));

    let start_in_tray = std::env::args().any(|a| a == "--tray");

    #[cfg(unix)]
    if start_in_tray {
        tray_only_main();
        return Ok(());
    }

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
            if let Err(e) = run_gui(&rt) {
                eprintln!("Future Proxy GUI error: {e}");
                #[cfg(windows)]
                {
                    let msg = format!("无法初始化图形界面窗口 (OpenGL/WGL):\n{e}");
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(dir) = exe.parent() {
                            let _ = std::fs::write(dir.join("future-proxy-error.log"), &msg);
                        }
                    }
                    crate::platform::win_os::show_error_dialog("Future Proxy 图形界面错误", &msg);
                    break;
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
            relaunch_tray_and_exit(&rt.listener_slot);
        }

        #[cfg(windows)]
        {
            // winit 在 Drop 窗口时通过 PostMessageW 投递 DESTROY_MSG_ID 到主线程消息队列，
            // 必须立即 pump 消息队列以触发 window_proc 调用 DestroyWindow(hwnd) 彻底销毁窗口。
            for _ in 0..5 {
                crate::platform::win_os::pump_thread_messages();
                std::thread::sleep(Duration::from_millis(10));
            }
            if !rt.state.lock().unwrap().profiles.keep_core_on_exit {
                break;
            }
            crate::platform::win_os::trim_working_set();
            let guard = rt._wake_mutex.lock().unwrap();
            let mut guard = guard;
            loop {
                crate::platform::win_os::pump_thread_messages();
                if rt.tray_flags.quit.load(Ordering::SeqCst) {
                    break;
                }
                if rt.tray_flags.show.swap(false, Ordering::SeqCst) {
                    break;
                }
                let (g, _) = rt
                    ._wake
                    .wait_timeout(guard, Duration::from_millis(50))
                    .unwrap();
                guard = g;
            }
        }
    }

    release_instance(&rt.listener_slot);
    shutdown_core_if_needed(&rt);
    Ok(())
}
