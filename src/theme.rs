use eframe::egui;
use std::collections::VecDeque;

/// 净化节点/策略组标题：剥离 Linux 字体无法渲染的 Emoji 国旗/图标，完整保留原订阅标点
pub fn format_cupertino_title(raw: &str) -> String {
    let mut buf = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let is_flag_emoji = ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch);
        let is_misc_emoji = ('\u{1F300}'..='\u{1FAFF}').contains(&ch)
            || ('\u{2600}'..='\u{27BF}').contains(&ch)
            || ch == '\u{FE0F}'
            || ch == '\u{200D}';
        if !is_flag_emoji && !is_misc_emoji {
            buf.push(ch);
        }
    }
    let mut out = String::new();
    for word in buf.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.trim().to_string()
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct ThemePalette {
    pub dark: bool,
    pub bg_window: egui::Color32,
    pub bg_sidebar: egui::Color32,
    pub bg_rail: egui::Color32,
    pub bg_card: egui::Color32,
    pub bg_card_hover: egui::Color32,
    pub bg_selected: egui::Color32,
    pub primary_container: egui::Color32,
    pub bg_pill: egui::Color32,
    pub surface_container: egui::Color32,
    pub bg_pill_hover: egui::Color32,
    pub surface_hover: egui::Color32,
    pub bg_popup: egui::Color32,
    pub bg_popover: egui::Color32,
    pub border: egui::Color32,
    pub border_hover: egui::Color32,
    pub border_strong: egui::Color32,
    pub divider: egui::Color32,
    pub shadow: egui::Color32,
    pub text_primary: egui::Color32,
    pub text_secondary: egui::Color32,
    pub text_muted: egui::Color32,
    pub text_tertiary: egui::Color32,
    pub primary: egui::Color32,
    pub on_primary: egui::Color32,
    pub on_primary_container: egui::Color32,
    pub green: egui::Color32,
    pub orange: egui::Color32,
    pub red: egui::Color32,
    pub indigo: egui::Color32,
}

impl ThemePalette {
    pub fn new(dark: bool) -> Self {
        if dark {
            let bg_sidebar = egui::Color32::from_rgb(24, 27, 34);
            let bg_selected = egui::Color32::from_rgb(34, 52, 84);
            let bg_pill = egui::Color32::from_rgb(34, 38, 48);
            let bg_pill_hover = egui::Color32::from_rgb(45, 50, 64);
            let bg_popup = egui::Color32::from_rgb(30, 34, 43);
            let border_hover = egui::Color32::from_rgb(66, 75, 94);
            let text_muted = egui::Color32::from_rgb(122, 130, 146);
            Self {
                dark: true,
                bg_window: egui::Color32::from_rgb(18, 20, 26),
                bg_sidebar,
                bg_rail: bg_sidebar,
                bg_card: egui::Color32::from_rgb(28, 32, 40),
                bg_card_hover: egui::Color32::from_rgb(35, 40, 50),
                bg_selected,
                primary_container: bg_selected,
                bg_pill,
                surface_container: bg_pill,
                bg_pill_hover,
                surface_hover: bg_pill_hover,
                bg_popup,
                bg_popover: bg_popup,
                border: egui::Color32::from_rgb(38, 43, 54),
                border_hover,
                border_strong: border_hover,
                divider: egui::Color32::from_rgb(44, 49, 62),
                shadow: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 75),
                text_primary: egui::Color32::from_rgb(238, 240, 246),
                text_secondary: egui::Color32::from_rgb(162, 170, 186),
                text_muted,
                text_tertiary: text_muted,
                primary: egui::Color32::from_rgb(24, 118, 226),
                on_primary: egui::Color32::WHITE,
                on_primary_container: egui::Color32::from_rgb(220, 232, 255),
                green: egui::Color32::from_rgb(46, 204, 113),
                orange: egui::Color32::from_rgb(245, 158, 11),
                red: egui::Color32::from_rgb(239, 68, 68),
                indigo: egui::Color32::from_rgb(115, 103, 240),
            }
        } else {
            let bg_sidebar = egui::Color32::from_rgb(235, 238, 243);
            let bg_selected = egui::Color32::from_rgb(216, 224, 238);
            let bg_pill = egui::Color32::from_rgb(240, 243, 248);
            let bg_pill_hover = egui::Color32::from_rgb(228, 234, 244);
            let bg_popup = egui::Color32::from_rgb(255, 255, 255);
            let border_hover = egui::Color32::from_rgb(206, 214, 228);
            let text_muted = egui::Color32::from_rgb(118, 125, 138);
            Self {
                dark: false,
                bg_window: egui::Color32::from_rgb(243, 245, 248),
                bg_sidebar,
                bg_rail: bg_sidebar,
                bg_card: egui::Color32::from_rgb(255, 255, 255),
                bg_card_hover: egui::Color32::from_rgb(251, 253, 255),
                bg_selected,
                primary_container: bg_selected,
                bg_pill,
                surface_container: bg_pill,
                bg_pill_hover,
                surface_hover: bg_pill_hover,
                bg_popup,
                bg_popover: bg_popup,
                border: egui::Color32::from_rgb(234, 238, 244),
                border_hover,
                border_strong: border_hover,
                divider: egui::Color32::from_rgb(226, 230, 238),
                shadow: egui::Color32::from_rgba_unmultiplied(15, 23, 42, 10),
                text_primary: egui::Color32::from_rgb(24, 28, 35),
                text_secondary: egui::Color32::from_rgb(82, 90, 104),
                text_muted,
                text_tertiary: text_muted,
                primary: egui::Color32::from_rgb(14, 107, 214),
                on_primary: egui::Color32::WHITE,
                on_primary_container: egui::Color32::from_rgb(24, 30, 42),
                green: egui::Color32::from_rgb(26, 156, 84),
                orange: egui::Color32::from_rgb(217, 119, 6),
                red: egui::Color32::from_rgb(217, 56, 41),
                indigo: egui::Color32::from_rgb(79, 70, 229),
            }
        }
    }
}

