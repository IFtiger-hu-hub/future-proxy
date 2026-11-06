use eframe::egui;

use crate::config_patch::match_flag;
use crate::state::{AppState, ProxyNode};
use crate::theme::*;

pub fn render_proxies_tab(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut AppState,
    pal: &ThemePalette,
    action_ping: &mut bool,
    action_select_node: &mut Option<String>,
    hover_tooltip: &mut Option<(egui::Pos2, &'static str)>,
) {
    // --- Row 1: Title "代理" on Left + Unified White Pill `[ 🔍  ⚡ ]` & Circular `( ⋮ )` on Right ---
    ui.horizontal(|ui| {
        let title_g = ui.fonts(|f| {
            f.layout_no_wrap("代理".to_string(), cup_font_med(19.5), pal.text_primary)
        });
        let (t_rect, _) = ui.allocate_exact_size(title_g.size(), egui::Sense::hover());
        ui.painter().galley(t_rect.min, title_g, pal.text_primary);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // 1. Rightmost Circular White `( ⋮ )` More Button (38x38)
            let is_more_open = app.open_popup.as_deref() == Some("more");
            let (more_rect, more_resp) =
                ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::click());
            if more_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if !is_more_open {
                    *hover_tooltip = Some((
                        egui::pos2(more_rect.center().x, more_rect.bottom() + 14.0),
                        "更多",
                    ));
                }
            }
            let more_scale = interact_spring_scale(
                ctx,
                more_resp.id,
                more_resp.is_pointer_button_down_on(),
                more_resp.hovered(),
            );
            let mr = scale_rect(more_rect, more_scale);
            let mp = ui.painter();
            if !pal.dark {
                mp.circle_filled(mr.center() + egui::vec2(0.0, 1.5), 19.0, pal.shadow);
            }
            mp.circle_filled(
                mr.center(),
                19.0,
                if more_resp.hovered() {
                    pal.bg_card_hover
                } else {
                    pal.bg_card
                },
            );
            draw_cup_icon(&mp, CupIcon::MoreVert, mr.center(), pal.text_primary);
            if more_resp.clicked() {
                if is_more_open {
                    app.open_popup = None;
                    app.more_submenu = None;
                } else {
                    app.open_popup = Some("more".to_string());
                    app.more_submenu = None;
                    app.popup_anchor = egui::pos2(more_rect.right(), more_rect.top());
                    app.popup_width = 148.0;
                    app.popup_open_time = ctx.input(|i| i.time);
                }
            }

            // 2. Unified White Capsule `[ 🔍   ⚡ ]` (Morphs wider when search is active!)
            let search_active = app.search_expanded || !app.search_keyword.is_empty();
            let search_t =
                ctx.animate_bool_with_time(ui.id().with("fl_search_capsule"), search_active, 0.24);
            let max_cap_w = (ui.available_width() - 60.0).clamp(110.0, 240.0);
            let cap_w = 78.0 + (max_cap_w - 78.0) * ease_emphasized(search_t);
            let (cap_rect, _) =
                ui.allocate_exact_size(egui::vec2(cap_w, 38.0), egui::Sense::hover());
            let cp = ui.painter().clone();
            if !pal.dark {
                cp.rect_filled(
                    cap_rect.translate(egui::vec2(0.0, 1.5)),
                    19.0,
                    pal.shadow,
                );
            }
            cp.rect_filled(cap_rect, 19.0, pal.bg_card);

            // Right half of capsule: Bolt `⚡` Speed Test button
            let bolt_rect = egui::Rect::from_min_max(
                egui::pos2(cap_rect.right() - 39.0, cap_rect.top()),
                cap_rect.right_bottom(),
            );
            let bolt_resp =
                ui.interact(bolt_rect, ui.id().with("fl_cap_bolt"), egui::Sense::click());
            if bolt_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                cp.circle_filled(bolt_rect.center(), 16.0, pal.surface_hover);
                *hover_tooltip = Some((
                    egui::pos2(bolt_rect.center().x, bolt_rect.bottom() + 14.0),
                    "测速",
                ));
            }
            let bolt_scale = interact_spring_scale(
                ctx,
                bolt_resp.id,
                bolt_resp.is_pointer_button_down_on(),
                bolt_resp.hovered(),
            );
            draw_cup_icon(
                &cp,
                CupIcon::Bolt,
                scale_rect(bolt_rect, bolt_scale).center(),
                if app.is_testing_delay {
                    pal.primary
                } else {
                    pal.text_primary
                },
            );
            if bolt_resp.clicked() && !app.is_testing_delay && app.is_core_alive {
                *action_ping = true;
            }

            // Left part of capsule: Search icon `🔍` or expanded input
            if search_t < 0.05 {
                let srch_rect = egui::Rect::from_min_max(
                    cap_rect.left_top(),
                    egui::pos2(cap_rect.left() + 39.0, cap_rect.bottom()),
                );
                let srch_resp =
                    ui.interact(srch_rect, ui.id().with("fl_cap_srch"), egui::Sense::click());
                if srch_resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    cp.circle_filled(srch_rect.center(), 16.0, pal.surface_hover);
                    *hover_tooltip = Some((
                        egui::pos2(srch_rect.center().x, srch_rect.bottom() + 14.0),
                        "搜索",
                    ));
                }
                let srch_scale = interact_spring_scale(
                    ctx,
                    srch_resp.id,
                    srch_resp.is_pointer_button_down_on(),
                    srch_resp.hovered(),
                );
                draw_cup_icon(
                    &cp,
                    CupIcon::Search,
                    scale_rect(srch_rect, srch_scale).center(),
                    pal.text_primary,
                );
                if srch_resp.clicked() {
                    app.search_expanded = true;
                }
            } else {
                draw_cup_icon(
                    &cp,
                    CupIcon::Search,
                    egui::pos2(cap_rect.left() + 19.0, cap_rect.center().y),
                    pal.primary,
                );
                if cap_w > 120.0 {
                    let edit_rect = egui::Rect::from_min_max(
                        egui::pos2(cap_rect.left() + 34.0, cap_rect.top() + 7.0),
                        egui::pos2(bolt_rect.left() - 24.0, cap_rect.bottom() - 7.0),
                    );
                    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
                    child_ui.add(
                        egui::TextEdit::singleline(&mut app.search_keyword)
                            .frame(false)
                            .font(cup_font_reg(12.5))
                            .text_color(pal.text_primary)
                            .hint_text(
                                egui::RichText::new("搜索节点…")
                                    .color(pal.text_tertiary)
                                    .size(12.0),
                            )
                            .desired_width(edit_rect.width()),
                    );
                    let cls_rect = egui::Rect::from_center_size(
                        egui::pos2(bolt_rect.left() - 12.0, cap_rect.center().y),
                        egui::vec2(20.0, 20.0),
                    );
                    let cls_resp =
                        ui.interact(cls_rect, ui.id().with("fl_srch_cls"), egui::Sense::click());
                    if cls_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    draw_cup_icon(&cp, CupIcon::Close, cls_rect.center(), pal.text_secondary);
                    if cls_resp.clicked() {
                        app.search_keyword.clear();
                        app.search_expanded = false;
                    }
                }
            }
        });
    });

    ui.add_space(12.0);

    // --- Row 2: FlClash Strategy Group Capsule Pills (e.g. `[ 节点选择 ]` in Solid Blue) ---
    let display_groups: Vec<String> = if app.groups.is_empty() {
        vec!["节点选择".to_string()]
    } else {
        app.groups.clone()
    };
    let active_group = if app.group_name.is_empty() {
        display_groups[0].clone()
    } else {
        app.group_name.clone()
    };

    egui::ScrollArea::horizontal()
        .id_salt("group_tabs_scroll")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for g_name in display_groups {
                    let is_cur = g_name == active_group;
                    let clean_g = format_cupertino_title(&g_name);
                    let g_galley = ui.fonts(|f| {
                        f.layout_no_wrap(clean_g.clone(), cup_font_med(12.5), pal.text_primary)
                    });
                    let tab_w = (g_galley.size().x + 26.0).max(74.0);
                    let (tab_rect, tab_resp) =
                        ui.allocate_exact_size(egui::vec2(tab_w, 32.0), egui::Sense::click());
                    if tab_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let scale = interact_spring_scale(
                        ctx,
                        tab_resp.id,
                        tab_resp.is_pointer_button_down_on(),
                        tab_resp.hovered(),
                    );
                    let tr = scale_rect(tab_rect, scale);
                    let tp = ui.painter();
                    if is_cur {
                        tp.rect_filled(tr, 16.0, pal.primary);
                    } else if tab_resp.hovered() {
                        tp.rect_filled(tr, 16.0, pal.surface_hover);
                    } else {
                        tp.rect_filled(tr, 16.0, pal.surface_container);
                    }
                    let fg = if is_cur {
                        pal.on_primary
                    } else {
                        pal.text_secondary
                    };
                    tp.text(
                        tr.center(),
                        egui::Align2::CENTER_CENTER,
                        clean_g,
                        cup_font_med(12.5),
                        fg,
                    );
                    if tab_resp.clicked() && !is_cur {
                        app.group_name = g_name;
                        app.group_picked_by_user = true;
                        app.nodes.clear();
                    }
                }
            });
        });

    ui.add_space(12.0);

    // --- Row 3: FlClash 6-Column Pure White Node Cards Grid ---
    let keyword = app.search_keyword.to_lowercase();
    let hide_timeout = app.profiles.hide_timeout;
    let sort_mode = app.profiles.sort_mode.clone();

    let mut filtered_nodes: Vec<ProxyNode> = app
        .nodes
        .iter()
        .filter(|n| {
            if hide_timeout && n.delay == 0 && !n.is_selected {
                return false;
            }
            keyword.is_empty()
                || n.name.to_lowercase().contains(&keyword)
                || n.proto.to_lowercase().contains(&keyword)
        })
        .cloned()
        .collect();

    if sort_mode == "delay" {
        filtered_nodes.sort_by_key(|n| match n.delay {
            d if d > 0 => (0, d),
            -1 => (1, 0),
            _ => (2, 0),
        });
    } else if sort_mode == "name" {
        filtered_nodes.sort_by(|a, b| a.name.cmp(&b.name));
    }

    if app.profiles.subscriptions.is_empty() {
        let (empty_rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 130.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(empty_rect.shrink(1.0), 12.0, pal.bg_card);
        ui.painter().text(
            egui::pos2(empty_rect.center().x, empty_rect.center().y - 12.0),
            egui::Align2::CENTER_CENTER,
            "尚未添加任何订阅配置",
            cup_font_med(14.0),
            pal.text_primary,
        );
        ui.painter().text(
            egui::pos2(empty_rect.center().x, empty_rect.center().y + 12.0),
            egui::Align2::CENTER_CENTER,
            "请点击左侧导航栏「配置」导入您的 Clash / Mihomo 订阅链接",
            cup_font_reg(12.0),
            pal.text_secondary,
        );
    }

    // Card dimensions calibrated so 6 columns fit cleanly
    let (min_col_w, card_h) = match app.profiles.card_size.as_str() {
        "loose" => (178.0_f32, 62.0_f32),
        "standard" => (136.0_f32, 56.0_f32),
        _ => (132.0_f32, 54.0_f32), // "compact" default (6 columns on 1060px window)
    };

    let scroll_to_sel = app.scroll_to_selected;
    app.scroll_to_selected = false;

    egui::ScrollArea::vertical()
        .id_salt("proxies_grid_scroll")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            egui::Frame::none()
                .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                .show(ui, |ui| {
                    let gap = 12.0_f32;
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    let avail = ui.available_width();
                    let cols = ((avail + gap) / (min_col_w + gap)).floor().clamp(2.0, 12.0) as usize;
                    let card_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;

                    for row in filtered_nodes.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for node in row {
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(card_w, card_h),
                                    egui::Sense::click(),
                                );
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if scroll_to_sel && node.is_selected {
                                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                }

                                let is_sel = node.is_selected;
                                let card_scale = interact_spring_scale(
                                    ctx,
                                    resp.id,
                                    resp.is_pointer_button_down_on(),
                                    resp.hovered(),
                                );
                                let sel_t = ctx.animate_bool_with_time(
                                    resp.id.with("card_sel"),
                                    is_sel,
                                    0.22,
                                );
                                let draw_rect = scale_rect(rect, card_scale);

                                let fill = if resp.hovered() {
                                    pal.bg_card_hover
                                } else {
                                    pal.bg_card
                                };

                                let painter = ui.painter();
                                draw_bento_card(&painter, draw_rect, 11.0, fill, pal);

                                // --- Row 1: Optional Flag Icon + Node Name + Selected Blue Checkmark Circle `(✓)` ---
                                let badge_code = if node.flag.is_empty() {
                                    match_flag(&node.name)
                                } else {
                                    node.flag.as_str()
                                };
                                let has_flag =
                                    !matches!(badge_code, "DIR" | "AUTO" | "FAIL" | "NET");

                                let top_y = draw_rect.top() + card_h * 0.33;
                                let title_x = if has_flag {
                                    let flag_rect = egui::Rect::from_center_size(
                                        egui::pos2(draw_rect.left() + 18.0, top_y),
                                        egui::vec2(14.0, 10.0),
                                    );
                                    draw_flag_icon(&painter, flag_rect, badge_code, pal);
                                    29.0
                                } else {
                                    11.0
                                };

                                // Selected Blue Circle Checkmark on Top-Right
                                let right_pad = if sel_t > 0.01 {
                                    let chk_r = (8.0 * spring_overshoot(sel_t)).clamp(0.0, 9.2);
                                    let chk_center = egui::pos2(draw_rect.right() - 16.0, top_y);
                                    painter.circle_filled(chk_center, chk_r, pal.primary);
                                    draw_sf_checkmark(
                                        &painter,
                                        chk_center,
                                        (chk_r / 8.0) * 0.68,
                                        egui::Color32::WHITE,
                                    );
                                    28.0
                                } else {
                                    10.0
                                };

                                let display_title = format_cupertino_title(&node.name);
                                let mut job = egui::text::LayoutJob::single_section(
                                    display_title,
                                    egui::TextFormat {
                                        font_id: cup_font_reg(12.0),
                                        color: pal.text_primary,
                                        ..Default::default()
                                    },
                                );
                                job.wrap = egui::text::TextWrapping {
                                    max_width: (draw_rect.width() - title_x - right_pad).max(16.0),
                                    max_rows: 1,
                                    break_anywhere: true,
                                    overflow_character: Some('…'),
                                };
                                let galley = ui.fonts(|f| f.layout_job(job));
                                painter.galley(
                                    egui::pos2(
                                        draw_rect.left() + title_x,
                                        top_y - galley.size().y * 0.5,
                                    ),
                                    galley,
                                    pal.text_primary,
                                );

                                // --- Row 2: Protocol Type (`Vless` / `Direct` / `URLTest`) on Left + Delay (`77 ms` / `超时`) on Right ---
                                let bottom_y = draw_rect.bottom() - card_h * 0.27;
                                painter.text(
                                    egui::pos2(draw_rect.left() + 11.0, bottom_y),
                                    egui::Align2::LEFT_CENTER,
                                    &node.proto,
                                    cup_font_reg(10.5),
                                    pal.text_tertiary,
                                );

                                let (dtxt, delay_col) = if node.delay > 0 {
                                    let c = if node.delay < 260 {
                                        pal.green
                                    } else if node.delay < 500 {
                                        pal.orange
                                    } else {
                                        pal.red
                                    };
                                    (format!("{} ms", node.delay), c)
                                } else if node.delay == 0 {
                                    ("超时".to_string(), pal.red)
                                } else {
                                    ("".to_string(), pal.text_tertiary)
                                };

                                if !dtxt.is_empty() {
                                    painter.text(
                                        egui::pos2(draw_rect.right() - 11.0, bottom_y),
                                        egui::Align2::RIGHT_CENTER,
                                        dtxt,
                                        cup_font_med(10.5),
                                        delay_col,
                                    );
                                }

                                if !is_sel && resp.clicked() {
                                    *action_select_node = Some(node.name.clone());
                                    app.open_popup = None;
                                    app.more_submenu = None;
                                }
                            }
                        });
                    }
                });
        });
}

