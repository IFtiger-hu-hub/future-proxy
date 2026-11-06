use crate::config_patch::{
    days_until_expire, format_bytes, format_speed, format_uptime, match_flag,
};
use crate::state::{AppState, NavTab};
use crate::theme::{
    cup_font_med, cup_font_reg, draw_bento_card, draw_cup_icon, draw_flag_icon,
    draw_scalloped_flower_button, draw_sparkline_area, draw_status_wave_decor,
    draw_wave_progress, ellipsize_text, format_cupertino_title, interact_spring_scale,
    md3_switch, scale_rect, split_speed_parts, CupIcon, ThemePalette,
};
use eframe::egui;
use std::collections::VecDeque;

#[derive(Default)]
pub struct DashboardActions {
    pub toggle_sys_proxy: Option<bool>,
    pub toggle_tun: Option<bool>,
    pub toggle_allow_lan: Option<bool>,
    pub switch_mode: Option<String>,
    pub ping_all: bool,
    pub test_node_status: bool,
    pub restart_core: bool,
    pub update_active_sub: bool,
    pub trim_memory: bool,
}

pub fn render_dashboard_tab(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut AppState,
    pal: &ThemePalette,
) -> DashboardActions {
    let mut acts = DashboardActions::default();
    let shadow_pad = 6.0_f32;

    // === Top Header: 与下方 Bento 网格共享左右 6px 边距，保证标题左缘与按钮右缘像素级对齐 ===
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(shadow_pad, 0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let title_g = ui.fonts(|f| {
                    f.layout_no_wrap("仪表盘".to_string(), cup_font_med(19.5), pal.text_primary)
                });
                let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
                ui.painter().galley(t_rect.min, title_g, pal.text_primary);

                // --- 测速 / 重启内核 / 系统操作状态胶囊反馈 ---
                let is_busy_pill =
                    app.is_probing_status || app.is_testing_delay || app.is_restarting_core;
                let show_done_pill = !is_busy_pill
                    && (app
                        .last_ping_done_at
                        .map(|t| t.elapsed().as_secs_f32() < 5.5)
                        .unwrap_or(false)
                        || app
                            .last_action_done_at
                            .map(|t| t.elapsed().as_secs_f32() < 5.0)
                            .unwrap_or(false));
                if is_busy_pill || show_done_pill {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                    ui.add_space(10.0);
                    let is_err = !is_busy_pill && app.status_msg.starts_with('✗');
                    let pill_txt = if app.is_restarting_core {
                        "↻ 正在重启 Mihomo 内核并重载配置…".to_string()
                    } else if app.is_probing_status {
                        "⚡ 正在检测节点延迟与连通状态…".to_string()
                    } else if app.is_testing_delay {
                        format!(
                            "⚡ 正在测速 ({} / {})…",
                            app.ping_done,
                            app.ping_total.max(app.nodes.len()).max(1)
                        )
                    } else {
                        app.status_msg.clone()
                    };
                    let (bg_col, fg_col, stroke_col) = if is_busy_pill {
                        (
                            pal.bg_selected,
                            pal.primary,
                            egui::Color32::from_rgba_unmultiplied(
                                pal.primary.r(),
                                pal.primary.g(),
                                pal.primary.b(),
                                110,
                            ),
                        )
                    } else if is_err {
                        (
                            egui::Color32::from_rgba_unmultiplied(239, 68, 68, 28),
                            pal.red,
                            egui::Color32::from_rgba_unmultiplied(239, 68, 68, 110),
                        )
                    } else {
                        (
                            if pal.dark {
                                egui::Color32::from_rgba_unmultiplied(34, 197, 94, 36)
                            } else {
                                egui::Color32::from_rgb(220, 252, 231)
                            },
                            if pal.dark {
                                egui::Color32::from_rgb(74, 222, 128)
                            } else {
                                egui::Color32::from_rgb(21, 128, 61)
                            },
                            egui::Color32::from_rgba_unmultiplied(34, 197, 94, 110),
                        )
                    };
                    let pg =
                        ui.fonts(|f| f.layout_no_wrap(pill_txt.clone(), cup_font_med(11.5), fg_col));
                    let pw = (pg.size().x + 24.0).clamp(110.0, 360.0);
                    let (p_rect, _) =
                        ui.allocate_exact_size(egui::vec2(pw, 26.0), egui::Sense::hover());
                    let pp = ui.painter();
                    pp.rect(
                        p_rect,
                        13.0,
                        bg_col,
                        egui::Stroke::new(0.9_f32, stroke_col),
                    );
                    pp.text(
                        p_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        pill_txt,
                        cup_font_med(11.5),
                        fg_col,
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let just_restarted = !app.is_restarting_core
                        && app
                            .last_action_done_at
                            .map(|t| t.elapsed().as_secs_f32() < 3.5)
                            .unwrap_or(false)
                        && app.status_msg.contains("内核重启");
                    let (btn_rect, btn_resp) =
                        ui.allocate_exact_size(egui::vec2(104.0, 32.0), egui::Sense::click());
                    if btn_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let b_scale = interact_spring_scale(
                        ctx,
                        btn_resp.id,
                        btn_resp.is_pointer_button_down_on(),
                        btn_resp.hovered(),
                    );
                    let br = scale_rect(btn_rect, b_scale);
                    let bp = ui.painter().clone();
                    let fill = if app.is_restarting_core {
                        pal.bg_selected
                    } else if btn_resp.hovered() {
                        pal.bg_card_hover
                    } else {
                        pal.bg_card
                    };
                    draw_bento_card(&bp, br, 16.0, fill, pal);
                    if app.is_restarting_core || just_restarted {
                        bp.rect_stroke(
                            br,
                            16.0,
                            egui::Stroke::new(
                                1.0_f32,
                                if app.is_restarting_core {
                                    pal.primary
                                } else {
                                    pal.green
                                },
                            ),
                        );
                    }

                    let dot_col = if app.is_restarting_core {
                        pal.orange
                    } else if app.is_core_alive {
                        egui::Color32::from_rgb(34, 197, 94)
                    } else {
                        pal.red
                    };
                    bp.circle_filled(egui::pos2(br.left() + 13.5, br.center().y), 3.5, dot_col);

                    draw_cup_icon(
                        &bp,
                        CupIcon::Refresh,
                        egui::pos2(br.left() + 27.0, br.center().y),
                        if just_restarted {
                            pal.green
                        } else {
                            pal.primary
                        },
                    );
                    let (btn_lbl, lbl_col) = if app.is_restarting_core {
                        ("重启中…", pal.primary)
                    } else if just_restarted {
                        ("已重启 ✓", pal.green)
                    } else {
                        ("重启内核", pal.text_primary)
                    };
                    bp.text(
                        egui::pos2(br.left() + 38.0, br.center().y),
                        egui::Align2::LEFT_CENTER,
                        btn_lbl,
                        cup_font_med(11.5),
                        lbl_col,
                    );

                    if btn_resp.clicked() && !app.is_restarting_core {
                        acts.restart_core = true;
                    }
                    if btn_resp.hovered() {
                        btn_resp.on_hover_text("重启 Mihomo 核心进程并重新加载配置");
                    }
                });
            });
        });

    ui.add_space(8.0);

    // 预留 6px 阴影呼吸内边距，防止 ScrollArea 裁剪掉四周外缘卡片的高斯投影
    let viewport_avail_h = (ui.available_height() - shadow_pad * 2.0).max(502.0);

    egui::ScrollArea::vertical()
        .id_salt("bento_dashboard_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(shadow_pad, shadow_pad))
                .show(ui, |ui| {
                    let avail_w = ui.available_width();
                    let gap = (avail_w * 0.016).clamp(13.0, 20.0).round();
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);

                    // 4 行黄金比例纵向分配 (27.5% : 9.2% : 24.8% : 38.5%)
                    let net_h = (viewport_avail_h - gap * 3.0).max(502.0);
                    let row1_h = (net_h * 0.275).clamp(138.0, 240.0).round();
                    let row2_h = (net_h * 0.092).clamp(46.0, 68.0).round();
                    let row3_h = (net_h * 0.248).clamp(126.0, 214.0).round();
                    let row4_h = (net_h - row1_h - row2_h - row3_h - 2.0).max(194.0).round();

                    // 响应式断点：宽度 >= 760px 采用 4 列 Bento 网格；窄窗口 (< 760px) 自动折行切换为 2 列紧凑网格
                    if avail_w >= 760.0 {
                        let col_w = ((avail_w - gap * 3.0) / 4.0).max(140.0);
                        let span2_w = col_w * 2.0 + gap;
                        let total_grid_w = col_w * 4.0 + gap * 3.0;
                        // 卡片内部字体与控件的等比响应式缩放系数 s (1060x660 基准为 1.0，全屏大窗平滑放大至 1.32)
                        let s =
                            ((col_w / 202.0) * 0.42 + (net_h / 502.0) * 0.58).clamp(0.95, 1.32);

                        // ROW 1: Status (2x) + Upload (1x) + Download (1x)
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_status_card(ui, ctx, app, pal, &mut acts, span2_w, row1_h, s);
                            draw_traffic_card(
                                ui,
                                pal,
                                "↑ 上传",
                                app.upload_rate,
                                app.peak_upload,
                                app.total_upload,
                                &app.upload_history,
                                col_w,
                                row1_h,
                                s,
                            );
                            draw_traffic_card(
                                ui,
                                pal,
                                "↓ 下载",
                                app.download_rate,
                                app.peak_download,
                                app.total_download,
                                &app.download_history,
                                col_w,
                                row1_h,
                                s,
                            );
                        });

                        // ROW 2: Outbound Mode (2x) + System Proxy (1x) + TUN (1x)
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_outbound_mode_card(
                                ui, ctx, app, pal, &mut acts, span2_w, row2_h, s,
                            );
                            if draw_switch_pill_card(
                                ui,
                                pal,
                                "系统代理",
                                app.is_system_proxy_on,
                                col_w,
                                row2_h,
                                s,
                            ) {
                                acts.toggle_sys_proxy = Some(!app.is_system_proxy_on);
                            }
                            if draw_switch_pill_card(
                                ui,
                                pal,
                                "TUN",
                                app.profiles.tun,
                                col_w,
                                row2_h,
                                s,
                            ) {
                                acts.toggle_tun = Some(!app.profiles.tun);
                            }
                        });

                        // ROW 3: Intranet IP (1x) + Profile (1x) + Service Status (1x) + Memory Info (1x)
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_intranet_ip_card(ui, app, pal, &mut acts, col_w, row3_h, s);
                            draw_profile_card(ui, app, pal, &mut acts, col_w, row3_h, s);
                            draw_service_status_card(ui, app, pal, &mut acts, col_w, row3_h, s);
                            draw_memory_info_card(ui, ctx, app, pal, &mut acts, col_w, row3_h, s);
                        });

                        // ROW 4: Live Connections (62%) + Proxy Groups (38%)
                        let left_w = (total_grid_w * 0.62).max(320.0);
                        let right_w = (total_grid_w - left_w - gap).max(180.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_live_connections_card(ui, app, pal, left_w, row4_h, s);
                            draw_proxy_groups_card(ui, app, pal, right_w, row4_h, s);
                        });
                    } else {
                        // === 窄屏 / 分屏 2 列响应式布局 ===
                        let col2_w = ((avail_w - gap) / 2.0).max(140.0);
                        let full_w = col2_w * 2.0 + gap;
                        let s = 1.0_f32;
                        let r1_h = 138.0_f32;
                        let r2_h = 46.0_f32;
                        let r3_h = 128.0_f32;
                        let r4_h = 204.0_f32;

                        draw_status_card(ui, ctx, app, pal, &mut acts, full_w, r1_h, s);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_traffic_card(
                                ui,
                                pal,
                                "↑ 上传",
                                app.upload_rate,
                                app.peak_upload,
                                app.total_upload,
                                &app.upload_history,
                                col2_w,
                                r1_h,
                                s,
                            );
                            draw_traffic_card(
                                ui,
                                pal,
                                "↓ 下载",
                                app.download_rate,
                                app.peak_download,
                                app.total_download,
                                &app.download_history,
                                col2_w,
                                r1_h,
                                s,
                            );
                        });

                        draw_outbound_mode_card(ui, ctx, app, pal, &mut acts, full_w, r2_h, s);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            if draw_switch_pill_card(
                                ui,
                                pal,
                                "系统代理",
                                app.is_system_proxy_on,
                                col2_w,
                                r2_h,
                                s,
                            ) {
                                acts.toggle_sys_proxy = Some(!app.is_system_proxy_on);
                            }
                            if draw_switch_pill_card(
                                ui,
                                pal,
                                "TUN",
                                app.profiles.tun,
                                col2_w,
                                r2_h,
                                s,
                            ) {
                                acts.toggle_tun = Some(!app.profiles.tun);
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_intranet_ip_card(ui, app, pal, &mut acts, col2_w, r3_h, s);
                            draw_profile_card(ui, app, pal, &mut acts, col2_w, r3_h, s);
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            draw_service_status_card(ui, app, pal, &mut acts, col2_w, r3_h, s);
                            draw_memory_info_card(ui, ctx, app, pal, &mut acts, col2_w, r3_h, s);
                        });

                        draw_live_connections_card(ui, app, pal, full_w, r4_h, s);
                        draw_proxy_groups_card(ui, app, pal, full_w, r4_h, s);
                    }
                });
        });

    acts
}

