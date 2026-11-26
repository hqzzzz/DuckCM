//! DuckCM 2 —— 养殖用房屋设备自动化计算系统（Rust / egui 版）
//!
//! 界面原则：简洁、美观、高效
//! - 单一主窗口：左侧参数（分组卡片），右侧结果 + 曲线
//! - 输入即算（无需点「刷新」），计算在后台线程之外同步完成（纯函数，微秒级）
//! - 统一配色与字号，输入/结果视觉区分，单位统一放在行尾

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use duckcm_core::data::ProjectState;
use duckcm_core::{compute_all, heat::HeatingCurve, CalcOutcome};

mod app;
mod chart;
mod material_ui;
mod section;
mod theme;
mod updater;

fn main() -> eframe::Result {
    // 窗口/任务栏图标：blur.ico（最大尺寸帧 → RGBA）
    let icon = image::load_from_memory(include_bytes!("../resources/blur.ico"))
        .ok()
        .map(|img| {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            std::sync::Arc::new(egui::IconData {
                rgba: img.into_raw(),
                width: w,
                height: h,
            })
        });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1080.0, 700.0])
            .with_title("DuckCM 2 · 养殖用房屋设备自动化计算系统")
            .with_icon(icon.unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "DuckCM 2",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::DuckApp::new()))
        }),
    )
}

/// 每次输入变化后重算（放在 app 内部调用）
pub fn recalculate(state: &ProjectState, curve: Option<&HeatingCurve>) -> CalcOutcome {
    compute_all(state, curve)
}
