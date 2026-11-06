use eframe::egui;

use crate::config_patch::{
    days_until_expire, format_bytes, format_speed, format_unix_date, match_flag,
};
use crate::core_api::close_all_connections;
use crate::platform::set_autostart;
use crate::state::{AppState, Subscription};
use crate::theme::{
    cup_font_med, cup_font_reg, cupertino_button, draw_bento_card, draw_cup_icon, draw_flag_icon,
    draw_sf_checkmark, draw_wave_progress, ellipsize_text, format_cupertino_title, md3_switch,
    CupBtnStyle, CupIcon, ThemePalette,
};

#[allow(clippy::too_many_arguments)]
pub fn render_profiles_tab(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    action_update_sub: &mut Option<Subscription>,
    action_delete_sub: &mut Option<String>,
    action_switch_sub: &mut Option<String>,
) {
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(6.0, 0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let title_g = ui.fonts(|f| {
                    f.layout_no_wrap("配置".to_string(), cup_font_med(19.5), pal.text_primary)
                });
                let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
                ui.painter().galley(t_rect.min, title_g, pal.text_primary);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cupertino_button(
                        ui,
                        "导入新订阅",
                        Some(CupIcon::Plus),
                        CupBtnStyle::Filled(pal.primary),
                        true,
                        pal,
                    ) {
                        app.show_import_dialog = true;
                    }
                });
            });
        });

    ui.add_space(10.0);

    let subs = app.profiles.subscriptions.clone();
    let active_id = app.profiles.active.clone();

    if subs.is_empty() {
        let (empty_rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 148.0), egui::Sense::hover());
        let ep = ui.painter().clone();
        draw_bento_card(&ep, empty_rect, 16.0, pal.bg_card, pal);
        ep.text(
            empty_rect.center(),
            egui::Align2::CENTER_CENTER,
            "暂无订阅配置，点击右上角「导入新订阅」添加机场链接",
            cup_font_reg(13.0),
            pal.text_secondary,
        );
        return;
    }

    let subs_len = subs.len();
    egui::ScrollArea::vertical()
        .id_salt("profiles_list_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                .show(ui, |ui| {
                    for sub in subs {
                let is_act = sub.id == active_id;
                let (c_rect, c_resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 112.0),
                    egui::Sense::click(),
                );
                if c_resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let cp = ui.painter().clone();
                let card_fill = if c_resp.hovered() && !is_act {
                    pal.bg_card_hover
                } else {
                    pal.bg_card
                };
                draw_bento_card(&cp, c_rect, 16.0, card_fill, pal);
                let text_clip_r = egui::Rect::from_min_max(
                    c_rect.min,
                    egui::pos2((c_rect.right() - 165.0).max(c_rect.left() + 80.0), c_rect.bottom()),
                );
                let cp_txt = cp.with_clip_rect(text_clip_r);

                if is_act {
                    cp.rect_stroke(
                        c_rect,
                        16.0,
                        egui::Stroke::new(
                            1.5_f32,
                            egui::Color32::from_rgba_unmultiplied(
                                pal.primary.r(),
                                pal.primary.g(),
                                pal.primary.b(),
                                115,
                            ),
                        ),
                    );
                    let chk_center = egui::pos2(c_rect.right() - 154.0, c_rect.center().y);
                    cp.circle_filled(chk_center, 9.0, pal.primary);
                    draw_sf_checkmark(&cp, chk_center, 0.74, egui::Color32::WHITE);
                }

                // Row 1: 订阅名称 + 当前使用徽章 + 剩余天数标签
                let title_str = format_cupertino_title(&sub.name);
                let name_galley = ui.fonts(|f| {
                    f.layout_no_wrap(title_str, cup_font_med(15.0), pal.text_primary)
                });
                let name_w = name_galley.size().x;
                cp_txt.galley(
                    egui::pos2(c_rect.left() + 20.0, c_rect.top() + 15.0),
                    name_galley,
                    pal.text_primary,
                );

                let mut badge_x = c_rect.left() + 28.0 + name_w;
                if is_act {
                    let badge_rect = egui::Rect::from_min_size(
                        egui::pos2(badge_x, c_rect.top() + 15.0),
                        egui::vec2(56.0, 20.0),
                    );
                    cp_txt.rect_filled(badge_rect, 10.0, pal.bg_selected);
                    cp_txt.text(
                        badge_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "当前使用",
                        cup_font_med(10.5),
                        pal.primary,
                    );
                    badge_x += 64.0;
                }

                if let Some(days) = days_until_expire(sub.expire) {
                    let day_txt = format!("剩余 {} 天", days);
                    let day_g = ui.fonts(|f| {
                        f.layout_no_wrap(day_txt, cup_font_med(10.5), pal.text_secondary)
                    });
                    let day_rect = egui::Rect::from_min_size(
                        egui::pos2(badge_x, c_rect.top() + 15.0),
                        egui::vec2(day_g.size().x + 14.0, 20.0),
                    );
                    cp_txt.rect_filled(day_rect, 10.0, pal.surface_container);
                    cp_txt.galley(
                        egui::pos2(
                            day_rect.left() + 7.0,
                            day_rect.center().y - day_g.size().y * 0.5,
                        ),
                        day_g,
                        pal.text_secondary,
                    );
                }

                // Row 2: 百分比 + 已用 / 未用（剩余） / 总计流量 + FlClash 正弦波浪进度条 `~~~~~~~`
                let bar_max_w = (c_rect.width() - 210.0).clamp(140.0, 480.0);
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(c_rect.left() + 20.0, c_rect.top() + 66.0),
                    egui::vec2(bar_max_w, 8.0),
                );

                let track_col = if pal.dark {
                    egui::Color32::from_rgb(52, 57, 68)
                } else {
                    egui::Color32::from_rgb(225, 233, 245)
                };

                if sub.total > 0 {
                    let used = sub.upload.saturating_add(sub.download);
                    let unused = sub.total.saturating_sub(used);
                    let ratio = (used as f32 / sub.total as f32).clamp(0.0, 1.0);
                    let pct_str = format!("{:.1}% 已用", ratio * 100.0);
                    let pct_g = ui.fonts(|f| {
                        f.layout_no_wrap(pct_str, cup_font_med(12.5), pal.primary)
                    });
                    let pct_w = pct_g.size().x;
                    cp_txt.galley(
                        egui::pos2(c_rect.left() + 20.0, c_rect.top() + 43.0),
                        pct_g,
                        pal.primary,
                    );

                    let detail_txt = format!(
                        "已用 {}   /   未用 {}   （共 {}）",
                        format_bytes(used),
                        format_bytes(unused),
                        format_bytes(sub.total)
                    );
                    cp_txt.text(
                        egui::pos2(c_rect.left() + 30.0 + pct_w, c_rect.top() + 51.0),
                        egui::Align2::LEFT_CENTER,
                        detail_txt,
                        cup_font_med(12.0),
                        pal.text_primary,
                    );

                    let wave_col = if ratio > 0.85 { pal.orange } else { pal.primary };
                    draw_wave_progress(&cp, bar_rect, ratio.max(0.05), wave_col, track_col);
                } else if !sub.remaining_hint.is_empty() {
                    let traffic_txt = format!(
                        "未用（剩余流量）: {}   ·   点击右侧「更新订阅」同步精确已用/总量",
                        sub.remaining_hint
                    );
                    cp_txt.text(
                        egui::pos2(c_rect.left() + 20.0, c_rect.top() + 51.0),
                        egui::Align2::LEFT_CENTER,
                        traffic_txt,
                        cup_font_med(12.0),
                        pal.primary,
                    );
                    draw_wave_progress(&cp, bar_rect, 0.25, pal.primary, track_col);
                } else {
                    cp_txt.text(
                        egui::pos2(c_rect.left() + 20.0, c_rect.top() + 51.0),
                        egui::Align2::LEFT_CENTER,
                        "已用 -- / 未用 --   （点击右侧「更新订阅」拉取流量配额）",
                        cup_font_reg(12.0),
                        pal.text_secondary,
                    );
                    draw_wave_progress(&cp, bar_rect, 0.0, pal.primary, track_col);
                }

                // Row 3: 到期时间 + 最后同步时间
                let exp_str = if sub.expire > 0 {
                    format!("到期: {}", format_unix_date(sub.expire))
                } else if !sub.expire_hint.is_empty() {
                    format!("到期: {}", sub.expire_hint)
                } else {
                    "到期: 长期有效".to_string()
                };
                cp_txt.text(
                    egui::pos2(c_rect.left() + 20.0, c_rect.bottom() - 17.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{}   ·   最后同步: {}", exp_str, sub.updated_at),
                    cup_font_reg(11.5),
                    pal.text_secondary,
                );

                let btn_right_offset = if subs_len > 1 { 92.0 } else { 58.0 };
                let btn_rect = egui::Rect::from_center_size(
                    egui::pos2(c_rect.right() - btn_right_offset, c_rect.center().y),
                    egui::vec2(84.0, 32.0),
                );
                let mut b_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
                let upd_txt = if app.is_updating_sub {
                    "同步中…"
                } else {
                    "更新订阅"
                };
                let mut sub_btn_clicked = false;
                if cupertino_button(
                    &mut b_ui,
                    upd_txt,
                    Some(CupIcon::Refresh),
                    CupBtnStyle::Tinted(pal.primary),
                    !app.is_updating_sub,
                    pal,
                ) {
                    *action_update_sub = Some(sub.clone());
                    sub_btn_clicked = true;
                }

                if subs_len > 1 {
                    let del_rect = egui::Rect::from_center_size(
                        egui::pos2(c_rect.right() - 28.0, c_rect.center().y),
                        egui::vec2(28.0, 28.0),
                    );
                    let del_resp = ui.interact(
                        del_rect,
                        ui.id().with(("del_sub", &sub.id)),
                        egui::Sense::click(),
                    );
                    if del_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        cp.circle_filled(del_rect.center(), 14.0, pal.surface_hover);
                    }
                    draw_cup_icon(
                        &cp,
                        CupIcon::Close,
                        del_rect.center(),
                        if del_resp.hovered() {
                            pal.red
                        } else {
                            pal.text_tertiary
                        },
                    );
                    if del_resp.clicked() {
                        *action_delete_sub = Some(sub.id.clone());
                        sub_btn_clicked = true;
                    }
                }

                if c_resp.clicked() && !is_act && !sub_btn_clicked {
                    *action_switch_sub = Some(sub.id.clone());
                }
                ui.add_space(12.0);
            }
                });
        });
}