#[allow(clippy::too_many_arguments)]
fn draw_status_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (st_rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let sp = ui.painter().clone();
    let rnd = (14.0 * s).round();
    let pad_x = (16.0 * s).round();
    draw_bento_card(&sp, st_rect, rnd, pal.bg_card, pal);
    draw_status_wave_decor(&sp, st_rect, rnd, pal.primary, pal.dark);
    sp.rect_stroke(
        st_rect,
        rnd,
        egui::Stroke::new(
            0.85_f32,
            if pal.dark {
                egui::Color32::from_rgba_unmultiplied(56, 64, 82, 210)
            } else {
                egui::Color32::from_rgba_unmultiplied(220, 228, 240, 195)
            },
        ),
    );

    // 1. 顶部状态栏（按卡片高度比例定位）
    let top_y = st_rect.top() + height * 0.155;
    let st_lbl_g = ui.fonts(|f| {
        f.layout_no_wrap("状态".to_string(), cup_font_med(12.0 * s), pal.text_secondary)
    });
    let st_lbl_w = st_lbl_g.size().x;
    sp.galley(
        egui::pos2(st_rect.left() + pad_x, top_y - st_lbl_g.size().y * 0.5),
        st_lbl_g,
        pal.text_secondary,
    );

    let (pill_bg, pill_fg, pill_txt) = if app.is_core_alive {
        (
            egui::Color32::from_rgb(110, 231, 135),
            egui::Color32::from_rgb(20, 83, 45),
            "• 运行中",
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(239, 68, 68, 35),
            pal.red,
            "• 未启动",
        )
    };
    let pill_g = ui.fonts(|f| f.layout_no_wrap(pill_txt.to_string(), cup_font_med(10.5 * s), pill_fg));
    let pill_w = (pill_g.size().x + 16.0 * s).max(58.0 * s);
    let pill_h = 19.0 * s;
    let run_pill = egui::Rect::from_min_size(
        egui::pos2(st_rect.left() + pad_x + st_lbl_w + 8.0 * s, top_y - pill_h * 0.5),
        egui::vec2(pill_w, pill_h),
    );
    sp.rect_filled(run_pill, pill_h * 0.5, pill_bg);
    sp.text(
        run_pill.center(),
        egui::Align2::CENTER_CENTER,
        pill_txt,
        cup_font_med(10.5 * s),
        pill_fg,
    );

    // 2. 中部核心节点区（国旗圆章与右侧两行文字的整体视觉中心对齐）
    let cur_node_raw = if app.current_selected.is_empty() {
        "未选择节点".to_string()
    } else {
        app.current_selected.clone()
    };
    let leaf_raw = if !app.resolved_node.is_empty() {
        app.resolved_node.clone()
    } else {
        cur_node_raw.clone()
    };
    let flag_code = {
        let f = match_flag(&leaf_raw);
        if matches!(f, "DIR" | "AUTO" | "NET" | "AL") {
            match_flag(&cur_node_raw)
        } else {
            f
        }
    };

    let mid_cy = st_rect.top() + height * 0.485;
    let title_cy = mid_cy - 11.5 * s;
    let tag_cy = mid_cy + 11.5 * s;
    let av_r = 15.0 * s;
    let avatar_center = egui::pos2(st_rect.left() + pad_x + av_r, mid_cy);
    sp.circle_filled(avatar_center, av_r, pal.surface_container);
    sp.circle_stroke(
        avatar_center,
        av_r,
        egui::Stroke::new(0.8_f32, pal.border),
    );
    if !matches!(flag_code, "DIR" | "AUTO" | "FAIL" | "NET" | "AL") {
        let flag_r = egui::Rect::from_center_size(avatar_center, egui::vec2(15.0 * s, 10.5 * s));
        draw_flag_icon(&sp, flag_r, flag_code, pal);
    } else {
        draw_cup_icon(&sp, CupIcon::Bolt, avatar_center, pal.primary);
    }

    let content_x = st_rect.left() + pad_x + av_r * 2.0 + 9.0 * s;
    let max_text_right = st_rect.right() - 88.0 * s;
    let cur_title = ellipsize_text(
        ui,
        &format_cupertino_title(&cur_node_raw),
        cup_font_med(16.0 * s),
        (max_text_right - content_x).max(60.0),
    );
    let leaf_title = format_cupertino_title(&leaf_raw);
    let title_g = ui.fonts(|f| {
        f.layout_no_wrap(cur_title.clone(), cup_font_med(16.0 * s), pal.text_primary)
    });
    let title_w = title_g.size().x;
    sp.text(
        egui::pos2(content_x, title_cy),
        egui::Align2::LEFT_CENTER,
        &cur_title,
        cup_font_med(16.0 * s),
        pal.text_primary,
    );
    if leaf_title != cur_title && !leaf_title.is_empty() {
        let sub_x = content_x + title_w + 6.0 * s;
        let avail_sub_w = max_text_right - sub_x;
        if avail_sub_w > 28.0 {
            let sub_txt = ellipsize_text(
                ui,
                &format!("· {}", leaf_title),
                cup_font_reg(11.5 * s),
                avail_sub_w,
            );
            sp.text(
                egui::pos2(sub_x, title_cy + 0.5 * s),
                egui::Align2::LEFT_CENTER,
                sub_txt,
                cup_font_reg(11.5 * s),
                pal.text_secondary,
            );
        }
    }

    let just_finished_probe = !app.is_probing_status
        && !app.is_testing_delay
        && app
            .last_ping_done_at
            .map(|t| t.elapsed().as_secs_f32() < 3.5)
            .unwrap_or(false);
    let test_lbl = if app.is_probing_status || app.is_testing_delay {
        "↻ 正在测速…"
    } else if just_finished_probe {
        "✓ 测速完成"
    } else {
        "↻ 测试节点"
    };
    let tag_h = 20.0 * s;
    let tag1_g = ui.fonts(|f| {
        f.layout_no_wrap(
            test_lbl.to_string(),
            cup_font_med(10.0 * s),
            pal.text_secondary,
        )
    });
    let tag1_w = (tag1_g.size().x + 14.0 * s).max(64.0 * s);
    let tag1_rect = egui::Rect::from_min_size(
        egui::pos2(content_x, tag_cy - tag_h * 0.5),
        egui::vec2(tag1_w, tag_h),
    );
    let tag1_resp = ui.interact(
        tag1_rect,
        ui.id().with("dash_test_node_btn"),
        egui::Sense::click(),
    );
    if tag1_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let t1_scale = interact_spring_scale(
        ctx,
        tag1_resp.id,
        tag1_resp.is_pointer_button_down_on(),
        tag1_resp.hovered(),
    );
    let t1_draw = scale_rect(tag1_rect, t1_scale);
    sp.rect(
        t1_draw,
        tag_h * 0.5,
        if app.is_probing_status || app.is_testing_delay {
            pal.bg_selected
        } else if tag1_resp.hovered() {
            pal.surface_hover
        } else {
            pal.surface_container
        },
        egui::Stroke::new(
            0.8_f32,
            if tag1_resp.hovered() || app.is_probing_status || app.is_testing_delay {
                pal.primary
            } else if just_finished_probe {
                pal.green
            } else {
                pal.border
            },
        ),
    );
    sp.text(
        t1_draw.center(),
        egui::Align2::CENTER_CENTER,
        test_lbl,
        cup_font_med(10.0 * s),
        if tag1_resp.hovered() || app.is_probing_status || app.is_testing_delay {
            pal.primary
        } else if just_finished_probe {
            pal.green
        } else {
            pal.text_secondary
        },
    );
    if tag1_resp.clicked() {
        acts.test_node_status = true;
    }
    if tag1_resp.hovered() {
        tag1_resp.on_hover_text("点击立即测试当前节点真实延迟与连通状态");
    }

    let cur_d = if app.resolved_delay >= 0 {
        app.resolved_delay
    } else {
        app.nodes
            .iter()
            .find(|n| n.name == leaf_raw || n.name == cur_node_raw)
            .map(|n| n.delay)
            .unwrap_or(-1)
    };
    let (q_bg, q_fg, q_txt) = if app.is_probing_status {
        (pal.bg_selected, pal.primary, "正在测速…".to_string())
    } else if cur_d > 0 && cur_d < 180 {
        (
            egui::Color32::from_rgb(110, 231, 135),
            egui::Color32::from_rgb(20, 83, 45),
            format!("{} ms · 极佳", cur_d),
        )
    } else if cur_d >= 180 && cur_d < 380 {
        (
            egui::Color32::from_rgb(167, 243, 208),
            egui::Color32::from_rgb(6, 95, 70),
            format!("{} ms · 良好", cur_d),
        )
    } else if cur_d >= 380 {
        (
            egui::Color32::from_rgba_unmultiplied(245, 158, 11, 42),
            pal.orange,
            format!("{} ms · 偏慢", cur_d),
        )
    } else if cur_d == 0 {
        (
            egui::Color32::from_rgba_unmultiplied(239, 68, 68, 35),
            pal.red,
            "连接超时 · 重试".to_string(),
        )
    } else {
        (
            egui::Color32::from_rgb(110, 231, 135),
            egui::Color32::from_rgb(20, 83, 45),
            "质量优良 · 测速".to_string(),
        )
    };
    let q_g = ui.fonts(|f| f.layout_no_wrap(q_txt.clone(), cup_font_med(10.0 * s), q_fg));
    let tag2_w = (q_g.size().x + 16.0 * s).max(64.0 * s);
    let tag2_rect = egui::Rect::from_min_size(
        egui::pos2(tag1_rect.right() + 6.0 * s, tag_cy - tag_h * 0.5),
        egui::vec2(tag2_w, tag_h),
    );
    let tag2_resp = ui.interact(
        tag2_rect,
        ui.id().with("dash_quality_pill"),
        egui::Sense::click(),
    );
    if tag2_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let t2_scale = interact_spring_scale(
        ctx,
        tag2_resp.id,
        tag2_resp.is_pointer_button_down_on(),
        tag2_resp.hovered(),
    );
    let t2_draw = scale_rect(tag2_rect, t2_scale);
    sp.rect_filled(t2_draw, tag_h * 0.5, q_bg);
    sp.text(
        t2_draw.center(),
        egui::Align2::CENTER_CENTER,
        q_txt,
        cup_font_med(10.0 * s),
        q_fg,
    );
    if tag2_resp.clicked() {
        acts.test_node_status = true;
    }
    if tag2_resp.hovered() {
        tag2_resp.on_hover_text("点击重新检测节点响应延迟");
    }

    // 3. 底部信息栏（运行时长 + 当前订阅）
    let bot_lbl_y = st_rect.bottom() - height * 0.23;
    let bot_val_y = st_rect.bottom() - height * 0.11;
    sp.text(
        egui::pos2(st_rect.left() + pad_x, bot_lbl_y),
        egui::Align2::LEFT_CENTER,
        "运行时长",
        cup_font_reg(10.0 * s),
        pal.text_muted,
    );
    sp.text(
        egui::pos2(st_rect.left() + pad_x, bot_val_y),
        egui::Align2::LEFT_CENTER,
        format_uptime(app.started_at.elapsed().as_secs()),
        cup_font_med(12.5 * s),
        pal.text_primary,
    );

    let active_sub_name = app
        .profiles
        .subscriptions
        .iter()
        .find(|s| s.id == app.profiles.active)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "默认配置".to_string());
    let sub_col_x = st_rect.left() + pad_x + (width * 0.22).clamp(84.0 * s, 118.0 * s);
    sp.text(
        egui::pos2(sub_col_x, bot_lbl_y),
        egui::Align2::LEFT_CENTER,
        "当前订阅",
        cup_font_reg(10.0 * s),
        pal.text_muted,
    );
    sp.text(
        egui::pos2(sub_col_x, bot_val_y),
        egui::Align2::LEFT_CENTER,
        format_cupertino_title(&active_sub_name),
        cup_font_med(12.5 * s),
        pal.text_primary,
    );

    // 4. 右侧花朵光晕启停主按钮
    let flower_r = 22.5 * s;
    let flower_center = egui::pos2(st_rect.right() - 42.0 * s, st_rect.center().y + 4.0 * s);
    let flower_hit = egui::Rect::from_center_size(flower_center, egui::vec2(54.0 * s, 54.0 * s));
    let flower_resp = ui.interact(
        flower_hit,
        ui.id().with("dash_flower_btn"),
        egui::Sense::click(),
    );
    if flower_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let f_scale = interact_spring_scale(
        ctx,
        flower_resp.id,
        flower_resp.is_pointer_button_down_on(),
        flower_resp.hovered(),
    );
    let is_active_proxy = app.is_system_proxy_on || app.profiles.tun;
    let flower_col = if is_active_proxy {
        pal.primary
    } else {
        egui::Color32::from_rgb(148, 163, 184)
    };
    draw_scalloped_flower_button(
        &sp,
        flower_center,
        flower_r * f_scale,
        flower_col,
        is_active_proxy,
    );
    if flower_resp.clicked() {
        acts.toggle_sys_proxy = Some(!app.is_system_proxy_on);
    }
    if flower_resp.hovered() {
        flower_resp.on_hover_text(if is_active_proxy {
            "点击暂停系统代理"
        } else {
            "点击开启系统代理"
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_traffic_card(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    label: &str,
    rate: u64,
    peak: u64,
    total: u64,
    history: &VecDeque<u64>,
    width: f32,
    height: f32,
    s: f32,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&p, rect, (14.0 * s).round(), pal.bg_card, pal);

    let hdr_y = rect.top() + height * 0.155;
    p.text(
        egui::pos2(rect.left() + pad, hdr_y),
        egui::Align2::LEFT_CENTER,
        label,
        cup_font_med(11.5 * s),
        pal.text_secondary,
    );
    p.text(
        egui::pos2(rect.right() - pad, hdr_y),
        egui::Align2::RIGHT_CENTER,
        format!("峰值 {}", format_speed(peak)),
        cup_font_reg(9.8 * s),
        pal.text_muted,
    );

    // 主速率数字与单位严格基线对齐
    let (num, unit) = split_speed_parts(rate);
    let num_g = ui.fonts(|f| f.layout_no_wrap(num, cup_font_med(21.0 * s), pal.text_primary));
    let unit_g = ui.fonts(|f| {
        f.layout_no_wrap(
            unit.to_string(),
            cup_font_med(11.0 * s),
            pal.text_secondary,
        )
    });
    let num_w = num_g.size().x;
    let num_h = num_g.size().y;
    let unit_h = unit_g.size().y;
    let num_top = rect.top() + height * 0.25;
    p.galley(
        egui::pos2(rect.left() + pad, num_top),
        num_g,
        pal.text_primary,
    );
    p.galley(
        egui::pos2(
            rect.left() + pad + 4.0 * s + num_w,
            num_top + num_h - unit_h - 2.5 * s,
        ),
        unit_g,
        pal.text_secondary,
    );

    p.text(
        egui::pos2(rect.left() + pad, rect.top() + height * 0.50),
        egui::Align2::LEFT_CENTER,
        format!("总计 {}", format_bytes(total)),
        cup_font_reg(10.0 * s),
        pal.text_muted,
    );
    let spark = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 8.0 * s, rect.top() + height * 0.59),
        egui::pos2(rect.right() - 8.0 * s, rect.bottom() - 7.0 * s),
    );
    draw_sparkline_area(&p, spark, history, pal.primary);
}