#[inline]
pub fn scale_rect(rect: egui::Rect, scale: f32) -> egui::Rect {
    egui::Rect::from_center_size(rect.center(), rect.size() * scale)
}

#[inline]
pub fn spring_overshoot(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.75_f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

#[inline]
pub fn ease_emphasized(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

pub fn interact_spring_scale(ctx: &egui::Context, id: egui::Id, pressed: bool, hovered: bool) -> f32 {
    let target = if pressed {
        0.93
    } else if hovered {
        1.015
    } else {
        1.0
    };
    ctx.animate_value_with_time(id.with("press_spring"), target, 0.14)
}

pub fn cup_font_reg(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Proportional)
}

pub fn cup_font_med(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("Medium".into()))
}

pub fn apply_app_theme(ctx: &egui::Context, pal: &ThemePalette) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 5.0,
        handle_min_length: 28.0,
        bar_inner_margin: 4.0,
        bar_outer_margin: 2.0,
        floating: true,
        ..Default::default()
    };

    let mut visuals = if pal.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = pal.bg_window;
    visuals.window_fill = pal.bg_card;
    visuals.extreme_bg_color = pal.bg_card;
    visuals.faint_bg_color = pal.bg_pill;
    visuals.widgets.noninteractive.bg_fill = pal.bg_card;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, pal.border);
    visuals.widgets.noninteractive.fg_stroke.color = pal.text_primary;
    visuals.widgets.inactive.bg_fill = pal.bg_pill;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.inactive.fg_stroke.color = pal.text_primary;
    visuals.widgets.hovered.bg_fill = pal.bg_pill_hover;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.fg_stroke.color = pal.text_primary;
    visuals.widgets.active.bg_fill = pal.primary;
    visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;
    visuals.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(
        pal.primary.r(),
        pal.primary.g(),
        pal.primary.b(),
        55,
    );
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, pal.primary);
    style.visuals = visuals;
    ctx.set_style(style);
}

/// 高斯柔化投影 + 圆角卡片绘制（使用 epaint 原生顶点渐变模糊阴影 + 多层环境光漫反射）
pub fn draw_bento_card(
    painter: &egui::Painter,
    rect: egui::Rect,
    rounding: f32,
    fill: egui::Color32,
    pal: &ThemePalette,
) {
    if !pal.dark {
        // 远景大范围柔化漫反射主阴影
        let main_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 5.0),
            blur: 18.0,
            spread: 1.5,
            color: egui::Color32::from_rgba_unmultiplied(15, 23, 42, 24),
        };
        painter.add(main_shadow.as_shape(rect, rounding));

        // 近景接触阴影，增强卡片边缘立体悬浮感
        let contact_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 2.0),
            blur: 6.0,
            spread: 0.0,
            color: egui::Color32::from_rgba_unmultiplied(15, 23, 42, 14),
        };
        painter.add(contact_shadow.as_shape(rect, rounding));
    } else {
        // 深色模式下的深邃立体投影
        let dark_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 6.0),
            blur: 22.0,
            spread: 2.0,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
        };
        painter.add(dark_shadow.as_shape(rect, rounding));

        let dark_contact = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 2.0),
            blur: 6.0,
            spread: 0.5,
            color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 75),
        };
        painter.add(dark_contact.as_shape(rect, rounding));
    }
    painter.rect(
        rect,
        rounding,
        fill,
        egui::Stroke::new(
            0.85_f32,
            if pal.dark {
                egui::Color32::from_rgba_unmultiplied(56, 64, 82, 210)
            } else {
                egui::Color32::from_rgba_unmultiplied(220, 228, 240, 195)
            },
        ),
    );
}