pub fn render_connections_tab(ui: &mut egui::Ui, app: &mut AppState, pal: &ThemePalette) {
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(6.0, 0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let title_g = ui.fonts(|f| {
                    f.layout_no_wrap(
                        format!("实时连接 ({})", app.active_connections),
                        cup_font_med(19.5),
                        pal.text_primary,
                    )
                });
                let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
                ui.painter().galley(t_rect.min, title_g, pal.text_primary);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cupertino_button(
                        ui,
                        "断开全部旧连接",
                        Some(CupIcon::Close),
                        CupBtnStyle::Tinted(pal.primary),
                        true,
                        pal,
                    ) {
                        close_all_connections();
                        app.status_msg = "已清理全部活跃长连接".to_string();
                    }
                });
            });
        });

    ui.add_space(8.0);

    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(6.0, 6.0))
        .show(ui, |ui| {
            // Top Summary Bento Card
            let (c_rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 80.0), egui::Sense::hover());
            let cp = ui.painter().clone();
            draw_bento_card(&cp, c_rect, 16.0, pal.bg_card, pal);
            cp.text(
                egui::pos2(c_rect.left() + 20.0, c_rect.top() + 26.0),
                egui::Align2::LEFT_CENTER,
                format!(
                    "实时下行：{}   ·   实时上行：{}   ·   累计下载：{}   ·   累计上传：{}",
                    format_speed(app.download_rate),
                    format_speed(app.upload_rate),
                    format_bytes(app.total_download),
                    format_bytes(app.total_upload)
                ),
                cup_font_med(13.5),
                pal.text_primary,
            );
            cp.text(
                egui::pos2(c_rect.left() + 20.0, c_rect.bottom() - 22.0),
                egui::Align2::LEFT_CENTER,
                format!(
                    "当前出站节点：{}   ·   切换节点自动清理旧连接已启用",
                    format_cupertino_title(&app.current_selected)
                ),
                cup_font_reg(11.5),
                pal.text_secondary,
            );

            ui.add_space(14.0);

            // Live Connections Table Bento Card — 自动撑满剩余视口高度并根据高度动态渲染更多连接行
            let table_h = (ui.available_height() - 4.0).max(200.0);
            let (t_card, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), table_h),
                egui::Sense::hover(),
            );
            let tp = ui.painter().clone();
            draw_bento_card(&tp, t_card, 16.0, pal.bg_card, pal);

            let col_x = [
                t_card.left() + 20.0,
                t_card.left() + t_card.width() * 0.21,
                t_card.left() + t_card.width() * 0.54,
                t_card.left() + t_card.width() * 0.78,
                t_card.left() + t_card.width() * 0.90,
            ];
            let headers = [
                "来源进程 / 网络",
                "目标地址 (Host:Port)",
                "出口策略链",
                "下载流量",
                "上传流量",
            ];
            for (hi, h_lbl) in headers.iter().enumerate() {
                tp.text(
                    egui::pos2(col_x[hi], t_card.top() + 22.0),
                    egui::Align2::LEFT_CENTER,
                    *h_lbl,
                    cup_font_med(11.5),
                    pal.text_secondary,
                );
            }
            tp.line_segment(
                [
                    egui::pos2(t_card.left() + 18.0, t_card.top() + 38.0),
                    egui::pos2(t_card.right() - 18.0, t_card.top() + 38.0),
                ],
                egui::Stroke::new(0.8_f32, pal.border),
            );

            if app.live_connections.is_empty() {
                tp.text(
                    egui::pos2(t_card.center().x, t_card.center().y + 10.0),
                    egui::Align2::CENTER_CENTER,
                    "当前暂无活跃连接",
                    cup_font_reg(12.5),
                    pal.text_secondary,
                );
            } else {
                let max_rows = ((table_h - 54.0) / 36.0).floor().max(5.0) as usize;
                for (ri, item) in app.live_connections.iter().take(max_rows).enumerate() {
                    let ry = t_card.top() + 58.0 + ri as f32 * 36.0;
                    if ry + 14.0 > t_card.bottom() {
                        break;
                    }
                    let badge_r = egui::Rect::from_center_size(
                        egui::pos2(col_x[0] + 9.0, ry),
                        egui::vec2(18.0, 18.0),
                    );
                    tp.rect_filled(badge_r, 5.0, pal.surface_container);
                    let ch = item
                        .source
                        .chars()
                        .next()
                        .unwrap_or('N')
                        .to_ascii_uppercase();
                    tp.text(
                        badge_r.center(),
                        egui::Align2::CENTER_CENTER,
                        ch.to_string(),
                        cup_font_med(9.5),
                        pal.primary,
                    );
                    let src_txt = ellipsize_text(
                        ui,
                        &item.source,
                        cup_font_med(12.0),
                        (col_x[1] - (col_x[0] + 24.0) - 12.0).max(32.0),
                    );
                    tp.text(
                        egui::pos2(col_x[0] + 24.0, ry),
                        egui::Align2::LEFT_CENTER,
                        src_txt,
                        cup_font_med(12.0),
                        pal.text_primary,
                    );
                    let dst_txt = ellipsize_text(
                        ui,
                        &item.target,
                        cup_font_reg(12.0),
                        (col_x[2] - col_x[1] - 14.0).max(48.0),
                    );
                    tp.text(
                        egui::pos2(col_x[1], ry),
                        egui::Align2::LEFT_CENTER,
                        dst_txt,
                        cup_font_reg(12.0),
                        pal.text_secondary,
                    );
                    let ext_flag = match_flag(&item.exit);
                    let mut ext_x = col_x[2];
                    if !matches!(ext_flag, "DIR" | "AUTO" | "FAIL" | "NET") {
                        let f_rect = egui::Rect::from_center_size(
                            egui::pos2(ext_x + 7.0, ry),
                            egui::vec2(13.0, 9.5),
                        );
                        draw_flag_icon(&tp, f_rect, ext_flag, pal);
                        ext_x += 18.0;
                    }
                    let ext_txt = ellipsize_text(
                        ui,
                        &format_cupertino_title(&item.exit),
                        cup_font_reg(12.0),
                        (col_x[3] - ext_x - 14.0).max(36.0),
                    );
                    tp.text(
                        egui::pos2(ext_x, ry),
                        egui::Align2::LEFT_CENTER,
                        ext_txt,
                        cup_font_reg(12.0),
                        pal.text_secondary,
                    );
                    tp.text(
                        egui::pos2(col_x[3], ry),
                        egui::Align2::LEFT_CENTER,
                        format_bytes(item.download),
                        cup_font_med(11.5),
                        pal.text_primary,
                    );
                    tp.text(
                        egui::pos2(col_x[4], ry),
                        egui::Align2::LEFT_CENTER,
                        format_bytes(item.upload),
                        cup_font_med(11.5),
                        pal.text_secondary,
                    );
                }
            }
        });
}