#[allow(clippy::too_many_arguments)]
fn draw_outbound_mode_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (om_rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let op = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&op, om_rect, (12.0 * s).round(), pal.bg_card, pal);
    draw_cup_icon(
        &op,
        CupIcon::Network,
        egui::pos2(om_rect.left() + pad + 5.0 * s, om_rect.center().y),
        pal.text_secondary,
    );
    op.text(
        egui::pos2(om_rect.left() + pad + 19.0 * s, om_rect.center().y),
        egui::Align2::LEFT_CENTER,
        "出站模式",
        cup_font_med(12.5 * s),
        pal.text_primary,
    );

    let modes = [
        ("rule", "规则", CupIcon::Dashboard),
        ("global", "全局", CupIcon::Proxy),
        ("direct", "直连", CupIcon::Locate),
    ];
    let cur_mode = app.profiles.mode.clone();
    let pill_w = (68.0 * s).round();
    let pill_h = (height - 14.0 * s).clamp(28.0, 42.0);
    let pill_gap = (6.0 * s).round();
    let right_start = om_rect.right() - 8.0 * s - (pill_w * 3.0 + pill_gap * 2.0);
    for (i, (m_key, m_lbl, m_ic)) in modes.iter().enumerate() {
        let p_rect = egui::Rect::from_min_size(
            egui::pos2(
                right_start + i as f32 * (pill_w + pill_gap),
                om_rect.center().y - pill_h * 0.5,
            ),
            egui::vec2(pill_w, pill_h),
        );
        let p_resp = ui.interact(
            p_rect,
            ui.id().with(("bento_mode", m_key)),
            egui::Sense::click(),
        );
        if p_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let is_m = cur_mode.eq_ignore_ascii_case(m_key);
        let p_scale = interact_spring_scale(
            ctx,
            p_resp.id,
            p_resp.is_pointer_button_down_on(),
            p_resp.hovered(),
        );
        let pr = scale_rect(p_rect, p_scale);
        if is_m {
            op.rect_filled(pr, 9.0 * s, pal.primary);
        } else {
            op.rect_filled(
                pr,
                9.0 * s,
                if p_resp.hovered() {
                    pal.surface_hover
                } else {
                    pal.surface_container
                },
            );
        }
        let fg = if is_m {
            pal.on_primary
        } else {
            pal.text_secondary
        };
        // 将 [图标 + 间距 + 文字] 作为整体在胶囊按钮内部水平居中，消除视觉偏左
        let lbl_g = ui.fonts(|f| f.layout_no_wrap(m_lbl.to_string(), cup_font_med(11.5 * s), fg));
        let icon_span = 14.0 * s;
        let total_w = icon_span + lbl_g.size().x;
        let start_x = pr.center().x - total_w * 0.5;
        draw_cup_icon(
            &op,
            *m_ic,
            egui::pos2(start_x + 4.5 * s, pr.center().y),
            fg,
        );
        op.galley(
            egui::pos2(start_x + icon_span, pr.center().y - lbl_g.size().y * 0.5),
            lbl_g,
            fg,
        );
        if p_resp.clicked() && !is_m {
            acts.switch_mode = Some(m_key.to_string());
        }
    }
}

