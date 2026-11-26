//! 制热拟合图表：双 Y 轴 + 面积填充 + 圆点标记 + 悬停读数
//!
//! 复刻原 C#（LiveCharts）「制热拟合」图：
//!
//! ```text
//! X  轴：环境温度 −15 … +14 °C
//! 左 轴：温度 24 … 50 °C   —— 房间温度 / 平均水温 / 增加一台机组 / 增加20%末端
//! 右 轴：能力 200 … 650 kW —— 热源能力
//! ```
//!
//! 5 条曲线（与原图例一致）：
//! 1. **热源能力**（红，右侧 kW，浅红填充）`= 曲线(T)·机组数`
//! 2. **房间温度**（蓝，圆点标记，浅紫填充）
//! 3. **平均水温**（深红）
//! 4. **增加一台机组**（黄）
//! 5. **增加20%末端**（灰）
//!
//! 双 Y 轴实现：把 kW 线性映射进温度数据区间绘图，右轴刻度用
//! formatter 反算回 kW（egui_plot 的坐标轴共用一个数据标度）。

use duckcm_core::data::ProjectState;
use duckcm_core::heat::{fit_factor, poly_eval, units_factor, HeatingCurve};
use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};
use egui_plot::{AxisHints, Legend, Line, MarkerShape, Plot, PlotPoints, Points};

use crate::theme::{self, text};

// ── 配色（沿用原图）────────────────────────────────────────────
pub const C_HEAT_SOURCE: Color32 = Color32::from_rgb(244, 63, 94); // 热源能力
pub const C_ROOM: Color32 = Color32::from_rgb(37, 99, 235); // 房间温度
pub const C_WATER: Color32 = Color32::from_rgb(220, 38, 38); // 平均水温
pub const C_PLUS_UNIT: Color32 = Color32::from_rgb(234, 179, 8); // 增加一台机组
pub const C_PLUS_TERM: Color32 = Color32::from_rgb(100, 116, 139); // 增加20%末端

// ── 坐标范围（X 轴固定，Y 轴按数据自适应）────────────────────
const X_MIN: f64 = -20.0;
const X_MAX: f64 = 16.0;

/// 图表数据（纯计算，便于测试）
#[derive(Debug, Clone, Default)]
pub struct ChartSeries {
    /// 环境温度 [°C]
    pub x: Vec<f64>,
    /// 热源能力 [kW]
    pub heat_source_kw: Vec<f64>,
    /// 房间温度 [°C]
    pub room_temp: Vec<f64>,
    /// 平均水温 [°C]
    pub water_temp: Vec<f64>,
    /// 增加一台机组后的房间温度 [°C]
    pub room_plus_unit: Vec<f64>,
    /// 增加 20% 末端后的房间温度 [°C]
    pub room_plus_terminals: Vec<f64>,
    /// 直吹式热源（燃气/燃油热风机）：无水温系列、供热量不含 0.92 热网系数
    pub no_terminal: bool,
}