pub fn render_tools_tab(
    ui: &mut egui::Ui,
    app: &mut AppState,
    pal: &ThemePalette,
    toggle_sys_proxy: &mut Option<bool>,
    toggle_tun: &mut Option<bool>,
    action_trim_mem: &mut bool,
) {
    egui::Frame::none()
        .inner_margin(egui::Margin::symmetric(6.0, 0.0))
        .show(ui, |ui| {
            let title_g = ui.fonts(|f| {
                f.layout_no_wrap("工具".to_string(), cup_font_med(19.5), pal.text_primary)
            });
            let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
            ui.painter().galley(t_rect.min, title_g, pal.text_primary);
        });
    ui.add_space(10.0);

    egui::ScrollArea::vertical()
        .id_salt("tools_list_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                .show(ui, |ui| {
            // 内存监控与闲置回收卡片
            {
                let (m_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 74.0),
                    egui::Sense::hover(),
                );
                let mp = ui.painter().clone();
                draw_bento_card(&mp, m_rect, 15.0, pal.bg_card, pal);
                let mp_txt = mp.with_clip_rect(egui::Rect::from_min_max(
                    m_rect.min,
                    egui::pos2((m_rect.right() - 128.0).max(m_rect.left() + 80.0), m_rect.bottom()),
                ));
                let gui_mb = app.mem_gui_kb as f64 / 1024.0;
                let gui_rss_mb = app.mem_gui_rss_kb as f64 / 1024.0;
                let core_mb = app.mem_core_kb as f64 / 1024.0;
                let core_rss_mb = app.mem_core_rss_kb as f64 / 1024.0;
                let tot_mb = (gui_mb + core_mb).max(1.0);
                mp_txt.text(
                    egui::pos2(m_rect.left() + 20.0, m_rect.top() + 24.0),
                    egui::Align2::LEFT_CENTER,
                    format!(
                        "内存占用监控：总计 {:.1} MB   （界面私有堆 {:.1} MB / 内核 {:.1} MB）",
                        tot_mb, gui_mb, core_mb
                    ),
                    cup_font_med(13.5),
                    pal.text_primary,
                );
                mp_txt.text(
                    egui::pos2(m_rect.left() + 20.0, m_rect.bottom() - 21.0),
                    egui::Align2::LEFT_CENTER,
                    format!(
                        "含共享显存/字体 RSS：界面 {:.1} MB · 内核 {:.1} MB   |   系统物理内存：{:.1} / {:.1} GB",
                        gui_rss_mb, core_rss_mb, app.mem_sys_used_gb, app.mem_sys_total_gb
                    ),
                    cup_font_reg(11.5),
                    pal.text_secondary,
                );
                let btn_r = egui::Rect::from_center_size(
                    egui::pos2(m_rect.right() - 68.0, m_rect.center().y),
                    egui::vec2(104.0, 32.0),
                );
                let mut b_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_r));
                if cupertino_button(
                    &mut b_ui,
                    "释放闲置内存",
                    Some(CupIcon::Refresh),
                    CupBtnStyle::Tinted(pal.primary),
                    true,
                    pal,
                ) {
                    *action_trim_mem = true;
                }
                ui.add_space(12.0);
            }

            let draw_setting_row =
                |ui: &mut egui::Ui, title: &str, subtitle: &str, val: bool| -> bool {
                    let (r_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 68.0),
                        egui::Sense::hover(),
                    );
                    let rp = ui.painter().clone();
                    draw_bento_card(&rp, r_rect, 15.0, pal.bg_card, pal);
                    let rp_txt = rp.with_clip_rect(egui::Rect::from_min_max(
                        r_rect.min,
                        egui::pos2((r_rect.right() - 72.0).max(r_rect.left() + 80.0), r_rect.bottom()),
                    ));
                    rp_txt.text(
                        egui::pos2(r_rect.left() + 20.0, r_rect.top() + 23.0),
                        egui::Align2::LEFT_CENTER,
                        title,
                        cup_font_med(13.5),
                        pal.text_primary,
                    );
                    rp_txt.text(
                        egui::pos2(r_rect.left() + 20.0, r_rect.bottom() - 20.0),
                        egui::Align2::LEFT_CENTER,
                        subtitle,
                        cup_font_reg(11.5),
                        pal.text_secondary,
                    );
                    let sw_rect = egui::Rect::from_center_size(
                        egui::pos2(r_rect.right() - 40.0, r_rect.center().y),
                        egui::vec2(44.0, 24.0),
                    );
                    let mut sw_ui = ui.new_child(egui::UiBuilder::new().max_rect(sw_rect));
                    let clicked = md3_switch(&mut sw_ui, val, pal);
                    ui.add_space(12.0);
                    clicked
                };

            if draw_setting_row(
                ui,
                "系统代理",
                "将桌面 HTTP / HTTPS / SOCKS5 系统代理指向 127.0.0.1:7890",
                app.is_system_proxy_on,
            ) {
                *toggle_sys_proxy = Some(!app.is_system_proxy_on);
            }

            if draw_setting_row(
                ui,
                "虚拟网卡 (TUN 模式)",
                "开启 Mihomo 虚拟网卡透明接管全局三层流量",
                app.profiles.tun,
            ) {
                *toggle_tun = Some(!app.profiles.tun);
            }

            if draw_setting_row(
                ui,
                "局域网共享 (Allow LAN)",
                "允许同一局域网下的其他设备通过本机内网 IP 连接代理端口",
                app.profiles.allow_lan,
            ) {
                app.profiles.allow_lan = !app.profiles.allow_lan;
                app.save_profiles();
            }

            if draw_setting_row(
                ui,
                "切换节点时自动打断旧连接 (FlClash closeConnections)",
                "切换节点或策略组后立即清理残留长连接，网页与视频秒切不卡顿",
                app.profiles.close_connections,
            ) {
                app.profiles.close_connections = !app.profiles.close_connections;
                app.save_profiles();
            }

            if draw_setting_row(
                ui,
                "深色外观主题",
                "开启后使用深色配色，关闭则使用 FlClash 浅色弥散投影主题",
                app.profiles.dark_mode,
            ) {
                app.profiles.dark_mode = !app.profiles.dark_mode;
                app.save_profiles();
            }

            if draw_setting_row(
                ui,
                "关闭窗口后后台常驻",
                "关闭主界面窗口后保持 Mihomo 内核与系统托盘继续运行",
                app.profiles.keep_core_on_exit,
            ) {
                app.profiles.keep_core_on_exit = !app.profiles.keep_core_on_exit;
                app.save_profiles();
            }

            let cur_auto = app.profiles.autostart;
            if draw_setting_row(
                ui,
                "开机自动启动",
                "登录桌面时由 systemd 用户服务在后台静默守护运行",
                cur_auto,
            ) {
                let next_auto = !cur_auto;
                match set_autostart(next_auto) {
                    Ok(()) => {
                        app.profiles.autostart = next_auto;
                        if next_auto {
                            app.profiles.keep_core_on_exit = true;
                        }
                        app.save_profiles();
                        app.status_msg = if next_auto {
                            "已开启开机自启".to_string()
                        } else {
                            "已关闭开机自启".to_string()
                        };
                    }
                    Err(e) => {
                        app.status_msg = format!("设置自启失败: {}", e);
                    }
                }
            }
                });
        });
}