fn draw_switch_pill_card(
    ui: &mut egui::Ui,
    pal: &ThemePalette,
    label: &str,
    val: bool,
    width: f32,
    height: f32,
    s: f32,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&p, rect, (12.0 * s).round(), pal.bg_card, pal);
    p.text(
        egui::pos2(rect.left() + pad, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        cup_font_med(12.5 * s),
        pal.text_primary,
    );
    let sw_w = 42.0 * s.min(1.15);
    let sw_h = 23.0 * s.min(1.15);
    let sw_r = egui::Rect::from_center_size(
        egui::pos2(rect.right() - pad - sw_w * 0.5, rect.center().y),
        egui::vec2(sw_w, sw_h),
    );
    let mut sw_ui = ui.new_child(egui::UiBuilder::new().max_rect(sw_r));
    md3_switch(&mut sw_ui, val, pal)
}

fn draw_intranet_ip_card(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (ip_rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let ipp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&ipp, ip_rect, (14.0 * s).round(), pal.bg_card, pal);
    let hdr_y = ip_rect.top() + height * 0.16;
    ipp.text(
        egui::pos2(ip_rect.left() + pad, hdr_y),
        egui::Align2::LEFT_CENTER,
        "内网 IP",
        cup_font_med(11.5 * s),
        pal.text_secondary,
    );
    let lan_h = (height * 0.30).clamp(35.0 * s, 46.0 * s).round();
    let lan_box = egui::Rect::from_min_max(
        egui::pos2(ip_rect.left() + 10.0 * s, ip_rect.bottom() - 10.0 * s - lan_h),
        egui::pos2(ip_rect.right() - 10.0 * s, ip_rect.bottom() - 10.0 * s),
    );
    let mask_hit = egui::Rect::from_min_max(
        egui::pos2(ip_rect.left() + 10.0 * s, ip_rect.top() + 8.0 * s),
        egui::pos2(ip_rect.right() - 10.0 * s, lan_box.top() - 4.0),
    );
    let mask_resp = ui.interact(
        mask_hit,
        ui.id().with("dash_ip_mask"),
        egui::Sense::click(),
    );
    if mask_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    draw_cup_icon(
        &ipp,
        CupIcon::EyeOff,
        egui::pos2(ip_rect.right() - pad - 5.0 * s, hdr_y),
        if app.hide_ip || mask_resp.hovered() {
            pal.primary
        } else {
            pal.text_muted
        },
    );
    let shown_ip = if app.hide_ip {
        let parts: Vec<&str> = app.intranet_ip.split('.').collect();
        if parts.len() == 4 {
            format!("{}.{}.*.*", parts[0], parts[1])
        } else {
            "192.168.*.*".to_string()
        }
    } else {
        app.intranet_ip.clone()
    };
    let ip_y = (hdr_y + 7.0 * s + lan_box.top()) * 0.5;
    ipp.text(
        egui::pos2(ip_rect.left() + pad, ip_y),
        egui::Align2::LEFT_CENTER,
        &shown_ip,
        cup_font_med(17.5 * s),
        pal.text_primary,
    );
    if mask_resp.clicked() {
        app.hide_ip = !app.hide_ip;
    }

    ipp.rect_filled(lan_box, 9.0 * s, pal.surface_container);
    ipp.text(
        egui::pos2(lan_box.left() + 10.0 * s, lan_box.top() + lan_h * 0.34),
        egui::Align2::LEFT_CENTER,
        "局域网共享",
        cup_font_med(11.0 * s),
        pal.text_primary,
    );
    ipp.text(
        egui::pos2(lan_box.left() + 10.0 * s, lan_box.bottom() - lan_h * 0.27),
        egui::Align2::LEFT_CENTER,
        if app.profiles.allow_lan {
            "已开启"
        } else {
            "已关闭"
        },
        cup_font_reg(9.5 * s),
        pal.text_muted,
    );
    let sw_w = 42.0 * s.min(1.15);
    let sw_h = 23.0 * s.min(1.15);
    let sw_lan_r = egui::Rect::from_center_size(
        egui::pos2(lan_box.right() - 10.0 * s - sw_w * 0.5, lan_box.center().y),
        egui::vec2(sw_w, sw_h),
    );
    let mut sw_lan_ui = ui.new_child(egui::UiBuilder::new().max_rect(sw_lan_r));
    if md3_switch(&mut sw_lan_ui, app.profiles.allow_lan, pal) {
        acts.toggle_allow_lan = Some(!app.profiles.allow_lan);
    }
}