pub fn render_more_popover(ctx: &egui::Context, app: &mut AppState, pal: &ThemePalette) {
    if app.open_popup.as_deref() != Some("more") {
        return;
    }
    let mut close_pop = false;
    let now = ctx.input(|i| i.time);
    let elapsed = ((now - app.popup_open_time) as f32 / 0.22).clamp(0.0, 1.0);
    if elapsed < 1.0 {
        ctx.request_repaint();
    }

    // Target height morphs smoothly with spring physics when switching between main menu and submenus (`排序` / `尺寸` / `风格` / `布局`)
    let target_h = match app.more_submenu.as_deref() {
        Some("sort") | Some("size") => 162.0_f32,
        Some("style") | Some("layout") => 128.0_f32,
        _ => 244.0_f32,
    };
    let anim_h = ctx.animate_value_with_time(egui::Id::new("fl_more_morph_h"), target_h, 0.20);
    let pop_scale = (0.72 + 0.28 * spring_overshoot(elapsed)).clamp(0.70, 1.05);
    let pop_w = 148.0_f32;

    let full_rect = egui::Rect::from_min_size(
        egui::pos2(app.popup_anchor.x - pop_w, app.popup_anchor.y),
        egui::vec2(pop_w, anim_h),
    );
    let origin = egui::pos2(full_rect.right() - 19.0, full_rect.top() + 19.0);
    let anim_min = origin + (full_rect.min - origin) * pop_scale;
    let anim_max = origin + (full_rect.max - origin) * pop_scale;
    let menu_rect = egui::Rect::from_min_max(anim_min, anim_max);

    egui::Area::new(egui::Id::new("fl_more_popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu_rect.min)
        .show(ctx, |ui| {
            let (_, _) = ui.allocate_exact_size(menu_rect.size(), egui::Sense::hover());
            let mp = ui.painter().clone();

            mp.rect_filled(
                menu_rect.translate(egui::vec2(0.0, 4.0)).expand(2.0),
                22.0,
                egui::Color32::from_rgba_unmultiplied(15, 23, 42, if pal.dark { 100 } else { 22 }),
            );
            mp.rect(
                menu_rect,
                22.0,
                pal.bg_popover,
                egui::Stroke::new(0.8_f32, pal.border),
            );

            let mut y = menu_rect.top() + 8.0;
            let draw_menu_item = |ui: &mut egui::Ui,
                                  mp: &egui::Painter,
                                  y: &mut f32,
                                  id_salt: &str,
                                  icon: Option<CupIcon>,
                                  label: &str,
                                  right_indicator: Option<&str>,
                                  checked: bool|
             -> bool {
                let r_rect = egui::Rect::from_min_size(
                    egui::pos2(menu_rect.left() + 6.0, *y),
                    egui::vec2(menu_rect.width() - 12.0, 32.0),
                );
                let resp = ui.interact(r_rect, ui.id().with(id_salt), egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    mp.rect_filled(r_rect, 10.0, pal.surface_hover);
                }
                let text_x = if let Some(ic) = icon {
                    draw_cup_icon(
                        mp,
                        ic,
                        egui::pos2(r_rect.left() + 15.0, r_rect.center().y),
                        pal.text_primary,
                    );
                    r_rect.left() + 31.0
                } else {
                    r_rect.left() + 12.0
                };
                mp.text(
                    egui::pos2(text_x, r_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    cup_font_reg(12.5),
                    pal.text_primary,
                );
                if checked {
                    draw_sf_checkmark(
                        mp,
                        egui::pos2(r_rect.right() - 14.0, r_rect.center().y),
                        0.78,
                        pal.text_primary,
                    );
                } else if let Some(ind) = right_indicator {
                    mp.text(
                        egui::pos2(r_rect.right() - 12.0, r_rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        ind,
                        cup_font_reg(12.5),
                        pal.text_secondary,
                    );
                }
                *y += 32.0;
                resp.clicked()
            };

            match app.more_submenu.clone().as_deref() {
                None => {
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_scroll",
                        Some(CupIcon::Target),
                        "滚动到已选",
                        None,
                        false,
                    ) {
                        app.scroll_to_selected = true;
                        close_pop = true;
                    }
                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_style",
                        Some(CupIcon::StyleList),
                        "风格",
                        Some("›"),
                        false,
                    ) {
                        app.more_submenu = Some("style".to_string());
                    }
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_sort",
                        Some(CupIcon::Sort),
                        "排序",
                        Some("›"),
                        false,
                    ) {
                        app.more_submenu = Some("sort".to_string());
                    }
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_layout",
                        Some(CupIcon::Grid),
                        "布局",
                        Some("›"),
                        false,
                    ) {
                        app.more_submenu = Some("layout".to_string());
                    }
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_size",
                        Some(CupIcon::TextSize),
                        "尺寸",
                        Some("›"),
                        false,
                    ) {
                        app.more_submenu = Some("size".to_string());
                    }

                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "m_hide_to",
                        Some(CupIcon::EyeOff),
                        "隐藏超时代理",
                        None,
                        app.profiles.hide_timeout,
                    ) {
                        app.profiles.hide_timeout = !app.profiles.hide_timeout;
                        app.save_profiles();
                        close_pop = true;
                    }
                }
                Some("sort") => {
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "sub_sort_hdr",
                        Some(CupIcon::Sort),
                        "排序",
                        Some("⌄"),
                        false,
                    ) {
                        app.more_submenu = None;
                    }
                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    for (k, lbl) in [("default", "默认"), ("delay", "延迟"), ("name", "名称")] {
                        let is_cur = app.profiles.sort_mode == k;
                        if draw_menu_item(ui, &mp, &mut y, k, None, lbl, None, is_cur) {
                            app.profiles.sort_mode = k.to_string();
                            app.save_profiles();
                            close_pop = true;
                        }
                    }
                }
                Some("size") => {
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "sub_size_hdr",
                        Some(CupIcon::TextSize),
                        "尺寸",
                        Some("⌄"),
                        false,
                    ) {
                        app.more_submenu = None;
                    }
                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    for (k, lbl) in [("loose", "宽松"), ("standard", "标准"), ("compact", "紧凑")] {
                        let is_cur = app.profiles.card_size == k;
                        if draw_menu_item(ui, &mp, &mut y, k, None, lbl, None, is_cur) {
                            app.profiles.card_size = k.to_string();
                            app.save_profiles();
                            close_pop = true;
                        }
                    }
                }
                Some("style") => {
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "sub_style_hdr",
                        Some(CupIcon::StyleList),
                        "风格",
                        Some("⌄"),
                        false,
                    ) {
                        app.more_submenu = None;
                    }
                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "st_card",
                        None,
                        "卡片",
                        None,
                        !app.profiles.dark_mode,
                    ) {
                        app.profiles.dark_mode = false;
                        app.save_profiles();
                        close_pop = true;
                    }
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "st_dark",
                        None,
                        "深色",
                        None,
                        app.profiles.dark_mode,
                    ) {
                        app.profiles.dark_mode = true;
                        app.save_profiles();
                        close_pop = true;
                    }
                }
                Some(_) => {
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "sub_lay_hdr",
                        Some(CupIcon::Grid),
                        "布局",
                        Some("⌄"),
                        false,
                    ) {
                        app.more_submenu = None;
                    }
                    y += 3.0;
                    mp.line_segment(
                        [
                            egui::pos2(menu_rect.left() + 10.0, y),
                            egui::pos2(menu_rect.right() - 10.0, y),
                        ],
                        egui::Stroke::new(1.0_f32, pal.divider),
                    );
                    y += 4.0;

                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "lay_grid",
                        None,
                        "响应式网格",
                        None,
                        app.profiles.card_size != "loose",
                    ) {
                        app.profiles.card_size = "compact".to_string();
                        app.save_profiles();
                        close_pop = true;
                    }
                    if draw_menu_item(
                        ui,
                        &mp,
                        &mut y,
                        "lay_wide",
                        None,
                        "响应式宽卡",
                        None,
                        app.profiles.card_size == "loose",
                    ) {
                        app.profiles.card_size = "loose".to_string();
                        app.save_profiles();
                        close_pop = true;
                    }
                }
            }

            if ctx.input(|i| i.pointer.any_pressed()) {
                if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                    if !menu_rect.contains(p) {
                        close_pop = true;
                    }
                }
            }
        });

    if close_pop || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.open_popup = None;
        app.more_submenu = None;
    }
}