/// Dashboard 左上角 Status 卡片右侧的柔和光晕层叠装饰（100% 硬件抗锯齿圆弧，严格内缩不溢出圆角）
pub fn draw_status_wave_decor(
    painter: &egui::Painter,
    card_rect: egui::Rect,
    primary: egui::Color32,
    dark: bool,
) {
    // 缩小裁剪区域并在四角避开圆角外缘，使用原生抗锯齿圆环叠加以获得丝滑边缘，并随卡片高度自适应缩放
    let inner_clip = card_rect.shrink(1.2);
    let clipped = painter.with_clip_rect(inner_clip);
    let sc = (card_rect.height() / 138.0).clamp(0.95, 1.45);
    let anchor = egui::pos2(card_rect.right() - 42.0 * sc, card_rect.center().y + 4.0 * sc);

    let c_outer = if dark {
        egui::Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 18)
    } else {
        egui::Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 14)
    };
    let c_mid = if dark {
        egui::Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 28)
    } else {
        egui::Color32::from_rgba_unmultiplied(primary.r(), primary.g(), primary.b(), 22)
    };

    clipped.circle_filled(anchor + egui::vec2(14.0 * sc, -6.0 * sc), 88.0 * sc, c_outer);
    clipped.circle_filled(anchor + egui::vec2(6.0 * sc, 4.0 * sc), 56.0 * sc, c_mid);
}

/// 现代极简双层光晕启停主按钮（采用 egui 原生 SDF 抗锯齿圆与矢量图标，彻底消除多边形锯齿与抠图毛边感）
pub fn draw_scalloped_flower_button(
    painter: &egui::Painter,
    center: egui::Pos2,
    base_r: f32,
    fill: egui::Color32,
    is_running: bool,
) {
    // 1. 外层柔和呼吸光圈（自带 1px 硬件抗锯齿羽化）
    let halo_col = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 26);
    let ring_col = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 58);
    painter.circle_filled(center, base_r + 5.5, halo_col);
    painter.circle_stroke(center, base_r + 2.8, egui::Stroke::new(1.1_f32, ring_col));

    // 2. 底部弥散彩色投影
    let shadow_col = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 64);
    painter.circle_filled(center + egui::vec2(0.0, 3.0), base_r * 0.94, shadow_col);

    // 3. 主体高锐度实心圆 + 顶部内高光描边
    painter.circle_filled(center, base_r, fill);
    painter.circle_stroke(
        center,
        base_r - 0.8,
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 68)),
    );

    // 4. 中心高对比矢量图标：运行中显示圆角双竖线暂停 `II`，未开启显示播放三角 `▶`
    let k = (base_r / 22.5).clamp(0.85, 1.5);
    if is_running {
        let bar_w = 3.0_f32 * k;
        let bar_h = 12.0_f32 * k;
        let gap = 3.6_f32 * k;
        for dx in [-gap, gap] {
            let r = egui::Rect::from_center_size(
                center + egui::vec2(dx, 0.0),
                egui::vec2(bar_w, bar_h),
            );
            painter.rect_filled(r, 1.5 * k, egui::Color32::WHITE);
        }
    } else {
        let tri = vec![
            center + egui::vec2(-3.2 * k, -5.8 * k),
            center + egui::vec2(-3.2 * k, 5.8 * k),
            center + egui::vec2(6.0 * k, 0.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            tri,
            egui::Color32::WHITE,
            egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
        ));
    }
}

/// Upload / Download 卡片底部的平滑波浪曲线 + 垂直渐变填充图 (带单调性钳制，防止峰值回落触底越界)
pub fn draw_sparkline_area(
    painter: &egui::Painter,
    rect: egui::Rect,
    history: &VecDeque<u64>,
    stroke_col: egui::Color32,
) {
    if rect.width() < 8.0 || rect.height() < 8.0 {
        return;
    }
    let n = 28usize;
    let mut raw_vals = Vec::with_capacity(n);
    let hist_len = history.len();
    for i in 0..n {
        let v = if hist_len > 0 && i >= n.saturating_sub(hist_len) {
            let idx = i - (n - hist_len);
            history.get(idx).copied().unwrap_or(0) as f32
        } else {
            0.0
        };
        raw_vals.push(v);
    }
    let max_v = raw_vals
        .iter()
        .copied()
        .fold(0.0_f32, f32::max)
        .max(1024.0);

    // 生成平滑插值点（使用区间 [min(v1,v2), max(v1,v2)] 钳制，彻底消除峰值回落时的负向过冲贴底）
    let sub_steps = 4usize;
    let total_pts = (n - 1) * sub_steps + 1;
    let mut pts = Vec::with_capacity(total_pts);
    let draw_top = rect.top() + 3.0;
    let draw_bot = rect.bottom() - 3.0;
    let usable_h = (draw_bot - draw_top).max(4.0);

    for i in 0..(n - 1) {
        let v0 = raw_vals[i.saturating_sub(1)];
        let v1 = raw_vals[i];
        let v2 = raw_vals[i + 1];
        let v3 = raw_vals[(i + 2).min(n - 1)];
        let seg_min = v1.min(v2);
        let seg_max = v1.max(v2);
        for s in 0..sub_steps {
            if i > 0 && s == 0 {
                continue;
            }
            let t = s as f32 / sub_steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let raw_interp = 0.5
                * ((2.0 * v1)
                    + (-v0 + v2) * t
                    + (2.0 * v0 - 5.0 * v1 + 4.0 * v2 - v3) * t2
                    + (-v0 + 3.0 * v1 - 3.0 * v2 + v3) * t3);
            let interp = raw_interp.clamp(seg_min, seg_max);
            let norm = (interp / max_v).clamp(0.0, 1.0);
            let idx_f = i as f32 + t;
            let x = rect.left() + (idx_f / (n - 1) as f32) * rect.width();
            let subtle_idle = 0.14 + 0.035 * (idx_f * 0.42).sin();
            let h_ratio = (norm * 0.84).max(subtle_idle);
            let y = draw_bot - h_ratio * usable_h;
            pts.push(egui::pos2(x, y));
        }
    }
    let last_norm = (raw_vals[n - 1] / max_v).clamp(0.0, 1.0);
    let last_h = (last_norm * 0.84).max(0.14);
    pts.push(egui::pos2(rect.right(), draw_bot - last_h * usable_h));

    // 1. 绘制曲线下方的垂直半透明渐变 Mesh
    let top_fill = egui::Color32::from_rgba_unmultiplied(stroke_col.r(), stroke_col.g(), stroke_col.b(), 38);
    let bot_fill = egui::Color32::from_rgba_unmultiplied(stroke_col.r(), stroke_col.g(), stroke_col.b(), 2);
    let mut mesh = egui::epaint::Mesh::default();
    for p in &pts {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *p,
            uv: egui::epaint::WHITE_UV,
            color: top_fill,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(p.x, rect.bottom()),
            uv: egui::epaint::WHITE_UV,
            color: bot_fill,
        });
    }
    for i in 0..(pts.len().saturating_sub(1)) as u32 {
        let base = i * 2;
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
    }
    painter.add(egui::Shape::mesh(mesh));

    // 2. 绘制顶部平滑曲线描边
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.75_f32, stroke_col)));
}