fn draw_profile_card(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (pr_rect, pr_resp) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if pr_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let prp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&prp, pr_rect, (14.0 * s).round(), pal.bg_card, pal);
    let hdr_y = pr_rect.top() + height * 0.16;
    prp.text(
        egui::pos2(pr_rect.left() + pad, hdr_y),
        egui::Align2::LEFT_CENTER,
        "当前配置",
        cup_font_med(11.5 * s),
        pal.text_secondary,
    );
    let ref_hit = egui::Rect::from_center_size(
        egui::pos2(pr_rect.right() - pad - 16.0 * s, hdr_y),
        egui::vec2(20.0 * s, 20.0 * s),
    );
    let ref_resp = ui.interact(
        ref_hit,
        ui.id().with("dash_prof_ref"),
        egui::Sense::click(),
    );
    if ref_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    draw_cup_icon(
        &prp,
        CupIcon::Refresh,
        ref_hit.center(),
        if ref_resp.hovered() {
            pal.primary
        } else {
            pal.text_muted
        },
    );
    prp.text(
        egui::pos2(pr_rect.right() - pad, hdr_y),
        egui::Align2::RIGHT_CENTER,
        "›",
        cup_font_med(14.0 * s),
        pal.text_muted,
    );

    let active_sub = app
        .profiles
        .subscriptions
        .iter()
        .find(|s| s.id == app.profiles.active)
        .cloned();
    if let Some(sub) = active_sub {
        let y_name = pr_rect.top() + height * 0.42;
        let y_quota = pr_rect.top() + height * 0.66;
        let wave_r = egui::Rect::from_center_size(
            egui::pos2(pr_rect.center().x, pr_rect.top() + height * 0.85),
            egui::vec2(pr_rect.width() - pad * 2.0, 5.5 * s),
        );

        prp.text(
            egui::pos2(pr_rect.left() + pad, y_name),
            egui::Align2::LEFT_CENTER,
            format_cupertino_title(&sub.name),
            cup_font_med(14.0 * s),
            pal.text_primary,
        );
        let days_lbl = if let Some(d) = days_until_expire(sub.expire) {
            format!("剩余 {} 天", d)
        } else if !sub.expire_hint.is_empty() {
            sub.expire_hint.clone()
        } else {
            "长期有效".to_string()
        };
        prp.text(
            egui::pos2(pr_rect.right() - pad, y_name),
            egui::Align2::RIGHT_CENTER,
            days_lbl,
            cup_font_reg(10.0 * s),
            pal.text_muted,
        );

        let (pct_str, quota_str, ratio) = if sub.total > 0 {
            let used = sub.upload.saturating_add(sub.download);
            let r = (used as f32 / sub.total as f32).clamp(0.0, 1.0);
            (
                format!("{:.1}% 已用", r * 100.0),
                format!("{} / {}", format_bytes(used), format_bytes(sub.total)),
                r,
            )
        } else if !sub.remaining_hint.is_empty() {
            ("剩余可用".to_string(), sub.remaining_hint.clone(), 0.25_f32)
        } else {
            ("未同步".to_string(), "点击刷新".to_string(), 0.0_f32)
        };
        prp.text(
            egui::pos2(pr_rect.left() + pad, y_quota),
            egui::Align2::LEFT_CENTER,
            pct_str,
            cup_font_med(12.0 * s),
            pal.text_primary,
        );
        prp.text(
            egui::pos2(pr_rect.right() - pad, y_quota),
            egui::Align2::RIGHT_CENTER,
            quota_str,
            cup_font_reg(10.0 * s),
            pal.text_muted,
        );

        draw_wave_progress(&prp, wave_r, ratio, pal.primary, pal.surface_container);
    }

    if ref_resp.clicked() {
        acts.update_active_sub = true;
    } else if pr_resp.clicked() {
        app.nav_tab = NavTab::Profiles;
    }
}