pub fn render_import_modal(
    ctx: &egui::Context,
    app: &mut AppState,
    pal: &ThemePalette,
    action_import: &mut Option<(String, String)>,
) {
    if !app.show_import_dialog {
        return;
    }
    let mut close = false;
    let screen_rect = ctx.screen_rect();
    egui::Area::new(egui::Id::new("cup_import_sheet"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen_rect.min)
        .show(ctx, |ui| {
            let (_, bg_resp) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click());
            ui.painter().rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            );

            let sheet_w = (screen_rect.width() - 32.0).clamp(280.0, 430.0);
            let sheet_h = 250.0;
            let sheet_rect =
                egui::Rect::from_center_size(screen_rect.center(), egui::vec2(sheet_w, sheet_h));
            let _ = ui.interact(
                sheet_rect,
                ui.id().with("import_sheet_body"),
                egui::Sense::click(),
            );

            let sp = ui.painter();
            sp.rect_filled(
                sheet_rect.translate(egui::vec2(0.0, 8.0)).expand(4.0),
                18.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80),
            );
            sp.rect(
                sheet_rect,
                16.0,
                pal.bg_popover,
                egui::Stroke::new(1.0_f32, pal.border_strong),
            );

            let inner = sheet_rect.shrink(20.0);
            let mut s_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            s_ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let g = ui.fonts(|f| {
                        f.layout_no_wrap("添加新订阅".to_string(), cup_font_med(15.0), pal.text_primary)
                    });
                    let (r, _) = ui.allocate_exact_size(g.size(), egui::Sense::hover());
                    ui.painter().galley(r.min, g, pal.text_primary);
                });
                ui.add_space(12.0);

                let form_w = inner.width();
                let (f_rect, _) =
                    ui.allocate_exact_size(egui::vec2(form_w, 96.0), egui::Sense::hover());
                let fp = ui.painter().clone();
                fp.rect(
                    f_rect.shrink(0.5),
                    10.0,
                    pal.surface_container,
                    egui::Stroke::new(1.0_f32, pal.border),
                );
                fp.line_segment(
                    [
                        egui::pos2(f_rect.left() + 12.0, f_rect.center().y),
                        egui::pos2(f_rect.right() - 1.0, f_rect.center().y),
                    ],
                    egui::Stroke::new(1.0_f32, pal.divider),
                );

                let r1 = egui::Rect::from_min_max(
                    f_rect.left_top(),
                    egui::pos2(f_rect.right(), f_rect.center().y),
                );
                fp.text(
                    egui::pos2(r1.left() + 14.0, r1.center().y),
                    egui::Align2::LEFT_CENTER,
                    "订阅名称",
                    cup_font_med(12.5),
                    pal.text_primary,
                );
                let e1_rect = egui::Rect::from_min_max(
                    egui::pos2(r1.left() + 84.0, r1.top() + 12.0),
                    egui::pos2(r1.right() - 12.0, r1.bottom() - 12.0),
                );
                let mut e1_ui = ui.new_child(egui::UiBuilder::new().max_rect(e1_rect));
                e1_ui.add(
                    egui::TextEdit::singleline(&mut app.import_name)
                        .frame(false)
                        .font(cup_font_reg(12.5))
                        .text_color(pal.text_primary)
                        .hint_text(
                            egui::RichText::new("例如：主力节点 / 备用机场")
                                .color(pal.text_tertiary)
                                .size(12.0),
                        )
                        .desired_width(e1_rect.width()),
                );

                let r2 = egui::Rect::from_min_max(
                    egui::pos2(f_rect.left(), f_rect.center().y),
                    f_rect.right_bottom(),
                );
                fp.text(
                    egui::pos2(r2.left() + 14.0, r2.center().y),
                    egui::Align2::LEFT_CENTER,
                    "订阅地址",
                    cup_font_med(12.5),
                    pal.text_primary,
                );
                let e2_rect = egui::Rect::from_min_max(
                    egui::pos2(r2.left() + 84.0, r2.top() + 12.0),
                    egui::pos2(r2.right() - 12.0, r2.bottom() - 12.0),
                );
                let mut e2_ui = ui.new_child(egui::UiBuilder::new().max_rect(e2_rect));
                e2_ui.add(
                    egui::TextEdit::singleline(&mut app.import_url)
                        .frame(false)
                        .font(cup_font_reg(12.5))
                        .text_color(pal.text_primary)
                        .hint_text(
                            egui::RichText::new("https://… (Clash / Mihomo 订阅链接)")
                                .color(pal.text_tertiary)
                                .size(12.0),
                        )
                        .desired_width(e2_rect.width()),
                );

                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if cupertino_button(ui, "取消", None, CupBtnStyle::Secondary, true, pal) {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_submit = !app.import_url.trim().is_empty();
                        if cupertino_button(
                            ui,
                            "下载并添加",
                            None,
                            CupBtnStyle::Filled(pal.primary),
                            can_submit,
                            pal,
                        ) {
                            let mut name = app.import_name.trim().to_string();
                            if name.is_empty() {
                                name = "新订阅".to_string();
                            }
                            *action_import = Some((name, app.import_url.trim().to_string()));
                            close = true;
                        }
                    });
                });
            });

            if bg_resp.clicked()
                && !sheet_rect.contains(bg_resp.interact_pointer_pos().unwrap_or_default())
            {
                close = true;
            }
        });
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.show_import_dialog = false;
        app.import_name.clear();
        app.import_url.clear();
    }
}