/// 精致胶囊进度条（保证最小可视胶囊宽度，低占比下依然圆润清晰）
pub fn draw_wave_progress(
    painter: &egui::Painter,
    rect: egui::Rect,
    ratio: f32,
    active_col: egui::Color32,
    track_col: egui::Color32,
) {
    let r = ratio.clamp(0.0, 1.0);
    let bar_h = rect.height().clamp(5.0, 7.5);
    let bar_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), bar_h));
    let rounding = bar_h * 0.5;

    // 1. 完整圆角底轨
    painter.rect_filled(bar_rect, rounding, track_col);

    // 2. 已用进度胶囊（只要 ratio > 0 就保证最小圆润胶囊可视宽度）
    if r > 0.0005 {
        let min_vis_w = (bar_h * 2.2).min(bar_rect.width());
        let active_w = (bar_rect.width() * r).max(min_vis_w).min(bar_rect.width());
        let active_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(active_w, bar_h));
        painter.rect_filled(active_rect, rounding, active_col);
    }
}

/// 单行文本超出最大像素宽度时优雅截断并追加省略号 `…`，杜绝表格列重叠与半字裁切
pub fn ellipsize_text(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    max_width: f32,
) -> String {
    if max_width <= 8.0 || text.is_empty() {
        return String::new();
    }
    let full_w = ui.fonts(|f| {
        f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
    });
    if full_w <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        let w = ui.fonts(|f| {
            f.layout_no_wrap(candidate, font.clone(), egui::Color32::WHITE)
                .size()
                .x
        });
        if w <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        "…".to_string()
    } else {
        chars[..lo].iter().collect::<String>() + "…"
    }
}