fn draw_service_status_card(
    ui: &mut egui::Ui,
    app: &AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (sv_rect, sv_resp) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if sv_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let svp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&svp, sv_rect, (14.0 * s).round(), pal.bg_card, pal);
    let hdr_y = sv_rect.top() + height * 0.16;
    svp.text(
        egui::pos2(sv_rect.left() + pad, hdr_y),
        egui::Align2::LEFT_CENTER,
        "服务状态",
        cup_font_med(11.5 * s),
        pal.text_secondary,
    );
    let just_done = !app.is_probing_status
        && app
            .last_ping_done_at
            .map(|t| t.elapsed().as_secs_f32() < 3.5)
            .unwrap_or(false);
    svp.text(
        egui::pos2(sv_rect.right() - pad, hdr_y),
        egui::Align2::RIGHT_CENTER,
        if app.is_probing_status {
            "↻ 正在测速…"
        } else if just_done {
            "✓ 测速完成"
        } else {
            "点击测速 ›"
        },
        cup_font_med(10.0 * s),
        if app.is_probing_status {
            pal.primary
        } else if just_done {
            pal.green
        } else {
            pal.text_muted
        },
    );

    let fallback_d = if app.resolved_delay > 0 {
        app.resolved_delay
    } else {
        app.nodes
            .iter()
            .map(|n| n.delay)
            .filter(|&d| d > 0)
            .min()
            .unwrap_or(48)
    };
    let g_delay = if app.probe_google_ms >= 0 {
        app.probe_google_ms
    } else {
        fallback_d
    };
    let cf_delay = if app.probe_cf_ms >= 0 {
        app.probe_cf_ms
    } else {
        (fallback_d as f32 * 0.65).max(18.0) as i32
    };

    // 与左侧「内网 IP」卡片的底部内框下边缘保持同高对齐 (bottom - 10*s)
    let list_top = sv_rect.top() + height * 0.295;
    let list_bot = sv_rect.bottom() - 10.0 * s;
    let pill_gap = (6.5 * s).round();
    let srv_pill_h = ((list_bot - list_top - pill_gap) * 0.5).max(26.0);
    for (idx, (badge_ch, srv_name, delay_val)) in
        [("G", "Google", g_delay), ("⚡", "Cloudflare", cf_delay)]
            .iter()
            .enumerate()
    {
        let pill_y = list_top + idx as f32 * (srv_pill_h + pill_gap);
        let s_pill = egui::Rect::from_min_size(
            egui::pos2(sv_rect.left() + 10.0 * s, pill_y),
            egui::vec2(sv_rect.width() - 20.0 * s, srv_pill_h),
        );
        let row_resp = ui.interact(
            s_pill,
            ui.id().with(("srv_probe_row", idx)),
            egui::Sense::click(),
        );
        if row_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        svp.rect_filled(
            s_pill,
            8.5 * s,
            if row_resp.hovered() {
                pal.surface_hover
            } else {
                pal.surface_container
            },
        );
        let ic_c = egui::pos2(s_pill.left() + 16.0 * s, s_pill.center().y);
        svp.circle_filled(ic_c, 9.5 * s, pal.bg_card);
        svp.text(
            ic_c,
            egui::Align2::CENTER_CENTER,
            *badge_ch,
            cup_font_med(10.5 * s),
            pal.primary,
        );
        svp.text(
            egui::pos2(s_pill.left() + 31.0 * s, s_pill.center().y),
            egui::Align2::LEFT_CENTER,
            *srv_name,
            cup_font_med(11.0 * s),
            pal.text_primary,
        );
        let (d_str, d_col) = if app.is_probing_status {
            ("测速中…".to_string(), pal.primary)
        } else if *delay_val > 0 {
            let c = if *delay_val < 220 {
                pal.green
            } else if *delay_val < 450 {
                pal.orange
            } else {
                pal.red
            };
            (format!("{} ms", delay_val), c)
        } else {
            ("超时".to_string(), pal.red)
        };
        svp.text(
            egui::pos2(s_pill.right() - 10.0 * s, s_pill.center().y),
            egui::Align2::RIGHT_CENTER,
            d_str,
            cup_font_med(11.0 * s),
            d_col,
        );
        if row_resp.clicked() {
            acts.test_node_status = true;
        }
    }
    if sv_resp.clicked() {
        acts.test_node_status = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_memory_info_card(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &AppState,
    pal: &ThemePalette,
    acts: &mut DashboardActions,
    width: f32,
    height: f32,
    s: f32,
) {
    let (mm_rect, mm_resp) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let mmp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&mmp, mm_rect, (14.0 * s).round(), pal.bg_card, pal);

    // 5 行严格按卡片内部相对比例分层 (16% / 38% / 57% / 74% / 88%)，彻底消除进度条与文字重叠
    let hdr_y = mm_rect.top() + height * 0.16;
    let val_cy = mm_rect.top() + height * 0.38;
    let bar_cy = mm_rect.top() + height * 0.575;
    let leg1_y = mm_rect.top() + height * 0.735;
    let leg2_y = mm_rect.top() + height * 0.88;

    mmp.text(
        egui::pos2(mm_rect.left() + pad, hdr_y),
        egui::Align2::LEFT_CENTER,
        "内存信息",
        cup_font_med(11.5 * s),
        pal.text_secondary,
    );

    let trim_w = 46.0 * s;
    let trim_h = 20.0 * s;
    let trim_rect = egui::Rect::from_min_size(
        egui::pos2(mm_rect.right() - pad - trim_w, hdr_y - trim_h * 0.5),
        egui::vec2(trim_w, trim_h),
    );
    let trim_resp = ui.interact(
        trim_rect,
        ui.id().with("dash_mem_trim"),
        egui::Sense::click(),
    );
    if trim_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let trim_sc = interact_spring_scale(
        ctx,
        trim_resp.id,
        trim_resp.is_pointer_button_down_on(),
        trim_resp.hovered(),
    );
    let tr_draw = scale_rect(trim_rect, trim_sc);
    mmp.rect_filled(
        tr_draw,
        trim_h * 0.5,
        if trim_resp.hovered() {
            pal.bg_selected
        } else {
            pal.surface_container
        },
    );
    mmp.text(
        tr_draw.center(),
        egui::Align2::CENTER_CENTER,
        "↻ 释放",
        cup_font_med(9.5 * s),
        if trim_resp.hovered() {
            pal.primary
        } else {
            pal.text_secondary
        },
    );
    if trim_resp.clicked() {
        acts.trim_memory = true;
    }

    let gui_mb = app.mem_gui_kb as f64 / 1024.0;
    let gui_rss_mb = app.mem_gui_rss_kb as f64 / 1024.0;
    let core_mb = app.mem_core_kb as f64 / 1024.0;
    let core_rss_mb = app.mem_core_rss_kb as f64 / 1024.0;
    let tot_mb = (gui_mb + core_mb).max(1.0);

    // 主数值与 MB 单位严格基线对齐
    let mem_g = ui.fonts(|f| {
        f.layout_no_wrap(
            format!("{:.1}", tot_mb),
            cup_font_med(18.5 * s),
            pal.text_primary,
        )
    });
    let mb_g = ui.fonts(|f| {
        f.layout_no_wrap(
            "MB".to_string(),
            cup_font_med(10.5 * s),
            pal.text_secondary,
        )
    });
    let mem_w = mem_g.size().x;
    let mem_h = mem_g.size().y;
    let mb_h = mb_g.size().y;
    let mem_top = val_cy - mem_h * 0.5;
    let baseline_y = mem_top + mem_h - mb_h * 0.5 - 2.0 * s;

    mmp.galley(
        egui::pos2(mm_rect.left() + pad, mem_top),
        mem_g,
        pal.text_primary,
    );
    mmp.galley(
        egui::pos2(
            mm_rect.left() + pad + 3.5 * s + mem_w,
            mem_top + mem_h - mb_h - 2.0 * s,
        ),
        mb_g,
        pal.text_secondary,
    );
    if app.mem_sys_total_gb > 0.1 {
        let sys_pct = (app.mem_sys_used_gb / app.mem_sys_total_gb * 100.0).clamp(0.0, 100.0);
        mmp.text(
            egui::pos2(mm_rect.right() - pad, baseline_y),
            egui::Align2::RIGHT_CENTER,
            format!("系统 {:.0}%", sys_pct),
            cup_font_reg(9.8 * s),
            pal.text_muted,
        );
    }

    // 进度条严格居中在 bar_cy，上下均留足 16% 净空
    let bar_h = 5.5 * s;
    let mbar = egui::Rect::from_center_size(
        egui::pos2(mm_rect.center().x, bar_cy),
        egui::vec2(mm_rect.width() - pad * 2.0, bar_h),
    );
    let core_track_col = if pal.dark {
        egui::Color32::from_rgba_unmultiplied(147, 197, 253, 85)
    } else {
        egui::Color32::from_rgb(191, 219, 254)
    };
    mmp.rect_filled(mbar, bar_h * 0.5, core_track_col);
    let app_ratio = (gui_mb / tot_mb).clamp(0.12, 0.88) as f32;
    let app_bar = egui::Rect::from_min_size(
        mbar.min,
        egui::vec2(mbar.width() * app_ratio, mbar.height()),
    );
    mmp.rect_filled(app_bar, bar_h * 0.5, pal.primary);

    // 图例第 1 行：界面
    mmp.circle_filled(
        egui::pos2(mm_rect.left() + pad + 3.0 * s, leg1_y),
        2.8 * s,
        pal.primary,
    );
    mmp.text(
        egui::pos2(mm_rect.left() + pad + 10.5 * s, leg1_y),
        egui::Align2::LEFT_CENTER,
        format!("界面  {:.1} MB", gui_mb),
        cup_font_med(10.2 * s),
        pal.text_secondary,
    );
    if gui_rss_mb > gui_mb + 1.0 {
        mmp.text(
            egui::pos2(mm_rect.right() - pad, leg1_y),
            egui::Align2::RIGHT_CENTER,
            format!("RSS {:.0}M", gui_rss_mb),
            cup_font_reg(9.5 * s),
            pal.text_muted,
        );
    }

    // 图例第 2 行：内核
    mmp.circle_filled(
        egui::pos2(mm_rect.left() + pad + 3.0 * s, leg2_y),
        2.8 * s,
        egui::Color32::from_rgb(147, 197, 253),
    );
    mmp.text(
        egui::pos2(mm_rect.left() + pad + 10.5 * s, leg2_y),
        egui::Align2::LEFT_CENTER,
        format!("内核  {:.1} MB", core_mb),
        cup_font_med(10.2 * s),
        pal.text_secondary,
    );
    if app.mem_sys_total_gb > 0.1 {
        mmp.text(
            egui::pos2(mm_rect.right() - pad, leg2_y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.1}/{:.0}G", app.mem_sys_used_gb, app.mem_sys_total_gb),
            cup_font_reg(9.5 * s),
            pal.text_muted,
        );
    }

    if mm_resp.hovered() {
        egui::show_tooltip_at_pointer(
            ctx,
            egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("mem_card_tip")),
            egui::Id::new("mem_card_tip_box"),
            |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "界面私有堆内存: {:.1} MB (含共享显存/字体总 RSS: {:.1} MB)\n\
                         Mihomo 内核内存: {:.1} MB (总 RSS: {:.1} MB)\n\
                         系统物理内存: {:.1} GB / {:.1} GB\n\
                         提示: 点击右上角「↻ 释放」可立即回收闲置内存",
                        gui_mb,
                        gui_rss_mb,
                        core_mb,
                        core_rss_mb,
                        app.mem_sys_used_gb,
                        app.mem_sys_total_gb
                    ))
                    .size(11.5),
                );
            },
        );
    }
}