impl ChartSeries {
    /// 温度系列的取值范围 [min, max]（含少量边距）；
    /// 直吹式无末端水路，水温系列（求解器饱和值）不参与拉程
    pub fn temp_range(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        let water_it: Box<dyn Iterator<Item = &f64>> = if self.no_terminal {
            Box::new(std::iter::empty())
        } else {
            Box::new(self.water_temp.iter().chain(self.room_plus_terminals.iter()))
        };
        for v in self
            .room_temp
            .iter()
            .chain(water_it)
            .chain(self.room_plus_unit.iter())
        {
            if v.is_finite() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
        if !lo.is_finite() || !hi.is_finite() || (hi - lo).abs() < 1.0 {
            return (24.0, 50.0);
        }
        // 下沿贴住数据（不空出一条），上沿留 8 %
        (lo - ((hi - lo) * 0.02).max(0.5), hi + (hi - lo) * 0.08)
    }

    /// 热源能力的取值范围 [min, max]——轴随数据（不强制从 0 起），
    /// 恒功率机型（锅炉/直吹热风）以值 ±8% 定轴，曲线居中可见
    pub fn kw_range(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for v in &self.heat_source_kw {
            if v.is_finite() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            return (0.0, 100.0);
        }
        if hi - lo < 1.0 {
            let c = lo.max(1.0);
            return (c * 0.92, c * 1.08);
        }
        (lo - (hi - lo) * 0.15, hi + (hi - lo) * 0.15)
    }
}

/// kW 值映射到温度数据带的下缘占比（热源能力线保持在温度曲线下方）
const KW_BAND: f64 = 1.0;

/// kW → 温度数据坐标（双轴共用标度，映射到带下 KW_BAND 区）
pub fn kw_to_y(kw: f64, kw_range: (f64, f64), t_range: (f64, f64)) -> f64 {
    let (k0, k1) = kw_range;
    let (t0, t1) = t_range;
    if (k1 - k0).abs() < 1e-9 {
        return t0 + (t1 - t0) * KW_BAND / 2.0;
    }
    t0 + (kw - k0) / (k1 - k0) * (t1 - t0) * KW_BAND
}

/// 温度数据坐标 → kW（右轴刻度反算）
pub fn y_to_kw(y: f64, kw_range: (f64, f64), t_range: (f64, f64)) -> f64 {
    let (k0, k1) = kw_range;
    let (t0, t1) = t_range;
    if (t1 - t0).abs() < 1e-9 {
        return k0;
    }
    k0 + (y - t0) / ((t1 - t0) * KW_BAND) * (k1 - k0)
}

/// 计算 5 条曲线（X 从 −15 到 +14，步长 1）
///
/// 与 C# `RunRoomCal` 的系列一一对应：
/// * `vKT` → 热源能力
/// * `RFWaterTem` → 平均水温
/// * `RFRoomTemp`（基准）→ 房间温度
/// * `RFRoomTemp`（机组数 +1）→ 增加一台机组
/// * `RFRoomTemp`（末端 ×1.2）→ 增加 20% 末端
pub fn compute_series(state: &ProjectState, curve: &HeatingCurve) -> ChartSeries {
    let h = &state.house;
    let hp = &state.heat;
    let base = duckcm_core::house::geometry(h).unwrap_or_default();
    // 直吹式热源（燃气/燃油热风机）：无末端水路、无热网系数
    let no_net = duckcm_core::heat::heater_no_terminal(&hp.heater_model_name);

    // 围护系数（含开口）
    let vent = &state.vent;
    let wk_wall = duckcm_core::house::wall_total_conductance(&base, vent);
    let wk_air = duckcm_core::house::air_total_conductance(&base, vent);

    // 末端总能力 [kW/K]：原 RunRoomCal 的 hEndWK = 末端组数 × 单组能量
    // （WaterEndVol × WaterEnergy_WK，默认 82 × 300 = 24.6 kW/K）
    let terminal_kw = curve.terminal_count * curve.unit_energy_w_per_k / 1000.0;
    let max_water = hp.water_temp;

    // 「增加一台机组」：热源机组供热量 +1 台（物理模型下台数直接影响舍温/水温）
    let fit = fit_factor(curve);
    let net_f = if no_net { 1.0 } else { 0.92 };
    let q_src = |units: f64, x: f64| -> f64 {
        let p = match duckcm_core::heat::find_heater_model(&hp.heater_model_name) {
            Some(m) => m.capacity_kw(x),
            None => poly_eval(&curve.coeff, x),
        };
        p * units * fit * net_f // 机组可用供热 = 单台能力 × 台数 × 拟合 ×（热网 0.92）
    };
    let units_now = units_factor(curve);

    let mut s = ChartSeries::default();
    s.no_terminal = no_net;
    let n = (X_MAX - X_MIN).round() as i32;
    for i in 0..=n {
        let x = X_MIN + i as f64;
        s.x.push(x);
        // 热源能力：选了机组型号 → 用其制热能力曲线（随外温变化）；
        // 否则回退到常数曲线 coeff。总能力 = 单台曲线 × 机组台数 × 拟合系数
        let heat_cap = match duckcm_core::heat::find_heater_model(&state.heat.heater_model_name) {
            Some(m) => m.capacity_kw(x),
            None => poly_eval(&curve.coeff, x),
        };
        s.heat_source_kw
            .push(heat_cap * units_factor(curve) * fit_factor(curve));
        // 热网运行点（heat_network_point）：热源不足时台数↑ → 舍温/水温/末端散热同升，
        // 充足后饱和于末端-围护平衡（= 可达温度）
        let p_base = duckcm_core::heat::heat_network_point(
            x,
            q_src(units_now, x),
            terminal_kw,
            h.roof_height,
            base.wk_floor,
            wk_wall,
            base.wk_roof,
            wk_air,
            max_water,
        );
        s.water_temp.push(p_base.avg_water);
        s.room_temp.push(p_base.room_temp);
        let p_unit = duckcm_core::heat::heat_network_point(
            x,
            q_src(units_now + 1.0, x),
            terminal_kw,
            h.roof_height,
            base.wk_floor,
            wk_wall,
            base.wk_roof,
            wk_air,
            max_water,
        );
        s.room_plus_unit.push(p_unit.room_temp);
        let p_term = duckcm_core::heat::heat_network_point(
            x,
            q_src(units_now, x),
            terminal_kw * 1.2,
            h.roof_height,
            base.wk_floor,
            wk_wall,
            base.wk_roof,
            wk_air,
            max_water,
        );
        s.room_plus_terminals.push(p_term.room_temp);
    }
    s
}

/// 绘制图表（双 Y 轴 + 填充 + 标记 + 悬停读数，默认高度）
///
/// `id` 必须在同帧内唯一（否则 egui 会报 widget ID 冲突）。
#[allow(dead_code)]
pub fn draw(ui: &mut Ui, series: &ChartSeries, id: &str) {
    draw_with_height(ui, series, id, 360.0);
}

/// 同 [`draw`]，但可指定绘图高度
pub fn draw_with_height(ui: &mut Ui, series: &ChartSeries, id: &str, height: f32) {
    if series.x.is_empty() {
        ui.label(text::caption("无数据"));
        return;
    }

    // 动态坐标范围（右轴 kW 映射进温度标度）
    let t_range = series.temp_range();
    let kw_range = series.kw_range();
    let (t0, t1) = t_range;

    // ── 绘图 ──────────────────────────────────────────────
    let mut hovered_x: Option<f64> = None;

    let kw_axis = AxisHints::new_y()
        .placement(egui_plot::HPlacement::Right)
        .label(egui::RichText::new("能力 kW").color(theme::TEXT_MUTED))
        .formatter(move |mark, _range| format!("{:.0}", y_to_kw(mark.value, kw_range, t_range)));

    let response = Plot::new(format!("heat_fit_plot_{id}"))
        .height(height)
        .legend(Legend::default())
        // X 轴标题省略：刻度值即环境温度，避免底部多出一条空白带
        .y_axis_label("温度 °C")
        .y_axis_formatter(move |mark, _range| format!("{:.0}", mark.value))
        .custom_y_axes(vec![AxisHints::new_y().label("温度 °C"), kw_axis])
        .include_x(X_MIN)
        .include_x(X_MAX)
        .include_y(t0)
        .include_y(t1)
        // 坐标轴自动锁定：X 固定 −20…+16，Y 按数据自动定程后不可拖动/缩放
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .show(ui, |plot_ui| {
            hovered_x = plot_ui.pointer_coordinate().map(|p| p.x);

            // 1. 热源能力（右轴，纯线条，不包裹）
            let heat: Vec<[f64; 2]> = series
                .x
                .iter()
                .zip(series.heat_source_kw.iter())
                .map(|(&x, &k)| [x, kw_to_y(k, kw_range, t_range)])
                .collect();
            plot_ui.line(
                Line::new(PlotPoints::new(heat))
                    .name("热源能力")
                    .color(C_HEAT_SOURCE)
                    .width(2.0_f32),
            );

            // 2. 平均水温（深红，向下包裹浅红填充）；直吹式无水路不绘制
            if !series.no_terminal {
                let water: Vec<[f64; 2]> = series
                    .x
                    .iter()
                    .zip(series.water_temp.iter())
                    .map(|(&x, &t)| [x, t])
                    .collect();
                plot_ui.line(
                    Line::new(PlotPoints::new(water))
                        .name("平均水温")
                        .color(C_WATER)
                        .width(2.0_f32)
                        .fill(t0 as f32)
                        .fill_alpha(0.10_f32),
                );
            }

            // 4. 增加一台机组（黄）
            let pu: Vec<[f64; 2]> = series
                .x
                .iter()
                .zip(series.room_plus_unit.iter())
                .map(|(&x, &t)| [x, t])
                .collect();
            plot_ui.line(
                Line::new(PlotPoints::new(pu))
                    .name("增加一台机组")
                    .color(C_PLUS_UNIT)
                    .width(1.8_f32),
            );

            // 5. 增加 20% 末端（灰）；直吹式无末端不绘制
            if !series.no_terminal {
                let pt: Vec<[f64; 2]> = series
                    .x
                    .iter()
                    .zip(series.room_plus_terminals.iter())
                    .map(|(&x, &t)| [x, t])
                    .collect();
                plot_ui.line(
                    Line::new(PlotPoints::new(pt))
                        .name("增加20%末端")
                        .color(C_PLUS_TERM)
                        .width(1.8_f32),
                );
            }

            // 6. 房间温度（蓝，圆点 + 浅蓝填充）——最后绘制，填充压在最上层
            let room: Vec<[f64; 2]> = series
                .x
                .iter()
                .zip(series.room_temp.iter())
                .map(|(&x, &t)| [x, t])
                .collect();
            plot_ui.line(
                Line::new(PlotPoints::new(room.clone()))
                    .name("房间温度")
                    .color(C_ROOM)
                    .width(2.2_f32)
                    .fill(t0 as f32)
                    .fill_alpha(0.14_f32),
            );
            plot_ui.points(
                Points::new(PlotPoints::new(room))
                    .name("房间温度")
                    .color(C_ROOM)
                    .filled(true)
                    .shape(MarkerShape::Circle)
                    .radius(3.5_f32),
            );
        });

    // ── 悬停读数（复刻原图浮动框，紧凑并自动避让）────────────
    // 仅当图表真正被悬停时才显示：被弹窗等更高层遮挡时 hovered() 为 false，
    // 避免读数浮框出现在遮挡窗口之上。
    if response.response.hovered() {
        if let Some(x) = hovered_x {
            if let Some(i) = nearest_index(&series.x, x) {
                let pos = response.response.hover_pos().unwrap_or_else(|| {
                    response
                        .transform
                        .position_from_point(&egui_plot::PlotPoint::new(
                            series.x[i],
                            series.room_temp[i],
                        ))
                });
                draw_tooltip(ui, pos, response.response.rect, series, i, id);
            }
        }
    }
}

/// 最近的 x 索引
fn nearest_index(xs: &[f64], x: f64) -> Option<usize> {
    if xs.is_empty() {
        return None;
    }
    let mut best = 0usize;
    let mut best_d = f64::INFINITY;
    for (i, &v) in xs.iter().enumerate() {
        let d = (v - x).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    Some(best)
}

/// 悬停浮框（紧凑固定宽，越界自动翻到鼠标左侧/收进绘图区）
fn draw_tooltip(ui: &mut Ui, cursor: Pos2, bounds: Rect, s: &ChartSeries, i: usize, id: &str) {
    const W: f32 = 206.0;
    const H: f32 = 126.0;

    // 默认鼠标右下；右侧越界翻到左侧；底部越界上移；整体夹在绘图区内
    let mut x = cursor.x + 16.0;
    let mut y = cursor.y - 14.0;
    if x + W > bounds.right() - 2.0 {
        x = cursor.x - W - 16.0;
    }
    if y + H > bounds.bottom() - 2.0 {
        y = cursor.y - H - 10.0;
    }
    x = x.clamp(
        bounds.left() + 2.0,
        (bounds.right() - W - 2.0).max(bounds.left() + 2.0),
    );
    y = y.clamp(
        bounds.top() + 2.0,
        (bounds.bottom() - H - 2.0).max(bounds.top() + 2.0),
    );

    egui::Area::new(egui::Id::new(("chart_tooltip", id)))
        .fixed_pos(Pos2::new(x, y))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(W);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 242))
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_max_width(W - 16.0);
                    ui.label(
                        egui::RichText::new(format!("环境温度 {:.0} °C", s.x[i]))
                            .strong()
                            .size(11.5_f32)
                            .color(theme::TEXT),
                    );
                    ui.add_space(2.0);
                    egui::Grid::new(("tip_grid", id))
                        .num_columns(3)
                        .spacing([6.0, 1.0])
                        .show(ui, |ui| {
                            tooltip_row(ui, "热源能力", C_HEAT_SOURCE, s.heat_source_kw[i], "kW");
                            tooltip_row(ui, "房间温度", C_ROOM, s.room_temp[i], "°C");
                            if !s.no_terminal {
                                tooltip_row(ui, "平均水温", C_WATER, s.water_temp[i], "°C");
                            }
                            tooltip_row(ui, "增加一台机组", C_PLUS_UNIT, s.room_plus_unit[i], "°C");
                            if !s.no_terminal {
                                tooltip_row(
                                    ui,
                                    "增加20%末端",
                                    C_PLUS_TERM,
                                    s.room_plus_terminals[i],
                                    "°C",
                                );
                            }
                        });
                });
        });
}