/// 将速率拆分为 `(大字号数值, 小字号单位)`，用于复刻 FlClash `50.6 KB/s` 基线对齐排版
pub fn split_speed_parts(bytes_per_sec: u64) -> (String, &'static str) {
    if bytes_per_sec < 1024 {
        (format!("{}", bytes_per_sec), "B/s")
    } else if bytes_per_sec < 1024 * 1024 {
        (format!("{:.1}", bytes_per_sec as f64 / 1024.0), "KB/s")
    } else {
        (format!("{:.1}", bytes_per_sec as f64 / (1024.0 * 1024.0)), "MB/s")
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum CupIcon {
    SidebarToggle,
    Dashboard,
    Doc,
    Network,
    Toolbox,
    StyleList,
    TextSize,
    Refresh,
    Plus,
    Bolt,
    Sliders,
    Search,
    Locate,
    Target,
    MoreVert,
    Sort,
    Grid,
    EyeOff,
    Proxy,
    Folder,
    Close,
    Sun,
    Moon,
}

#[derive(Clone, Copy)]
pub enum CupBtnStyle {
    Filled(egui::Color32),
    Tinted(egui::Color32),
    Secondary,
}

fn draw_sf_sidebar_toggle(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    let rect = egui::Rect::from_center_size(center, egui::vec2(r * 2.1, r * 1.65));
    painter.rect_stroke(rect, 2.2, s);
    let div_x = rect.left() + rect.width() * 0.36;
    painter.line_segment([egui::pos2(div_x, rect.top()), egui::pos2(div_x, rect.bottom())], s);
}

fn draw_sf_doc(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    let rect = egui::Rect::from_center_size(center, egui::vec2(r * 1.65, r * 2.05));
    painter.rect_stroke(rect, 2.0, s);
    let s_thin = egui::Stroke::new(1.15_f32, col);
    for dy in [-r * 0.38, 0.0, r * 0.38] {
        painter.line_segment(
            [egui::pos2(rect.left() + 2.6, center.y + dy), egui::pos2(rect.right() - 2.6, center.y + dy)],
            s_thin,
        );
    }
}

fn draw_sf_network(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.3_f32, col);
    let top = center + egui::vec2(0.0, -r * 0.62);
    let bl = center + egui::vec2(-r * 0.68, r * 0.58);
    let br = center + egui::vec2(r * 0.68, r * 0.58);
    let hub = center + egui::vec2(0.0, r * 0.05);
    painter.line_segment([top, hub], s);
    painter.line_segment([bl, hub], s);
    painter.line_segment([br, hub], s);
    for p in [top, bl, br] {
        painter.circle_filled(p, 2.1, col);
    }
}

fn draw_sf_toolbox(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    let body = egui::Rect::from_center_size(center + egui::vec2(0.0, r * 0.18), egui::vec2(r * 2.05, r * 1.45));
    painter.rect_stroke(body, 2.0, s);
    let handle = egui::Rect::from_min_max(
        egui::pos2(center.x - r * 0.45, body.top() - r * 0.42),
        egui::pos2(center.x + r * 0.45, body.top()),
    );
    painter.rect_stroke(handle, 1.2, s);
    painter.line_segment([egui::pos2(body.left(), body.center().y), egui::pos2(body.right(), body.center().y)], s);
}

fn draw_sf_style_list(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.4_f32, col);
    for dy in [-r * 0.55, 0.0, r * 0.55] {
        let y = center.y + dy;
        painter.circle_filled(egui::pos2(center.x - r * 0.78, y), 1.3, col);
        painter.line_segment([egui::pos2(center.x - r * 0.32, y), egui::pos2(center.x + r * 0.85, y)], s);
    }
}

fn draw_sf_text_size(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.45_f32, col);
    let lx = center.x - r * 0.35;
    painter.line_segment([egui::pos2(lx - r * 0.55, center.y - r * 0.65), egui::pos2(lx + r * 0.55, center.y - r * 0.65)], s);
    painter.line_segment([egui::pos2(lx, center.y - r * 0.65), egui::pos2(lx, center.y + r * 0.68)], s);
    let rx = center.x + r * 0.52;
    let s2 = egui::Stroke::new(1.3_f32, col);
    painter.line_segment([egui::pos2(rx - r * 0.36, center.y - r * 0.12), egui::pos2(rx + r * 0.36, center.y - r * 0.12)], s2);
    painter.line_segment([egui::pos2(rx, center.y - r * 0.12), egui::pos2(rx, center.y + r * 0.68)], s2);
}

fn draw_sf_globe(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    painter.circle_stroke(center, r, s);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(r, 0.0)], s);
    painter.line_segment([center + egui::vec2(0.0, -r), center + egui::vec2(0.0, r)], s);
    let s_thin = egui::Stroke::new(1.0_f32, col);
    painter.line_segment(
        [center + egui::vec2(-r * 0.84, -r * 0.45), center + egui::vec2(r * 0.84, -r * 0.45)],
        s_thin,
    );
    painter.line_segment(
        [center + egui::vec2(-r * 0.84, r * 0.45), center + egui::vec2(r * 0.84, r * 0.45)],
        s_thin,
    );
}

fn draw_sf_bolt(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let t1 = vec![
        center + egui::vec2(r * 0.15, -r * 0.95),
        center + egui::vec2(-r * 0.65, r * 0.10),
        center + egui::vec2(r * 0.08, r * 0.10),
    ];
    let t2 = vec![
        center + egui::vec2(-r * 0.08, -r * 0.10),
        center + egui::vec2(r * 0.65, -r * 0.10),
        center + egui::vec2(-r * 0.15, r * 0.95),
    ];
    painter.add(egui::Shape::convex_polygon(t1, col, egui::Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(t2, col, egui::Stroke::NONE));
}

fn draw_sf_refresh(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let steps = 14;
    let start_ang = -std::f32::consts::FRAC_PI_4;
    let sweep = std::f32::consts::PI * 1.55;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let a = start_ang + sweep * (i as f32 / steps as f32);
        pts.push(center + egui::vec2(a.cos() * r, a.sin() * r));
    }
    painter.add(egui::Shape::line(pts, egui::Stroke::new(1.4_f32, col)));
    let tip = center + egui::vec2(start_ang.cos() * r, start_ang.sin() * r);
    let s = egui::Stroke::new(1.4_f32, col);
    painter.line_segment([tip, tip + egui::vec2(-3.2, -0.8)], s);
    painter.line_segment([tip, tip + egui::vec2(0.5, 3.2)], s);
}

fn draw_sf_plus(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.5_f32, col);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(r, 0.0)], s);
    painter.line_segment([center + egui::vec2(0.0, -r), center + egui::vec2(0.0, r)], s);
}