fn draw_live_connections_card(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    width: f32,
    height: f32,
    s: f32,
) {
    let (lc_rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let lcp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&lcp, lc_rect, (14.0 * s).round(), pal.bg_card, pal);

    let conn_cnt = app.active_connections.max(app.live_connections.len());
    let tab_labels = [
        format!("{} 实时连接", conn_cnt),
        "近期请求".to_string(),
        "DNS 查询".to_string(),
    ];
    let mut tab_x = lc_rect.left() + pad;
    let tab_h = 24.0 * s;
    for (t_idx, t_lbl) in tab_labels.iter().enumerate() {
        let tg = ui.fonts(|f| {
            f.layout_no_wrap(t_lbl.clone(), cup_font_med(10.8 * s), pal.text_primary)
        });
        let tw = tg.size().x + 20.0 * s;
        let tr = egui::Rect::from_min_size(
            egui::pos2(tab_x, lc_rect.top() + 11.0 * s),
            egui::vec2(tw, tab_h),
        );
        let t_resp = ui.interact(
            tr,
            ui.id().with(("dash_conn_t", t_idx)),
            egui::Sense::click(),
        );
        if t_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let is_cur = app.dash_conn_tab == t_idx;
        if is_cur {
            lcp.rect_filled(tr, tab_h * 0.5, pal.primary);
        } else {
            lcp.rect_filled(
                tr,
                tab_h * 0.5,
                if t_resp.hovered() {
                    pal.surface_hover
                } else {
                    pal.surface_container
                },
            );
        }
        lcp.text(
            tr.center(),
            egui::Align2::CENTER_CENTER,
            t_lbl,
            cup_font_med(10.8 * s),
            if is_cur {
                pal.on_primary
            } else {
                pal.text_secondary
            },
        );
        if t_resp.clicked() {
            app.dash_conn_tab = t_idx;
        }
        tab_x += tw + 6.0 * s;
    }

    // 5 列黄金比例坐标：拉宽「出口节点」与「下载」之间的安全间距，彻底杜绝 Singapore 04 与 102.7 KB 粘连
    let hdr_y = lc_rect.top() + 47.0 * s;
    let c_src = lc_rect.left() + pad;
    let c_dst = lc_rect.left() + (width * 0.185).max(84.0 * s);
    let c_ext = lc_rect.left() + width * 0.51;
    let c_ul = lc_rect.right() - pad;
    let dl_col_reserve = (width * 0.135).clamp(58.0 * s, 82.0 * s);
    let c_dl = c_ul - dl_col_reserve;

    let col_headers = match app.dash_conn_tab {
        1 => ["类型", "近期目标主机", "命中策略", "下行", "上行"],
        2 => ["协议", "查询域名 (Host)", "解析出口", "响应", "上行"],
        _ => ["来源", "目标地址", "出口节点", "下载", "上传"],
    };

    for (x, align, lbl) in [
        (c_src, egui::Align2::LEFT_CENTER, col_headers[0]),
        (c_dst, egui::Align2::LEFT_CENTER, col_headers[1]),
        (c_ext, egui::Align2::LEFT_CENTER, col_headers[2]),
        (c_dl, egui::Align2::RIGHT_CENTER, col_headers[3]),
        (c_ul, egui::Align2::RIGHT_CENTER, col_headers[4]),
    ] {
        lcp.text(
            egui::pos2(x, hdr_y),
            align,
            lbl,
            cup_font_med(10.2 * s),
            pal.text_muted,
        );
    }
    let sep_y = hdr_y + 11.0 * s;
    lcp.line_segment(
        [
            egui::pos2(lc_rect.left() + pad, sep_y),
            egui::pos2(lc_rect.right() - pad, sep_y),
        ],
        egui::Stroke::new(0.8_f32, pal.divider),
    );

    if app.live_connections.is_empty() {
        lcp.text(
            egui::pos2(lc_rect.center().x, lc_rect.center().y + 18.0 * s),
            egui::Align2::CENTER_CENTER,
            "当前暂无活跃长连接（产生网络请求后实时显示）",
            cup_font_reg(11.5 * s),
            pal.text_muted,
        );
    } else {
        let avail_list_h = (lc_rect.bottom() - 10.0 * s - sep_y).max(90.0);
        let target_row_h = 30.0 * s;
        let max_rows = (avail_list_h / target_row_h).floor().max(4.0) as usize;
        let mut display_rows = app.live_connections.clone();
        if app.dash_conn_tab == 1 {
            display_rows.sort_by(|a, b| {
                (b.download + b.upload).cmp(&(a.download + a.upload))
            });
        } else if app.dash_conn_tab == 2 {
            display_rows.retain(|c| c.target.chars().any(|ch| ch.is_ascii_alphabetic()));
            if display_rows.is_empty() {
                display_rows = app.live_connections.clone();
            }
        }
        let actual_rows = display_rows.len().min(max_rows);
        let row_step = if actual_rows > 0 {
            (avail_list_h / actual_rows as f32).clamp(28.0 * s, 36.0 * s)
        } else {
            target_row_h
        };
        for (r_i, conn) in display_rows.iter().take(max_rows).enumerate() {
            let ry = sep_y + row_step * 0.52 + r_i as f32 * row_step;
            if ry + 10.0 > lc_rect.bottom() {
                break;
            }
            let src_label = if app.dash_conn_tab == 2 {
                "DNS"
            } else {
                conn.source.as_str()
            };
            let init_ch = src_label
                .chars()
                .next()
                .unwrap_or('A')
                .to_ascii_uppercase();
            let ic_pos = egui::pos2(c_src + 7.5 * s, ry);
            lcp.circle_filled(ic_pos, 7.8 * s, pal.surface_container);
            lcp.text(
                ic_pos,
                egui::Align2::CENTER_CENTER,
                init_ch.to_string(),
                cup_font_med(9.0 * s),
                pal.text_secondary,
            );
            let src_txt = ellipsize_text(
                ui,
                src_label,
                cup_font_med(10.8 * s),
                (c_dst - (c_src + 20.0 * s) - 8.0 * s).max(24.0),
            );
            lcp.text(
                egui::pos2(c_src + 20.0 * s, ry),
                egui::Align2::LEFT_CENTER,
                src_txt,
                cup_font_med(10.8 * s),
                pal.text_primary,
            );

            let raw_target = if app.dash_conn_tab == 2 {
                conn.target.split(':').next().unwrap_or(&conn.target)
            } else {
                conn.target.as_str()
            };
            let dst_max_w = (c_ext - c_dst - 12.0 * s).max(40.0);
            let dst_txt = ellipsize_text(ui, raw_target, cup_font_reg(10.8 * s), dst_max_w);
            lcp.text(
                egui::pos2(c_dst, ry),
                egui::Align2::LEFT_CENTER,
                dst_txt,
                cup_font_reg(10.8 * s),
                pal.text_secondary,
            );

            let ext_flag = match_flag(&conn.exit);
            let mut ext_text_x = c_ext;
            if ext_flag != "DIR" && ext_flag != "AUTO" && ext_flag != "NET" {
                let f_rect = egui::Rect::from_center_size(
                    egui::pos2(c_ext + 6.5 * s, ry),
                    egui::vec2(12.5 * s, 9.2 * s),
                );
                draw_flag_icon(&lcp, f_rect, ext_flag, pal);
                ext_text_x += 16.5 * s;
            }
            // 为右侧右对齐的「下载」数值预留 58*s 宽度 + 12*s 净空，彻底避免出口节点与下载流量相撞
            let dl_text = format_bytes(conn.download);
            let dl_w = ui.fonts(|f| {
                f.layout_no_wrap(dl_text.clone(), cup_font_reg(10.6 * s), pal.text_secondary)
                    .size()
                    .x
            });
            let ext_max_w = (c_dl - dl_w - 14.0 * s - ext_text_x).max(28.0);
            let ext_txt = ellipsize_text(
                ui,
                &format_cupertino_title(&conn.exit),
                cup_font_reg(10.6 * s),
                ext_max_w,
            );
            lcp.text(
                egui::pos2(ext_text_x, ry),
                egui::Align2::LEFT_CENTER,
                ext_txt,
                cup_font_reg(10.6 * s),
                pal.text_secondary,
            );

            lcp.text(
                egui::pos2(c_dl, ry),
                egui::Align2::RIGHT_CENTER,
                dl_text,
                cup_font_reg(10.6 * s),
                pal.text_secondary,
            );
            lcp.text(
                egui::pos2(c_ul, ry),
                egui::Align2::RIGHT_CENTER,
                format_bytes(conn.upload),
                cup_font_reg(10.6 * s),
                pal.text_secondary,
            );
        }
    }
}

