//! 视觉主题：统一配色、中文字体、控件风格
//!
//! egui 自带字体不含中文字形，必须从系统加载（微软雅黑 / 黑体 / 等线），
//! 否则界面会出现豆腐块。

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke,
};

// ── 配色（简洁扁平）─────────────────────────────────────────────
/// 主色（深蓝）
pub const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
/// 主色悬停
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(29, 78, 216);
/// 主文字
pub const TEXT: Color32 = Color32::from_rgb(30, 41, 59);
/// 次要文字
pub const TEXT_MUTED: Color32 = Color32::from_rgb(100, 116, 139);
/// 单位文字
pub const UNIT: Color32 = Color32::from_rgb(148, 163, 184);
/// 背景
pub const BG: Color32 = Color32::from_rgb(248, 250, 252);
/// 卡片背景
pub const CARD: Color32 = Color32::from_rgb(255, 255, 255);
/// 边框
pub const BORDER: Color32 = Color32::from_rgb(226, 232, 240);
/// 输入框背景（可编辑）
pub const INPUT_BG: Color32 = Color32::from_rgb(255, 255, 255);
/// 结果背景（只读）
pub const RESULT_BG: Color32 = Color32::from_rgb(241, 245, 249);
/// 警告色
pub const WARN: Color32 = Color32::from_rgb(217, 119, 6);
/// 错误色
pub const ERR: Color32 = Color32::from_rgb(220, 38, 38);
/// 成功/图表主色
pub const OK: Color32 = Color32::from_rgb(22, 163, 74);

/// 应用主题（配色 + 中文字体）
pub fn apply(ctx: &Context) {
    load_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    style.visuals.dark_mode = false;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = RESULT_BG;
    style.visuals.faint_bg_color = RESULT_BG;
    style.visuals.code_bg_color = RESULT_BG;
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.slider_trailing_fill = true;

    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);
    style.visuals.widgets.noninteractive.weak_bg_fill = RESULT_BG;
    style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);

    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.weak_bg_fill = INPUT_BG;
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(6);

    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(239, 246, 255);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);

    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, ACCENT_HOVER);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(219, 234, 254);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(6);

    style.visuals.window_corner_radius = CornerRadius::same(10);
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(14.0, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.slider_width = 180.0;

    ctx.set_style(style);
}

/// 加载系统中文字体（雅黑 → 黑体 → 等线 → 雅楷）
fn load_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    let candidates = [
        "C:\\Windows\\Fonts\\msyh.ttc",   // 微软雅黑
        "C:\\Windows\\Fonts\\msyhl.ttc",  // 微软雅黑 Light
        "C:\\Windows\\Fonts\\simhei.ttf", // 黑体
        "C:\\Windows\\Fonts\\Deng.ttf",   // 等线
        "C:\\Windows\\Fonts\\simkai.ttf", // 楷体
        "C:\\Windows\\Fonts\\simsun.ttc", // 宋体
    ];

    let mut loaded = false;
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "cjk".to_owned());
            }
            loaded = true;
            break;
        }
    }

    if !loaded {
        // 没有中文字体时至少保持英文可用（界面会有豆腐块，但不至于崩溃）
        log_warn("未找到系统中文字体，界面中文可能无法显示");
    }

    // 楷体（启动画面书法字「鑫光正」专用 family）
    if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\simkai.ttf") {
        fonts.font_data.insert(
            "kai".to_owned(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(FontFamily::Name("kai".into()))
            .or_default()
            .push("kai".to_owned());
    }

    ctx.set_fonts(fonts);
}

/// 字体尺寸（统一管理，避免出现 5 种字号混杂——原 C# 界面问题之一）
pub mod text {
    use super::*;

    pub fn group_header(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(13.5))
            .strong()
            .color(TEXT)
    }
    pub fn label(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(12.5))
            .color(TEXT)
    }
    pub fn unit(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(11.0))
            .color(UNIT)
    }
    pub fn value(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(13.5))
            .strong()
            .color(ACCENT)
    }
    pub fn warn(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(12.0))
            .color(WARN)
    }
    pub fn button(name: &str) -> RichText {
        RichText::new(name).font(FontId::proportional(13.0))
    }
    pub fn caption(name: &str) -> RichText {
        RichText::new(name)
            .font(FontId::proportional(11.0))
            .color(TEXT_MUTED)
    }
}

fn log_warn(msg: &str) {
    eprintln!("[theme] {msg}");
}