fn draw_sf_search(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let c = center + egui::vec2(-1.1, -1.1);
    let cr = r * 0.72;
    painter.circle_stroke(c, cr, egui::Stroke::new(1.5_f32, col));
    let p1 = c + egui::vec2(cr * 0.707, cr * 0.707);
    let p2 = center + egui::vec2(r * 0.85, r * 0.85);
    painter.line_segment([p1, p2], egui::Stroke::new(1.6_f32, col));
}

fn draw_sf_locate(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.45_f32, col);
    painter.circle_stroke(center, r * 0.72, s);
    painter.circle_filled(center, r * 0.28, col);
    painter.line_segment([center + egui::vec2(0.0, -r), center + egui::vec2(0.0, -r * 0.72)], s);
    painter.line_segment([center + egui::vec2(0.0, r * 0.72), center + egui::vec2(0.0, r)], s);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(-r * 0.72, 0.0)], s);
    painter.line_segment([center + egui::vec2(r * 0.72, 0.0), center + egui::vec2(r, 0.0)], s);
}

fn draw_sf_more_vert(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let dot_r = (r * 0.24).max(1.35);
    painter.circle_filled(center + egui::vec2(0.0, -r * 0.72), dot_r, col);
    painter.circle_filled(center, dot_r, col);
    painter.circle_filled(center + egui::vec2(0.0, r * 0.72), dot_r, col);
}

fn draw_sf_sort(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.45_f32, col);
    painter.line_segment([center + egui::vec2(-r, -r * 0.55), center + egui::vec2(r, -r * 0.55)], s);
    painter.line_segment([center + egui::vec2(-r, 0.0), center + egui::vec2(r * 0.35, 0.0)], s);
    painter.line_segment([center + egui::vec2(-r, r * 0.55), center + egui::vec2(-r * 0.2, r * 0.55)], s);
}

fn draw_sf_grid(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.3_f32, col);
    let d = r * 0.82;
    let g = r * 0.16;
    for (dx, dy) in [(-d, -d), (g, -d), (-d, g), (g, g)] {
        let rect = egui::Rect::from_min_size(center + egui::vec2(dx, dy), egui::vec2(d - g, d - g));
        painter.rect_stroke(rect, 1.2, s);
    }
}

fn draw_sf_eye_off(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    painter.circle_stroke(center, r * 0.55, s);
    painter.line_segment([center + egui::vec2(-r * 0.85, r * 0.85), center + egui::vec2(r * 0.85, -r * 0.85)], s);
}

fn draw_sf_sliders(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32, bg: egui::Color32) {
    let s = egui::Stroke::new(1.35_f32, col);
    for (dy, kx) in [(-4.2_f32, -2.0_f32), (0.0_f32, 2.2_f32), (4.2_f32, -1.0_f32)] {
        let y = center.y + dy;
        painter.line_segment([egui::pos2(center.x - r, y), egui::pos2(center.x + r, y)], s);
        let kc = egui::pos2(center.x + kx, y);
        painter.circle_filled(kc, 2.0, bg);
        painter.circle_stroke(kc, 1.8, egui::Stroke::new(1.25_f32, col));
    }
}

pub fn draw_sf_checkmark(painter: &egui::Painter, center: egui::Pos2, scale: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.7_f32 * scale, col);
    let p1 = center + egui::vec2(-3.8 * scale, 0.2 * scale);
    let p2 = center + egui::vec2(-1.0 * scale, 3.0 * scale);
    let p3 = center + egui::vec2(4.0 * scale, -2.8 * scale);
    painter.line_segment([p1, p2], s);
    painter.line_segment([p2, p3], s);
}

fn draw_sf_xmark(painter: &egui::Painter, center: egui::Pos2, r: f32, col: egui::Color32) {
    let s = egui::Stroke::new(1.45_f32, col);
    painter.line_segment([center + egui::vec2(-r, -r), center + egui::vec2(r, r)], s);
    painter.line_segment([center + egui::vec2(r, -r), center + egui::vec2(-r, r)], s);
}

pub fn draw_cup_icon(painter: &egui::Painter, icon: CupIcon, center: egui::Pos2, col: egui::Color32) {
    let r = 6.2_f32;
    match icon {
        CupIcon::SidebarToggle => draw_sf_sidebar_toggle(painter, center, r, col),
        CupIcon::Dashboard | CupIcon::Grid | CupIcon::Folder => draw_sf_grid(painter, center, r, col),
        CupIcon::Doc => draw_sf_doc(painter, center, r, col),
        CupIcon::Network => draw_sf_network(painter, center, r, col),
        CupIcon::Toolbox => draw_sf_toolbox(painter, center, r, col),
        CupIcon::StyleList => draw_sf_style_list(painter, center, r, col),
        CupIcon::TextSize => draw_sf_text_size(painter, center, r, col),
        CupIcon::Search => draw_sf_search(painter, center, r, col),
        CupIcon::Locate | CupIcon::Target => draw_sf_locate(painter, center, r, col),
        CupIcon::MoreVert => draw_sf_more_vert(painter, center, r, col),
        CupIcon::Bolt => draw_sf_bolt(painter, center, r, col),
        CupIcon::Refresh => draw_sf_refresh(painter, center, r, col),
        CupIcon::Plus => draw_sf_plus(painter, center, r, col),
        CupIcon::Sliders => draw_sf_sliders(painter, center, r, col, egui::Color32::TRANSPARENT),
        CupIcon::Sort => draw_sf_sort(painter, center, r, col),
        CupIcon::EyeOff => draw_sf_eye_off(painter, center, r, col),
        CupIcon::Proxy => draw_sf_globe(painter, center, r, col),
        CupIcon::Close => draw_sf_xmark(painter, center, r * 0.72, col),
        CupIcon::Sun | CupIcon::Moon => {
            painter.circle_stroke(center, r * 0.72, egui::Stroke::new(1.5_f32, col));
            painter.circle_filled(center, r * 0.28, col);
        }
    }
}