fn draw_proxy_groups_card(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    width: f32,
    height: f32,
    s: f32,
) {
    let (pg_rect, pg_resp) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if pg_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let pgp = ui.painter().clone();
    let pad = (14.0 * s).round();
    draw_bento_card(&pgp, pg_rect, (14.0 * s).round(), pal.bg_card, pal);
    let hdr_y = pg_rect.top() + 20.0 * s;
    let hdr_g = ui.fonts(|f| {
        f.layout_no_wrap("策略组".to_string(), cup_font_med(11.5 * s), pal.text_secondary)
    });
    let hdr_w = hdr_g.size().x;
    pgp.galley(
        egui::pos2(pg_rect.left() + pad, hdr_y - hdr_g.size().y * 0.5),
        hdr_g,
        pal.text_secondary,
    );
    let cnt_pill = egui::Rect::from_center_size(
        egui::pos2(pg_rect.left() + pad + hdr_w + 15.0 * s, hdr_y),
        egui::vec2(19.0 * s, 15.5 * s),
    );
    pgp.rect_filled(cnt_pill, 7.5 * s, pal.surface_container);
    pgp.text(
        cnt_pill.center(),
        egui::Align2::CENTER_CENTER,
        format!("{}", app.group_summaries.len().max(1)),
        cup_font_med(9.5 * s),
        pal.text_secondary,
    );
    pgp.text(
        egui::pos2(pg_rect.right() - pad, hdr_y),
        egui::Align2::RIGHT_CENTER,
        "›",
        cup_font_med(14.0 * s),
        pal.text_muted,
    );

    let summaries = if app.group_summaries.is_empty() {
        vec![
            ("节点选择".to_string(), app.current_selected.clone()),
            ("自动选择".to_string(), app.resolved_node.clone()),
        ]
    } else {
        app.group_summaries.clone()
    };

    let list_top = pg_rect.top() + 36.0 * s;
    let list_bot = pg_rect.bottom() - 10.0 * s;
    let avail_h = (list_bot - list_top).max(100.0);
    let item_gap = 6.0 * s;
    let max_fit = ((avail_h + item_gap) / (36.0 * s + item_gap))
        .floor()
        .max(3.0) as usize;
    let vis_cnt = summaries.len().min(max_fit).max(1);
    // 自适应均分卡片垂直空间，使最后一个策略组项下边缘刚好对齐 list_bot (bottom - 10*s)
    let box_h = ((avail_h - item_gap * (vis_cnt as f32 - 1.0)) / vis_cnt as f32)
        .clamp(34.0 * s, 52.0 * s);

    let mut clicked_group = false;
    let badge_letters = ["P", "A", "S", "J", "U", "H", "G", "F"];
    for (g_i, (g_name, g_now)) in summaries.iter().take(vis_cnt).enumerate() {
        let gy = list_top + g_i as f32 * (box_h + item_gap);
        if gy + box_h - 2.0 > pg_rect.bottom() {
            break;
        }
        let g_box = egui::Rect::from_min_size(
            egui::pos2(pg_rect.left() + 10.0 * s, gy),
            egui::vec2(pg_rect.width() - 20.0 * s, box_h),
        );
        let g_resp = ui.interact(
            g_box,
            ui.id().with(("dash_pg_row", g_i)),
            egui::Sense::click(),
        );
        if g_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        pgp.rect_filled(
            g_box,
            9.0 * s,
            if g_resp.hovered() {
                pal.surface_hover
            } else {
                pal.surface_container
            },
        );

        let avatar_c = egui::pos2(g_box.left() + 18.0 * s, g_box.center().y);
        pgp.circle_filled(avatar_c, 10.5 * s, pal.bg_card);
        let badge_ltr = badge_letters.get(g_i).copied().unwrap_or("G");
        pgp.text(
            avatar_c,
            egui::Align2::CENTER_CENTER,
            badge_ltr,
            cup_font_med(10.0 * s),
            pal.primary,
        );

        let text_left = g_box.left() + 35.0 * s;
        let max_text_w = (g_box.right() - 10.0 * s - text_left).max(40.0);
        let g_title = ellipsize_text(
            ui,
            &format_cupertino_title(g_name),
            cup_font_med(11.2 * s),
            max_text_w,
        );
        pgp.text(
            egui::pos2(text_left, g_box.top() + box_h * 0.34),
            egui::Align2::LEFT_CENTER,
            g_title,
            cup_font_med(11.2 * s),
            pal.text_primary,
        );

        let flag = match_flag(g_now);
        let mut sub_x = text_left;
        let sub_y = g_box.bottom() - box_h * 0.28;
        if flag != "DIR" && flag != "AUTO" && flag != "NET" {
            let f_r = egui::Rect::from_center_size(
                egui::pos2(sub_x + 6.5 * s, sub_y),
                egui::vec2(12.5 * s, 9.2 * s),
            );
            draw_flag_icon(&pgp, f_r, flag, pal);
            sub_x += 16.5 * s;
        }
        let g_sub = ellipsize_text(
            ui,
            &format_cupertino_title(g_now),
            cup_font_reg(10.0 * s),
            (g_box.right() - 10.0 * s - sub_x).max(28.0),
        );
        pgp.text(
            egui::pos2(sub_x, sub_y),
            egui::Align2::LEFT_CENTER,
            g_sub,
            cup_font_reg(10.0 * s),
            pal.text_muted,
        );

        if g_resp.clicked() {
            if app.groups.contains(g_name) {
                app.group_name = g_name.clone();
                app.group_picked_by_user = true;
            }
            app.nav_tab = NavTab::Proxies;
            clicked_group = true;
        }
    }
    if pg_resp.clicked() && !clicked_group {
        app.nav_tab = NavTab::Proxies;
    }
}