/// 浮框行：色点 | 名称（定宽） | 数值 单位
fn tooltip_row(ui: &mut Ui, name: &str, color: Color32, value: f64, unit: &str) {
    // 色点
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 14.0), egui::Sense::hover());
    ui.painter_at(rect).circle_filled(rect.center(), 4.5, color);

    // 名称列加宽以容纳「增加20%末端」等长名称
    ui.add_sized(
        [84.0, 14.0],
        egui::Label::new(text::caption(name)).truncate(),
    );

    // 数值 + 单位
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.label(
            egui::RichText::new(format!("{value:.3}"))
                .size(11.0_f32)
                .strong()
                .color(theme::TEXT),
        );
        ui.label(text::unit(unit));
    });

    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kw_mapping_round_trips() {
        let kw_range = (200.0, 650.0);
        let t_range = (24.0, 50.0);
        for kw in [200.0, 378.32, 425.0, 650.0] {
            let y = kw_to_y(kw, kw_range, t_range);
            let back = y_to_kw(y, kw_range, t_range);
            assert!((back - kw).abs() < 1e-9, "kw={kw} back={back}");
        }
        assert!((kw_to_y(200.0, kw_range, t_range) - 24.0).abs() < 1e-9);
        // 全带映射（KW_BAND=1）：kW 上端 = 右轴最大值 = 轴顶 50
        assert!((kw_to_y(650.0, kw_range, t_range) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn dynamic_ranges_track_data() {
        let state = ProjectState::default();
        let curve = HeatingCurve {
            coeff: vec![300.0, 10.0],
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 1.0,
        };
        let s = compute_series(&state, &curve);
        let (t0, t1) = s.temp_range();
        assert!(t1 > t0);
        // 所有温度值应落在范围内
        for v in s.room_temp.iter().chain(s.water_temp.iter()) {
            if v.is_finite() {
                assert!(*v >= t0 && *v <= t1, "v={v} range=({t0},{t1})");
            }
        }
        let (k0, k1) = s.kw_range();
        assert!(k1 > k0);
        // 热源能力映射后也应落在温度范围内
        for v in &s.heat_source_kw {
            let y = kw_to_y(*v, (k0, k1), (t0, t1));
            assert!(y >= t0 - 1e-6 && y <= t1 + 1e-6, "y={y}");
        }
    }

    #[test]
    fn series_covers_full_x_range() {
        // 常数曲线回退路径：清空默认机组型号
        let mut state = ProjectState::default();
        state.heat.heater_model_name = String::new();
        let curve = HeatingCurve {
            coeff: vec![300.0, 10.0],
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 1.0,
        };
        let s = compute_series(&state, &curve);
        assert_eq!(s.x.len(), 37);
        assert_eq!(s.x[0], -20.0);
        assert_eq!(*s.x.last().unwrap(), 16.0);
        assert_eq!(s.room_temp.len(), 37);
        // 热源能力 = 曲线(T)·机组数·拟合（T = X_MIN = -20 → 300 − 200 = 100）
        assert!((s.heat_source_kw[0] - (300.0 - 10.0 * 20.0)).abs() < 1e-9);
    }

    #[test]
    fn series_uses_heater_model_curve_by_default() {
        // 默认状态（ZGH-170IADG2）：−20°C 热源能力 = 85.6 × 1 台 × fit
        let state = ProjectState::default();
        let curve = HeatingCurve {
            coeff: duckcm_core::heat::find_heater_model("ZGH-170IADG2")
                .unwrap()
                .poly_coeffs(),
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 1.0,
        };
        let s = compute_series(&state, &curve);
        assert!((s.heat_source_kw[0] - 85.6).abs() < 1e-6);
        // 三次曲线平滑单调上升
        for w in s.heat_source_kw.windows(2) {
            assert!(w[1] >= w[0] - 1e-6, "heat curve not monotonic");
        }
        // 高温端封顶 1.15×170
        assert!(*s.heat_source_kw.last().unwrap() <= 170.0 * 1.15 + 1e-6);
    }

    #[test]
    fn adding_unit_raises_room_temp() {
        let state = ProjectState::default();
        let curve = HeatingCurve {
            coeff: vec![300.0, 10.0],
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 1.0,
        };
        let s = compute_series(&state, &curve);
        for i in 0..s.x.len() {
            if s.room_temp[i].is_finite() && s.room_plus_unit[i].is_finite() {
                assert!(
                    s.room_plus_unit[i] >= s.room_temp[i] - 1e-6,
                    "i={i}: {} vs {}",
                    s.room_plus_unit[i],
                    s.room_temp[i]
                );
            }
        }
    }

    #[test]
    fn nearest_index_picks_closest() {
        let xs = vec![-15.0, -14.0, -13.0];
        assert_eq!(nearest_index(&xs, -14.2), Some(1));
        assert_eq!(nearest_index(&xs, 999.0), Some(2));
    }
}