pub fn draw_flag_icon(painter: &egui::Painter, rect: egui::Rect, code: &str, pal: &ThemePalette) {
    let r = 2.2_f32;
    match code {
        "JP" => {
            painter.rect(rect, r, egui::Color32::WHITE, egui::Stroke::new(0.6_f32, pal.border));
            painter.circle_filled(rect.center(), rect.height() * 0.28, egui::Color32::from_rgb(220, 38, 38));
        }
        "HK" | "CN" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(222, 41, 16));
            painter.circle_filled(rect.center(), rect.height() * 0.24, egui::Color32::WHITE);
        }
        "AL" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(220, 38, 38));
            painter.circle_filled(rect.center(), rect.height() * 0.22, egui::Color32::from_rgb(28, 28, 28));
        }
        "TW" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(222, 41, 16));
            let canton = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 0.52, rect.height() * 0.52));
            painter.rect_filled(canton, egui::Rounding { nw: r, ne: 0.0, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(0, 40, 104));
            painter.circle_filled(canton.center(), 1.8, egui::Color32::WHITE);
        }
        "SG" => {
            painter.rect(rect, r, egui::Color32::WHITE, egui::Stroke::new(0.6_f32, pal.border));
            let top = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.center().y));
            painter.rect_filled(top, egui::Rounding { nw: r, ne: r, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(239, 51, 64));
            painter.circle_filled(egui::pos2(rect.left() + 3.8, top.center().y), 1.3, egui::Color32::WHITE);
        }
        "US" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(191, 10, 48));
            for i in [1, 3] {
                let y = rect.top() + rect.height() * (i as f32 / 5.0);
                let stripe = egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), rect.height() * 0.2));
                painter.rect_filled(stripe, 0.0, egui::Color32::WHITE);
            }
            let canton = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 0.48, rect.height() * 0.55));
            painter.rect_filled(canton, egui::Rounding { nw: r, ne: 0.0, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(0, 40, 104));
        }
        "KR" => {
            painter.rect(rect, r, egui::Color32::WHITE, egui::Stroke::new(0.6_f32, pal.border));
            painter.circle_filled(rect.center(), rect.height() * 0.26, egui::Color32::from_rgb(205, 46, 58));
            painter.circle_filled(rect.center() + egui::vec2(0.0, 1.0), rect.height() * 0.15, egui::Color32::from_rgb(0, 71, 160));
        }
        "DE" => {
            let h3 = rect.height() / 3.0;
            painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h3)), egui::Rounding { nw: r, ne: r, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(26, 26, 26));
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + h3), egui::vec2(rect.width(), h3)), 0.0, egui::Color32::from_rgb(221, 0, 0));
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + h3 * 2.0), egui::vec2(rect.width(), h3)), egui::Rounding { nw: 0.0, ne: 0.0, sw: r, se: r }, egui::Color32::from_rgb(255, 206, 0));
        }
        "NL" => {
            let h3 = rect.height() / 3.0;
            painter.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h3)), egui::Rounding { nw: r, ne: r, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(174, 28, 40));
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + h3), egui::vec2(rect.width(), h3)), 0.0, egui::Color32::WHITE);
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + h3 * 2.0), egui::vec2(rect.width(), h3)), egui::Rounding { nw: 0.0, ne: 0.0, sw: r, se: r }, egui::Color32::from_rgb(33, 70, 139));
        }
        "UK" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(1, 33, 105));
            let s_w = egui::Stroke::new(2.2_f32, egui::Color32::WHITE);
            let s_r = egui::Stroke::new(1.2_f32, egui::Color32::from_rgb(200, 16, 46));
            painter.line_segment([egui::pos2(rect.left() + 1.0, rect.center().y), egui::pos2(rect.right() - 1.0, rect.center().y)], s_w);
            painter.line_segment([egui::pos2(rect.center().x, rect.top() + 1.0), egui::pos2(rect.center().x, rect.bottom() - 1.0)], s_w);
            painter.line_segment([egui::pos2(rect.left() + 1.0, rect.center().y), egui::pos2(rect.right() - 1.0, rect.center().y)], s_r);
            painter.line_segment([egui::pos2(rect.center().x, rect.top() + 1.0), egui::pos2(rect.center().x, rect.bottom() - 1.0)], s_r);
        }
        "MY" => {
            painter.rect_filled(rect, r, egui::Color32::from_rgb(204, 0, 1));
            let canton = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 0.52, rect.height() * 0.56));
            painter.rect_filled(canton, egui::Rounding { nw: r, ne: 0.0, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(1, 0, 102));
            painter.circle_filled(canton.center(), 1.8, egui::Color32::from_rgb(255, 204, 0));
        }
        _ => {}
    }
}

pub fn md3_switch(ui: &mut egui::Ui, on: bool, pal: &ThemePalette) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(42.0, 23.0), egui::Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let t = ui.ctx().animate_value_with_time(resp.id, if on { 1.0 } else { 0.0 }, 0.22);
    let spring_t = if on { spring_overshoot(t) } else { t };

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let draw_r = rect.shrink(0.5);
        let r = draw_r.height() * 0.5;
        let off_col = if pal.dark {
            egui::Color32::from_rgb(56, 61, 74)
        } else {
            egui::Color32::from_rgb(203, 213, 225)
        };
        let track_col = egui::Color32::from_rgb(
            (off_col.r() as f32 + (pal.primary.r() as f32 - off_col.r() as f32) * t.clamp(0.0, 1.0)) as u8,
            (off_col.g() as f32 + (pal.primary.g() as f32 - off_col.g() as f32) * t.clamp(0.0, 1.0)) as u8,
            (off_col.b() as f32 + (pal.primary.b() as f32 - off_col.b() as f32) * t.clamp(0.0, 1.0)) as u8,
        );
        painter.rect_filled(draw_r, r, track_col);
        let knob_r = r - 2.5;
        let start_x = draw_r.left() + r;
        let end_x = draw_r.right() - r;
        let knob_x = start_x + (end_x - start_x) * spring_t;
        let knob_center = egui::pos2(knob_x, draw_r.center().y);
        painter.circle_filled(
            knob_center + egui::vec2(0.0, 1.0),
            knob_r,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 45),
        );
        painter.circle_filled(knob_center, knob_r, egui::Color32::WHITE);
    }
    resp.clicked()
}

pub fn cupertino_button(
    ui: &mut egui::Ui,
    label: &str,
    icon: Option<CupIcon>,
    style: CupBtnStyle,
    enabled: bool,
    pal: &ThemePalette,
) -> bool {
    let text_galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), cup_font_med(12.0), egui::Color32::WHITE));
    let icon_pad = if icon.is_some() { 16.0 } else { 0.0 };
    let w = (text_galley.size().x + icon_pad + 22.0).max(56.0);
    let h = 30.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), sense);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let id = ui.id().with(label);
    let scale = interact_spring_scale(
        ui.ctx(),
        id,
        enabled && resp.hovered(),
        enabled && resp.is_pointer_button_down_on(),
    );

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let scaled_rect = egui::Rect::from_center_size(rect.center(), rect.size() * scale);
        let draw_r = scaled_rect.shrink(0.5);
        let hovered = enabled && resp.hovered();
        let active = enabled && resp.is_pointer_button_down_on();

        let (bg, fg, stroke) = match style {
            CupBtnStyle::Filled(c) => {
                let fill = if !enabled {
                    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 90)
                } else if active {
                    egui::Color32::from_rgb(c.r().saturating_sub(18), c.g().saturating_sub(18), c.b().saturating_sub(18))
                } else if hovered {
                    egui::Color32::from_rgb(c.r().saturating_add(14), c.g().saturating_add(14), c.b().saturating_add(14))
                } else {
                    c
                };
                let txt_c = if enabled { egui::Color32::WHITE } else { pal.text_muted };
                (fill, txt_c, egui::Stroke::NONE)
            }
            CupBtnStyle::Tinted(c) => {
                let alpha = if !enabled { 16 } else if active { 64 } else if hovered { 42 } else { 26 };
                let fill = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
                let border = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), if hovered { 110 } else { 70 });
                let txt_c = if enabled { c } else { pal.text_muted };
                (fill, txt_c, egui::Stroke::new(1.0_f32, border))
            }
            CupBtnStyle::Secondary => {
                let fill = if !enabled {
                    pal.bg_pill
                } else if active || hovered {
                    pal.bg_pill_hover
                } else {
                    pal.bg_pill
                };
                let border = if hovered { pal.border_hover } else { pal.border };
                let txt_c = if enabled { pal.text_primary } else { pal.text_muted };
                (fill, txt_c, egui::Stroke::new(1.0_f32, border))
            }
        };

        painter.rect(draw_r, 15.0, bg, stroke);

        let content_w = text_galley.size().x + icon_pad;
        let start_x = draw_r.center().x - content_w * 0.5;
        if let Some(ic) = icon {
            let ic_center = egui::pos2(start_x + 5.5, draw_r.center().y);
            draw_cup_icon(painter, ic, ic_center, fg);
        }
        painter.text(
            egui::pos2(start_x + icon_pad, draw_r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            cup_font_med(12.0),
            fg,
        );
    }
    enabled && resp.clicked()
}

